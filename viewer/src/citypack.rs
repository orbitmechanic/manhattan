//! CityPack v2: hierarchical (county -> city -> neighborhood -> leaf) binary
//! CityDataset plus a top-down, distance-budgeted streamer.
//!
//! The pack holds 330k buildings and 29.8k street segments in 1,096
//! neighborhoods nested under cities and counties. Streaming and point
//! resolution both descend the hierarchy; an out-of-range county terminates its
//! subtree before cities or neighborhoods are enumerated.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::camera::FlyCamera;
use crate::minimap::MapZoom;

pub const PACK_PATH: &str = "assets/datasets/seattle.citypack";
pub const RADIUS_M: f32 = 2600.0;
pub const STREAM_WORLD_OFFSET: f32 = 60_000.0;
pub const LOADS_PER_FRAME: usize = 2;
pub const LOAD_BUDGET_MS: f32 = 6.0;
pub const UNLOADS_PER_FRAME: usize = 4;

pub const LEVEL_COUNTY: u8 = 0;
pub const LEVEL_CITY: u8 = 1;
pub const LEVEL_NEIGHBORHOOD: u8 = 2;

#[derive(Clone, Debug, Default)]
pub struct PackNode {
    pub level: u8,
    pub parent: i32,
    pub first_child: u32,
    pub child_count: u16,
    pub buildings: u32,
    pub streets: u32,
    pub center: [f32; 2],
    pub bounds: [f32; 4],
    pub mean_height_m: f32,
    pub p95_height_m: f32,
    pub glass_pct: u8,
    pub name: String,
    pub chunk_offset: u32,
    pub chunk_bytes: u32,
}

impl PackNode {
    pub fn children(&self) -> std::ops::Range<usize> {
        self.first_child as usize..(self.first_child as usize + self.child_count as usize)
    }

    pub fn leaf(&self) -> bool {
        self.level == LEVEL_NEIGHBORHOOD
    }
}

#[derive(Clone, Debug, Default)]
pub struct PackBuilding {
    pub id: u32,
    pub centroid: [f32; 2],
    pub height_m: f32,
    pub floor_height_m: f32,
    pub base_elevation_m: f32,
    pub floor_count: u16,
    pub year_built: u16,
    pub levels: u8,
    pub source_idx: u8,
    pub category_idx: u8,
    pub perimeter: Vec<[f32; 2]>,
}

#[derive(Clone, Debug, Default)]
pub struct PackStreet {
    pub id: u32,
    pub kind_idx: u8,
    pub name_idx: u16,
    pub points: Vec<[f32; 2]>,
}

#[derive(Clone, Debug, Default)]
pub struct ChunkHints {
    pub dominant_category: u8,
    pub dominant_source: u8,
    pub mean_height_m: f32,
    pub p95_height_m: f32,
    pub glass_pct: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Chunk {
    pub center: [f32; 2],
    pub bounds: [f32; 4],
    pub hints: ChunkHints,
    pub strings: Vec<String>,
    pub buildings: Vec<PackBuilding>,
    pub streets: Vec<PackStreet>,
}

impl Chunk {
    pub fn string(&self, index: u16) -> &str {
        self.strings
            .get(index as usize)
            .map(String::as_str)
            .unwrap_or("unknown")
    }

    pub fn source(&self, building: &PackBuilding) -> &str {
        self.string(building.source_idx as u16)
    }

    pub fn category(&self, building: &PackBuilding) -> &str {
        self.string(building.category_idx as u16)
    }

    pub fn street_name(&self, street: &PackStreet) -> &str {
        self.string(street.name_idx)
    }

    pub fn street_kind(&self, street: &PackStreet) -> &str {
        self.string(street.kind_idx as u16)
    }
}

#[derive(Clone, Debug)]
pub struct CityPack {
    pub path: PathBuf,
    pub nodes: Vec<PackNode>,
    pub building_count: u32,
    pub street_count: u32,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(count)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }

    fn i16(&mut self) -> Option<i16> {
        self.take(2).map(|b| i16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> Option<i32> {
        self.take(4)
            .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f32(&mut self) -> Option<f32> {
        self.take(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn string16(&mut self) -> Option<String> {
        let len = self.u16()? as usize;
        let raw = self.take(len)?;
        Some(String::from_utf8_lossy(raw).to_string())
    }

    fn string8(&mut self) -> Option<String> {
        let len = self.u8()? as usize;
        let raw = self.take(len)?;
        Some(String::from_utf8_lossy(raw).to_string())
    }
}

impl CityPack {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        let mut header = [0u8; 26];
        file.read_exact(&mut header)?;
        if &header[0..4] != b"CDPK" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bad CityPack magic",
            ));
        }
        let mut cursor = Cursor::new(&header);
        cursor.pos = 4;
        let version = cursor.u16().unwrap_or(0);
        if version != 2 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unsupported CityPack version {version}"),
            ));
        }
        let header_bytes = cursor.u32().unwrap_or(0);
        let node_count = cursor.u32().unwrap_or(0);
        let building_count = cursor.u32().unwrap_or(0);
        let street_count = cursor.u32().unwrap_or(0);
        let _chunk_count = cursor.u32().unwrap_or(0);
        let mut directory = vec![0u8; header_bytes as usize];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut directory)?;
        let mut cursor = Cursor::new(&directory);
        cursor.pos = 26;
        let mut nodes = Vec::with_capacity(node_count as usize);
        for _ in 0..node_count {
            let level = cursor.u8().ok_or_else(bad_pack)?;
            let parent = cursor.i32().ok_or_else(bad_pack)?;
            let first_child = cursor.u32().ok_or_else(bad_pack)?;
            let child_count = cursor.u16().ok_or_else(bad_pack)?;
            let buildings = cursor.u32().ok_or_else(bad_pack)?;
            let streets = cursor.u32().ok_or_else(bad_pack)?;
            let center = [
                cursor.f32().ok_or_else(bad_pack)?,
                cursor.f32().ok_or_else(bad_pack)?,
            ];
            let bounds = [
                cursor.f32().ok_or_else(bad_pack)?,
                cursor.f32().ok_or_else(bad_pack)?,
                cursor.f32().ok_or_else(bad_pack)?,
                cursor.f32().ok_or_else(bad_pack)?,
            ];
            let mean_height_m = cursor.u16().ok_or_else(bad_pack)? as f32 / 100.0;
            let p95_height_m = cursor.u16().ok_or_else(bad_pack)? as f32 / 100.0;
            let glass_pct = cursor.u8().ok_or_else(bad_pack)?;
            let _dom_category = cursor.u8().ok_or_else(bad_pack)?;
            let _dom_source = cursor.u8().ok_or_else(bad_pack)?;
            let name = cursor.string8().ok_or_else(bad_pack)?;
            let chunk_offset = cursor.u32().ok_or_else(bad_pack)?;
            let chunk_bytes = cursor.u32().ok_or_else(bad_pack)?;
            nodes.push(PackNode {
                level,
                parent,
                first_child,
                child_count,
                buildings,
                streets,
                center,
                bounds,
                mean_height_m,
                p95_height_m,
                glass_pct,
                name,
                chunk_offset,
                chunk_bytes,
            });
        }
        Ok(Self {
            path,
            nodes,
            building_count,
            street_count,
        })
    }

    pub fn neighborhoods(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_index, node)| node.level == LEVEL_NEIGHBORHOOD)
            .map(|(index, _node)| index)
            .collect()
    }

    pub fn read_chunk(&self, node_index: usize) -> std::io::Result<Chunk> {
        let node = self
            .nodes
            .get(node_index)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "node"))?;
        if !node.leaf() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a neighborhood",
            ));
        }
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(node.chunk_offset as u64))?;
        let mut bytes = vec![0u8; node.chunk_bytes as usize];
        file.read_exact(&mut bytes)?;
        parse_chunk(&bytes)
    }
}

fn bad_pack() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, "truncated CityPack")
}

fn parse_chunk(bytes: &[u8]) -> std::io::Result<Chunk> {
    let mut cursor = Cursor::new(bytes);
    let _chunk_bytes = cursor.u32().ok_or_else(bad_pack)?;
    let building_count = cursor.u32().ok_or_else(bad_pack)?;
    let street_count = cursor.u32().ok_or_else(bad_pack)?;
    let center = [
        cursor.f32().ok_or_else(bad_pack)?,
        cursor.f32().ok_or_else(bad_pack)?,
    ];
    let bounds = [
        cursor.f32().ok_or_else(bad_pack)?,
        cursor.f32().ok_or_else(bad_pack)?,
        cursor.f32().ok_or_else(bad_pack)?,
        cursor.f32().ok_or_else(bad_pack)?,
    ];
    let hints = ChunkHints {
        dominant_category: cursor.u8().ok_or_else(bad_pack)?,
        dominant_source: cursor.u8().ok_or_else(bad_pack)?,
        mean_height_m: cursor.u16().ok_or_else(bad_pack)? as f32 / 100.0,
        p95_height_m: cursor.u16().ok_or_else(bad_pack)? as f32 / 100.0,
        glass_pct: cursor.u8().ok_or_else(bad_pack)?,
    };
    let _reserved = cursor.u8().ok_or_else(bad_pack)?;
    let string_count = cursor.u16().ok_or_else(bad_pack)?;
    let mut strings = Vec::with_capacity(string_count as usize);
    for _ in 0..string_count {
        strings.push(cursor.string16().ok_or_else(bad_pack)?);
    }
    let mut buildings = Vec::with_capacity(building_count as usize);
    for _ in 0..building_count {
        let id = cursor.u32().ok_or_else(bad_pack)?;
        let qx = cursor.i16().ok_or_else(bad_pack)?;
        let qz = cursor.i16().ok_or_else(bad_pack)?;
        let height_cm = cursor.u16().ok_or_else(bad_pack)?;
        let floor_height_cm = cursor.u16().ok_or_else(bad_pack)?;
        let base_cm = cursor.u16().ok_or_else(bad_pack)?;
        let floor_count = cursor.u16().ok_or_else(bad_pack)?;
        let year_built = cursor.u16().ok_or_else(bad_pack)?;
        let levels = cursor.u8().ok_or_else(bad_pack)?;
        let source_idx = cursor.u8().ok_or_else(bad_pack)?;
        let category_idx = cursor.u8().ok_or_else(bad_pack)?;
        let vertex_count = cursor.u16().ok_or_else(bad_pack)?;
        let centroid = [center[0] + qx as f32 * 0.25, center[1] + qz as f32 * 0.25];
        let mut perimeter = Vec::with_capacity(vertex_count as usize);
        for _ in 0..vertex_count {
            let vx = cursor.i16().ok_or_else(bad_pack)?;
            let vz = cursor.i16().ok_or_else(bad_pack)?;
            perimeter.push([
                centroid[0] + vx as f32 * 0.25,
                centroid[1] + vz as f32 * 0.25,
            ]);
        }
        buildings.push(PackBuilding {
            id,
            centroid,
            height_m: height_cm as f32 / 100.0,
            floor_height_m: floor_height_cm as f32 / 100.0,
            base_elevation_m: base_cm as f32 / 100.0,
            floor_count,
            year_built,
            levels,
            source_idx,
            category_idx,
            perimeter,
        });
    }
    let mut streets = Vec::with_capacity(street_count as usize);
    for _ in 0..street_count {
        let id = cursor.u32().ok_or_else(bad_pack)?;
        let kind_idx = cursor.u8().ok_or_else(bad_pack)?;
        let name_idx = cursor.u16().ok_or_else(bad_pack)?;
        let point_count = cursor.u16().ok_or_else(bad_pack)?;
        let mut points = Vec::with_capacity(point_count as usize);
        for _ in 0..point_count {
            let vx = cursor.i16().ok_or_else(bad_pack)?;
            let vz = cursor.i16().ok_or_else(bad_pack)?;
            points.push([center[0] + vx as f32 * 0.25, center[1] + vz as f32 * 0.25]);
        }
        streets.push(PackStreet {
            id,
            kind_idx,
            name_idx,
            points,
        });
    }
    Ok(Chunk {
        center,
        bounds,
        hints,
        strings,
        buildings,
        streets,
    })
}

// --------------------------------------------------------------------------
// Top-down hierarchy traversal
// --------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct Evaluations {
    pub counties: usize,
    pub cities: usize,
    pub neighborhoods: usize,
}

impl Evaluations {
    pub fn total(self) -> usize {
        self.counties + self.cities + self.neighborhoods
    }
}

pub fn aabb_distance2(bounds: &[f32; 4], point: [f32; 2]) -> f32 {
    let dx = (bounds[0] - point[0]).max(0.0).max(point[0] - bounds[2]);
    let dz = (bounds[1] - point[1]).max(0.0).max(point[1] - bounds[3]);
    dx * dx + dz * dz
}

/// Descend county -> city -> neighborhood and return the neighborhoods whose
/// AABB is within `radius`. Every distance test is counted per level.
pub fn wanted_neighborhoods(
    nodes: &[PackNode],
    center: [f32; 2],
    radius: f32,
    evaluations: &mut Evaluations,
) -> Vec<usize> {
    let radius2 = radius * radius;
    let mut out = Vec::new();
    for county in nodes.iter().filter(|node| node.level == LEVEL_COUNTY) {
        evaluations.counties += 1;
        if aabb_distance2(&county.bounds, center) > radius2 {
            continue;
        }
        for city_index in county.children() {
            let Some(city) = nodes.get(city_index) else {
                continue;
            };
            if city.level != LEVEL_CITY {
                continue;
            }
            evaluations.cities += 1;
            if aabb_distance2(&city.bounds, center) > radius2 {
                continue;
            }
            for neighborhood_index in city.children() {
                let Some(neighborhood) = nodes.get(neighborhood_index) else {
                    continue;
                };
                if neighborhood.level != LEVEL_NEIGHBORHOOD {
                    continue;
                }
                evaluations.neighborhoods += 1;
                if aabb_distance2(&neighborhood.bounds, center) <= radius2 {
                    out.push(neighborhood_index);
                }
            }
        }
    }
    out
}

/// Four-level point resolver over loaded neighborhoods. Equality with a
/// brute-force footprint scan is unit-tested.
#[derive(Default)]
pub struct HierarchyLookup {
    pub nodes: Vec<PackNode>,
    pub chunks: HashMap<usize, Chunk>,
}

impl HierarchyLookup {
    pub fn from_pack(pack: &CityPack) -> std::io::Result<Self> {
        let mut chunks = HashMap::new();
        for index in pack.neighborhoods() {
            chunks.insert(index, pack.read_chunk(index)?);
        }
        Ok(Self {
            nodes: pack.nodes.clone(),
            chunks,
        })
    }

    pub fn resolve_point(&self, point: [f32; 2]) -> Option<u32> {
        let mut hits: Vec<(u32, f32)> = Vec::new();
        let mut edges: Vec<(u32, f32)> = Vec::new();
        for county in self.nodes.iter().filter(|node| node.level == LEVEL_COUNTY) {
            if !contains(&county.bounds, point) {
                continue;
            }
            for city_index in county.children() {
                let Some(city) = self.nodes.get(city_index) else {
                    continue;
                };
                if !contains(&city.bounds, point) {
                    continue;
                }
                for neighborhood_index in city.children() {
                    let Some(neighborhood) = self.nodes.get(neighborhood_index) else {
                        continue;
                    };
                    if !contains(&neighborhood.bounds, point) {
                        continue;
                    }
                    let Some(chunk) = self.chunks.get(&neighborhood_index) else {
                        continue;
                    };
                    for building in &chunk.buildings {
                        if destructibles_core::math::point_in_polygon(point, &building.perimeter) {
                            let dx = point[0] - building.centroid[0];
                            let dz = point[1] - building.centroid[1];
                            hits.push((building.id, (dx * dx + dz * dz).sqrt()));
                        } else {
                            let edge = destructibles_core::math::distance_to_polygon_edges(
                                point,
                                &building.perimeter,
                            );
                            if edge <= 1.0 {
                                edges.push((building.id, edge));
                            }
                        }
                    }
                }
            }
        }
        let mut pool = if hits.is_empty() {
            edges
                .into_iter()
                .map(|(id, edge)| (id, edge * 4.0))
                .collect::<Vec<_>>()
        } else {
            hits
        };
        pool.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        pool.first().map(|(id, _)| *id)
    }
}

fn contains(bounds: &[f32; 4], point: [f32; 2]) -> bool {
    point[0] >= bounds[0] && point[0] <= bounds[2] && point[1] >= bounds[1] && point[1] <= bounds[3]
}

// --------------------------------------------------------------------------
// Runtime streamer
// --------------------------------------------------------------------------

#[derive(Component)]
pub struct StreamHud;

#[derive(Component)]
pub struct StreamText;

#[derive(Resource)]
pub struct StreamState {
    pub pack: CityPack,
    pub neighborhoods: Vec<usize>,
    pub loaded: HashMap<usize, Entity>,
    pub loaded_counts: HashMap<usize, usize>,
    pub loaded_buildings: usize,
    pub material: Handle<StandardMaterial>,
    pub radius: f32,
    pub last_loads: usize,
    pub last_unloads: usize,
    pub last_ms: f32,
    pub total_loads: usize,
    pub total_unloads: usize,
    pub last_event: String,
    pub max_loaded: usize,
    pub captures: u8,
    pub finished: bool,
    pub last_evaluations: Evaluations,
}

/// Hierarchy metadata for the minimap and reports.
#[derive(Resource, Clone, Default)]
pub struct HierarchyMap {
    pub nodes: Vec<PackNode>,
}

pub struct CityPackPlugin;

impl Plugin for CityPackPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_stream).add_systems(
            Update,
            (
                stream_flight,
                stream_partitions,
                stream_hud,
                stream_capture,
                map_capture,
            )
                .chain(),
        );
    }
}

pub fn stream_enabled() -> bool {
    std::env::var_os("SEATTLE_STREAM").is_some()
}

fn setup_stream(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    if !stream_enabled() {
        return;
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(PACK_PATH);
    let pack = match CityPack::open(&path) {
        Ok(pack) => pack,
        Err(error) => {
            error!("STREAM: cannot open {}: {error}", path.display());
            return;
        }
    };
    info!(
        "STREAM: pack {} nodes, {} buildings, {} streets",
        pack.nodes.len(),
        pack.building_count,
        pack.street_count
    );
    let neighborhoods = pack.neighborhoods();
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.82,
        metallic: 0.0,
        cull_mode: None,
        ..default()
    });
    commands.insert_resource(HierarchyMap {
        nodes: pack.nodes.clone(),
    });
    commands.insert_resource(StreamState {
        pack,
        neighborhoods,
        loaded: HashMap::new(),
        loaded_counts: HashMap::new(),
        loaded_buildings: 0,
        material,
        radius: RADIUS_M,
        last_loads: 0,
        last_unloads: 0,
        last_ms: 0.0,
        total_loads: 0,
        total_unloads: 0,
        last_event: String::from("start"),
        max_loaded: 0,
        captures: 0,
        finished: false,
        last_evaluations: Evaluations::default(),
    });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                left: px(12),
                padding: UiRect::all(px(10)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.82)),
            StreamHud,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("STREAM starting"),
                TextFont {
                    font_size: bevy::text::FontSize::Px(15.0),
                    ..default()
                },
                TextColor(Color::srgb(0.75, 0.95, 0.85)),
                StreamText,
                StreamHud,
            ));
        });
}

pub fn ancestry_path(nodes: &[PackNode], index: usize) -> String {
    let mut parts = Vec::new();
    let mut current = Some(index);
    while let Some(node_index) = current {
        let Some(node) = nodes.get(node_index) else {
            break;
        };
        parts.push(node.name.clone());
        current = (node.parent >= 0).then_some(node.parent as usize);
    }
    parts.reverse();
    parts.join(" -> ")
}

fn stream_flight(
    time: Res<Time>,
    state: Option<Res<StreamState>>,
    mut camera: Query<&mut Transform, With<FlyCamera>>,
) {
    if state.is_none() || !stream_enabled() {
        return;
    }
    if std::env::var_os("MINIMAP_CAPTURE").is_some() || std::env::var_os("SOUTH_CAPTURE").is_some()
    {
        if let Ok(mut transform) = camera.single_mut() {
            let (position, look_offset) = if std::env::var_os("SOUTH_CAPTURE").is_some() {
                (
                    Vec3::new(STREAM_WORLD_OFFSET - 8_000.0, 500.0, 36_000.0),
                    Vec3::new(0.0, -350.0, 700.0),
                )
            } else {
                (
                    Vec3::new(STREAM_WORLD_OFFSET, 250.0, 0.0),
                    Vec3::new(900.0, -120.0, -40.0),
                )
            };
            transform.translation = position;
            let look = transform.translation + look_offset;
            let direction = (look - transform.translation).normalize_or_zero();
            transform.rotation = Quat::from_euler(
                EulerRot::YXZ,
                (-direction.x).atan2(-direction.z),
                direction.y.asin(),
                0.0,
            );
        }
        return;
    }
    let elapsed = time.elapsed_secs() - 1.5;
    let duration = 34.0_f32;
    let t = (elapsed / duration).clamp(0.0, 1.0);
    let start = Vec3::new(STREAM_WORLD_OFFSET, 250.0, 0.0);
    let end = Vec3::new(STREAM_WORLD_OFFSET + 10_100.0, 250.0, -420.0);
    let position = start.lerp(end, t);
    if let Ok(mut transform) = camera.single_mut() {
        transform.translation = position;
        let look = position + Vec3::new(700.0, -80.0, -20.0);
        let direction = (look - position).normalize_or_zero();
        transform.rotation = Quat::from_euler(
            EulerRot::YXZ,
            (-direction.x).atan2(-direction.z),
            direction.y.asin(),
            0.0,
        );
    }
}

fn stream_partitions(
    mut commands: Commands,
    mut state: Option<ResMut<StreamState>>,
    camera: Query<&Transform, With<FlyCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(state) = state.as_mut() else {
        return;
    };
    if !stream_enabled() {
        return;
    }
    let Ok(transform) = camera.single() else {
        return;
    };
    let center = [
        transform.translation.x - STREAM_WORLD_OFFSET,
        transform.translation.z,
    ];
    let mut evaluations = Evaluations::default();
    let wanted = wanted_neighborhoods(&state.pack.nodes, center, state.radius, &mut evaluations);
    state.last_evaluations = evaluations;
    let wanted_set: HashSet<usize> = wanted.iter().copied().collect();
    let mut unloads = 0;
    let stale: Vec<usize> = state
        .loaded
        .keys()
        .copied()
        .filter(|index| !wanted_set.contains(index))
        .take(UNLOADS_PER_FRAME)
        .collect();
    for index in stale {
        if let Some(entity) = state.loaded.remove(&index) {
            let count = state.loaded_counts.remove(&index).unwrap_or(0);
            state.loaded_buildings = state.loaded_buildings.saturating_sub(count);
            commands.entity(entity).despawn();
            state.total_unloads += 1;
            state.last_event = format!("- {}", ancestry_path(&state.pack.nodes, index));
            unloads += 1;
        }
    }
    let started = Instant::now();
    let mut loads = 0;
    for index in wanted {
        if state.loaded.contains_key(&index) {
            continue;
        }
        if loads >= LOADS_PER_FRAME || started.elapsed().as_secs_f32() * 1000.0 > LOAD_BUDGET_MS {
            break;
        }
        match state.pack.read_chunk(index) {
            Ok(chunk) => {
                let mesh = build_partition_mesh(&chunk);
                let handle = meshes.add(mesh);
                let entity = commands
                    .spawn((
                        Mesh3d(handle),
                        MeshMaterial3d(state.material.clone()),
                        Transform::from_xyz(STREAM_WORLD_OFFSET, 0.0, 0.0),
                        Name::new(ancestry_path(&state.pack.nodes, index)),
                    ))
                    .id();
                state.loaded_buildings += chunk.buildings.len();
                state.loaded_counts.insert(index, chunk.buildings.len());
                state.loaded.insert(index, entity);
                state.total_loads += 1;
                state.last_event = format!("+ {}", ancestry_path(&state.pack.nodes, index));
                loads += 1;
            }
            Err(error) => {
                error!("STREAM: load {index}: {error}");
            }
        }
    }
    state.last_loads = loads;
    state.last_unloads = unloads;
    state.last_ms = started.elapsed().as_secs_f32() * 1000.0;
    state.max_loaded = state.max_loaded.max(state.loaded.len());
}

fn stream_hud(state: Option<Res<StreamState>>, mut text: Query<&mut Text, With<StreamText>>) {
    let Some(state) = state else {
        return;
    };
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    text.0 = format!(
        "STREAM  {}/{} neighborhoods  {} buildings  max {}\ntotal +{} / -{}   last {}   evals {}/{}/{}   {:.2} ms",
        state.loaded.len(),
        state.neighborhoods.len(),
        state.loaded_buildings,
        state.max_loaded,
        state.total_loads,
        state.total_unloads,
        state.last_event,
        state.last_evaluations.counties,
        state.last_evaluations.cities,
        state.last_evaluations.neighborhoods,
        state.last_ms
    );
}

fn stream_capture(
    mut commands: Commands,
    time: Res<Time>,
    mut state: Option<ResMut<StreamState>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(state) = state.as_mut() else {
        return;
    };
    if !stream_enabled() || std::env::var_os("MINIMAP_CAPTURE").is_some() {
        return;
    }
    if std::env::var_os("SOUTH_CAPTURE").is_some() {
        let elapsed = time.elapsed_secs();
        if elapsed >= 3.0 && state.captures == 0 {
            state.captures = 1;
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("captures")
                .join("stream_far_south.png");
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
        }
        if elapsed > 4.5 {
            let _ = exit.write(AppExit::Success);
        }
        return;
    }
    let elapsed = time.elapsed_secs() - 1.5;
    let marks = [
        (8.0, "stream_downtown.png"),
        (20.0, "stream_lake.png"),
        (32.0, "stream_bellevue.png"),
    ];
    let next = state.captures as usize;
    if next >= marks.len() {
        if elapsed > 36.0 && !state.finished {
            state.finished = true;
            let _ = exit.write(AppExit::Success);
        }
        return;
    }
    let (mark, name) = marks[next];
    if elapsed >= mark {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("captures")
            .join(name);
        state.captures += 1;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
}

fn map_capture(
    mut commands: Commands,
    time: Res<Time>,
    mut zoom: ResMut<MapZoom>,
    mut state: Local<(u8, bool, f32, bool)>,
    mut exit: MessageWriter<AppExit>,
) {
    if std::env::var_os("MINIMAP_CAPTURE").is_none() || !stream_enabled() {
        return;
    }
    let elapsed = time.elapsed_secs();
    if state.3 {
        if elapsed > state.2 + 1.0 {
            let _ = exit.write(AppExit::Success);
        }
        return;
    }
    let marks = [
        (1.6_f32, "minimap_neighborhood.png"),
        (3.4, "minimap_city.png"),
        (5.2, "minimap_county.png"),
    ];
    let index = state.0 as usize;
    if index >= marks.len() {
        state.3 = true;
        state.2 = elapsed;
        return;
    }
    let (mark, name) = marks[index];
    if !state.1 {
        if elapsed >= mark {
            zoom.0 = index as u8;
            state.1 = true;
        }
        return;
    }
    if elapsed >= mark + 0.7 {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("captures")
            .join(name);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        state.0 += 1;
        state.1 = false;
        if state.0 as usize >= marks.len() {
            state.3 = true;
            state.2 = elapsed;
        }
    }
}

fn build_partition_mesh(chunk: &Chunk) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for building in &chunk.buildings {
        let base = building.base_elevation_m;
        let top = base + building.height_m.max(2.0);
        let category = chunk.category(building);
        let color = match category {
            "residential" => [0.62, 0.55, 0.46, 1.0],
            "industrial" => [0.42, 0.39, 0.34, 1.0],
            _ => [0.48, 0.52, 0.56, 1.0],
        };
        let lift = (building.height_m / 120.0).clamp(0.0, 0.35);
        let color = [
            (color[0] + lift).min(1.0),
            (color[1] + lift).min(1.0),
            (color[2] + lift).min(1.0),
            color[3],
        ];
        let perimeter = &building.perimeter;
        if perimeter.len() < 3 {
            continue;
        }
        let base_index = positions.len() as u32;
        for point in perimeter {
            positions.push([point[0], base, point[1]]);
        }
        for point in perimeter {
            positions.push([point[0], top, point[1]]);
        }
        for point in perimeter {
            let dx = point[0] - building.centroid[0];
            let dz = point[1] - building.centroid[1];
            let length = (dx * dx + dz * dz).sqrt().max(1e-4);
            normals.push([dx / length, 0.0, dz / length]);
            colors.push(color);
        }
        for point in perimeter {
            let dx = point[0] - building.centroid[0];
            let dz = point[1] - building.centroid[1];
            let length = (dx * dx + dz * dz).sqrt().max(1e-4);
            normals.push([dx / length, 0.0, dz / length]);
            colors.push(color);
        }
        let n = perimeter.len();
        for index in 0..n {
            let next = (index + 1) % n;
            let i0 = base_index + index as u32;
            let i1 = base_index + next as u32;
            let i2 = base_index + n as u32 + index as u32;
            let i3 = base_index + n as u32 + next as u32;
            indices.extend([i0, i1, i2, i1, i3, i2]);
        }
        let top_index = positions.len() as u32;
        for point in perimeter {
            positions.push([point[0], top, point[1]]);
            normals.push([0.0, 1.0, 0.0]);
            colors.push(color);
        }
        for index in 1..n - 1 {
            indices.extend([
                top_index,
                top_index + index as u32,
                top_index + index as u32 + 1,
            ]);
        }
    }
    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn fixture_path() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/datasets/seattle_test.citypack")
    }

    fn full_path() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(PACK_PATH)
    }

    #[test]
    fn fixture_round_trip_matches_expected_json() {
        let pack = CityPack::open(fixture_path()).expect("fixture pack");
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/datasets/seattle_test_expected.json"),
        )
        .expect("expected json");
        let expected: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            pack.nodes.len(),
            expected["nodes"].as_u64().unwrap() as usize
        );
        assert_eq!(
            pack.building_count,
            expected["buildings"].as_u64().unwrap() as u32
        );
        assert_eq!(
            pack.street_count,
            expected["streets"].as_u64().unwrap() as u32
        );
        let names = expected["names"].as_array().unwrap();
        let neighborhoods = pack.neighborhoods();
        for (index, name) in names.iter().enumerate() {
            assert_eq!(
                pack.nodes[neighborhoods[index]].name,
                name.as_str().unwrap()
            );
        }
        let chunk = pack.read_chunk(neighborhoods[0]).expect("chunk");
        let samples = expected["samples"].as_array().unwrap();
        for sample in samples {
            let id = sample["id"].as_u64().unwrap() as u32;
            let building = chunk
                .buildings
                .iter()
                .find(|building| building.id == id)
                .unwrap_or_else(|| panic!("building {id}"));
            let expected_height = sample["height_m"].as_f64().unwrap() as f32;
            assert!((building.height_m - expected_height).abs() < 0.011);
            let centroid = sample["centroid"].as_array().unwrap();
            assert!((building.centroid[0] - centroid[0].as_f64().unwrap() as f32).abs() < 0.26);
            assert!((building.centroid[1] - centroid[1].as_f64().unwrap() as f32).abs() < 0.26);
            let perimeter = sample["perimeter"].as_array().unwrap();
            assert_eq!(building.perimeter.len(), perimeter.len());
            for (quantized, original) in building.perimeter.iter().zip(perimeter) {
                let x = original[0].as_f64().unwrap() as f32;
                let z = original[1].as_f64().unwrap() as f32;
                assert!((quantized[0] - x).abs() < 0.26);
                assert!((quantized[1] - z).abs() < 0.26);
            }
        }
    }

    #[test]
    fn hierarchy_containment_and_paths() {
        let pack = CityPack::open(full_path()).expect("full pack");
        assert_eq!(
            pack.nodes
                .iter()
                .filter(|node| node.level == LEVEL_COUNTY)
                .count(),
            2
        );
        assert_eq!(
            pack.nodes
                .iter()
                .filter(|node| node.level == LEVEL_CITY)
                .count(),
            3
        );
        assert_eq!(
            pack.nodes
                .iter()
                .filter(|node| node.level == LEVEL_NEIGHBORHOOD)
                .count(),
            1096
        );
        for node in &pack.nodes {
            if node.parent < 0 {
                continue;
            }
            let parent = &pack.nodes[node.parent as usize];
            assert!(
                node.bounds[0] >= parent.bounds[0] - 0.011
                    && node.bounds[1] >= parent.bounds[1] - 0.011
                    && node.bounds[2] <= parent.bounds[2] + 0.011
                    && node.bounds[3] <= parent.bounds[3] + 0.011,
                "{} child {:?} parent {:?}",
                node.name,
                node.bounds,
                parent.bounds
            );
        }
        let city = pack
            .nodes
            .iter()
            .find(|node| node.name == "Seattle" && node.level == LEVEL_CITY)
            .unwrap();
        let neighborhood = pack.nodes[city.children().next().unwrap()].name.clone();
        let path = ancestry_path(&pack.nodes, city.children().next().unwrap());
        assert_eq!(path, format!("King -> Seattle -> {neighborhood}"));
    }

    #[test]
    fn top_down_lookup_equals_bruteforce() {
        let pack = CityPack::open(full_path()).expect("full pack");
        let lookup = HierarchyLookup::from_pack(&pack).expect("lookup");
        let all: Vec<&PackBuilding> = lookup
            .chunks
            .values()
            .flat_map(|chunk| chunk.buildings.iter())
            .collect();
        assert!(all.len() > 300_000);
        let mut checked = 0;
        for building in all.iter().step_by(all.len() / 50) {
            // Sample strictly inside the record. The resolver deliberately
            // snaps points within 1 m of an edge, so offset samples can never
            // be compared against a strict point-in-polygon brute force.
            // Concave footprints can put their centroid outside, so fall back
            // to the vertex mean and skip records with no interior sample.
            let mean = building
                .perimeter
                .iter()
                .fold([0.0_f64, 0.0_f64], |sum, point| {
                    [sum[0] + point[0] as f64, sum[1] + point[1] as f64]
                });
            let count = building.perimeter.len() as f64;
            let mean = [(mean[0] / count) as f32, (mean[1] / count) as f32];
            let Some(point) = [building.centroid, mean].into_iter().find(|point| {
                destructibles_core::math::point_in_polygon(*point, &building.perimeter)
            }) else {
                continue;
            };
            let resolved = lookup.resolve_point(point);
            let inside = all
                .iter()
                .filter(|candidate| {
                    destructibles_core::math::point_in_polygon(point, &candidate.perimeter)
                })
                .min_by(|a, b| {
                    let da =
                        (a.centroid[0] - point[0]).powi(2) + (a.centroid[1] - point[1]).powi(2);
                    let db =
                        (b.centroid[0] - point[0]).powi(2) + (b.centroid[1] - point[1]).powi(2);
                    da.total_cmp(&db).then_with(|| a.id.cmp(&b.id))
                })
                .map(|building| building.id);
            assert_eq!(resolved, inside, "point {point:?}");
            checked += 1;
        }
        assert!(checked >= 40, "only {checked} interior samples");
    }

    #[test]
    fn gated_descent_skips_out_of_range_subtrees() {
        let pack = CityPack::open(full_path()).expect("full pack");
        let mut evaluations = Evaluations::default();
        let wanted =
            wanted_neighborhoods(&pack.nodes, [9_000_000.0, 0.0], 2_600.0, &mut evaluations);
        assert!(wanted.is_empty());
        assert_eq!(evaluations.counties, 2, "both counties tested");
        assert_eq!(evaluations.cities, 0, "no city enumerated");
        assert_eq!(evaluations.neighborhoods, 0, "no neighborhood enumerated");
        let mut local = Evaluations::default();
        let wanted = wanted_neighborhoods(&pack.nodes, [0.0, 0.0], 2_600.0, &mut local);
        assert!(!wanted.is_empty());
        assert!(local.total() < pack.neighborhoods().len());
    }

    #[test]
    fn budget_planner_and_unload_cap() {
        let pack = CityPack::open(full_path()).expect("full pack");
        let mut evaluations = Evaluations::default();
        let wanted = wanted_neighborhoods(&pack.nodes, [0.0, 0.0], 1_000_000.0, &mut evaluations);
        assert_eq!(wanted.len(), pack.neighborhoods().len());
        let mut loaded = HashSet::new();
        for _frame in 0..3 {
            let next: Vec<usize> = wanted
                .iter()
                .copied()
                .filter(|index| !loaded.contains(index))
                .take(LOADS_PER_FRAME)
                .collect();
            for index in next {
                loaded.insert(index);
            }
        }
        assert_eq!(loaded.len(), 3 * LOADS_PER_FRAME);
        let mut far = Evaluations::default();
        let none = wanted_neighborhoods(&pack.nodes, [9_000_000.0, 0.0], 2_600.0, &mut far);
        assert!(none.is_empty());
        assert_eq!(far.cities, 0);
        let stale = loaded
            .iter()
            .copied()
            .take(UNLOADS_PER_FRAME)
            .collect::<Vec<_>>();
        for index in stale {
            loaded.remove(&index);
        }
        assert_eq!(loaded.len(), 3 * LOADS_PER_FRAME - UNLOADS_PER_FRAME);
    }
}
