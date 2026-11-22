//! Overture base theme: water, land use and land cover.
//!
//! These layers fill surfaces that globally sourced land cover can only show
//! as a broad class: reservoirs, canals, parks, industrial estates, protected
//! areas and named natural features. Only OSM-compatible source tags are kept,
//! and OSM-sourced geometries are skipped because the OSM extract already has
//! a more exact copy of them.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use colored::Colorize;
use rayon::prelude::*;
use reqwest::blocking::Client;
use serde_json::Value as JsonValue;

use super::pmtiles::{self, Archive};
use super::{cache, mvt, release_candidates};
use crate::clipping::clip_way_to_bbox;
use crate::coordinate_system::geographic::{LLBBox, LLPoint};
use crate::osm_parser::{ProcessedElement, ProcessedNode, ProcessedWay};
use crate::projection::ProjectionSpec;

const OVERTURE_TILES_BUCKET: &str =
    "https://overturemaps-extras-us-west-2.s3.us-west-2.amazonaws.com";
const THEME: &str = "base";
const QUERY_ZOOM: u8 = 13;
const MAX_TILES: u64 = 4096;
const TILE_BATCH: usize = 48;
const MAX_FEATURES: usize = 1_000_000;
const BASE_ID_HIGH_BIT: u64 = 0x2000_0000_0000_0000;

const WATER_LAYER: &str = "water";
const LAND_USE_LAYER: &str = "land_use";
const LAND_COVER_LAYER: &str = "land_cover";

/// Surface features from Overture's base theme, ready to merge before the
/// mapped OSM water and land overrides are applied.
#[derive(Default)]
pub struct OvertureBaseData {
    pub elements: Vec<ProcessedElement>,
}

pub fn cache_files(root: &Path, bbox: &LLBBox) -> Option<Vec<PathBuf>> {
    if pmtiles::tile_count(bbox, QUERY_ZOOM) > MAX_TILES {
        return None;
    }
    let overture = cache::cache_root_in(root);
    let release = cache::last_good_release_in(&overture)?;
    let dir = overture.join(release).join("tiles").join(THEME);
    let (min_x, max_x, min_y, max_y) = pmtiles::tile_range(bbox, QUERY_ZOOM);
    let mut files = Vec::new();
    for x in min_x..=max_x {
        for y in min_y..=max_y {
            files.push(pmtiles::tile_cache_path(&dir, QUERY_ZOOM, x, y));
        }
    }
    Some(files)
}

/// Fetch base water and surface polygons. Failure is non-fatal, like the
/// building and transport supplements.
pub fn fetch_overture_base(
    bbox: &LLBBox,
    projection: &ProjectionSpec,
    debug: bool,
) -> OvertureBaseData {
    match fetch_inner(bbox, projection, debug) {
        Ok(data) => data,
        Err(error) => {
            crate::net::note_uncached("Overture base water and land use");
            eprintln!(
                "{} Failed to fetch Overture base water and land use: {error}",
                "Warning:".yellow().bold()
            );
            OvertureBaseData::default()
        }
    }
}

fn fetch_inner(
    bbox: &LLBBox,
    projection: &ProjectionSpec,
    debug: bool,
) -> Result<OvertureBaseData, Box<dyn std::error::Error>> {
    let wanted = pmtiles::tile_count(bbox, QUERY_ZOOM);
    if wanted > MAX_TILES {
        return Err(format!(
            "the area needs {wanted} z{QUERY_ZOOM} tiles; base data supports at most {MAX_TILES}"
        )
        .into());
    }
    let client = super::overture_client()?;
    let releases = release_candidates(&client, debug);
    if releases.is_empty() {
        return Err("no Overture release candidates".into());
    }
    let (release, features) = read_with_fallback(&client, bbox, &releases, debug)?;
    cache::set_last_good_release(&release);

    let (transformer, xzbbox) = projection.transformer(bbox)?;
    let clip_bbox = projection.clip_bbox(&xzbbox);
    let mut elements = Vec::new();
    let mut seen: HashSet<u64> = HashSet::new();
    for feature in features {
        for (part, line) in feature.lines.into_iter().enumerate() {
            if line.len() < 2 {
                continue;
            }
            // One vector-tile feature can hold several polygons. Tile buffers can
            // repeat the same polygon in adjacent tiles, so dedupe each piece by
            // its geometry rather than dropping every part after the first.
            let id = line_id(feature.id, part, &line);
            if !seen.insert(id) {
                continue;
            }
            let nodes: Vec<ProcessedNode> = line
                .into_iter()
                .filter_map(|(lng, lat)| {
                    let ll = LLPoint::new(lat, lng).ok()?;
                    let xz = transformer.transform_point(ll);
                    Some(ProcessedNode {
                        id,
                        tags: HashMap::new(),
                        x: xz.x,
                        z: xz.z,
                    })
                })
                .collect();
            let clipped = clip_way_to_bbox(&nodes, &clip_bbox);
            if clipped.len() < 2 {
                continue;
            }
            elements.push(ProcessedElement::Way(ProcessedWay {
                id,
                nodes: clipped,
                tags: feature.tags.clone(),
            }));
        }
    }
    if debug {
        println!(
            "Overture base ({release}): {} surface element(s) added",
            elements.len()
        );
    }
    Ok(OvertureBaseData { elements })
}

fn read_with_fallback(
    client: &Client,
    bbox: &LLBBox,
    releases: &[String],
    debug: bool,
) -> Result<(String, Vec<RawFeature>), Box<dyn std::error::Error>> {
    let mut last_error = "no readable Overture base archive".to_string();
    let mut ordered = releases.to_vec();
    if !crate::net::offline() {
        ordered.sort_by(|a, b| b.cmp(a));
    }
    for release in ordered {
        match read_release(client, bbox, &release, debug) {
            Ok(features) => return Ok((release, features)),
            Err(error) => {
                last_error = format!("{release}: {error}");
                if debug {
                    eprintln!("Overture base unavailable for {release}: {error}");
                }
            }
        }
    }
    Err(last_error.into())
}

fn read_release(
    client: &Client,
    bbox: &LLBBox,
    release: &str,
    debug: bool,
) -> Result<Vec<RawFeature>, String> {
    let dir = cache::release_dir(release).map(|dir| dir.join("tiles").join(THEME));
    let url = format!("{OVERTURE_TILES_BUCKET}/tiles/{release}/{THEME}.pmtiles");
    let mut archive = Archive::open(client, &url, dir)?;
    let (min_zoom, max_zoom) = (archive.header().min_zoom, archive.header().max_zoom);
    if !(min_zoom..=max_zoom).contains(&QUERY_ZOOM) {
        return Err(format!(
            "archive covers zoom {min_zoom}..={max_zoom}, not z{QUERY_ZOOM}"
        ));
    }

    let (min_x, max_x, min_y, max_y) = pmtiles::tile_range(bbox, QUERY_ZOOM);
    let mut located = Vec::new();
    for x in min_x..=max_x {
        for y in min_y..=max_y {
            if let Some(location) = archive.locate(client, QUERY_ZOOM, x, y)? {
                located.push((x, y, location));
            }
        }
    }
    if debug {
        println!(
            "Overture base: {} z13 tile(s) cover the bbox, {} hold data",
            pmtiles::tile_count(bbox, QUERY_ZOOM),
            located.len()
        );
    }

    let pool = fetch_pool()?;
    let archive = &archive;
    let mut features = Vec::new();
    let mut failed = 0usize;
    for batch in located.chunks(TILE_BATCH) {
        let results: Vec<Result<Vec<RawFeature>, String>> = pool.install(|| {
            batch
                .par_iter()
                .map(|&(x, y, location)| {
                    let raw = archive.tile(client, QUERY_ZOOM, x, y, location)?;
                    if raw.is_empty() {
                        return Ok(Vec::new());
                    }
                    parse_tile(mvt::decode_tile(&raw)?, bbox, x, y)
                })
                .collect()
        });
        for result in results {
            match result {
                Ok(mut found) => features.append(&mut found),
                Err(error) => {
                    failed += 1;
                    if debug {
                        eprintln!("Warning: Overture base tile could not be read: {error}");
                    }
                }
            }
        }
        if features.len() >= MAX_FEATURES {
            break;
        }
    }
    if !located.is_empty() && failed == located.len() {
        return Err(format!("all {failed} base tile(s) failed"));
    }
    if failed > 0 {
        eprintln!(
            "{} Overture base data is incomplete: {failed} tile(s) could not be read.",
            "Warning:".yellow().bold()
        );
    }
    if features.len() > MAX_FEATURES {
        features.truncate(MAX_FEATURES);
        eprintln!(
            "{} Reached the Overture base feature limit ({MAX_FEATURES}); select a smaller area.",
            "Warning:".yellow().bold()
        );
    }
    Ok(features)
}

fn fetch_pool() -> Result<&'static rayon::ThreadPool, String> {
    static POOL: OnceLock<Result<rayon::ThreadPool, String>> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(
                std::thread::available_parallelism()
                    .map(std::num::NonZeroUsize::get)
                    .unwrap_or(2)
                    .clamp(2, 8),
            )
            .thread_name(|i| format!("overture-base-{i}"))
            .build()
            .map_err(|error| format!("could not start the Overture base pool: {error}"))
    })
    .as_ref()
    .map_err(String::clone)
}

struct RawFeature {
    id: u64,
    tags: HashMap<String, String>,
    lines: Vec<Vec<(f64, f64)>>,
}

fn parse_tile(
    layers: Vec<mvt::Layer>,
    bbox: &LLBBox,
    tile_x: u32,
    tile_y: u32,
) -> Result<Vec<RawFeature>, String> {
    let mut out = Vec::new();
    for layer in layers.iter().filter(|layer| {
        matches!(
            layer.name.as_str(),
            WATER_LAYER | LAND_USE_LAYER | LAND_COVER_LAYER
        )
    }) {
        let extent = f64::from(layer.extent.max(1));
        for feature in &layer.features {
            if sources_are_osm(layer.attr_str(feature, "sources")) {
                continue;
            }
            let tags = feature_tags(&layer.name, layer, feature);
            if tags.is_empty() {
                continue;
            }
            let key = layer
                .attr_str(feature, "id")
                .map(str::to_owned)
                .or_else(|| feature.id.map(|id| id.to_string()))
                .unwrap_or_else(|| format!("{}/{}/{}", layer.name, tile_x, tile_y));
            let id = stable_id(&format!("{}:{key}", layer.name));
            let mut lines = Vec::new();
            for ring in &feature.rings {
                if ring.points.len() < 2 {
                    continue;
                }
                let line: Vec<(f64, f64)> = ring
                    .points
                    .iter()
                    .map(|&(x, y)| {
                        (
                            pmtiles::tile_x_to_lon(QUERY_ZOOM, tile_x, f64::from(x) / extent),
                            pmtiles::tile_y_to_lat(QUERY_ZOOM, tile_y, f64::from(y) / extent),
                        )
                    })
                    .collect();
                if line_overlaps_bbox(&line, bbox) {
                    lines.push(line);
                }
            }
            if !lines.is_empty() {
                out.push(RawFeature { id, tags, lines });
            }
        }
    }
    Ok(out)
}

fn feature_tags(
    layer_name: &str,
    layer: &mvt::Layer,
    feature: &mvt::Feature,
) -> HashMap<String, String> {
    let mut tags = HashMap::new();
    if let Some(raw) = layer.attr_str(feature, "source_tags") {
        if let Ok(JsonValue::Object(source)) = serde_json::from_str(raw) {
            for (key, value) in source {
                if allowed_tag(&key) {
                    if let Some(value) = value.as_str().filter(|value| !value.is_empty()) {
                        tags.insert(key, value.to_string());
                    }
                }
            }
        }
    }
    if tags.is_empty() {
        match layer_name {
            WATER_LAYER => {
                tags.insert("natural".to_string(), "water".to_string());
            }
            LAND_COVER_LAYER => {
                let subtype = layer
                    .attr_str(feature, "subtype")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if let Some((key, value)) = land_cover_tag(&subtype) {
                    tags.insert(key.to_string(), value.to_string());
                }
            }
            _ => {}
        }
    }
    if !tags.is_empty() {
        tags.insert("source".to_string(), "overture_base".to_string());
        tags.insert("arnis:surface_source".to_string(), layer_name.to_string());
    }
    tags
}

fn allowed_tag(key: &str) -> bool {
    matches!(
        key,
        "waterway"
            | "natural"
            | "water"
            | "landuse"
            | "leisure"
            | "amenity"
            | "military"
            | "place"
            | "boundary"
            | "wetland"
            | "name"
            | "name:en"
            | "name:zh"
            | "int_ref"
    )
}

fn land_cover_tag(subtype: &str) -> Option<(&'static str, &'static str)> {
    Some(match subtype {
        "forest" | "tree_cover" => ("natural", "wood"),
        "shrubland" | "moss" => ("natural", "scrub"),
        "grassland" => ("natural", "grassland"),
        "cropland" => ("landuse", "farmland"),
        "bare" | "bare_sparse" => ("natural", "bare_rock"),
        "snow" | "snow_ice" => ("natural", "glacier"),
        "water" => ("natural", "water"),
        "wetland" => ("natural", "wetland"),
        "mangroves" => ("wetland", "mangrove"),
        _ => return None,
    })
}

fn sources_are_osm(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    let Ok(JsonValue::Array(entries)) = serde_json::from_str(raw) else {
        return false;
    };
    entries.iter().any(|entry| {
        entry
            .get("dataset")
            .and_then(JsonValue::as_str)
            .is_some_and(|dataset| dataset.eq_ignore_ascii_case("OpenStreetMap"))
    })
}

fn line_overlaps_bbox(line: &[(f64, f64)], bbox: &LLBBox) -> bool {
    line.iter().any(|&(lng, lat)| {
        (bbox.min().lng()..=bbox.max().lng()).contains(&lng)
            && (bbox.min().lat()..=bbox.max().lat()).contains(&lat)
    }) || {
        let (mut min_lng, mut max_lng) = (f64::MAX, f64::MIN);
        let (mut min_lat, mut max_lat) = (f64::MAX, f64::MIN);
        for &(lng, lat) in line {
            min_lng = min_lng.min(lng);
            max_lng = max_lng.max(lng);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
        }
        max_lng >= bbox.min().lng()
            && min_lng <= bbox.max().lng()
            && max_lat >= bbox.min().lat()
            && min_lat <= bbox.max().lat()
    }
}

fn stable_id(value: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash | BASE_ID_HIGH_BIT
}

fn line_id(feature_id: u64, part: usize, line: &[(f64, f64)]) -> u64 {
    let mut hash = stable_id(&feature_id.to_string());
    hash ^= (part as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    for (lng, lat) in line {
        hash ^= lng.to_bits();
        hash = hash.rotate_left(13).wrapping_mul(0x1000_0000_01b3);
        hash ^= lat.to_bits();
        hash = hash.rotate_left(17).wrapping_mul(0x1000_0000_01b3);
    }
    hash | BASE_ID_HIGH_BIT
}

/// Merge base surfaces before OSM's water and land overrides inspect them.
pub fn merge_base(parsed_elements: &mut Vec<ProcessedElement>, base: OvertureBaseData) {
    let before = parsed_elements.len();
    let mut existing: HashSet<u64> = parsed_elements.iter().map(ProcessedElement::id).collect();
    for element in base.elements {
        if existing.insert(element.id()) {
            parsed_elements.push(element);
        }
    }
    let added = parsed_elements.len() - before;
    if added > 0 {
        println!(
            "  Added {} water and land-use features from Overture base",
            added.to_string().bright_white().bold()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(raw: &str) -> HashMap<String, String> {
        let parsed: JsonValue = serde_json::from_str(raw).unwrap();
        let layer = mvt::Layer {
            name: LAND_USE_LAYER.into(),
            extent: 4096,
            keys: vec!["source_tags".into()],
            values: vec![mvt::Value::Str(raw.into())],
            features: Vec::new(),
        };
        let feature = mvt::Feature {
            id: Some(1),
            geom_type: mvt::GEOM_POLYGON,
            tags: vec![0, 0],
            rings: Vec::new(),
        };
        let _ = parsed;
        feature_tags(LAND_USE_LAYER, &layer, &feature)
    }

    #[test]
    fn only_osm_compatible_source_tags_survive() {
        let mapped = tags(r#"{"leisure":"park","landuse":"forest","random":"x","name":"公园"}"#);
        assert_eq!(mapped.get("leisure").map(String::as_str), Some("park"));
        assert_eq!(mapped.get("landuse").map(String::as_str), Some("forest"));
        assert_eq!(mapped.get("name").map(String::as_str), Some("公园"));
        assert_eq!(
            mapped.get("source").map(String::as_str),
            Some("overture_base")
        );
        assert!(!mapped.contains_key("random"));
    }

    #[test]
    fn osm_sourced_base_rows_are_skipped() {
        assert!(sources_are_osm(Some(
            r#"[{"dataset":"OpenStreetMap","record_id":"w1"}]"#
        )));
        assert!(!sources_are_osm(Some(
            r#"[{"dataset":"ESA WorldCover 2021"}]"#
        )));
        assert!(!sources_are_osm(None));
    }

    #[test]
    fn multipart_surfaces_keep_distinct_ids() {
        let line = [(121.1, 31.1), (121.2, 31.1), (121.2, 31.2)];
        assert_ne!(line_id(7, 0, &line), line_id(7, 1, &line));
        assert_eq!(line_id(7, 0, &line), line_id(7, 0, &line));
    }

    #[test]
    #[ignore = "network integration test: reads Taicang base PMTiles ranges"]
    fn fetches_taicang_water_and_land_use() {
        let bbox = LLBBox::from_str("31.42,121.08,31.48,121.18").unwrap();
        let data = fetch_inner(&bbox, &ProjectionSpec::local(1.0), true)
            .expect("Taicang Overture base fetch");
        let water = data
            .elements
            .iter()
            .filter(|element| element.tags().get("natural").map(String::as_str) == Some("water"))
            .count();
        println!(
            "TAICANG_BASE water={water} elements={}",
            data.elements.len()
        );
        assert!(!data.elements.is_empty());
    }
}
