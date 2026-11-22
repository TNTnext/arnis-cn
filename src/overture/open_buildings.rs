//! VIDA Open Buildings for China.
//!
//! Overture is the monthly global aggregate, but rural Chinese counties are
//! still unevenly represented. VIDA's country PMTiles archive combines Google
//! Open Buildings, Microsoft/OSM building data and other open footprints. It is
//! a 700 MB immutable snapshot, read with the same range-request PMTiles
//! reader as Overture: opening a county reads its directory plus only the
//! z14 tiles touching the selection.
//!
//! The archive has no heights. Footprints from this source therefore receive
//! ordinary OSM-compatible building tags and the normal height inference;
//! Overture, OSM and measured-height data continue to win wherever available.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use colored::Colorize;
use rayon::prelude::*;

use super::pmtiles::{self, Archive};
use super::{mvt, OVERTURE_ID_HIGH_BIT};
use crate::clipping::clip_way_to_bbox;
use crate::coordinate_system::geographic::{LLBBox, LLPoint};
use crate::osm_parser::{ProcessedElement, ProcessedNode, ProcessedWay};
use crate::projection::ProjectionSpec;

const ARCHIVE_URL: &str =
    "https://data.source.coop/vida/google-microsoft-osm-open-buildings/pmtiles/by_country/country_iso=CHN/CHN.pmtiles";
const SNAPSHOT: &str = "CHN-2024-08-27";
const LAYER: &str = "CHN";
const QUERY_ZOOM: u8 = 14;
const MAX_TILES: u64 = 4096;
const TILE_BATCH: usize = 48;
const MAX_FOOTPRINTS: usize = 1_500_000;
const VIDA_ID_HIGH_BIT: u64 = 0x4000_0000_0000_0000;

/// VIDA footprints ready to merge after OSM and Overture.
#[derive(Default)]
pub struct OpenBuildingsData {
    pub elements: Vec<ProcessedElement>,
}

/// Whether an id was assigned to a footprint from a source other than OSM.
/// Water plausibility and other source-specific filters use this rather than
/// assuming every external footprint has Overture's id bit.
pub(super) fn is_external_footprint_id(id: u64) -> bool {
    id & (OVERTURE_ID_HIGH_BIT | VIDA_ID_HIGH_BIT) != 0
}

/// Root of VIDA's cached PMTiles header, directories and tile bodies.
pub fn cache_root() -> PathBuf {
    crate::elevation::cache::user_cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("arnis-open-buildings-cache")
}

/// Remove every cached VIDA header, directory and tile, leaving the root.
pub fn clear_cache() -> crate::elevation::cache::CacheClearStats {
    crate::elevation::cache::clear_cache_dir(&cache_root())
}

fn cache_root_in(root: &Path) -> PathBuf {
    root.join("arnis-open-buildings-cache")
}

fn archive_cache_dir_in(root: &Path) -> PathBuf {
    cache_root_in(root).join(SNAPSHOT).join("tiles")
}

fn archive_cache_dir() -> PathBuf {
    cache_root().join(SNAPSHOT).join("tiles")
}

/// Tile files a plan reads for this area.
pub fn cache_files(root: &Path, bbox: &LLBBox) -> Option<Vec<PathBuf>> {
    if pmtiles::tile_count(bbox, QUERY_ZOOM) > MAX_TILES {
        return None;
    }
    let (min_x, max_x, min_y, max_y) = pmtiles::tile_range(bbox, QUERY_ZOOM);
    let dir = archive_cache_dir_in(root);
    let mut files = Vec::new();
    for x in min_x..=max_x {
        for y in min_y..=max_y {
            files.push(pmtiles::tile_cache_path(&dir, QUERY_ZOOM, x, y));
        }
    }
    Some(files)
}

/// Fetch China Open Buildings for the selected area. Failure is additive and
/// non-fatal, matching Overture's supplementary layers.
pub fn fetch_open_buildings(
    bbox: &LLBBox,
    projection: &ProjectionSpec,
    debug: bool,
) -> OpenBuildingsData {
    match fetch_inner(bbox, projection, debug) {
        Ok(data) => data,
        Err(error) => {
            crate::net::note_uncached("VIDA China Open Buildings");
            eprintln!(
                "{} Failed to fetch VIDA China Open Buildings: {error}",
                "Warning:".yellow().bold()
            );
            OpenBuildingsData::default()
        }
    }
}

fn fetch_inner(
    bbox: &LLBBox,
    projection: &ProjectionSpec,
    debug: bool,
) -> Result<OpenBuildingsData, Box<dyn std::error::Error>> {
    let wanted_tiles = pmtiles::tile_count(bbox, QUERY_ZOOM);
    if wanted_tiles > MAX_TILES {
        return Err(format!(
            "the area needs {wanted_tiles} z{QUERY_ZOOM} tiles; VIDA supports at most {MAX_TILES}. \
             Select a city or county, or use Overture Complete for a larger area."
        )
        .into());
    }
    let client = super::overture_client()?;
    let mut archive = Archive::open(&client, ARCHIVE_URL, Some(archive_cache_dir()))?;
    let (min_zoom, max_zoom) = (archive.header().min_zoom, archive.header().max_zoom);
    if !(min_zoom..=max_zoom).contains(&QUERY_ZOOM) {
        return Err(
            format!("VIDA archive covers zoom {min_zoom}..={max_zoom}, not z{QUERY_ZOOM}").into(),
        );
    }

    let (min_x, max_x, min_y, max_y) = pmtiles::tile_range(bbox, QUERY_ZOOM);
    let mut located = Vec::new();
    for x in min_x..=max_x {
        for y in min_y..=max_y {
            if let Some(location) = archive.locate(&client, QUERY_ZOOM, x, y)? {
                located.push((x, y, location));
            }
        }
    }
    if debug {
        println!(
            "VIDA Open Buildings: {wanted_tiles} tile(s) cover the bbox, {} hold data",
            located.len()
        );
    }

    let pool = fetch_pool()?;
    let archive = &archive;
    let mut best: HashMap<u64, Candidate> = HashMap::new();
    let mut failed = 0usize;
    for batch in located.chunks(TILE_BATCH) {
        let results: Vec<Result<Vec<Candidate>, String>> = pool.install(|| {
            batch
                .par_iter()
                .map(|&(x, y, location)| {
                    let raw = archive.tile(&client, QUERY_ZOOM, x, y, location)?;
                    if raw.is_empty() {
                        return Ok(Vec::new());
                    }
                    parse_tile(mvt::decode_tile(&raw)?, bbox, x, y)
                })
                .collect()
        });
        for result in results {
            match result {
                Ok(found) => {
                    for candidate in found {
                        match best.entry(candidate.id) {
                            std::collections::hash_map::Entry::Occupied(mut slot) => {
                                if candidate.area2 > slot.get().area2 {
                                    slot.insert(candidate);
                                }
                            }
                            std::collections::hash_map::Entry::Vacant(slot) => {
                                slot.insert(candidate);
                            }
                        }
                    }
                }
                Err(error) => {
                    failed += 1;
                    if debug {
                        eprintln!("Warning: VIDA tile could not be read: {error}");
                    }
                }
            }
        }
    }
    if !located.is_empty() && failed == located.len() {
        return Err(format!("all {failed} VIDA data tile(s) could not be read").into());
    }
    if failed > 0 {
        eprintln!(
            "{} VIDA building data is incomplete: {failed} tile(s) could not be read.",
            "Warning:".yellow().bold()
        );
    }

    let mut candidates: Vec<Candidate> = best.into_values().collect();
    candidates.sort_unstable_by_key(|candidate| candidate.id);
    let candidate_count = candidates.len();
    let truncated = candidates.len() > MAX_FOOTPRINTS;
    candidates.truncate(MAX_FOOTPRINTS);
    if truncated {
        eprintln!(
            "{} Reached the VIDA footprint limit ({MAX_FOOTPRINTS}); use a smaller selection \
             for complete coverage.",
            "Warning:".yellow().bold()
        );
    }

    let (transformer, xzbbox) = projection.transformer(bbox)?;
    let clip_bbox = projection.clip_bbox(&xzbbox);
    let elements: Vec<ProcessedElement> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let nodes: Vec<ProcessedNode> = candidate
                .ring
                .iter()
                .filter_map(|&(lng, lat)| {
                    let ll = LLPoint::new(lat, lng).ok()?;
                    let xz = transformer.transform_point(ll);
                    Some(ProcessedNode {
                        id: candidate.id,
                        tags: HashMap::new(),
                        x: xz.x,
                        z: xz.z,
                    })
                })
                .collect();
            let clipped = clip_way_to_bbox(&nodes, &clip_bbox);
            if clipped.len() < 3 {
                return None;
            }
            let mut tags = HashMap::from([
                ("building".to_string(), "yes".to_string()),
                ("source".to_string(), "open_buildings_vida".to_string()),
            ]);
            if let Some(source) = candidate.footprint_source {
                tags.insert("source:footprint".to_string(), source);
            }
            Some(ProcessedElement::Way(ProcessedWay {
                id: candidate.id,
                nodes: clipped,
                tags,
            }))
        })
        .collect();

    if debug {
        println!(
            "VIDA Open Buildings: {} unique non-OSM footprint(s), {} clipped element(s)",
            candidate_count,
            elements.len()
        );
    }
    Ok(OpenBuildingsData { elements })
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
            .thread_name(|i| format!("vida-open-buildings-{i}"))
            .build()
            .map_err(|error| format!("could not start the VIDA fetch pool: {error}"))
    })
    .as_ref()
    .map_err(String::clone)
}

struct Candidate {
    id: u64,
    ring: Vec<(f64, f64)>,
    area2: f64,
    footprint_source: Option<String>,
}

fn parse_tile(
    layers: Vec<mvt::Layer>,
    bbox: &LLBBox,
    tile_x: u32,
    tile_y: u32,
) -> Result<Vec<Candidate>, String> {
    let mut out = Vec::new();
    for layer in layers.iter().filter(|layer| layer.name == LAYER) {
        let extent = f64::from(layer.extent.max(1));
        for feature in &layer.features {
            if feature.geom_type != mvt::GEOM_POLYGON {
                continue;
            }
            let Some(numeric_id) = feature.id else {
                continue;
            };
            let source = layer
                .attr_str(feature, "bf_source")
                .unwrap_or_default()
                .to_ascii_lowercase();
            if source == "osm" || source.contains("openstreetmap") {
                continue;
            }
            if layer
                .attr_f64(feature, "area_in_meters")
                .is_some_and(|area| !(2.0..=1_000_000.0).contains(&area))
            {
                continue;
            }
            let Some(ring) = feature
                .rings
                .iter()
                .filter(|ring| ring.points.len() >= 3)
                .max_by_key(|ring| ring.area2_abs())
            else {
                continue;
            };
            let ring: Vec<(f64, f64)> = ring
                .points
                .iter()
                .map(|&(x, y)| {
                    (
                        pmtiles::tile_x_to_lon(QUERY_ZOOM, tile_x, f64::from(x) / extent),
                        pmtiles::tile_y_to_lat(QUERY_ZOOM, tile_y, f64::from(y) / extent),
                    )
                })
                .collect();
            if !ring_overlaps_bbox(&ring, bbox) {
                continue;
            }
            out.push(Candidate {
                id: numeric_id | VIDA_ID_HIGH_BIT,
                area2: ring_area2(&ring),
                footprint_source: (!source.is_empty()).then_some(source),
                ring,
            });
        }
    }
    Ok(out)
}

fn ring_overlaps_bbox(ring: &[(f64, f64)], bbox: &LLBBox) -> bool {
    let (mut min_lng, mut max_lng) = (f64::MAX, f64::MIN);
    let (mut min_lat, mut max_lat) = (f64::MAX, f64::MIN);
    for &(lng, lat) in ring {
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

fn ring_area2(ring: &[(f64, f64)]) -> f64 {
    let mut sum = 0.0;
    for index in 0..ring.len() {
        let (x1, y1) = ring[index];
        let (x2, y2) = ring[(index + 1) % ring.len()];
        sum += x1 * y2 - x2 * y1;
    }
    sum.abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_footprint_ids_cover_both_sources() {
        assert!(is_external_footprint_id(OVERTURE_ID_HIGH_BIT | 1));
        assert!(is_external_footprint_id(VIDA_ID_HIGH_BIT | 1));
        assert!(!is_external_footprint_id(123));
    }

    #[test]
    fn the_vida_snapshot_is_addressed_separately_from_overture() {
        let root = Path::new("cache");
        assert!(archive_cache_dir_in(root).ends_with("CHN-2024-08-27/tiles"));
        assert_ne!(cache_root_in(root), root.join("arnis-overture-cache"));
    }

    #[test]
    fn the_live_archive_uses_the_documented_cache_folder_once() {
        let cache = archive_cache_dir();
        assert!(cache.ends_with("arnis-open-buildings-cache/CHN-2024-08-27/tiles"));
        assert!(!cache
            .to_string_lossy()
            .contains("arnis-open-buildings-cache\\arnis-open-buildings-cache"));
        assert!(!cache
            .to_string_lossy()
            .contains("arnis-open-buildings-cache/arnis-open-buildings-cache"));
    }

    #[test]
    #[ignore = "network integration test: reads Taicang VIDA PMTiles ranges"]
    fn fetches_taicang_open_buildings_without_the_whole_archive() {
        let bbox_text = std::env::var("ARNIS_VIDA_TEST_BBOX")
            .unwrap_or_else(|_| "31.42,121.08,31.48,121.18".to_string());
        let bbox = LLBBox::from_str(&bbox_text).unwrap();
        let data = fetch_inner(&bbox, &ProjectionSpec::local(1.0), true)
            .expect("Taicang VIDA Open Buildings fetch");
        let buildings = data
            .elements
            .iter()
            .filter(|element| element.tags().get("building").map(String::as_str) == Some("yes"))
            .count();
        println!(
            "TAICANG_VIDA buildings={buildings} elements={}",
            data.elements.len()
        );
        assert!(
            buildings > 10_000,
            "a county selection should have additional non-OSM footprints"
        );
    }
}
