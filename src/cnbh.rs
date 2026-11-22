//! CNBH-10m building heights for China.
//!
//! CNBH-10m is a 2020, 10 m building-height prediction for China (CC BY 4.0,
//! DOI 10.5281/zenodo.7923866). It complements OSM, Overture and VIDA
//! footprints: those supply detailed outlines, while CNBH fills a height on
//! buildings that carry neither an explicit height nor a floor count.
//!
//! The dataset is split into 2-degree GeoTIFFs. A county normally needs one,
//! and a selection that crosses a 2-degree edge needs at most four. Each TIFF
//! is tiled at 256 x 256 pixels, so this module opens only the tiles under
//! building centroids instead of decoding a multi-gigabyte raster in memory.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use colored::Colorize;
use reqwest::blocking::Client;
use reqwest::StatusCode;
use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;

use crate::coordinate_system::cartesian::XZBBox;
use crate::coordinate_system::geographic::LLBBox;
use crate::osm_parser::ProcessedElement;
use crate::projection::{ProjectionKind, ProjectionSpec};

const SNAPSHOT: &str = "CNBH10m-2020";
const FILE_TEMPLATE: &str =
    "https://zenodo.org/api/records/7923866/files/CNBH10m_X{x}Y{y}.tif/content";
const DEGREES_PER_FILE: i32 = 2;
const MAX_FILES: usize = 16;
const MIN_HEIGHT_M: f32 = 2.0;
const MAX_HEIGHT_M: f32 = 350.0;
const HEIGHT_SOURCE: &str = "cnbh10m_2020";

/// CNBH GeoTIFFs covering a selection, with their georeferencing metadata.
#[derive(Default)]
pub struct CnbhData {
    pub tiles: Vec<CnbhTile>,
}

/// One downloaded CNBH GeoTIFF. All offsets are in the TIFF's native UTM CRS.
#[derive(Clone, Debug)]
pub struct CnbhTile {
    path: PathBuf,
    width: u32,
    height: u32,
    tile_width: u32,
    tile_height: u32,
    tiles_across: u32,
    pixel_x: f64,
    pixel_y: f64,
    top_left_x: f64,
    top_left_y: f64,
    utm_zone: u8,
    south: bool,
}

/// Root of the CNBH GeoTIFF cache.
pub fn cache_root() -> PathBuf {
    cache_root_in(&crate::elevation::cache::user_cache_dir().unwrap_or_else(|| PathBuf::from(".")))
}

fn cache_root_in(root: &Path) -> PathBuf {
    root.join("arnis-cnbh-cache")
}

fn snapshot_dir_in(root: &Path) -> PathBuf {
    cache_root_in(root).join(SNAPSHOT)
}

pub fn clear_cache() -> crate::elevation::cache::CacheClearStats {
    crate::elevation::cache::clear_cache_dir(&cache_root())
}

/// The CNBH files a plan reads. `None` means the selection is too large for
/// this layer; `Some(empty)` means it lies outside China's covered grid.
pub fn cache_files(root: &Path, bbox: &LLBBox) -> Option<Vec<PathBuf>> {
    let cells = tile_cells(bbox).ok()?;
    let dir = snapshot_dir_in(root);
    Some(
        cells
            .into_iter()
            .map(|(x, y)| dir.join(tile_file_name(x, y)))
            .collect(),
    )
}

/// Fetch the GeoTIFFs touching `bbox`. Failure is supplementary and non-fatal.
pub fn fetch_cnbh(bbox: &LLBBox, debug: bool) -> CnbhData {
    match fetch_inner(bbox, debug) {
        Ok(data) => data,
        Err(error) => {
            crate::net::note_uncached("CNBH-10m China building heights");
            eprintln!(
                "{} Failed to fetch CNBH-10m building heights: {error}",
                "Warning:".yellow().bold()
            );
            CnbhData::default()
        }
    }
}

fn fetch_inner(bbox: &LLBBox, debug: bool) -> Result<CnbhData, Box<dyn std::error::Error>> {
    let cells = tile_cells(bbox)?;
    if cells.is_empty() {
        return Ok(CnbhData::default());
    }
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(20))
        // A 2-degree compressed GeoTIFF can be hundreds of megabytes. The
        // connection is still bounded; this only lets one large body finish.
        .timeout(Duration::from_secs(60 * 60))
        .user_agent(concat!(
            "Arnis/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/louis-e/arnis)"
        ))
        .build()?;

    let dir = snapshot_dir_in(
        &crate::elevation::cache::user_cache_dir().unwrap_or_else(|| PathBuf::from(".")),
    );
    let mut tiles = Vec::with_capacity(cells.len());
    for (x, y) in cells {
        let path = dir.join(tile_file_name(x, y));
        let metadata = match read_metadata(&path) {
            Ok(tile) => Some(tile),
            Err(_) => None,
        };
        let tile = if let Some(tile) = metadata {
            tile
        } else {
            crate::net::ensure_online("CNBH-10m China building heights")?;
            download_tile(&client, x, y, &path, debug)?;
            read_metadata(&path)?
        };
        if debug {
            println!(
                "CNBH-10m {}: {} x {} pixels, UTM zone {}{}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("tile"),
                tile.width,
                tile.height,
                tile.utm_zone,
                if tile.south { "S" } else { "N" }
            );
        }
        tiles.push(tile);
    }
    println!(
        "  Using {} CNBH-10m building-height raster(s) for this area",
        tiles.len().to_string().bright_white().bold()
    );
    Ok(CnbhData { tiles })
}

fn tile_file_name(x: i32, y: i32) -> String {
    format!("CNBH10m_X{x}Y{y}.tif")
}

fn tile_cells(bbox: &LLBBox) -> Result<Vec<(i32, i32)>, String> {
    let min_x = bbox.min().lng().floor() as i32;
    let max_x = bbox.max().lng().floor() as i32;
    let min_y = bbox.min().lat().floor() as i32;
    let max_y = bbox.max().lat().floor() as i32;
    let count_x = max_x.saturating_sub(min_x).saturating_add(1);
    let count_y = max_y.saturating_sub(min_y).saturating_add(1);
    let count = u64::from(count_x.max(0) as u32).saturating_mul(u64::from(count_y.max(0) as u32));
    if count as usize > MAX_FILES {
        return Err(format!(
            "the selection spans {count} CNBH 2-degree files; CNBH supports at most {MAX_FILES}. \
             Select a city or county, or a smaller rectangle."
        ));
    }
    let mut out = Vec::with_capacity(count as usize);
    for x in min_x..=max_x {
        for y in min_y..=max_y {
            // CNBH indexes its 2-degree rasters from the lower-left corner
            // using odd coordinates: X121Y31 covers 120..122 E and 30..32 N.
            out.push((
                x.div_euclid(DEGREES_PER_FILE) * DEGREES_PER_FILE + 1,
                y.div_euclid(DEGREES_PER_FILE) * DEGREES_PER_FILE + 1,
            ));
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

fn tile_url(x: i32, y: i32) -> String {
    FILE_TEMPLATE
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string())
}

fn download_tile(
    client: &Client,
    x: i32,
    y: i32,
    path: &Path,
    debug: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let part = path.with_extension("tif.part");
    let offset = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if debug {
        println!(
            "CNBH-10m: downloading {} ({} so far)",
            tile_file_name(x, y),
            crate::elevation::cache::format_size(offset)
        );
    }

    let _permit = crate::net::request_permit();
    let mut request = client.get(tile_url(x, y));
    if offset > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    let mut response = request.send()?;

    if response.status() == StatusCode::RANGE_NOT_SATISFIABLE && offset > 0 {
        drop(response);
        if read_metadata(&part).is_ok() {
            finish_download(&part, path)?;
            return Ok(());
        }
        let _ = fs::remove_file(&part);
        return Err("the server rejected the saved partial CNBH download".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "{} returned HTTP {}",
            tile_file_name(x, y),
            response.status().as_u16()
        )
        .into());
    }

    let resume = offset > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(resume)
        .truncate(!resume)
        .open(&part)?;
    let copied = std::io::copy(&mut response, &mut file)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    if copied == 0 && offset == 0 {
        return Err("the CNBH server returned an empty GeoTIFF".into());
    }
    if read_metadata(&part).is_err() {
        let _ = fs::remove_file(&part);
        return Err("the downloaded CNBH file is not a readable GeoTIFF".into());
    }
    finish_download(&part, path)?;
    Ok(())
}

fn finish_download(part: &Path, path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(part, path)
}

fn read_metadata(path: &Path) -> Result<CnbhTile, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let mut decoder = Decoder::new(BufReader::new(file))?;
    let (width, height) = decoder.dimensions()?;
    if width == 0 || height == 0 {
        return Err("the CNBH GeoTIFF has zero dimensions".into());
    }
    let scale = decoder.get_tag_f64_vec(Tag::ModelPixelScaleTag)?;
    let tie = decoder.get_tag_f64_vec(Tag::ModelTiepointTag)?;
    if scale.len() < 2 || tie.len() < 6 {
        return Err("the CNBH GeoTIFF lacks pixel scale or tiepoint tags".into());
    }
    let geo_keys = decoder.get_tag_u16_vec(Tag::GeoKeyDirectoryTag)?;
    let (utm_zone, south) =
        parse_utm_geo_keys(&geo_keys).ok_or("the CNBH GeoTIFF is not in a WGS 84 UTM zone")?;
    let tile_width = decoder
        .get_tag_u32(Tag::TileWidth)
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(256);
    let tile_height = decoder
        .get_tag_u32(Tag::TileLength)
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(tile_width);
    let tiles_across = width.div_ceil(tile_width);
    let tiles_down = height.div_ceil(tile_height);
    let expected_chunks = tiles_across
        .checked_mul(tiles_down)
        .ok_or("CNBH TIFF tile count overflow")?;
    let chunks = decoder.tile_count()?;
    if chunks < expected_chunks {
        return Err(format!(
            "CNBH GeoTIFF has {chunks} tiles but its dimensions need {expected_chunks}"
        )
        .into());
    }
    if !scale[0].is_finite() || !scale[1].is_finite() || scale[0] <= 0.0 || scale[1] <= 0.0 {
        return Err("the CNBH GeoTIFF has an invalid pixel scale".into());
    }
    Ok(CnbhTile {
        path: path.to_path_buf(),
        width,
        height,
        tile_width,
        tile_height,
        tiles_across,
        pixel_x: scale[0],
        pixel_y: scale[1],
        top_left_x: tie[3],
        top_left_y: tie[4],
        utm_zone,
        south,
    })
}

fn parse_utm_geo_keys(keys: &[u16]) -> Option<(u8, bool)> {
    if keys.len() < 4 {
        return None;
    }
    let count = usize::from(keys[3]);
    for entry in keys.get(4..4 + count * 4)?.chunks_exact(4) {
        if entry[0] != 3072 || entry[1] != 0 {
            continue;
        }
        let epsg = u32::from(entry[3]);
        if let Some(zone) = epsg
            .checked_sub(32_600)
            .filter(|zone| (1..=60).contains(zone))
        {
            return Some((zone as u8, false));
        }
        if let Some(zone) = epsg
            .checked_sub(32_700)
            .filter(|zone| (1..=60).contains(zone))
        {
            return Some((zone as u8, true));
        }
    }
    None
}

/// Fill CNBH heights on building ways that have no explicit height or levels.
/// The layer is additive: OSM, Overture, VIDA and measured survey tags win.
pub fn apply_heights(
    elements: &mut [ProcessedElement],
    data: CnbhData,
    bbox: &LLBBox,
    projection: ProjectionSpec,
    xzbbox: &XZBBox,
) -> usize {
    if data.tiles.is_empty() {
        return 0;
    }
    let mut assigned: HashMap<usize, f32> = HashMap::new();
    let mut failed_chunks = 0usize;
    for tile in &data.tiles {
        let mut grouped: HashMap<u32, Vec<Sample>> = HashMap::new();
        for (index, element) in elements.iter().enumerate() {
            if assigned.contains_key(&index) || !needs_cnbh_height(element) {
                continue;
            }
            let ProcessedElement::Way(way) = element else {
                continue;
            };
            let Some((x, z)) = polygon_centroid(&way.nodes) else {
                continue;
            };
            let Some((lat, lng)) = xz_to_geo(x + 0.5, z + 0.5, bbox, projection, xzbbox) else {
                continue;
            };
            if let Some(sample) = tile.sample_at(lat, lng) {
                grouped.entry(sample.chunk).or_default().push(Sample {
                    element: index,
                    local_x: sample.local_x,
                    local_y: sample.local_y,
                });
            }
        }
        if grouped.is_empty() {
            continue;
        }
        let file = match File::open(&tile.path) {
            Ok(file) => file,
            Err(_) => continue,
        };
        let mut decoder = match Decoder::new(BufReader::new(file)) {
            Ok(decoder) => decoder,
            Err(_) => continue,
        };
        let mut chunks: Vec<(u32, Vec<Sample>)> = grouped.into_iter().collect();
        chunks.sort_unstable_by_key(|(chunk, _)| *chunk);
        for (chunk, samples) in chunks {
            let decoded = match decoder.read_chunk(chunk) {
                Ok(decoded) => decoded,
                Err(_) => {
                    failed_chunks += 1;
                    continue;
                }
            };
            let DecodingResult::F32(values) = decoded else {
                failed_chunks += 1;
                continue;
            };
            let chunk_x = chunk % tile.tiles_across;
            let chunk_y = chunk / tile.tiles_across;
            let chunk_width = tile
                .tile_width
                .min(tile.width.saturating_sub(chunk_x * tile.tile_width));
            let chunk_height = tile
                .tile_height
                .min(tile.height.saturating_sub(chunk_y * tile.tile_height));
            if values.len() < (chunk_width * chunk_height) as usize {
                failed_chunks += 1;
                continue;
            }
            for sample in samples {
                if let Some(height) = sample_height(
                    &values,
                    chunk_width,
                    chunk_height,
                    sample.local_x,
                    sample.local_y,
                ) {
                    assigned.entry(sample.element).or_insert(height);
                }
            }
        }
    }
    for (index, height) in assigned.iter() {
        let Some(ProcessedElement::Way(way)) = elements.get_mut(*index) else {
            continue;
        };
        if way.tags.contains_key("height") || way.tags.contains_key("building:levels") {
            continue;
        }
        way.tags
            .insert("height".to_string(), format!("{height:.1}"));
        way.tags
            .insert("arnis:height_source".to_string(), HEIGHT_SOURCE.to_string());
    }
    if failed_chunks > 0 {
        eprintln!(
            "{} CNBH-10m: {failed_chunks} raster tile(s) could not be decoded.",
            "Warning:".yellow().bold()
        );
    }
    if !assigned.is_empty() {
        println!(
            "  Filled CNBH-10m heights on {} buildings",
            assigned.len().to_string().bright_white().bold()
        );
    }
    assigned.len()
}

fn needs_cnbh_height(element: &ProcessedElement) -> bool {
    let tags = element.tags();
    let Some(building) = tags.get("building") else {
        return false;
    };
    building != "no"
        && !tags.contains_key("height")
        && !tags.contains_key("building:levels")
        && !tags.contains_key("building:part")
}

fn polygon_centroid(nodes: &[crate::osm_parser::ProcessedNode]) -> Option<(f64, f64)> {
    if nodes.len() < 3 {
        return None;
    }
    let mut cross_sum = 0.0;
    let mut x_sum = 0.0;
    let mut z_sum = 0.0;
    for index in 0..nodes.len() {
        let a = &nodes[index];
        let b = &nodes[(index + 1) % nodes.len()];
        let cross = f64::from(a.x) * f64::from(b.z) - f64::from(b.x) * f64::from(a.z);
        cross_sum += cross;
        x_sum += (f64::from(a.x) + f64::from(b.x)) * cross;
        z_sum += (f64::from(a.z) + f64::from(b.z)) * cross;
    }
    if cross_sum.abs() > 1e-9 {
        return Some((x_sum / (3.0 * cross_sum), z_sum / (3.0 * cross_sum)));
    }
    let count = nodes.len() as f64;
    Some((
        nodes.iter().map(|node| f64::from(node.x)).sum::<f64>() / count,
        nodes.iter().map(|node| f64::from(node.z)).sum::<f64>() / count,
    ))
}

fn xz_to_geo(
    x: f64,
    z: f64,
    bbox: &LLBBox,
    projection: ProjectionSpec,
    xzbbox: &XZBBox,
) -> Option<(f64, f64)> {
    match projection.kind {
        ProjectionKind::Local => {
            let width = f64::from(xzbbox.max_x() - xzbbox.min_x() + 1);
            let height = f64::from(xzbbox.max_z() - xzbbox.min_z() + 1);
            if width <= 0.0 || height <= 0.0 {
                return None;
            }
            let rel_x = ((x - f64::from(xzbbox.min_x())) / width).clamp(0.0, 1.0);
            let rel_z = ((z - f64::from(xzbbox.min_z())) / height).clamp(0.0, 1.0);
            let lng = bbox.min().lng() + rel_x * (bbox.max().lng() - bbox.min().lng());
            let lat = bbox.max().lat() - rel_z * (bbox.max().lat() - bbox.min().lat());
            Some((lat, lng))
        }
        ProjectionKind::WebMercator => {
            let mercator = projection.mercator(bbox);
            Some((mercator.lat_for_z(z), mercator.lon_for_x(x)))
        }
    }
}

struct Sample {
    element: usize,
    local_x: u32,
    local_y: u32,
}

struct PixelSample {
    chunk: u32,
    local_x: u32,
    local_y: u32,
}

impl CnbhTile {
    fn sample_at(&self, lat: f64, lng: f64) -> Option<PixelSample> {
        let (easting, northing) = utm_from_wgs84(lat, lng, self.utm_zone, self.south)?;
        let px = (easting - self.top_left_x) / self.pixel_x;
        let py = (self.top_left_y - northing) / self.pixel_y;
        if !px.is_finite()
            || !py.is_finite()
            || px < 0.0
            || py < 0.0
            || px >= f64::from(self.width)
            || py >= f64::from(self.height)
        {
            return None;
        }
        let image_x = px.floor() as u32;
        let image_y = py.floor() as u32;
        let chunk_x = image_x / self.tile_width;
        let chunk_y = image_y / self.tile_height;
        Some(PixelSample {
            chunk: chunk_y * self.tiles_across + chunk_x,
            local_x: image_x % self.tile_width,
            local_y: image_y % self.tile_height,
        })
    }
}

fn sample_height(values: &[f32], width: u32, height: u32, x: u32, y: u32) -> Option<f32> {
    let mut candidates = Vec::with_capacity(9);
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let px = i64::from(x) + i64::from(dx);
            let py = i64::from(y) + i64::from(dy);
            if px < 0 || py < 0 || px >= i64::from(width) || py >= i64::from(height) {
                continue;
            }
            let value = values[py as usize * width as usize + px as usize];
            if value.is_finite() && (MIN_HEIGHT_M..=MAX_HEIGHT_M).contains(&value) {
                candidates.push(value);
            }
        }
    }
    candidates.sort_by(f32::total_cmp);
    candidates.get(candidates.len() / 2).copied()
}

/// WGS 84 latitude/longitude to UTM easting/northing. CNBH is projected in
/// UTM, while every Arnis input is in WGS 84.
fn utm_from_wgs84(lat: f64, lng: f64, zone: u8, south: bool) -> Option<(f64, f64)> {
    if !(1..=60).contains(&zone)
        || !(-80.0..=84.0).contains(&lat)
        || !(-180.0..=180.0).contains(&lng)
    {
        return None;
    }
    const A: f64 = 6_378_137.0;
    const E2: f64 = 0.006_694_379_990_141_317;
    const K0: f64 = 0.9996;
    let ep2 = E2 / (1.0 - E2);
    let lat_rad = lat.to_radians();
    let lon_rad = lng.to_radians();
    let central_meridian = (f64::from(zone) * 6.0 - 183.0).to_radians();
    let n = A / (1.0 - E2 * lat_rad.sin().powi(2)).sqrt();
    let t = lat_rad.tan().powi(2);
    let c = ep2 * lat_rad.cos().powi(2);
    let a = (lon_rad - central_meridian).cos() * (lon_rad - central_meridian);
    let m = A
        * ((1.0 - E2 / 4.0 - 3.0 * E2.powi(2) / 64.0 - 5.0 * E2.powi(3) / 256.0) * lat_rad
            - (3.0 * E2 / 8.0 + 3.0 * E2.powi(2) / 32.0 + 45.0 * E2.powi(3) / 1024.0)
                * (2.0 * lat_rad).sin()
            + (15.0 * E2.powi(2) / 256.0 + 45.0 * E2.powi(3) / 1024.0) * (4.0 * lat_rad).sin()
            - (35.0 * E2.powi(3) / 3072.0) * (6.0 * lat_rad).sin());
    let easting = 500_000.0
        + K0 * n
            * (a + (1.0 - t + c) * a.powi(3) / 6.0
                + (5.0 - 18.0 * t + t.powi(2) + 72.0 * c - 58.0 * ep2) * a.powi(5) / 120.0);
    let mut northing = K0
        * (m + n
            * lat_rad.tan()
            * (a.powi(2) / 2.0
                + (5.0 - t + 9.0 * c + 4.0 * c.powi(2)) * a.powi(4) / 24.0
                + (61.0 - 58.0 * t + t.powi(2) + 600.0 * c - 330.0 * ep2) * a.powi(6) / 720.0));
    if south {
        northing += 10_000_000.0;
    }
    Some((easting, northing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osm_parser::{ProcessedNode, ProcessedWay};
    use std::collections::HashMap;

    fn bbox(text: &str) -> LLBBox {
        LLBBox::from_str(text).unwrap()
    }

    #[test]
    fn a_county_selects_only_its_two_degree_file() {
        assert_eq!(
            tile_cells(&bbox("31.42,121.08,31.48,121.18")).unwrap(),
            vec![(121, 31)]
        );
        assert_eq!(
            cache_files(Path::new("cache"), &bbox("31.42,121.08,31.48,121.18"))
                .unwrap()
                .last()
                .unwrap()
                .file_name()
                .unwrap(),
            "CNBH10m_X121Y31.tif"
        );
    }

    #[test]
    fn crossing_two_edges_selects_at_most_four_unique_files() {
        let cells = tile_cells(&bbox("31.9,121.9,32.1,122.1")).unwrap();
        assert_eq!(cells, vec![(121, 31), (121, 33), (123, 31), (123, 33)]);
        assert_eq!(cells.len(), 4);
    }

    #[test]
    fn unsupported_geo_keys_are_rejected() {
        // WGS 84 geographic coordinates, not a UTM projected CRS.
        let keys = [1, 1, 0, 1, 1024, 0, 1, 1];
        assert_eq!(parse_utm_geo_keys(&keys), None);
        let utm = [1, 1, 0, 1, 3072, 0, 1, 32_651];
        assert_eq!(parse_utm_geo_keys(&utm), Some((51, false)));
    }

    #[test]
    fn taicang_projects_into_the_zone_51_raster() {
        let (easting, northing) = utm_from_wgs84(31.45, 121.13, 51, false).unwrap();
        assert!((200_000.0..400_000.0).contains(&easting), "{easting}");
        assert!((3_400_000.0..3_600_000.0).contains(&northing), "{northing}");
    }

    #[test]
    fn the_centroid_of_a_square_is_its_middle() {
        let node = |x, z| ProcessedNode {
            id: 1,
            tags: HashMap::new(),
            x,
            z,
        };
        let nodes = vec![
            node(0, 0),
            node(10, 0),
            node(10, 10),
            node(0, 10),
            node(0, 0),
        ];
        assert_eq!(polygon_centroid(&nodes), Some((5.0, 5.0)));
    }

    #[test]
    fn explicit_heights_and_building_parts_are_not_replaced() {
        let base = |tags: &[(&str, &str)]| {
            ProcessedElement::Way(ProcessedWay {
                id: 1,
                nodes: Vec::new(),
                tags: tags
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect(),
            })
        };
        assert!(needs_cnbh_height(&base(&[("building", "yes")])));
        assert!(!needs_cnbh_height(&base(&[
            ("building", "yes"),
            ("height", "12")
        ])));
        assert!(!needs_cnbh_height(&base(&[
            ("building", "yes"),
            ("building:levels", "4")
        ])));
        assert!(!needs_cnbh_height(&base(&[
            ("building", "yes"),
            ("building:part", "yes")
        ])));
    }

    #[test]
    fn a_three_by_three_sample_ignores_nodata_and_outliers() {
        let values = [-9999.0, 6.0, 7.0, 6.5, 0.0, 65.0, 7.0, 8.0, -9999.0];
        assert_eq!(sample_height(&values, 3, 3, 1, 1), Some(7.0));
    }

    #[test]
    #[ignore = "integration test: set ARNIS_CNBH_TEST_TIFF to a real CNBH GeoTIFF"]
    fn reads_real_georeferencing_and_maps_a_china_coordinate_into_the_raster() {
        let path = std::env::var("ARNIS_CNBH_TEST_TIFF")
            .expect("ARNIS_CNBH_TEST_TIFF must name a CNBH GeoTIFF");
        let tile = read_metadata(Path::new(&path)).expect("read CNBH GeoTIFF metadata");
        assert!(tile.width > 10_000 && tile.height > 10_000);
        assert_eq!(tile.tile_width, 256);
        assert_eq!(tile.tile_height, 256);
        let sample = tile
            .sample_at(31.0, 101.0)
            .expect("a central China coordinate must fall inside X101Y31");
        assert!(sample.chunk < tile.tiles_across * tile.height.div_ceil(tile.tile_height));
    }

    #[test]
    #[ignore = "network integration test: downloads the Taicang CNBH GeoTIFF"]
    fn fetches_taicang_height_from_the_compressed_raster_block() {
        let bbox = bbox("31.42,121.08,31.48,121.18");
        let data = fetch_inner(&bbox, true).expect("Taicang CNBH fetch");
        let tile = data
            .tiles
            .first()
            .expect("Taicang must select one CNBH GeoTIFF");
        assert_eq!(
            tile.path.file_name().and_then(|name| name.to_str()),
            Some("CNBH10m_X121Y31.tif")
        );
        let sample = tile
            .sample_at(31.45, 121.13)
            .expect("Taicang must fall inside the selected raster");
        let file = File::open(&tile.path).expect("open the cached GeoTIFF");
        let mut decoder = Decoder::new(BufReader::new(file)).expect("open the TIFF decoder");
        let DecodingResult::F32(values) = decoder.read_chunk(sample.chunk).expect("read the block")
        else {
            panic!("the CNBH block must contain Float32 heights");
        };
        let chunk_x = sample.chunk % tile.tiles_across;
        let chunk_y = sample.chunk / tile.tiles_across;
        let chunk_width = tile
            .tile_width
            .min(tile.width.saturating_sub(chunk_x * tile.tile_width));
        let chunk_height = tile
            .tile_height
            .min(tile.height.saturating_sub(chunk_y * tile.tile_height));
        let height = sample_height(
            &values,
            chunk_width,
            chunk_height,
            sample.local_x,
            sample.local_y,
        )
        .expect("Taicang must have a plausible building height");
        assert!((MIN_HEIGHT_M..=MAX_HEIGHT_M).contains(&height), "{height}");
    }
}
