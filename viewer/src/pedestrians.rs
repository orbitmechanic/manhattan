//! Seattle pedestrians: walkers on derived sidewalk bands.
//!
//! Seattle-realistic density: a compact downtown core at roughly half the
//! Manhattan peak, park hotspots derived from the park-regime trees (parks are
//! where Seattle actually walks), and a light sprinkle everywhere else.
//! Audited per capture as expected vs actual per district. Walkers
//! are zero-structure props with one shared mesh and tint variation; they
//! follow draped street polylines on a 2.2 m band beside the lane, pause and
//! resume, and pitch with the grade like the vehicles.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::time::Real;

use crate::camera::FlyCamera;
use crate::city_rhythm::CityFields;
use crate::citypack::{STREAM_WORLD_OFFSET, wanted_neighborhoods};
use crate::clock::SimClock;
use crate::geology::Geology;

const LOAD_M: f32 = 650.0;
const UNLOAD_M: f32 = 800.0;
const MAX_ACTIVE: usize = 500;
const SPAWN_PER_FRAME: usize = 32;
const BASE_PER_CHUNK: f32 = 26.0;
const BAND: f32 = 2.2;

fn hash01(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846C_A68F);
    value ^= value >> 16;
    (value & 0x00FF_FFFF) as f32 / 16_777_216.0
}
/// Downtown core window: full strength inside 800 m, zero past 1.7 km, half
/// the Manhattan peak per the player's lived comparison.
const CORE_INNER_M: f32 = 800.0;
const CORE_OUTER_M: f32 = 1_700.0;
const CORE_GAIN: f32 = 0.35;
/// Parks are the real Seattle walk. One park cell should comfortably clear a
/// plaza crowd, so the park term carries this gain over the base.
const PARK_GAIN: f32 = 1.4;
/// The two genuine downtown plaza crowds: Pike Place and the Westlake seam.
/// These are pedestrians, not scenery, and they are the only downtown points
/// that should ever read as crowded.
const PLAZAS: [([f32; 2], f32); 2] = [([-518.6, -399.4], 120.0), ([-138.4, -614.3], 90.0)];
const PLAZA_GAIN: f32 = 2.0;
/// Everything else: oddities, not fauna. This is correct at Seattle scale.
const SPRINKLE: f32 = 0.02;

/// One derived park hotspot: a dense cell of park-regime trees, in pack-local
/// metres, with 0..1 strength from its tree count.
#[derive(Clone, Copy, Debug)]
pub struct ParkSite {
    pub x: f32,
    pub z: f32,
    pub strength: f32,
}

/// Park walker field, built once from the tree dataset's park regime. The
/// polygons themselves live only in the offline tree build; the regime flag is
/// the runtime's evidence that a tree stands in a park.
#[derive(Resource, Default, Clone)]
pub struct ParkField {
    sites: Vec<ParkSite>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

const PARK_GRID_M: f32 = 400.0;
const PARK_RADIUS_M: f32 = 170.0;
const PARK_CELL_M: f32 = 120.0;

impl ParkField {
    pub fn sites(&self) -> usize {
        self.sites.len()
    }

    /// Strength at a point: the strongest park cell within its radius.
    pub fn density(&self, x: f32, z: f32) -> f32 {
        let gx = (x / PARK_GRID_M).floor() as i32;
        let gz = (z / PARK_GRID_M).floor() as i32;
        let mut best = 0.0f32;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(indices) = self.grid.get(&(gx + dx, gz + dz)) else {
                    continue;
                };
                for index in indices {
                    let site = self.sites[*index as usize];
                    let d = ((site.x - x).powi(2) + (site.z - z).powi(2)).sqrt();
                    if d > PARK_RADIUS_M * 2.0 {
                        continue;
                    }
                    let falloff = (-(d / PARK_RADIUS_M).powi(2)).exp();
                    best = best.max(site.strength * falloff);
                }
            }
        }
        best
    }

    /// Strongest site whose centre lies inside a partition's bounds.
    pub fn best_in_bounds(&self, bounds: [f32; 4]) -> Option<ParkSite> {
        self.sites
            .iter()
            .filter(|site| {
                site.x >= bounds[0]
                    && site.x <= bounds[2]
                    && site.z >= bounds[1]
                    && site.z <= bounds[3]
            })
            .max_by(|a, b| a.strength.total_cmp(&b.strength))
            .copied()
    }

    /// Strongest site on dry, elevated ground (park lawns, not the shoreline
    /// strip that sits at water level).
    pub fn best_dry(&self, dem: &crate::geology::Dem, min_height: f32) -> Option<ParkSite> {
        self.sites
            .iter()
            .filter(|site| dem.height(site.x, site.z) >= min_height)
            .max_by(|a, b| a.strength.total_cmp(&b.strength))
            .copied()
    }

    /// Nearest site inside `radius`, for camera anchoring.
    pub fn nearest(&self, x: f32, z: f32, radius: f32) -> Option<ParkSite> {
        self.sites
            .iter()
            .map(|site| {
                let d = ((site.x - x).powi(2) + (site.z - z).powi(2)).sqrt();
                (d, *site)
            })
            .filter(|(d, _)| *d <= radius)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, site)| site)
    }
}

fn build_park_field(
    trees: &crate::trees::TreeDataset,
    pack: &crate::citypack::CityPack,
) -> ParkField {
    let mut cells: HashMap<(i32, i32), (u32, f32, f32)> = HashMap::new();
    for index in 0..pack.nodes.len() {
        let node = &pack.nodes[index];
        if node.level != crate::citypack::LEVEL_NEIGHBORHOOD {
            continue;
        }
        // Seattle parks only: Tacoma and Bellevue are deliberately empty.
        let city = pack
            .nodes
            .get(node.parent as usize)
            .map(|parent| parent.name.as_str())
            .unwrap_or("");
        if city != "Seattle" {
            continue;
        }
        let count = trees.count(index);
        if count == 0 {
            continue;
        }
        for tree in trees.read_chunk(index, node.center) {
            if tree.regime != 1 {
                continue;
            }
            let key = (
                (tree.x / PARK_CELL_M).floor() as i32,
                (tree.z / PARK_CELL_M).floor() as i32,
            );
            let entry = cells.entry(key).or_insert((0, 0.0, 0.0));
            entry.0 += 1;
            entry.1 += tree.x;
            entry.2 += tree.z;
        }
    }
    let mut sites = cells
        .into_iter()
        .filter(|(_, (count, _, _))| *count >= 6)
        .map(|(_, (count, sx, sz))| ParkSite {
            x: sx / count as f32,
            z: sz / count as f32,
            strength: (count as f32 / 40.0).clamp(0.35, 1.0),
        })
        .collect::<Vec<_>>();
    sites.sort_by(|a, b| b.strength.total_cmp(&a.strength));
    sites.truncate(600);
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (index, site) in sites.iter().enumerate() {
        grid.entry((
            (site.x / PARK_GRID_M).floor() as i32,
            (site.z / PARK_GRID_M).floor() as i32,
        ))
        .or_default()
        .push(index as u32);
    }
    ParkField { sites, grid }
}

/// Park activity: people walk parks at midday and into the evening, not at the
/// 8:30 commute peak.
pub fn park_pace(hour: f32) -> f32 {
    let hour = hour.rem_euclid(24.0);
    let midday = (-((hour - 13.0) / 3.0).powi(2)).exp();
    let evening = (-((hour - 18.5) / 2.0).powi(2)).exp() * 0.55;
    0.08 + 0.92 * midday.max(evening)
}

/// Seattle walker density: core + parks + sprinkle, never summed.
pub fn seattle_walker_density(
    fields: &CityFields,
    parks: &ParkField,
    hour: f32,
    x: f32,
    z: f32,
) -> f32 {
    let d = Vec2::new(x, z).distance(crate::city_rhythm::SEATTLE_EPICENTER);
    let core = 1.0 - ((d - CORE_INNER_M) / (CORE_OUTER_M - CORE_INNER_M)).clamp(0.0, 1.0);
    let downtown = CORE_GAIN * core * fields.ad_density(x, z) * fields.pace(hour);
    let park = parks.density(x, z) * park_pace(hour) * PARK_GAIN;
    let plaza = PLAZAS
        .iter()
        .map(|(anchor, radius)| {
            let d = ((x - anchor[0]).powi(2) + (z - anchor[1]).powi(2)).sqrt();
            if d > *radius * 2.5 {
                return 0.0;
            }
            (-(d / radius).powi(2)).exp() * PLAZA_GAIN * fields.pace(hour)
        })
        .fold(0.0_f32, f32::max);
    let sprinkle = SPRINKLE * (0.35 + 0.65 * fields.pace(hour));
    downtown.max(park).max(plaza).max(sprinkle)
}

#[derive(Clone, Copy, Debug)]
pub struct WalkerState {
    pub points: [([f32; 2], f32); 8],
    pub count: usize,
    pub length: f32,
    pub s: f32,
    pub speed: f32,
    pub direction: f32,
    pub pause: f32,
    pub tint: u8,
    /// Sideways offset from the polyline: the sidewalk band for street
    /// walkers, zero for park strollers who walk the loop itself.
    pub offset_m: f32,
}

impl WalkerState {
    pub fn sample(&self, s: f32) -> ([f32; 2], [f32; 2]) {
        let mut remaining = s.rem_euclid(self.length);
        for index in 0..self.count.saturating_sub(1) {
            let (a, _) = self.points[index];
            let (b, _) = self.points[index + 1];
            let seg = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2))
                .sqrt()
                .max(1.0e-3);
            if remaining <= seg {
                let t = remaining / seg;
                return (
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                    [(b[0] - a[0]) / seg, (b[1] - a[1]) / seg],
                );
            }
            remaining -= seg;
        }
        (self.points[0].0, [0.0, 1.0])
    }
}

pub fn walker_on_street(
    street: &crate::citypack::PackStreet,
    seed: u32,
    direction: f32,
    speed: f32,
) -> Option<WalkerState> {
    if street.points.len() < 2 {
        return None;
    }
    let mut points = [([0.0_f32, 0.0_f32], 0.0_f32); 8];
    let mut count = 0;
    let mut length = 0.0;
    for point in street.points.iter().take(8) {
        if count > 0 {
            let (prev, _) = points[count - 1];
            length += ((point[0] - prev[0]).powi(2) + (point[1] - prev[1]).powi(2)).sqrt();
        }
        points[count] = (*point, length);
        count += 1;
    }
    if count < 2 || length < 1.0 {
        return None;
    }
    Some(WalkerState {
        points,
        count,
        length,
        s: (seed as f32 / 89.0) % length,
        speed,
        direction,
        pause: 0.0,
        tint: (seed % 3) as u8,
        offset_m: 3.2 + BAND,
    })
}

/// Park strollers: a seeded loop around a park cell. Parks are the one place
/// Seattle genuinely fills with feet, and they have no street polylines, so
/// the loop is the path.
pub fn walker_in_park(site: ParkSite, seed: u32) -> WalkerState {
    let radius = 22.0 + (seed % 17) as f32;
    let phase = hash01(seed ^ 0xA53C_9E11) * std::f32::consts::TAU;
    let mut points = [([0.0_f32, 0.0_f32], 0.0_f32); 8];
    let mut length = 0.0;
    for index in 0..8 {
        let angle = phase + index as f32 / 8.0 * std::f32::consts::TAU;
        // Slightly elliptical and off-centre: a stroll, not a racetrack.
        let wobble = 1.0 + 0.18 * (seed.wrapping_mul(index as u32 + 3) % 7) as f32 / 7.0;
        let x = site.x + angle.cos() * radius * wobble;
        let z = site.z + angle.sin() * radius * 0.82;
        if index > 0 {
            let (prev, _) = points[index - 1];
            length += ((x - prev[0]).powi(2) + (z - prev[1]).powi(2)).sqrt();
        }
        points[index] = ([x, z], length);
    }
    WalkerState {
        points,
        count: 8,
        length,
        s: (seed as f32 / 73.0) % length,
        speed: 0.9 + (seed % 5) as f32 * 0.08,
        direction: if seed & 1 == 0 { 1.0 } else { -1.0 },
        pause: 0.0,
        tint: (seed % 3) as u8,
        offset_m: 0.0,
    }
}

/// Pose on the sidewalk band: offset beside the lane, y from the DEM, pitch
/// with the drape (the hill law applies to feet too).
pub fn walker_pose(state: &WalkerState, dem: &crate::geology::Dem) -> (Vec3, Quat) {
    let (pos, dir) = state.sample(state.s);
    let sign = state.direction.signum();
    let nx = -dir[1] * sign;
    let nz = dir[0] * sign;
    let x = pos[0] + nx * state.offset_m;
    let z = pos[1] + nz * state.offset_m;
    let y = dem.height(x, z) + 0.9;
    let dx = dir[0] * sign;
    let dz = dir[1] * sign;
    let ahead_x = pos[0] + dx * 1.5;
    let ahead_z = pos[1] + dz * 1.5;
    let dy = dem.height(ahead_x, ahead_z) - dem.height(pos[0], pos[1]);
    let yaw = dx.atan2(dz);
    let pitch = (dy / 1.5).atan();
    (
        Vec3::new(x + STREAM_WORLD_OFFSET, y, z),
        Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0),
    )
}

/// Expected walkers for a neighborhood at an anchor (the chunk centre, or a
/// park site inside the chunk when the park wins).
pub fn expected_walkers(
    fields: &CityFields,
    parks: &ParkField,
    hour: f32,
    anchor: [f32; 2],
) -> f32 {
    let density = seattle_walker_density(fields, parks, hour, anchor[0], anchor[1]);
    (density * BASE_PER_CHUNK * 0.5).max(0.0)
}

#[derive(Resource)]
pub struct PedAssets {
    mesh: Handle<Mesh>,
    tinted: Vec<Handle<StandardMaterial>>,
}

#[derive(Resource, Default)]
pub struct Pedestrians {
    pub loaded: HashMap<usize, Vec<Entity>>,
    pub states: HashMap<Entity, WalkerState>,
    pending: Vec<usize>,
    pub active: usize,
    pub expected: f32,
    /// Expected walkers per loaded neighborhood, for the per-district capture
    /// audit (expected vs actual).
    pub expected_by_chunk: HashMap<usize, f32>,
    /// Chunks whose walkers are anchored to a park/plaza crowd rather than
    /// the street sprinkle. Audited separately: a park's 37 walkers must not
    /// read as 37 residential walkers.
    pub park_chunks: std::collections::HashSet<usize>,
    pub enabled: bool,
}

pub struct PedestriansPlugin;

impl Plugin for PedestriansPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PedCaptureState>()
            .init_resource::<ParkField>()
            .add_systems(PostStartup, build_parks)
            .add_systems(Startup, setup_pedestrians)
            .add_systems(Update, (ped_stream, walk_pedestrians, ped_capture));
    }
}

fn tint_material(materials: &mut Assets<StandardMaterial>, tint: u8) -> Handle<StandardMaterial> {
    let color = match tint {
        0 => Color::srgb(0.55, 0.12, 0.10),
        1 => Color::srgb(0.12, 0.20, 0.45),
        _ => Color::srgb(0.20, 0.28, 0.16),
    };
    materials.add(StandardMaterial {
        base_color: color,
        perceptual_roughness: 0.8,
        ..default()
    })
}

fn build_parks(
    trees: Option<Res<crate::trees::TreeDataset>>,
    geology: Option<Res<Geology>>,
    mut parks: ResMut<ParkField>,
) {
    let (Some(trees), Some(geology)) = (trees, geology) else {
        return;
    };
    *parks = build_park_field(&trees, &geology.pack);
    if let Some(hot) = parks.sites.first() {
        let gas = parks.nearest(82.0, -4_390.0, 900.0);
        info!(
            "PEDESTRIANS parks: {} park sites (strongest {:.2} at ({:.0},{:.0}); gas works {:?})",
            parks.sites(),
            hot.strength,
            hot.x,
            hot.z,
            gas.map(|site| (site.x, site.z, site.strength))
        );
    }
}

fn setup_pedestrians(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !crate::geology::s2_enabled() {
        return;
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    let half = Vec3::new(0.24, 0.85, 0.18);
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let corners = [
        Vec3::new(-half.x, -half.y, -half.z),
        Vec3::new(half.x, -half.y, -half.z),
        Vec3::new(half.x, half.y, -half.z),
        Vec3::new(-half.x, half.y, -half.z),
        Vec3::new(-half.x, -half.y, half.z),
        Vec3::new(half.x, -half.y, half.z),
        Vec3::new(half.x, half.y, half.z),
        Vec3::new(-half.x, half.y, half.z),
    ];
    for corner in corners {
        positions.push(corner.to_array());
        normals.push(corner.normalize_or_zero().to_array());
        colors.push([1.0, 1.0, 1.0, 1.0]);
    }
    for face in [
        [0, 1, 2, 0, 2, 3],
        [4, 6, 5, 4, 7, 6],
        [0, 4, 5, 0, 5, 1],
        [3, 2, 6, 3, 6, 7],
        [0, 3, 7, 0, 7, 4],
        [1, 5, 6, 1, 6, 2],
    ] {
        indices.extend(face.iter().map(|i| *i as u32));
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    let mesh = meshes.add(mesh);
    let tinted = (0..3)
        .map(|tint| tint_material(&mut materials, tint))
        .collect();
    commands.insert_resource(PedAssets { mesh, tinted });
    commands.insert_resource(Pedestrians {
        enabled: std::env::var_os("PEDESTRIANS_OFF").is_none(),
        ..default()
    });
}

fn ped_stream(world: &mut World) {
    let Some(peds) = world.get_resource::<Pedestrians>() else {
        return;
    };
    if !crate::geology::s2_enabled() || !peds.enabled {
        return;
    }
    if world.get_resource::<PedAssets>().is_none() {
        return;
    }
    {
        let pending = {
            let mut peds = world.resource_mut::<Pedestrians>();
            peds.pending.drain(..).take(2).collect::<Vec<_>>()
        };
        for node in pending {
            spawn_walkers(world, node);
        }
        let mut peds = world.resource_mut::<Pedestrians>();
        peds.active = peds.loaded.values().map(Vec::len).sum();
    }
    let Some(camera) = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .copied()
    else {
        return;
    };
    let center = [
        camera.translation.x - STREAM_WORLD_OFFSET,
        camera.translation.z,
    ];
    let (wanted, unload): (Vec<usize>, Vec<(usize, Vec<Entity>)>) = {
        let state = world.resource::<Geology>();
        let candidates = wanted_neighborhoods(
            &state.pack.nodes,
            center,
            LOAD_M,
            &mut crate::citypack::Evaluations::default(),
        );
        let peds = world.resource::<Pedestrians>();
        let wanted = if peds.active + SPAWN_PER_FRAME > MAX_ACTIVE {
            Vec::new()
        } else {
            candidates
                .into_iter()
                .filter(|index| !peds.loaded.contains_key(index))
                .filter(|index| !peds.pending.contains(index))
                .take(1)
                .collect::<Vec<_>>()
        };
        let unload = peds
            .loaded
            .iter()
            .filter(|(index, _)| {
                let node = &state.pack.nodes[**index];
                crate::citypack::aabb_distance2(&node.bounds, center) > UNLOAD_M * UNLOAD_M
            })
            .map(|(index, entities)| (*index, entities.clone()))
            .collect();
        (wanted, unload)
    };
    for (index, entities) in unload {
        for entity in entities {
            let _ = world.despawn(entity);
        }
        let mut peds = world.resource_mut::<Pedestrians>();
        for entity in peds.loaded.remove(&index).unwrap_or_default() {
            peds.states.remove(&entity);
        }
        peds.active = peds.loaded.values().map(Vec::len).sum();
    }
    for index in wanted {
        world.resource_mut::<Pedestrians>().pending.push(index);
        world
            .resource_mut::<Pedestrians>()
            .loaded
            .insert(index, Vec::new());
    }
}

fn spawn_walkers(world: &mut World, node_index: usize) {
    let hour = world.resource::<SimClock>().minute_of_day as f32 / 60.0;
    let fields = CityFields::seattle();
    let (chunk_center, bounds, mut streets) = {
        let state = world.resource::<Geology>();
        let node = &state.pack.nodes[node_index];
        let Ok(chunk) = state.pack.read_chunk(node_index) else {
            return;
        };
        (node.center, node.bounds, chunk.streets.clone())
    };
    let parks = world.resource::<ParkField>().clone();
    let at_center = expected_walkers(&fields, &parks, hour, chunk_center);
    // A park inside the partition can carry more walkers than its centre: use
    // the park site as the anchor and prefer streets near it.
    let mut park_anchor = None;
    let mut anchor = (at_center, chunk_center);
    if let Some(site) = parks.best_in_bounds(bounds) {
        let at_park = expected_walkers(&fields, &parks, hour, [site.x, site.z]);
        if at_park > at_center {
            park_anchor = Some(site);
            anchor = (at_park, [site.x, site.z]);
        }
    }
    // A genuine plaza inside the partition (Pike Place, Westlake) anchors a
    // loop crowd the same way a park does.
    for (plaza, radius) in PLAZAS {
        let inside = plaza[0] >= bounds[0]
            && plaza[0] <= bounds[2]
            && plaza[1] >= bounds[1]
            && plaza[1] <= bounds[3];
        if !inside {
            continue;
        }
        let at_plaza = expected_walkers(&fields, &parks, hour, plaza);
        if at_plaza > anchor.0 {
            park_anchor = Some(ParkSite {
                x: plaza[0],
                z: plaza[1],
                strength: (at_plaza / (BASE_PER_CHUNK * 0.5)).min(1.0),
            });
            anchor = (at_plaza, plaza);
            let _ = radius;
        }
    }
    let raw = anchor.0;
    let base = raw.floor();
    let frac = raw - base;
    let roll = hash01(node_index as u32 ^ 0x5EED_1234);
    let expected = base as usize + usize::from(roll < frac);
    {
        let mut peds = world.resource_mut::<Pedestrians>();
        peds.expected += expected as f32;
        peds.expected_by_chunk.insert(node_index, expected as f32);
        if park_anchor.is_some() {
            peds.park_chunks.insert(node_index);
        }
    }
    // Park/frontage walkers go on the streets nearest the anchor (park
    // interiors have no street polylines; frontage is the honest analogue).
    if anchor.1 != chunk_center {
        streets.sort_by(|a, b| {
            let da = a
                .points
                .first()
                .map(|p| (p[0] - anchor.1[0]).hypot(p[1] - anchor.1[1]))
                .unwrap_or(f32::MAX);
            let db = b
                .points
                .first()
                .map(|p| (p[0] - anchor.1[0]).hypot(p[1] - anchor.1[1]))
                .unwrap_or(f32::MAX);
            da.total_cmp(&db)
        });
        streets.truncate(14);
    }
    let mut entities = Vec::new();
    if let Some(site) = park_anchor {
        // Parks carry the crowd: three fifths stroll the loop, the rest walk
        // the frontage streets.
        let target = (expected * 3).div_ceil(5);
        for index in 0..target {
            let seed = (node_index as u32)
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add(index as u32 * 7919)
                ^ 0x51ED_2701;
            let state = walker_in_park(site, seed);
            let (position, rotation) = {
                let dem = &world.resource::<Geology>().dem;
                walker_pose(&state, dem)
            };
            let mesh = world.resource::<PedAssets>().mesh.clone();
            let material = world.resource::<PedAssets>().tinted[state.tint as usize].clone();
            let entity = world
                .spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    Transform::from_translation(position).with_rotation(rotation),
                    NotShadowCaster,
                    Name::new("park walker"),
                ))
                .id();
            world
                .resource_mut::<Pedestrians>()
                .states
                .insert(entity, state);
            entities.push(entity);
        }
    }
    for (street_index, street) in streets.iter().enumerate() {
        if entities.len() >= expected {
            break;
        }
        for k in 0..3usize {
            if entities.len() >= expected {
                break;
            }
            let seed = (street.id.wrapping_mul(0x9E37_79B9))
                .wrapping_add(street_index as u32)
                .wrapping_add(k as u32 * 6151)
                .wrapping_add(node_index as u32);
            let direction = if seed & 1 == 0 { 1.0 } else { -1.0 };
            if let Some(state) = walker_on_street(street, seed, direction, 1.25) {
                let (position, rotation) = {
                    let dem = &world.resource::<Geology>().dem;
                    walker_pose(&state, dem)
                };
                let mesh = world.resource::<PedAssets>().mesh.clone();
                let material = world.resource::<PedAssets>().tinted[state.tint as usize].clone();
                let entity = world
                    .spawn((
                        Mesh3d(mesh),
                        MeshMaterial3d(material),
                        Transform::from_translation(position).with_rotation(rotation),
                        NotShadowCaster,
                        Name::new("pedestrian"),
                    ))
                    .id();
                world
                    .resource_mut::<Pedestrians>()
                    .states
                    .insert(entity, state);
                entities.push(entity);
            }
        }
    }
    let mut peds = world.resource_mut::<Pedestrians>();
    if let Some(entry) = peds.loaded.get_mut(&node_index) {
        entry.extend(entities);
    }
    peds.active = peds.loaded.values().map(Vec::len).sum();
}

fn walk_pedestrians(
    time: Res<Time<Real>>,
    dem: Option<Res<Geology>>,
    mut peds: Option<ResMut<Pedestrians>>,
    mut query: Query<&mut Transform>,
) {
    let (Some(dem), Some(peds)) = (dem, peds.as_mut()) else {
        return;
    };
    let dt = time.delta_secs().min(0.25);
    for (entity, state) in peds.states.iter_mut() {
        if state.pause > 0.0 {
            state.pause -= dt;
        } else {
            state.s = (state.s + state.speed * dt * state.direction).rem_euclid(state.length);
            // Occasional pause: deterministic-ish, seeded by position.
            let roll = ((state.s * 13.0).sin() * 43_758.547).fract();
            if roll.abs() < 0.002 {
                state.pause = 1.6;
            }
        }
        if let Ok(mut transform) = query.get_mut(*entity) {
            let (position, rotation) = walker_pose(state, &dem.dem);
            transform.translation = position;
            transform.rotation = rotation;
        }
    }
}

/// Measurement honesty: expected vs actual walkers per district, printed with
/// every capture and available on demand (PED_DENSITY=1 logs every 10 s).
fn log_ped_density(world: &mut World, label: &str) {
    let Some(geology) = world.get_resource::<Geology>() else {
        return;
    };
    let peds = world.resource::<Pedestrians>();
    let mut by_district: HashMap<String, (f32, f32)> = HashMap::new();
    let mut parks: HashMap<String, (f32, f32)> = HashMap::new();
    for (index, expected) in &peds.expected_by_chunk {
        let node = &geology.pack.nodes[*index];
        let district = geology
            .districts
            .get(&node.name)
            .cloned()
            .unwrap_or_else(|| node.name.clone());
        let actual = peds.loaded.get(index).map(Vec::len).unwrap_or(0) as f32;
        let target = if peds.park_chunks.contains(index) {
            &mut parks
        } else {
            &mut by_district
        };
        let entry = target.entry(district).or_insert((0.0, 0.0));
        entry.0 += *expected;
        entry.1 += actual;
    }
    let mut park_rows = parks.into_iter().collect::<Vec<_>>();
    park_rows.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
    let park_text = park_rows
        .iter()
        .take(6)
        .map(|(district, (expected, actual))| {
            format!("{district} exp={expected:.0} act={actual:.0}")
        })
        .collect::<Vec<_>>()
        .join(" | ");
    if !park_text.is_empty() {
        info!("PED CROWDS {label}: {park_text}");
    }
    let mut rows = by_district.into_iter().collect::<Vec<_>>();
    rows.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
    let text = rows
        .iter()
        .take(8)
        .map(|(district, (expected, actual))| {
            format!("{district} exp={expected:.0} act={actual:.0}")
        })
        .collect::<Vec<_>>()
        .join(" | ");
    info!("PED DENSITY {label}: {text}");
}

/// Derive the park frame from a park stroller: eye 9 m behind, 2.4 m up,
/// looking at the walker's chest. Falls back to the park overview before the
/// crowd has spawned.
fn park_pose(world: &mut World) -> bool {
    let site = {
        let geology = world.resource::<Geology>();
        world
            .resource::<ParkField>()
            .nearest(80.0, -8_270.0, 1_200.0)
            .filter(|site| geology.dem.height(site.x, site.z) > 8.0)
            .or_else(|| world.resource::<ParkField>().best_dry(&geology.dem, 8.0))
    };
    let Some(site) = site else {
        return false;
    };
    let target = {
        let peds = world.resource::<Pedestrians>();
        peds.states
            .iter()
            // Frontage walkers first: street-level sightlines beat a loop
            // buried in the trees.
            .filter(|(_, state)| state.offset_m > 0.0)
            .filter_map(|(entity, _)| world.get::<Transform>(*entity))
            .map(|transform| transform.translation)
            .min_by(|a, b| {
                let da = (a.x - (STREAM_WORLD_OFFSET + site.x)).hypot(a.z - site.z);
                let db = (b.x - (STREAM_WORLD_OFFSET + site.x)).hypot(b.z - site.z);
                da.total_cmp(&db)
            })
    };
    let ground = world.resource::<Geology>().dem.height(site.x, site.z);
    let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
    if let Some(target) = target {
        info!("PEDESTRIANS park target: ({:.0},{:.0})", target.x, target.z);
        let eye = target + Vec3::new(0.0, 1.8, 8.0);
        let look = target + Vec3::new(0.0, 1.1, 0.0);
        crate::camera_authority::stage_look_at(
            &mut staged,
            "pedestrian_capture",
            eye,
            look,
            "pedestrians_park_frontage",
        );
        true
    } else {
        crate::camera_authority::stage_look_at(
            &mut staged,
            "pedestrian_capture",
            Vec3::new(STREAM_WORLD_OFFSET + site.x, ground + 6.0, site.z - 20.0),
            Vec3::new(STREAM_WORLD_OFFSET + site.x, ground + 1.4, site.z),
            "pedestrians_park_frontage",
        );
        true
    }
}

fn ped_capture(world: &mut World) {
    if std::env::var_os("PEDESTRIAN_CAPTURE").is_none() || !crate::geology::s2_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed < 5.0 {
        return;
    }
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRST: AtomicBool = AtomicBool::new(false);
        if !FIRST.swap(true, Ordering::Relaxed) {
            info!(
                "PEDESTRIANS capture: first frame, active {}",
                world.resource::<Pedestrians>().active
            );
        }
    }
    let (index, entered, since, hour_state, done) = {
        let state = world.resource::<PedCaptureState>();
        (
            state.index,
            state.entered,
            state.since,
            state.hour,
            state.done,
        )
    };
    if done {
        return;
    }
    const POSES: [(&str, f32, &str); 6] = [
        ("pedestrians_am_westlake", 8.5, "am"),
        ("pedestrians_graveyard", 3.0, "grave"),
        ("pedestrians_pike_place", 8.5, "plaza"),
        ("pedestrians_capitol_ascent", 8.5, "ascent"),
        ("pedestrians_park_frontage", 13.0, "park"),
        ("pedestrians_sprinkle_wallingford", 13.0, "residential"),
    ];
    if index >= POSES.len() {
        // Give the last screenshot observer a frame or two to land before the
        // app exits; the capture channel closes on exit.
        let mut state = world.resource_mut::<PedCaptureState>();
        if !state.done {
            state.done = true;
            state.since = elapsed;
            return;
        }
        if elapsed - state.since > 1.5 {
            world.write_message(AppExit::Success);
        }
        return;
    }
    let (name, hour, mode) = POSES[index];
    {
        let view = world.resource::<crate::camera_authority::StagedPose>();
        let direction = if view.active {
            (view.rotation * Vec3::NEG_Z).normalize_or_zero()
        } else {
            Vec3::new(-0.45, -0.75, -0.35).normalize()
        };
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
        let mut lights = world.query::<(&mut DirectionalLight, &mut Transform)>();
        for (mut light, mut transform) in lights.iter_mut(world) {
            light.illuminance = 26_000.0;
            light.color = Color::srgb(1.0, 0.97, 0.92);
            transform.rotation = rotation;
        }
    }
    if !entered || hour_state != hour {
        if hour_state != hour {
            let entities = {
                let peds = world.resource::<Pedestrians>();
                peds.loaded.values().flatten().copied().collect::<Vec<_>>()
            };
            for entity in entities {
                let _ = world.despawn(entity);
            }
            let mut peds = world.resource_mut::<Pedestrians>();
            peds.loaded.clear();
            peds.states.clear();
            peds.pending.clear();
            peds.active = 0;
            peds.expected = 0.0;
        }
        {
            let mut clock = world.resource_mut::<SimClock>();
            clock.minute_of_day = hour as f64 * 60.0;
            clock.playing = false;
            let mut weather = world.resource_mut::<crate::weather::Weather>();
            weather.auto = false;
            weather.target = crate::weather::WeatherKind::Clear;
            weather.cloud_cover = 0.18;
            weather.precipitation = 0.0;
            weather.storm = 0.0;
            weather.snow = 0.0;
        }
        if index == 1 && state_has_am(world) {
            let (eye, look) = state_am(world);
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "pedestrian_capture",
                eye,
                look,
                name,
            );
        } else {
            // Curb-side template; plaza frames aim at the Pike Place anchor,
            // park frames at the Gas Works site (the Sound's busiest lawn).
            if mode == "park" {
                park_pose(world);
            } else if mode == "residential" {
                let ground = world.resource::<Geology>().dem.height(-160.0, -6452.0);
                let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
                crate::camera_authority::stage_look_at(
                    &mut staged,
                    "pedestrian_capture",
                    Vec3::new(STREAM_WORLD_OFFSET - 160.0, ground + 3.0, -6452.0),
                    Vec3::new(STREAM_WORLD_OFFSET - 184.0, ground + 2.6, -6485.0),
                    name,
                );
            } else if mode == "plaza" {
                let ground = {
                    let geology = world.resource::<Geology>();
                    geology.dem.height(-518.6, -399.4)
                };
                let anchor = Vec3::new(STREAM_WORLD_OFFSET - 518.6, ground, -399.4);
                // Framing law: stand clear of every subject. Try offsets around
                // the plaza and take the roomiest eye; a plaza crowd moves, so
                // a fixed bearing will eventually stand inside someone.
                let walkers = world
                    .resource::<Pedestrians>()
                    .states
                    .keys()
                    .filter_map(|entity| world.get::<Transform>(*entity))
                    .map(|transform| transform.translation)
                    .collect::<Vec<_>>();
                let mut best: Option<(f32, Vec3, Vec3)> = None;
                for step in 0..12 {
                    let angle = step as f32 / 12.0 * std::f32::consts::TAU;
                    for radius in [24.0f32, 30.0, 36.0] {
                        let offset = Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius);
                        let (eye, look) = crate::camera_authority::plaza_eye(anchor, offset);
                        let clearance = walkers
                            .iter()
                            .map(|walker| walker.distance(eye))
                            .fold(f32::MAX, f32::min);
                        if clearance < 4.5 {
                            continue;
                        }
                        if best
                            .as_ref()
                            .is_none_or(|(current, _, _)| clearance > *current)
                        {
                            best = Some((clearance, eye, look));
                        }
                    }
                }
                let (eye, look) = best.map(|(_, eye, look)| (eye, look)).unwrap_or_else(|| {
                    crate::camera_authority::plaza_eye(anchor, Vec3::new(0.0, 0.0, -24.0))
                });
                let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
                crate::camera_authority::stage_look_at(
                    &mut staged,
                    "pedestrian_capture",
                    eye,
                    look,
                    name,
                );
            } else if !curb_pose(world, name) {
                let (ground, look_ground) = {
                    let geology = world.resource::<Geology>();
                    (
                        geology.dem.height(-138.0, 80.0),
                        geology.dem.height(-138.0, -614.3),
                    )
                };
                let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
                crate::camera_authority::stage_look_at(
                    &mut staged,
                    "pedestrian_capture",
                    Vec3::new(STREAM_WORLD_OFFSET - 138.0, ground + 6.0, 80.0),
                    Vec3::new(STREAM_WORLD_OFFSET - 138.0, look_ground + 3.0, -614.3),
                    name,
                );
            }
        }
        let mut state = world.resource_mut::<PedCaptureState>();
        state.entered = true;
        state.hour = hour;
        state.since = elapsed;
        return;
    }
    if elapsed - since < 4.0 {
        // Park and plaza frames keep their own derived pose; curb_pose would
        // drag the camera back to a vehicle street and undo the anchor.
        if POSES[index].2 == "park" {
            park_pose(world);
        } else if POSES[index].2 == "residential" {
            let ground = world.resource::<Geology>().dem.height(-160.0, -6452.0);
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "pedestrian_capture",
                Vec3::new(STREAM_WORLD_OFFSET - 160.0, ground + 3.0, -6452.0),
                Vec3::new(STREAM_WORLD_OFFSET - 184.0, ground + 2.6, -6485.0),
                name,
            );
        } else if (index != 1 || !state_has_am(world)) && POSES[index].2 != "plaza" {
            curb_pose(world, name);
        }
        return;
    }
    if POSES[index].2 == "plaza" {
        // Minimum-crowd law: the plaza frame must show a crowd.
        let pose = world.resource::<crate::camera_authority::StagedPose>();
        let eye = pose.translation;
        let forward = pose.rotation * Vec3::NEG_Z;
        let anchor = Vec3::new(STREAM_WORLD_OFFSET - 518.6, eye.y - 1.6, -399.4);
        let crowd = {
            let peds = world.resource::<Pedestrians>();
            peds.states
                .keys()
                .filter_map(|entity| world.get::<Transform>(*entity))
                .filter(|transform| {
                    let to = transform.translation - eye;
                    to.length() < 60.0 && to.normalize_or_zero().dot(forward) > 0.25
                })
                .count()
        };
        let nearest = {
            let peds = world.resource::<Pedestrians>();
            peds.states
                .keys()
                .filter_map(|entity| world.get::<Transform>(*entity))
                .map(|transform| transform.translation.distance(eye))
                .fold(f32::MAX, f32::min)
        };
        if elapsed - since < 9.0 && (crowd < 15 || nearest < 4.0) {
            return;
        }
        info!(
            "PEDESTRIANS plaza crowd: {crowd} in view, nearest {nearest:.1} m from anchor {:?}",
            anchor
        );
        assert!(crowd >= 15, "plaza frame has only {crowd} walkers in view");
        assert!(
            nearest >= 4.0,
            "plaza eye stands inside the crowd ({nearest} m)"
        );
    }
    log_ped_density(world, name);
    let peds = world.resource::<Pedestrians>();
    let expected = peds.expected;
    let actual = peds.active;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("captures")
        .join(format!("{name}.png"));
    world
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(bevy::render::view::screenshot::save_to_disk(path));
    info!(
        "PEDESTRIANS capture {name} hour {hour}: expected {expected:.0}, actual {actual} (cap 500)"
    );
    let am = if index == 0 {
        let pose = world.resource::<crate::camera_authority::StagedPose>();
        Some((
            pose.translation,
            pose.translation + (pose.rotation * Vec3::NEG_Z) * 20.0,
        ))
    } else {
        None
    };
    let mut state = world.resource_mut::<PedCaptureState>();
    if let Some((eye, look)) = am {
        state.am_eye = eye;
        state.am_look = look;
        state.has_am = true;
    }
    state.index += 1;
    state.entered = false;
}

fn state_has_am(world: &World) -> bool {
    world.resource::<PedCaptureState>().has_am
}

fn state_am(world: &World) -> (Vec3, Vec3) {
    let state = world.resource::<PedCaptureState>();
    (state.am_eye, state.am_look)
}

fn curb_pose(world: &mut World, label: &str) -> bool {
    let fleet = {
        let peds = world.resource::<Pedestrians>();
        peds.states
            .keys()
            .filter_map(|entity| world.get::<Transform>(*entity))
            .map(|transform| (transform.translation, transform.rotation))
            .collect::<Vec<_>>()
    };
    for (position, rotation) in &fleet {
        let lane = *rotation * Vec3::NEG_Z;
        let (eye, look) = crate::camera_authority::curb_side_eye(*position, lane);
        if crate::camera_authority::clear_of_subjects(
            eye,
            fleet.iter().map(|(position, _)| *position),
            5.0,
        ) {
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "pedestrian_capture",
                eye,
                look,
                label,
            );
            return true;
        }
    }
    false
}

#[derive(Resource, Default)]
struct PedCaptureState {
    index: usize,
    entered: bool,
    since: f32,
    hour: f32,
    done: bool,
    am_eye: Vec3,
    am_look: Vec3,
    has_am: bool,
}
