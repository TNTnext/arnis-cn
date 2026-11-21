//! Monthly Overture Maps detail: roads, railways and named places.
//!
//! The OpenStreetMap extracts remain the semantic base map. This layer reads
//! only the z14 tiles that the requested area touches from Overture's monthly
//! `transportation` and `places` archives. It is deliberately additive:
//! OSM-sourced transport is skipped and OSM tags on existing places are never
//! overwritten. Places inside an OSM building add their name and category to
//! that building, which also gives Arnis signage a place to draw them.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use colored::Colorize;
use rayon::prelude::*;
use reqwest::blocking::Client;
use serde_json::Value as JsonValue;

use super::pmtiles::{self, Archive};
use super::{cache, mvt, release_candidates, OsmRef};
use crate::clipping::clip_way_to_bbox;
use crate::coordinate_system::cartesian::XZPoint;
use crate::coordinate_system::geographic::LLBBox;
use crate::osm_parser::{
    point_in_ring, ProcessedElement, ProcessedMemberRole, ProcessedNode, ProcessedWay,
};
use crate::projection::ProjectionSpec;

const OVERTURE_TILES_BUCKET: &str =
    "https://overturemaps-extras-us-west-2.s3.us-west-2.amazonaws.com";
const QUERY_ZOOM: u8 = 14;
const MAX_TILES: u64 = 4096;
const TILE_BATCH: usize = 48;
const FETCH_THREADS_MAX: usize = 8;
const MIN_PLACE_CONFIDENCE: f64 = 0.55;
const OVERTURE_ID_HIGH_BIT: u64 = 0x8000_0000_0000_0000;

const TRANSPORTATION_THEME: &str = "transportation";
const PLACES_THEME: &str = "places";
const TRANSPORTATION_LAYER: &str = "segment";
const PLACES_LAYER: &str = "place";

/// What the monthly detail pass contributes.
#[derive(Default)]
pub struct OvertureDetailData {
    /// New transport ways and standalone named places, ready to merge.
    pub elements: Vec<ProcessedElement>,
    pub places: usize,
}

/// Fetch the monthly transport and place layer. Failure is non-fatal: the
/// caller keeps the OSM base map and receives empty detail data.
pub fn fetch_overture_detail(
    bbox: &LLBBox,
    projection: &ProjectionSpec,
    debug: bool,
) -> OvertureDetailData {
    match fetch_inner(bbox, projection, debug) {
        Ok(data) => data,
        Err(error) => {
            crate::net::note_uncached("Overture Maps roads and places");
            eprintln!(
                "{} Failed to fetch Overture roads and places: {error}",
                "Warning:".yellow().bold()
            );
            OvertureDetailData::default()
        }
    }
}

fn fetch_inner(
    bbox: &LLBBox,
    projection: &ProjectionSpec,
    debug: bool,
) -> Result<OvertureDetailData, Box<dyn std::error::Error>> {
    if pmtiles::tile_count(bbox, QUERY_ZOOM) > MAX_TILES {
        return Err(format!(
            "the area needs {} z{QUERY_ZOOM} tiles; detailed data supports at most {MAX_TILES}",
            pmtiles::tile_count(bbox, QUERY_ZOOM)
        )
        .into());
    }
    let client = super::overture_client()?;
    let releases = release_candidates(&client, debug);
    if releases.is_empty() {
        return Err("no Overture release candidates".into());
    }

    let (release, transport, places) = read_detail_with_fallback(&client, bbox, &releases, debug)?;
    cache::set_last_good_release(&release);

    let (coord_transformer, xzbbox) = projection.transformer(bbox)?;
    let clip_bbox = projection.clip_bbox(&xzbbox);
    let mut elements = Vec::new();
    let mut roads = 0usize;
    let mut rails = 0usize;
    let mut seen_features = HashSet::new();
    for feature in transport {
        if !seen_features.insert(feature.id.clone()) {
            continue;
        }
        let (tags, is_rail) = transport_tags(&feature);
        if tags.is_empty() {
            continue;
        }
        for (part, line) in feature.lines.into_iter().enumerate() {
            if line.len() < 2 {
                continue;
            }
            let nodes: Vec<ProcessedNode> = line
                .into_iter()
                .map(|(lng, lat)| {
                    let ll = crate::coordinate_system::geographic::LLPoint::new(lat, lng)
                        .expect("Overture tile coordinates are within the latitude domain");
                    let xz = coord_transformer.transform_point(ll);
                    ProcessedNode {
                        id: stable_node_id(&feature.id, part, xz),
                        tags: HashMap::new(),
                        x: xz.x,
                        z: xz.z,
                    }
                })
                .collect();
            let clipped = clip_way_to_bbox(&nodes, &clip_bbox);
            if clipped.len() < 2 {
                continue;
            }
            elements.push(ProcessedElement::Way(ProcessedWay {
                id: stable_way_id(&feature.id, part),
                nodes: clipped,
                tags: tags.clone(),
            }));
            if is_rail {
                rails += 1;
            } else {
                roads += 1;
            }
        }
    }

    let mut places_read = 0usize;
    let mut seen = HashSet::new();
    for place in places {
        if !seen.insert(place.id.clone()) {
            continue;
        }
        let Some((lng, lat)) = place.point else {
            continue;
        };
        let ll = crate::coordinate_system::geographic::LLPoint::new(lat, lng)?;
        let xz = coord_transformer.transform_point(ll);
        if !xzbbox.contains(&xz) {
            continue;
        }
        places_read += 1;
        elements.push(ProcessedElement::Node(ProcessedNode {
            id: stable_place_id(&place.id),
            tags: place.tags,
            x: xz.x,
            z: xz.z,
        }));
    }

    if debug {
        println!(
            "Overture detail ({release}): {roads} road segment(s), \
             {rails} rail segment(s), {places_read} named place(s)"
        );
    }
    Ok(OvertureDetailData {
        elements,
        places: places_read,
    })
}

fn read_detail_with_fallback(
    client: &Client,
    bbox: &LLBBox,
    releases: &[String],
    debug: bool,
) -> Result<(String, Vec<RawFeature>, Vec<RawFeature>), Box<dyn std::error::Error>> {
    let mut last_error = "no readable transportation and places archives".to_string();
    let mut partial = None;
    let mut ordered = releases.to_vec();
    // Bad discovery should not pin a monthly source to the release that
    // happened to work last time. Offline runs keep the supplied order so the
    // cached release is opened before an unavailable newer one.
    if !crate::net::offline() {
        ordered.sort_by(|a, b| b.cmp(a));
    }
    for release in &ordered {
        let transport = read_theme(client, bbox, release, TRANSPORTATION_THEME, debug);
        let places = read_theme(client, bbox, release, PLACES_THEME, debug);
        match (transport, places) {
            (Ok(transport), Ok(places)) => return Ok((release.clone(), transport, places)),
            (transport, places) => {
                let error = match (transport.as_ref().err(), places.as_ref().err()) {
                    (Some(transport), Some(places)) => {
                        format!("transportation: {transport}; places: {places}")
                    }
                    (Some(transport), None) => format!("transportation: {transport}"),
                    (None, Some(places)) => format!("places: {places}"),
                    (None, None) => unreachable!("one of the two reads failed"),
                };
                last_error = format!("{release}: {error}");
                if debug {
                    eprintln!("Overture detail unavailable for {release}: {error}");
                }
                // A temporary outage of only one planet archive should not throw
                // away usable roads or places from the other theme.
                if partial.is_none() {
                    partial = Some((release.clone(), transport, places, error));
                }
            }
        }
    }
    if let Some((release, transport, places, error)) = partial {
        eprintln!(
            "{} Overture detail is partial for {release}: {error}",
            "Warning:".yellow().bold()
        );
        return Ok((
            release,
            transport.unwrap_or_default(),
            places.unwrap_or_default(),
        ));
    }
    Err(last_error.into())
}

fn read_theme(
    client: &Client,
    bbox: &LLBBox,
    release: &str,
    theme: &str,
    debug: bool,
) -> Result<Vec<RawFeature>, String> {
    let (min_x, max_x, min_y, max_y) = pmtiles::tile_range(bbox, QUERY_ZOOM);
    let mut located = Vec::new();
    let mut archive = Archive::open(
        client,
        &format!("{OVERTURE_TILES_BUCKET}/tiles/{release}/{theme}.pmtiles"),
        archive_cache_dir(release, theme),
    )?;
    let (min_zoom, max_zoom) = (archive.header().min_zoom, archive.header().max_zoom);
    if !(min_zoom..=max_zoom).contains(&QUERY_ZOOM) {
        return Err(format!(
            "archive covers zoom {min_zoom}..={max_zoom}, not z{QUERY_ZOOM}"
        ));
    }
    for x in min_x..=max_x {
        for y in min_y..=max_y {
            if let Some(location) = archive.locate(client, QUERY_ZOOM, x, y)? {
                located.push((x, y, location));
            }
        }
    }
    if debug {
        println!("Overture {theme}: {} z14 tile(s) hold data", located.len());
    }
    if located.is_empty() {
        return Ok(Vec::new());
    }

    let pool = fetch_pool()?;
    let archive = &archive;
    let mut features = Vec::new();
    let mut lost = 0usize;
    for batch in located.chunks(TILE_BATCH) {
        let results: Vec<Result<Vec<RawFeature>, String>> = pool.install(|| {
            batch
                .par_iter()
                .map(|&(x, y, location)| {
                    let raw = archive.tile(client, QUERY_ZOOM, x, y, location)?;
                    if raw.is_empty() {
                        return Ok(Vec::new());
                    }
                    parse_tile(mvt::decode_tile(&raw)?, theme, x, y)
                })
                .collect()
        });
        for result in results {
            match result {
                Ok(mut tile) => features.append(&mut tile),
                Err(error) => {
                    lost += 1;
                    if debug {
                        eprintln!("Warning: Overture {theme} tile could not be read: {error}");
                    }
                }
            }
        }
    }
    if lost == located.len() {
        return Err(format!(
            "all {} Overture {theme} data tile(s) failed",
            located.len()
        ));
    }
    if lost > 0 {
        eprintln!(
            "{} Overture {theme} data is incomplete: {lost} tile(s) could not be read.",
            "Warning:".yellow().bold()
        );
    }
    Ok(features)
}

fn archive_cache_dir(release: &str, theme: &str) -> Option<PathBuf> {
    cache::release_dir(release).map(|dir| dir.join("tiles").join(theme))
}

fn fetch_pool() -> Result<&'static rayon::ThreadPool, String> {
    static POOL: OnceLock<Result<rayon::ThreadPool, String>> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(
                std::thread::available_parallelism()
                    .map(std::num::NonZeroUsize::get)
                    .unwrap_or(2)
                    .clamp(2, FETCH_THREADS_MAX),
            )
            .thread_name(|i| format!("overture-detail-{i}"))
            .build()
            .map_err(|e| format!("could not start the Overture detail pool: {e}"))
    })
    .as_ref()
    .map_err(String::clone)
}

struct RawFeature {
    id: String,
    tags: HashMap<String, String>,
    lines: Vec<Vec<(f64, f64)>>,
    point: Option<(f64, f64)>,
}

fn parse_tile(
    layers: Vec<mvt::Layer>,
    theme: &str,
    tile_x: u32,
    tile_y: u32,
) -> Result<Vec<RawFeature>, String> {
    let wanted = if theme == TRANSPORTATION_THEME {
        TRANSPORTATION_LAYER
    } else {
        PLACES_LAYER
    };
    let mut out = Vec::new();
    for layer in layers.iter().filter(|layer| layer.name == wanted) {
        for feature in &layer.features {
            let Some(id) = layer.attr_str(feature, "id") else {
                continue;
            };
            let extent = f64::from(layer.extent.max(1));
            let coords = |points: &[(i32, i32)]| -> Vec<(f64, f64)> {
                points
                    .iter()
                    .map(|&(x, y)| {
                        (
                            pmtiles::tile_x_to_lon(QUERY_ZOOM, tile_x, f64::from(x) / extent),
                            pmtiles::tile_y_to_lat(QUERY_ZOOM, tile_y, f64::from(y) / extent),
                        )
                    })
                    .collect()
            };
            let sources = layer.attr_str(feature, "sources");
            let refs = sources.map(parse_osm_refs).unwrap_or_default();
            if theme == TRANSPORTATION_THEME {
                if feature.geom_type != mvt::GEOM_LINESTRING {
                    continue;
                }
                let mut tags = transport_attributes(layer, feature);
                if !tags.is_empty() && !refs.is_empty() {
                    let encoded: Vec<String> = refs
                        .iter()
                        .map(|reference| {
                            format!(
                                "{}{}",
                                match reference.kind {
                                    "way" => "w",
                                    "relation" => "r",
                                    _ => "n",
                                },
                                reference.id
                            )
                        })
                        .collect();
                    tags.insert("overture:osm_ref".into(), encoded.join(","));
                }
                out.push(RawFeature {
                    id: id.to_string(),
                    tags,
                    lines: feature
                        .rings
                        .iter()
                        .map(|ring| coords(&ring.points))
                        .collect(),
                    point: None,
                });
            } else if feature.geom_type == mvt::GEOM_POINT {
                let Some(first) = feature.rings.first().and_then(|ring| ring.points.first()) else {
                    continue;
                };
                let tags = place_attributes(layer, feature);
                if tags.is_empty() {
                    continue;
                }
                out.push(RawFeature {
                    id: id.to_string(),
                    tags,
                    lines: Vec::new(),
                    point: coords(&[*first]).into_iter().next(),
                });
            }
        }
    }
    Ok(out)
}

fn transport_attributes(layer: &mvt::Layer, feature: &mvt::Feature) -> HashMap<String, String> {
    let subtype = layer.attr_str(feature, "subtype").unwrap_or_default();
    let class = layer.attr_str(feature, "class").unwrap_or("unknown");
    if layer
        .attr(feature, "is_under_construction")
        .is_some_and(|value| {
            matches!(value, mvt::Value::Bool(true))
                || value
                    .as_str()
                    .is_some_and(|text| text.eq_ignore_ascii_case("true"))
        })
    {
        return HashMap::new();
    }
    let mut tags = HashMap::new();
    if subtype == "road" {
        tags.insert("highway".into(), road_class(class).into());
    } else if subtype == "rail" {
        tags.insert("railway".into(), rail_class(class).into());
    } else {
        return tags;
    }
    tags.insert("overture:subtype".into(), subtype.to_string());
    if let Some(name) = layer.attr_str(feature, "names").and_then(primary_name) {
        tags.insert("name".into(), name);
    }
    if let Some(surface) = layer
        .attr_str(feature, "road_surface")
        .and_then(array_value)
    {
        tags.insert("surface".into(), surface);
    }
    if let Some(flags) = layer.attr_str(feature, "road_flags") {
        if flag_is_set(flags, "is_bridge") {
            tags.insert("bridge".into(), "yes".into());
        }
        if flag_is_set(flags, "is_tunnel") {
            tags.insert("tunnel".into(), "yes".into());
        }
    }
    if let Some(reference) = layer.attr_str(feature, "routes").and_then(first_ref) {
        tags.insert("ref".into(), reference);
    }
    tags.insert("source".into(), "overture_maps".into());
    tags
}

fn transport_tags(feature: &RawFeature) -> (HashMap<String, String>, bool) {
    let is_rail = feature.tags.contains_key("railway");
    (feature.tags.clone(), is_rail)
}

fn road_class(class: &str) -> &'static str {
    match class {
        "motorway" => "motorway",
        "motorway_link" => "motorway_link",
        "trunk" => "trunk",
        "trunk_link" => "trunk_link",
        "primary" => "primary",
        "primary_link" => "primary_link",
        "secondary" => "secondary",
        "secondary_link" => "secondary_link",
        "tertiary" => "tertiary",
        "tertiary_link" => "tertiary_link",
        "residential" => "residential",
        "living_street" => "living_street",
        "service" => "service",
        "pedestrian" => "pedestrian",
        "footway" | "sidewalk" => "footway",
        "path" => "path",
        "cycleway" => "cycleway",
        "bridleway" => "bridleway",
        "steps" => "steps",
        "track" => "track",
        "raceway" => "raceway",
        "busway" => "busway",
        _ => "unclassified",
    }
}

fn rail_class(class: &str) -> &'static str {
    match class {
        "subway" => "subway",
        "tram" => "tram",
        "light_rail" => "light_rail",
        "narrow_gauge" => "narrow_gauge",
        "funicular" => "funicular",
        "monorail" => "monorail",
        _ => "rail",
    }
}

fn place_attributes(layer: &mvt::Layer, feature: &mvt::Feature) -> HashMap<String, String> {
    if layer
        .attr_f64(feature, "confidence")
        .is_some_and(|confidence| confidence < MIN_PLACE_CONFIDENCE)
    {
        return HashMap::new();
    }
    let Some(name) = layer.attr_str(feature, "names").and_then(primary_name) else {
        return HashMap::new();
    };
    let category = layer
        .attr_str(feature, "basic_category")
        .map(str::to_owned)
        .or_else(|| {
            layer
                .attr_str(feature, "taxonomy")
                .and_then(category_from_taxonomy)
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut tags = place_category_tags(&category);
    tags.insert("name".into(), name);
    if !category.is_empty() {
        tags.insert("overture:category".into(), category);
    }
    if let Some(brand) = layer.attr_str(feature, "brand").and_then(primary_name) {
        tags.entry("brand".into()).or_insert(brand);
    }
    if let Some(website) = layer.attr_str(feature, "websites").and_then(array_value) {
        tags.insert("website".into(), website);
    }
    if let Some(phone) = layer.attr_str(feature, "phones").and_then(array_value) {
        tags.insert("phone".into(), phone);
    }
    if let Some(email) = layer.attr_str(feature, "emails").and_then(array_value) {
        tags.insert("email".into(), email);
    }
    if let Some(address) = layer.attr_str(feature, "addresses").and_then(first_address) {
        tags.extend(address);
    }
    if let Some(confidence) = layer.attr_f64(feature, "confidence") {
        tags.insert("overture:confidence".into(), format!("{confidence:.2}"));
    }
    tags.insert("source".into(), "overture_maps".into());
    tags
}

fn place_category_tags(category: &str) -> HashMap<String, String> {
    let mut tags = HashMap::new();
    let mapped = match category {
        "restaurant" | "food_court" => Some(("amenity", "restaurant")),
        "cafe" | "coffee_shop" => Some(("amenity", "cafe")),
        "fast_food" => Some(("amenity", "fast_food")),
        "bar" => Some(("amenity", "bar")),
        "pub" => Some(("amenity", "pub")),
        "ice_cream" => Some(("amenity", "ice_cream")),
        "bakery" => Some(("shop", "bakery")),
        "supermarket" | "grocery_store" => Some(("shop", "supermarket")),
        "convenience_store" => Some(("shop", "convenience")),
        "department_store" | "shopping_mall" => Some(("shop", "department_store")),
        "clothing_store" => Some(("shop", "clothes")),
        "electronics_store" => Some(("shop", "electronics")),
        "furniture_store" => Some(("shop", "furniture")),
        "hardware_store" => Some(("shop", "hardware")),
        "home_goods_store" => Some(("shop", "houseware")),
        "sporting_goods_store" => Some(("shop", "sports")),
        "book_store" | "bookstore" => Some(("shop", "books")),
        "florist" => Some(("shop", "florist")),
        "pharmacy" | "drugstore" => Some(("amenity", "pharmacy")),
        "hospital" => Some(("amenity", "hospital")),
        "clinic" | "doctor" => Some(("amenity", "clinic")),
        "dentist" => Some(("amenity", "dentist")),
        "bank" => Some(("amenity", "bank")),
        "atm" => Some(("amenity", "atm")),
        "fuel" | "gas_station" => Some(("amenity", "fuel")),
        "parking" | "parking_lot" => Some(("amenity", "parking")),
        "school" | "elementary_school" | "middle_school" | "high_school" => {
            Some(("amenity", "school"))
        }
        "university" | "college" => Some(("amenity", "university")),
        "kindergarten" | "preschool" => Some(("amenity", "kindergarten")),
        "library" => Some(("amenity", "library")),
        "police" | "police_station" => Some(("amenity", "police")),
        "fire_station" => Some(("amenity", "fire_station")),
        "post_office" => Some(("amenity", "post_office")),
        "town_hall" | "city_hall" => Some(("amenity", "townhall")),
        "place_of_worship" => Some(("amenity", "place_of_worship")),
        "museum" => Some(("tourism", "museum")),
        "observatory" => Some(("man_made", "observatory")),
        "planetarium" => Some(("tourism", "planetarium")),
        "movie_theater" | "theater" => Some(("amenity", "theatre")),
        "hotel" | "motel" | "hostel" => Some(("tourism", "hotel")),
        "park" => Some(("leisure", "park")),
        "gym" | "fitness_center" => Some(("leisure", "fitness_centre")),
        "bus_station" => Some(("amenity", "bus_station")),
        "bus_stop" => Some(("highway", "bus_stop")),
        "train_station" | "railway_station" => Some(("railway", "station")),
        "subway_station" => Some(("railway", "station")),
        "airport" => Some(("aeroway", "aerodrome")),
        _ => None,
    };
    if let Some((key, value)) = mapped {
        tags.insert(key.into(), value.into());
    } else if category.contains("store") || category.contains("shop") {
        let value = category
            .trim_end_matches("_store")
            .trim_end_matches("store")
            .trim_matches('_');
        if !value.is_empty() {
            tags.insert("shop".into(), value.to_string());
        }
    }
    tags
}

/// Merge detail after buildings, so named places can land on every footprint.
pub fn merge_detail(parsed_elements: &mut Vec<ProcessedElement>, detail: OvertureDetailData) {
    let OvertureDetailData { elements, places } = detail;
    let mut existing_refs = HashSet::new();
    for element in parsed_elements.iter() {
        existing_refs.insert((element.kind(), element.id()));
    }
    let mut road_index = RoadIndex::new(parsed_elements);
    let mut roads_added = 0usize;
    let mut rails_added = 0usize;

    let mut place_nodes = Vec::new();
    for element in elements {
        match element {
            ProcessedElement::Way(way) => {
                if existing_refs.contains(&("way", way.id)) {
                    continue;
                }
                if way
                    .tags
                    .get("overture:osm_ref")
                    .is_some_and(|refs| osm_refs_exist(refs, &existing_refs))
                {
                    continue;
                }
                if way.tags.contains_key("railway") {
                    if road_index.is_duplicate(&way.nodes, 2.5) {
                        continue;
                    }
                    rails_added += 1;
                } else if way.tags.contains_key("highway") {
                    if road_index.is_duplicate(&way.nodes, 2.5) {
                        continue;
                    }
                    roads_added += 1;
                } else {
                    continue;
                }
                existing_refs.insert(("way", way.id));
                road_index.insert_nodes(&way.nodes);
                parsed_elements.push(ProcessedElement::Way(way));
            }
            ProcessedElement::Node(node) => place_nodes.push(ProcessedElement::Node(node)),
            ProcessedElement::Relation(_) => {}
        }
    }

    let (placed, standalone) = add_place_labels(parsed_elements, place_nodes);
    if roads_added > 0 || rails_added > 0 || placed > 0 || standalone > 0 {
        println!(
            "  Added {} roads, {} railways, {} place labels on buildings and {} standalone places",
            roads_added.to_string().bright_white().bold(),
            rails_added.to_string().bright_white().bold(),
            placed.to_string().bright_white().bold(),
            standalone.to_string().bright_white().bold()
        );
    }
    let _ = places;
}

fn add_place_labels(
    parsed_elements: &mut Vec<ProcessedElement>,
    detail_elements: Vec<ProcessedElement>,
) -> (usize, usize) {
    let mut places: Vec<ProcessedNode> = detail_elements
        .into_iter()
        .filter_map(|element| match element {
            ProcessedElement::Node(node) if node.tags.contains_key("name") => Some(node),
            _ => None,
        })
        .collect();
    if places.is_empty() {
        return (0, 0);
    }
    places.sort_by(|a, b| a.id.cmp(&b.id));
    let shapes = BuildingShapes::new(parsed_elements);
    let mut added_nodes = Vec::new();
    let mut placed = 0usize;
    let mut existing_places = existing_named_nodes(parsed_elements);
    let mut standalone = 0usize;
    for mut place in places {
        if let Some(building) = shapes.smallest_containing(place.x, place.z) {
            let tags = parsed_elements[building].tags_mut();
            let mut changed = false;
            for (key, value) in place.tags.drain() {
                if !tags.contains_key(&key) {
                    tags.insert(key, value);
                    changed = true;
                }
            }
            if changed {
                placed += 1;
            }
            continue;
        }
        let name = place
            .tags
            .get("name")
            .map(String::as_str)
            .unwrap_or_default();
        if existing_places.near_name(name, place.x, place.z) {
            continue;
        }
        existing_places.insert(name, place.x, place.z);
        added_nodes.push(ProcessedElement::Node(place));
        standalone += 1;
    }
    parsed_elements.extend(added_nodes);
    (placed, standalone)
}

trait TagsMut {
    fn tags_mut(&mut self) -> &mut HashMap<String, String>;
}

impl TagsMut for ProcessedElement {
    fn tags_mut(&mut self) -> &mut HashMap<String, String> {
        match self {
            ProcessedElement::Node(node) => &mut node.tags,
            ProcessedElement::Way(way) => &mut way.tags,
            ProcessedElement::Relation(relation) => &mut relation.tags,
        }
    }
}

struct BuildingShape {
    element: usize,
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
    ring: Vec<(f64, f64)>,
    area2: i64,
}

struct BuildingShapes {
    shapes: Vec<BuildingShape>,
    cells: HashMap<(i32, i32), Vec<usize>>,
}

impl BuildingShapes {
    fn new(elements: &[ProcessedElement]) -> Self {
        const CELL: i32 = 64;
        let mut shapes = Vec::new();
        let mut push = |element: usize, ring: &[ProcessedNode]| {
            if ring.len() < 3 {
                return;
            }
            let mut min_x = i32::MAX;
            let mut max_x = i32::MIN;
            let mut min_z = i32::MAX;
            let mut max_z = i32::MIN;
            let mut area2 = 0i64;
            for index in 0..ring.len() {
                let a = &ring[index];
                let b = &ring[(index + 1) % ring.len()];
                min_x = min_x.min(a.x);
                max_x = max_x.max(a.x);
                min_z = min_z.min(a.z);
                max_z = max_z.max(a.z);
                area2 += i64::from(a.x) * i64::from(b.z) - i64::from(b.x) * i64::from(a.z);
            }
            shapes.push(BuildingShape {
                element,
                min_x,
                max_x,
                min_z,
                max_z,
                ring: ring
                    .iter()
                    .map(|node| (f64::from(node.z), f64::from(node.x)))
                    .collect(),
                area2: area2.abs(),
            });
        };
        for (index, element) in elements.iter().enumerate() {
            let building = element.tags().contains_key("building")
                || element.tags().contains_key("building:part");
            if !building {
                continue;
            }
            match element {
                ProcessedElement::Way(way) => push(index, &way.nodes),
                ProcessedElement::Relation(relation) => {
                    for member in &relation.members {
                        if member.role == ProcessedMemberRole::Outer {
                            push(index, &member.way.nodes);
                        }
                    }
                }
                ProcessedElement::Node(_) => {}
            }
        }
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (index, shape) in shapes.iter().enumerate() {
            for cx in shape.min_x.div_euclid(CELL)..=shape.max_x.div_euclid(CELL) {
                for cz in shape.min_z.div_euclid(CELL)..=shape.max_z.div_euclid(CELL) {
                    cells.entry((cx, cz)).or_default().push(index);
                }
            }
        }
        Self { shapes, cells }
    }

    fn smallest_containing(&self, x: i32, z: i32) -> Option<usize> {
        const CELL: i32 = 64;
        let candidates = self.cells.get(&(x.div_euclid(CELL), z.div_euclid(CELL)))?;
        candidates
            .iter()
            .filter_map(|&index| {
                let shape = &self.shapes[index];
                (x >= shape.min_x
                    && x <= shape.max_x
                    && z >= shape.min_z
                    && z <= shape.max_z
                    && point_in_ring(z as f64, x as f64, &shape.ring))
                .then_some((shape.area2, shape.element))
            })
            .min()
            .map(|(_, element)| element)
    }
}

struct NamedNodes {
    cells: HashMap<(i32, i32), Vec<(String, i32, i32)>>,
}

impl NamedNodes {
    fn near_name(&self, name: &str, x: i32, z: i32) -> bool {
        const CELL: i32 = 32;
        let needle = normalize_name(name);
        self.cells
            .get(&(x.div_euclid(CELL), z.div_euclid(CELL)))
            .is_some_and(|nodes| {
                nodes.iter().any(|(other, ox, oz)| {
                    normalize_name(other) == needle
                        && (i64::from(*ox) - i64::from(x)).pow(2)
                            + (i64::from(*oz) - i64::from(z)).pow(2)
                            <= 24 * 24
                })
            })
    }

    fn insert(&mut self, name: &str, x: i32, z: i32) {
        const CELL: i32 = 32;
        self.cells
            .entry((x.div_euclid(CELL), z.div_euclid(CELL)))
            .or_default()
            .push((name.to_string(), x, z));
    }
}

fn existing_named_nodes(elements: &[ProcessedElement]) -> NamedNodes {
    let mut nodes = NamedNodes {
        cells: HashMap::new(),
    };
    for element in elements {
        if let ProcessedElement::Node(node) = element {
            if let Some(name) = node.tags.get("name") {
                nodes.insert(name, node.x, node.z);
            }
        }
    }
    nodes
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Clone, Copy)]
struct Segment {
    ax: i32,
    az: i32,
    bx: i32,
    bz: i32,
}

struct RoadIndex {
    cells: HashMap<(i32, i32), Vec<Segment>>,
}

impl RoadIndex {
    fn new(elements: &[ProcessedElement]) -> Self {
        let mut index = Self {
            cells: HashMap::new(),
        };
        for element in elements {
            let ProcessedElement::Way(way) = element else {
                continue;
            };
            if !way.tags.contains_key("highway")
                && !way.tags.contains_key("railway")
                && !way.tags.contains_key("waterway")
            {
                continue;
            }
            index.insert_nodes(&way.nodes);
        }
        index
    }

    fn insert_nodes(&mut self, nodes: &[ProcessedNode]) {
        const CELL: i32 = 64;
        for pair in nodes.windows(2) {
            let segment = Segment {
                ax: pair[0].x,
                az: pair[0].z,
                bx: pair[1].x,
                bz: pair[1].z,
            };
            for cx in segment.min_x().div_euclid(CELL)..=segment.max_x().div_euclid(CELL) {
                for cz in segment.min_z().div_euclid(CELL)..=segment.max_z().div_euclid(CELL) {
                    self.cells.entry((cx, cz)).or_default().push(segment);
                }
            }
        }
    }

    fn is_duplicate(&self, nodes: &[ProcessedNode], tolerance: f64) -> bool {
        if nodes.len() < 2 {
            return true;
        }
        let mut covered = 0usize;
        let total = nodes.len() - 1;
        for pair in nodes.windows(2) {
            let (ax, az) = (pair[0].x as f64, pair[0].z as f64);
            let (bx, bz) = (pair[1].x as f64, pair[1].z as f64);
            let mx = (ax + bx) * 0.5;
            let mz = (az + bz) * 0.5;
            if self.near_segment(mx, mz, ax, az, bx, bz, tolerance) {
                covered += 1;
            }
        }
        covered * 5 >= total * 4
    }

    fn near_segment(
        &self,
        mx: f64,
        mz: f64,
        ax: f64,
        az: f64,
        bx: f64,
        bz: f64,
        tolerance: f64,
    ) -> bool {
        const CELL: i32 = 64;
        let Some(candidates) = self.cells.get(&(
            (mx.floor() as i32).div_euclid(CELL),
            (mz.floor() as i32).div_euclid(CELL),
        )) else {
            return false;
        };
        candidates.iter().any(|segment| {
            let distance = distance_to_segment(mx, mz, segment);
            if distance > tolerance {
                return false;
            }
            let (cdx, cdz) = (bx - ax, bz - az);
            let (sdx, sdz) = (
                f64::from(segment.bx - segment.ax),
                f64::from(segment.bz - segment.az),
            );
            let cosine = (cdx * sdx + cdz * sdz)
                / ((cdx * cdx + cdz * cdz).sqrt() * (sdx * sdx + sdz * sdz).sqrt()).max(1e-9);
            cosine.abs() >= 0.82 || distance < 0.6
        })
    }
}

impl Segment {
    fn min_x(self) -> i32 {
        self.ax.min(self.bx)
    }
    fn max_x(self) -> i32 {
        self.ax.max(self.bx)
    }
    fn min_z(self) -> i32 {
        self.az.min(self.bz)
    }
    fn max_z(self) -> i32 {
        self.az.max(self.bz)
    }
}

fn distance_to_segment(px: f64, pz: f64, segment: &Segment) -> f64 {
    let (ax, az) = (f64::from(segment.ax), f64::from(segment.az));
    let (bx, bz) = (f64::from(segment.bx), f64::from(segment.bz));
    let dx = bx - ax;
    let dz = bz - az;
    let length2 = dx * dx + dz * dz;
    let t = if length2 == 0.0 {
        0.0
    } else {
        (((px - ax) * dx + (pz - az) * dz) / length2).clamp(0.0, 1.0)
    };
    let x = ax + t * dx;
    let z = az + t * dz;
    ((px - x).powi(2) + (pz - z).powi(2)).sqrt()
}

/// Cache files a plan reads for this area, from the last accepted release.
pub fn cache_files(root: &std::path::Path, bbox: &LLBBox) -> Option<Vec<PathBuf>> {
    if pmtiles::tile_count(bbox, QUERY_ZOOM) > MAX_TILES {
        return None;
    }
    let overture = cache::cache_root_in(root);
    let release = cache::last_good_release_in(&overture)?;
    let (min_x, max_x, min_y, max_y) = pmtiles::tile_range(bbox, QUERY_ZOOM);
    let mut files = Vec::new();
    for theme in [TRANSPORTATION_THEME, PLACES_THEME] {
        let dir = overture.join(&release).join("tiles").join(theme);
        for x in min_x..=max_x {
            for y in min_y..=max_y {
                files.push(pmtiles::tile_cache_path(&dir, QUERY_ZOOM, x, y));
            }
        }
    }
    Some(files)
}

fn parse_osm_refs(sources: &str) -> Vec<OsmRef> {
    let Ok(JsonValue::Array(entries)) = serde_json::from_str(sources) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let dataset = entry.get("dataset")?.as_str()?;
            if !dataset.eq_ignore_ascii_case("OpenStreetMap") {
                return None;
            }
            entry
                .get("record_id")
                .and_then(JsonValue::as_str)
                .and_then(super::parse_record_id)
        })
        .collect()
}

fn primary_name(raw: &str) -> Option<String> {
    let value = parse_json_or_string(raw);
    match value {
        JsonValue::Object(object) => object
            .get("primary")
            .or_else(|| object.get("common"))
            .and_then(|value| {
                value.as_str().map(str::to_owned).or_else(|| {
                    value
                        .get("name")
                        .and_then(JsonValue::as_str)
                        .map(str::to_owned)
                })
            })
            .filter(|name| !name.trim().is_empty()),
        JsonValue::String(name) => Some(name).filter(|name| !name.trim().is_empty()),
        _ => None,
    }
}

fn category_from_taxonomy(raw: &str) -> Option<String> {
    let value = parse_json_or_string(raw);
    match value {
        JsonValue::Object(object) => object
            .get("primary")
            .and_then(json_text)
            .or_else(|| object.values().find_map(json_text)),
        JsonValue::Array(values) => values.first().and_then(json_text),
        JsonValue::String(value) => Some(value),
        _ => None,
    }
}

fn parse_json_or_string(raw: &str) -> JsonValue {
    serde_json::from_str(raw).unwrap_or_else(|_| JsonValue::String(raw.to_string()))
}

fn array_value(raw: &str) -> Option<String> {
    let value = parse_json_or_string(raw);
    match value {
        JsonValue::Array(values) => values.iter().find_map(json_text),
        JsonValue::Object(object) => object
            .get("value")
            .and_then(json_text)
            .or_else(|| object.values().find_map(json_text)),
        JsonValue::String(value) => Some(value).filter(|value| !value.is_empty()),
        _ => None,
    }
}

fn json_text(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(value) => (!value.is_empty()).then(|| value.clone()),
        JsonValue::Object(object) => object
            .get("value")
            .or_else(|| object.get("name"))
            .and_then(json_text),
        _ => None,
    }
}

fn first_ref(raw: &str) -> Option<String> {
    let value = parse_json_or_string(raw);
    match value {
        JsonValue::Array(values) => values.iter().find_map(|value| {
            value
                .get("ref")
                .and_then(JsonValue::as_str)
                .map(str::to_owned)
        }),
        JsonValue::Object(object) => object
            .get("ref")
            .and_then(JsonValue::as_str)
            .map(str::to_owned),
        JsonValue::String(value) => Some(value).filter(|value| !value.is_empty()),
        _ => None,
    }
}

fn flag_is_set(raw: &str, key: &str) -> bool {
    fn find(value: &JsonValue, key: &str) -> Option<bool> {
        match value {
            JsonValue::Array(values) => values.iter().find_map(|value| find(value, key)),
            JsonValue::Object(object) => {
                if let Some(value) = object.get(key) {
                    if let Some(value) = value.as_bool() {
                        return Some(value);
                    }
                    if let Some(value) = value.get("value").and_then(JsonValue::as_bool) {
                        return Some(value);
                    }
                }
                object.values().find_map(|value| find(value, key))
            }
            _ => None,
        }
    }
    find(&parse_json_or_string(raw), key).unwrap_or(false)
}

fn first_address(raw: &str) -> Option<HashMap<String, String>> {
    let value = parse_json_or_string(raw);
    let JsonValue::Array(addresses) = value else {
        return None;
    };
    let address = addresses.first()?.as_object()?;
    let mut tags = HashMap::new();
    for (key, out) in [
        ("street", "addr:street"),
        ("freeform", "addr:full"),
        ("locality", "addr:city"),
        ("postcode", "addr:postcode"),
    ] {
        if let Some(value) = address.get(key).and_then(json_text) {
            tags.insert(out.into(), value);
        }
    }
    if let Some(number) = address.get("house_number").and_then(json_text) {
        tags.insert("addr:housenumber".into(), number);
    }
    Some(tags)
}

fn stable_place_id(id: &str) -> u64 {
    stable_hash(id.as_bytes(), 1) | OVERTURE_ID_HIGH_BIT
}

fn stable_way_id(id: &str, part: usize) -> u64 {
    stable_hash(id.as_bytes(), part as u64 + 2) | OVERTURE_ID_HIGH_BIT
}

fn stable_node_id(id: &str, part: usize, point: XZPoint) -> u64 {
    let mut bytes = id.as_bytes().to_vec();
    bytes.extend_from_slice(&(part as u32).to_le_bytes());
    bytes.extend_from_slice(&point.x.to_le_bytes());
    bytes.extend_from_slice(&point.z.to_le_bytes());
    stable_hash(&bytes, 0) | OVERTURE_ID_HIGH_BIT
}

fn stable_hash(bytes: &[u8], seed: u64) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn osm_refs_exist(refs: &str, existing: &HashSet<(&'static str, u64)>) -> bool {
    refs.split(',').any(|reference| {
        let reference = reference.trim();
        let (kind, id) = match reference.split_at_checked(1) {
            Some(("w", id)) => ("way", id),
            Some(("r", id)) => ("relation", id),
            Some(("n", id)) => ("node", id),
            _ => return false,
        };
        id.parse::<u64>()
            .is_ok_and(|id| existing.contains(&(kind, id)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u64, x: i32, z: i32) -> ProcessedNode {
        ProcessedNode {
            id,
            tags: HashMap::new(),
            x,
            z,
        }
    }

    fn road(id: u64, points: &[(i32, i32)]) -> ProcessedElement {
        ProcessedElement::Way(ProcessedWay {
            id,
            tags: HashMap::from([("highway".to_string(), "residential".to_string())]),
            nodes: points
                .iter()
                .enumerate()
                .map(|(index, &(x, z))| node(id + index as u64, x, z))
                .collect(),
        })
    }

    #[test]
    fn classes_keep_rails_and_paths_distinct() {
        assert_eq!(road_class("footway"), "footway");
        assert_eq!(road_class("cycleway"), "cycleway");
        assert_eq!(road_class("unknown"), "unclassified");
        assert_eq!(rail_class("standard_gauge"), "rail");
        assert_eq!(rail_class("subway"), "subway");
    }

    #[test]
    fn names_taxonomy_and_flags_accept_json_strings() {
        assert_eq!(
            primary_name(r#"{"primary":"太仓汽车总站","common":{"name":"Taicang"}}"#),
            Some("太仓汽车总站".into())
        );
        assert_eq!(
            primary_name(r#"{"common":{"name":"Taicang Station"}}"#),
            Some("Taicang Station".into())
        );
        assert!(flag_is_set(r#"[{"is_bridge":true}]"#, "is_bridge"));
        assert!(!flag_is_set(r#"[{"is_tunnel":false}]"#, "is_tunnel"));
    }

    #[test]
    fn place_categories_map_to_osm_tags() {
        assert_eq!(
            place_category_tags("restaurant")
                .get("amenity")
                .map(String::as_str),
            Some("restaurant")
        );
        assert_eq!(
            place_category_tags("supermarket")
                .get("shop")
                .map(String::as_str),
            Some("supermarket")
        );
        assert_eq!(
            place_category_tags("train_station")
                .get("railway")
                .map(String::as_str),
            Some("station")
        );
        assert_eq!(
            place_category_tags("observatory")
                .get("man_made")
                .map(String::as_str),
            Some("observatory")
        );
        assert_eq!(
            place_category_tags("planetarium")
                .get("tourism")
                .map(String::as_str),
            Some("planetarium")
        );
    }

    #[test]
    fn a_road_already_present_in_osm_is_not_added_twice() {
        let mut elements = vec![road(1, &[(0, 0), (20, 0)])];
        let detail = OvertureDetailData {
            elements: vec![road(OVERTURE_ID_HIGH_BIT | 9, &[(0, 0), (20, 0)])],
            ..OvertureDetailData::default()
        };
        merge_detail(&mut elements, detail);
        assert_eq!(elements.len(), 1);
    }

    #[test]
    fn a_parallel_road_is_kept() {
        let elements = vec![road(1, &[(0, 0), (20, 0)])];
        let detail = OvertureDetailData {
            elements: vec![road(OVERTURE_ID_HIGH_BIT | 9, &[(0, 8), (20, 8)])],
            ..OvertureDetailData::default()
        };
        let mut merged = elements;
        merge_detail(&mut merged, detail);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn later_overture_tiles_cannot_add_the_same_road_again() {
        let first = road(OVERTURE_ID_HIGH_BIT | 9, &[(0, 0), (20, 0)]);
        let second = road(OVERTURE_ID_HIGH_BIT | 10, &[(0, 0), (20, 0)]);
        let detail = OvertureDetailData {
            elements: vec![first, second],
            ..OvertureDetailData::default()
        };
        let mut elements = Vec::new();
        merge_detail(&mut elements, detail);
        assert_eq!(elements.len(), 1);
    }

    #[test]
    fn a_place_inside_a_building_labels_the_building() {
        let mut building = road(1, &[(0, 0), (20, 0), (20, 20), (0, 20), (0, 0)]);
        if let ProcessedElement::Way(way) = &mut building {
            way.tags = HashMap::from([("building".to_string(), "yes".to_string())]);
        }
        let mut elements = vec![building];
        let place = ProcessedNode {
            id: OVERTURE_ID_HIGH_BIT | 10,
            tags: HashMap::from([
                ("name".to_string(), "太仓汽车总站".to_string()),
                ("amenity".to_string(), "bus_station".to_string()),
            ]),
            x: 10,
            z: 10,
        };
        let detail = OvertureDetailData {
            elements: vec![ProcessedElement::Node(place)],
            places: 1,
            ..OvertureDetailData::default()
        };
        merge_detail(&mut elements, detail);
        assert_eq!(elements.len(), 1);
        assert_eq!(
            elements[0].tags().get("name").map(String::as_str),
            Some("太仓汽车总站")
        );
        assert_eq!(
            elements[0].tags().get("amenity").map(String::as_str),
            Some("bus_station")
        );
    }

    #[test]
    #[ignore = "network integration test: reads Taicang PMTiles ranges"]
    fn fetches_taicang_detail_without_downloading_whole_archives() {
        let bbox = LLBBox::from_str("31.42,121.08,31.48,121.18").unwrap();
        let projection = ProjectionSpec::local(1.0);
        let data = fetch_inner(&bbox, &projection, true).expect("Taicang detail fetch");
        let roads = data
            .elements
            .iter()
            .filter(|element| {
                matches!(element, ProcessedElement::Way(way) if way.tags.contains_key("highway"))
            })
            .count();
        let rails = data
            .elements
            .iter()
            .filter(|element| {
                matches!(element, ProcessedElement::Way(way) if way.tags.contains_key("railway"))
            })
            .count();
        let places = data
            .elements
            .iter()
            .filter(|element| {
                matches!(element, ProcessedElement::Node(node) if node.tags.contains_key("name"))
            })
            .count();
        println!(
            "TAICANG_DETAIL roads={roads} railways={rails} places={places} elements={}",
            data.elements.len()
        );
        assert!(!data.elements.is_empty());
    }
}
