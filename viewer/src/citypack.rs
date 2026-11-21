//! CityPack v1: chunked binary CityDataset + distance-budgeted partition streamer.
//!
//! The pack holds 330k Seattle buildings as ~1.1k independently loadable
//! partitions. The runtime parses the directory once, then reads/spawns chunks
//! by camera distance within a per-frame time budget. Point resolution is
//! two-level: partition AABBs first, per-building footprints only inside the
//! candidate partitions (the old 1,000 m uniform grid is not used here).

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::camera::FlyCamera;

pub const PACK_PATH: &str = "assets/datasets/seattle.citypack";
pub const RADIUS_M: f32 = 2600.0;
/// Keeps the Seattle stream away from the Manhattan map's world origin.
pub const STREAM_WORLD_OFFSET: f32 = 60_000.0;
pub const LOADS_PER_FRAME: usize = 2;
pub const LOAD_BUDGET_MS: f32 = 6.0;
pub const UNLOADS_PER_FRAME: usize = 4;

#[derive(Clone, Debug)]
pub struct PartitionInfo {
    pub name: String,
    pub offset: u32,
    pub bytes: u32,
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
}

impl Chunk {
    pub fn string(&self, index: u8) -> &str {
        self.strings
            .get(index as usize)
            .map(String::as_str)
            .unwrap_or("unknown")
    }

    pub fn source(&self, building: &PackBuilding) -> &str {
        self.string(building.source_idx)
    }

    pub fn category(&self, building: &PackBuilding) -> &str {
        self.string(building.category_idx)
    }
}

#[derive(Clone, Debug)]
pub struct CityPack {
    pub path: PathBuf,
    pub partitions: Vec<PartitionInfo>,
    pub building_count: u32,
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

    fn f32(&mut self) -> Option<f32> {
        self.take(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn string(&mut self) -> Option<String> {
        let len = self.u16()? as usize;
        let raw = self.take(len)?;
        Some(String::from_utf8_lossy(raw).to_string())
    }
}

impl CityPack {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        let mut header = [0u8; 22];
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
        if version != 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unsupported CityPack version {version}"),
            ));
        }
        let header_bytes = cursor.u32().unwrap_or(0);
        let partition_count = cursor.u32().unwrap_or(0);
        let building_count = cursor.u32().unwrap_or(0);
        let mut directory = vec![0u8; header_bytes as usize];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut directory)?;
        let mut cursor = Cursor::new(&directory);
        cursor.pos = 22;
        let mut partitions = Vec::with_capacity(partition_count as usize);
        for _ in 0..partition_count {
            let offset = cursor.u32().ok_or_else(bad_pack)?;
            let bytes = cursor.u32().ok_or_else(bad_pack)?;
            let name_len = cursor.u8().ok_or_else(bad_pack)? as usize;
            let name = cursor
                .take(name_len)
                .map(|raw| String::from_utf8_lossy(raw).to_string())
                .ok_or_else(bad_pack)?;
            partitions.push(PartitionInfo {
                name,
                offset,
                bytes,
            });
        }
        Ok(Self {
            path,
            partitions,
            building_count,
        })
    }

    pub fn read_center(&self, index: usize) -> std::io::Result<[f32; 2]> {
        let info = self
            .partitions
            .get(index)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "partition"))?;
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(info.offset as u64 + 8))?;
        let mut bytes = [0u8; 8];
        file.read_exact(&mut bytes)?;
        Ok([
            f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        ])
    }

    pub fn read_chunk(&self, index: usize) -> std::io::Result<Chunk> {
        let info = self
            .partitions
            .get(index)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "partition"))?;
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(info.offset as u64))?;
        let mut bytes = vec![0u8; info.bytes as usize];
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
    let count = cursor.u32().ok_or_else(bad_pack)?;
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
        strings.push(cursor.string().ok_or_else(bad_pack)?);
    }
    let mut buildings = Vec::with_capacity(count as usize);
    for _ in 0..count {
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
    Ok(Chunk {
        center,
        bounds,
        hints,
        strings,
        buildings,
    })
}

/// Two-level point resolver over loaded partitions. Partition AABBs first, then
/// per-building footprints inside candidates. Mirrors the JIT resolver rules:
/// inside-first, nearest centroid, then nearest edge within 1 m, then id.
#[derive(Default)]
pub struct PartitionLookup {
    pub partitions: Vec<(String, [f32; 4], Vec<PackBuilding>)>,
}

impl PartitionLookup {
    pub fn insert(&mut self, name: String, bounds: [f32; 4], buildings: Vec<PackBuilding>) {
        self.partitions.push((name, bounds, buildings));
    }

    pub fn resolve_point(&self, point: [f32; 2]) -> Option<u32> {
        let mut hits: Vec<(u32, f32)> = Vec::new();
        let mut edges: Vec<(u32, f32)> = Vec::new();
        for (_name, bounds, buildings) in &self.partitions {
            if point[0] < bounds[0]
                || point[0] > bounds[2]
                || point[1] < bounds[1]
                || point[1] > bounds[3]
            {
                continue;
            }
            for building in buildings {
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

/// Pure frame planner: wanted partitions by centre distance against a budget.
pub fn wanted_partitions(
    center: [f32; 2],
    radius: f32,
    partitions: &[PartitionInfo],
    centers: &[[f32; 2]],
) -> Vec<usize> {
    let radius2 = radius * radius;
    let mut wanted = partitions
        .iter()
        .enumerate()
        .filter_map(|(index, _)| {
            let c = centers.get(index)?;
            let dx = c[0] - center[0];
            let dz = c[1] - center[1];
            (dx * dx + dz * dz <= radius2).then_some((index, dx * dx + dz * dz))
        })
        .collect::<Vec<_>>();
    wanted.sort_by(|a, b| a.1.total_cmp(&b.1));
    wanted.into_iter().map(|(index, _)| index).collect()
}

// --------------------------------------------------------------------------
// Runtime streaming
// --------------------------------------------------------------------------

#[derive(Component)]
pub struct StreamHud;

#[derive(Component)]
pub struct StreamText;

#[derive(Resource)]
pub struct StreamState {
    pub pack: CityPack,
    pub centers: Vec<[f32; 2]>,
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
}

pub struct CityPackPlugin;

impl Plugin for CityPackPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_stream).add_systems(
            Update,
            (stream_flight, stream_partitions, stream_hud, stream_capture).chain(),
        );
    }
}

pub fn stream_enabled() -> bool {
    std::env::var_os("SEATTLE_STREAM").is_some()
}

fn setup_stream(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
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
        "STREAM: pack {} partitions, {} buildings",
        pack.partitions.len(),
        pack.building_count
    );
    let mut centers = Vec::with_capacity(pack.partitions.len());
    for index in 0..pack.partitions.len() {
        match pack.read_center(index) {
            Ok(center) => centers.push(center),
            Err(error) => {
                error!("STREAM: partition {index}: {error}");
                centers.push([0.0, 0.0]);
            }
        }
    }
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.82,
        metallic: 0.0,
        cull_mode: None,
        ..default()
    });
    let _ = &mut meshes;
    commands.insert_resource(StreamState {
        pack,
        centers,
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

fn stream_flight(
    time: Res<Time>,
    state: Option<Res<StreamState>>,
    mut camera: Query<&mut Transform, With<FlyCamera>>,
) {
    if state.is_none() || !stream_enabled() {
        return;
    }
    let elapsed = time.elapsed_secs() - 1.5;
    let duration = 34.0_f32;
    let t = (elapsed / duration).clamp(0.0, 1.0);
    // Downtown (0, 0) -> Bellevue CBD (~10.1 km east, 0.4 km north).
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
    time: Res<Time>,
    camera: Query<&Transform, With<FlyCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(state) = state.as_mut() else {
        return;
    };
    if !stream_enabled() {
        return;
    }
    let _ = time;
    let Ok(transform) = camera.single() else {
        return;
    };
    let center = [
        transform.translation.x - STREAM_WORLD_OFFSET,
        transform.translation.z,
    ];
    let wanted = wanted_partitions(center, state.radius, &state.pack.partitions, &state.centers);
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
            state.last_event = format!("- {}", state.pack.partitions[index].name);
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
                        Name::new(format!("Partition {}", state.pack.partitions[index].name)),
                    ))
                    .id();
                state.loaded_buildings += chunk.buildings.len();
                state.loaded_counts.insert(index, chunk.buildings.len());
                state.loaded.insert(index, entity);
                state.total_loads += 1;
                state.last_event = format!("+ {}", state.pack.partitions[index].name);
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
    **text = format!(
        "STREAM  {}/{} partitions  {} buildings  max {}\ntotal +{} / -{}   last {}   last-frame {:.2} ms   radius {:.1} km",
        state.loaded.len(),
        state.pack.partitions.len(),
        state.loaded_buildings,
        state.max_loaded,
        state.total_loads,
        state.total_unloads,
        state.last_event,
        state.last_ms,
        state.radius / 1000.0
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
    if !stream_enabled() {
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
            pack.partitions.len(),
            expected["partitions"].as_u64().unwrap() as usize
        );
        assert_eq!(
            pack.building_count,
            expected["buildings"].as_u64().unwrap() as u32
        );
        let names = expected["names"].as_array().unwrap();
        for (index, name) in names.iter().enumerate() {
            assert_eq!(pack.partitions[index].name, name.as_str().unwrap());
        }
        let chunk = pack.read_chunk(0).expect("chunk");
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
    fn at_scale_lookup_matches_bruteforce() {
        let pack = CityPack::open(full_path()).expect("full pack");
        let mut lookup = PartitionLookup::default();
        for index in 0..pack.partitions.len() {
            let chunk = pack.read_chunk(index).expect("chunk");
            lookup.insert(
                pack.partitions[index].name.clone(),
                chunk.bounds,
                chunk.buildings.clone(),
            );
        }
        let all: Vec<&PackBuilding> = lookup
            .partitions
            .iter()
            .flat_map(|(_, _, buildings)| buildings.iter())
            .collect();
        assert!(all.len() > 300_000);
        let mut checked = 0;
        for (index, building) in all.iter().enumerate().step_by(all.len() / 40) {
            let point = [
                building.centroid[0] + (index % 7) as f32 * 0.4 - 1.2,
                building.centroid[1] + (index % 5) as f32 * 0.4 - 0.8,
            ];
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
        assert!(checked >= 40);
    }

    #[test]
    fn budget_planner_is_distance_ordered_and_bounded() {
        let partitions = (0..10)
            .map(|index| PartitionInfo {
                name: format!("p{index}"),
                offset: 0,
                bytes: 0,
            })
            .collect::<Vec<_>>();
        let centers = (0..10)
            .map(|index| [index as f32 * 300.0, 0.0])
            .collect::<Vec<_>>();
        let wanted = wanted_partitions([0.0, 0.0], 800.0, &partitions, &centers);
        assert_eq!(wanted, vec![0, 1, 2]);
        let wider = wanted_partitions([0.0, 0.0], 10_000.0, &partitions, &centers);
        assert_eq!(wider.len(), 10);
        assert!(LOADS_PER_FRAME < wider.len());
    }
}
