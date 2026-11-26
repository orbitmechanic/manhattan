//! S1.5 horizon: packed far-field DEM + 360 azimuth profiles rendered as three
//! layered silhouettes (near black, mid grey-blue, distant white).
//!
//! The profile targets are recomputed at ~2.5 Hz or after 150 m of camera
//! travel; between recomputes the three line angles are interpolated. Rendering
//! is a 360-segment ring at 30 km with vertex colours and fog disabled, so the
//! bands stay legible under the existing atmosphere/cloud systems.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::camera::FlyCamera;
use crate::clock::SimClock;
use crate::weather::{Weather, WeatherKind};

pub const GRID: usize = 400;
pub const BINS: usize = 360;
pub const SAMPLES: usize = 400;
pub const PROFILE_STEP_M: f32 = 500.0;
pub const GRID_SPACING_M: f32 = 1000.0;
pub const GRID_RADIUS_M: f32 = 200_000.0;
pub const RENDER_RADIUS_M: f32 = 30_000.0;
pub const HORIZON_WORLD_OFFSET: f32 = 60_000.0;
pub const UPDATE_INTERVAL_S: f32 = 0.40;
pub const UPDATE_DISPLACEMENT_M: f32 = 150.0;

pub const HORIZON_PATH: &str = "assets/horizon/seattle_horizon.bin";
pub const PROFILES_PATH: &str = "assets/horizon/seattle_profiles.bin";

#[derive(Clone, Copy, Debug, Default)]
pub struct Lines {
    pub near: f32,
    pub mid: f32,
    pub far: f32,
}

#[derive(Component)]
pub struct HorizonRing;

/// Ridge palette. Night is a silhouette rule, not a colour tweak: under a
/// glowing deck the Cascades and Olympics are dark shapes with a faint warm
/// rim, never bright geometry.
#[derive(Clone, Copy)]
struct SkyPalette {
    near: [f32; 4],
    mid: [f32; 4],
    far: [f32; 4],
}

impl SkyPalette {
    fn for_sky(daylight: f32, glow: f32) -> Self {
        let day = SkyPalette {
            near: [0.010, 0.010, 0.014, 1.0],
            mid: [0.36, 0.43, 0.52, 1.0],
            far: [0.96, 0.98, 1.0, 1.0],
        };
        let glow = glow.clamp(0.0, 2.0);
        let night = SkyPalette {
            near: [0.004, 0.004, 0.006, 1.0],
            mid: [0.020, 0.018, 0.018, 1.0],
            far: [
                0.055 + 0.030 * glow,
                0.042 + 0.018 * glow,
                0.034 + 0.010 * glow,
                1.0,
            ],
        };
        let mix = |a: [f32; 4], b: [f32; 4]| {
            [
                a[0] + (b[0] - a[0]) * daylight,
                a[1] + (b[1] - a[1]) * daylight,
                a[2] + (b[2] - a[2]) * daylight,
                1.0,
            ]
        };
        SkyPalette {
            near: mix(night.near, day.near),
            mid: mix(night.mid, day.mid),
            far: mix(night.far, day.far),
        }
    }
}

#[derive(Resource)]
pub struct HorizonState {
    pub profiles: Vec<u16>,
    pub mesh: Handle<Mesh>,
    pub targets: Vec<Lines>,
    pub current: Vec<Lines>,
    pub last_update: f32,
    pub last_position: Vec3,
    pub update_ms: f32,
    pub triangles: usize,
    pub initialized: bool,
}

pub struct HorizonPlugin;

impl Plugin for HorizonPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_horizon)
            .add_systems(Update, (horizon_update, horizon_capture).chain());
    }
}

pub fn horizon_enabled() -> bool {
    cfg!(target_arch = "wasm32")
        || std::env::var_os("PILOT_HORIZON").is_some()
        || std::env::var_os("SEATTLE_STREAM").is_some()
        || std::env::var_os("SEATTLE_HORIZON").is_some()
}

pub fn capture_enabled() -> bool {
    std::env::var_os("HORIZON_CAPTURE").is_some()
}

#[allow(dead_code)]
fn asset_path(relative: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn setup_horizon(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !horizon_enabled() {
        return;
    }
    let profiles = match Ok::<Vec<u8>, std::io::Error>(crate::webassets::bytes(PROFILES_PATH)) {
        Ok(bytes) => bytes
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<u16>>(),
        Err(error) => {
            error!("HORIZON: cannot read profiles: {error}");
            return;
        }
    };
    if profiles.len() != BINS * SAMPLES {
        error!(
            "HORIZON: profile length {} != {}",
            profiles.len(),
            BINS * SAMPLES
        );
        return;
    }
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        fog_enabled: false,
        cull_mode: None,
        ..default()
    });
    let (initial_mesh, triangles) =
        build_mesh(&vec![Lines::default(); BINS], SkyPalette::for_sky(1.0, 0.0));
    let mesh = meshes.add(initial_mesh);
    let entity = commands
        .spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Name::new("Horizon ring"),
            HorizonRing,
        ))
        .id();
    let _ = entity;
    commands.insert_resource(HorizonState {
        profiles,
        mesh,
        targets: vec![Lines::default(); BINS],
        current: vec![Lines::default(); BINS],
        last_update: -10.0,
        last_position: Vec3::new(f32::INFINITY, 0.0, 0.0),
        update_ms: 0.0,
        triangles,
        initialized: false,
    });
    info!(
        "HORIZON: profiles loaded ({} bins x {} samples)",
        BINS, SAMPLES
    );
}

/// Pure profile → three band angles for one camera height.
pub fn band_lines(profiles: &[u16], camera_y: f32) -> Vec<Lines> {
    let mut out = Vec::with_capacity(BINS);
    for bin in 0..BINS {
        let mut near = f32::NEG_INFINITY;
        let mut mid = f32::NEG_INFINITY;
        let mut far = f32::NEG_INFINITY;
        for sample in 0..SAMPLES {
            let distance = (sample + 1) as f32 * PROFILE_STEP_M;
            let elevation = profiles[bin * SAMPLES + sample] as f32 * 0.5;
            let angle = (elevation - camera_y).atan2(distance);
            if distance <= 15_000.0 {
                near = near.max(angle);
            }
            if distance <= 90_000.0 {
                mid = mid.max(angle);
            } else {
                far = far.max(angle);
            }
        }
        let base = -0.02;
        let near = near.max(base);
        let mid = mid.max(near);
        let far = far.max(mid);
        out.push(Lines { near, mid, far });
    }
    out
}

fn horizon_update(
    time: Res<Time>,
    env: Res<crate::environment::Environment>,
    state: Option<ResMut<HorizonState>>,
    mut meshes: ResMut<Assets<Mesh>>,
    camera: Query<&Transform, With<FlyCamera>>,
    mut rings: Query<&mut Transform, (With<HorizonRing>, Without<FlyCamera>)>,
    mut ticks: Local<u32>,
) {
    let Some(mut state) = state else {
        return;
    };
    if !horizon_enabled() {
        return;
    }
    let Ok(transform) = camera.single() else {
        return;
    };
    let position = transform.translation;
    let moved = position.distance(state.last_position);
    let elapsed = time.elapsed_secs();
    if !state.initialized
        || (moved > UPDATE_DISPLACEMENT_M && elapsed - state.last_update > 0.12)
        || elapsed - state.last_update > UPDATE_INTERVAL_S
    {
        let started = crate::clock_compat::Instant::now();
        state.targets = band_lines(&state.profiles, position.y);
        state.update_ms = started.elapsed().as_secs_f32() * 1000.0;
        state.last_update = elapsed;
        state.last_position = position;
        if !state.initialized {
            state.current = state.targets.clone();
            state.initialized = true;
        }
    }
    let blend = 1.0 - (-time.delta_secs() * 6.0).exp();
    let HorizonState {
        current, targets, ..
    } = &mut *state;
    for (current, target) in current.iter_mut().zip(targets.iter()) {
        current.near += (target.near - current.near) * blend;
        current.mid += (target.mid - current.mid) * blend;
        current.far += (target.far - current.far) * blend;
    }
    if let Ok(mut ring) = rings.single_mut() {
        ring.translation = position;
    }
    let (new_mesh, triangles) = build_mesh(
        &state.current,
        SkyPalette::for_sky(env.daylight, env.city_glow),
    );
    state.triangles = triangles;
    if let Some(mut mesh) = meshes.get_mut(&state.mesh.clone()) {
        *mesh = new_mesh;
    }
    *ticks += 1;
    if (*ticks).is_multiple_of(120) {
        info!(
            "HORIZON: update {:.3} ms, {} triangles, {} segments, 1 draw",
            state.update_ms,
            state.triangles,
            BINS * 4
        );
    }
}

fn build_mesh(lines: &[Lines], palette: SkyPalette) -> (Mesh, usize) {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let black = palette.near;
    let grey = palette.mid;
    let white = palette.far;
    let base = -0.02_f32;
    let min_gap = 0.0012_f32;

    let mut push_quad = |a0: f32, a1: f32, lo: f32, hi: f32, color: [f32; 4]| {
        if hi <= lo + 1e-6 {
            return;
        }
        let start = positions.len() as u32;
        for (angle, yaw) in [(lo, a0), (hi, a0), (lo, a1), (hi, a1)] {
            positions.push([
                yaw.sin() * RENDER_RADIUS_M,
                angle.tan() * RENDER_RADIUS_M,
                -yaw.cos() * RENDER_RADIUS_M,
            ]);
            normals.push([0.0, 1.0, 0.0]);
            colors.push(color);
        }
        indices.extend([start, start + 1, start + 2, start + 1, start + 3, start + 2]);
    };

    let segments = BINS * 4; // 0.25 degree segments: smooth ridgelines, no stair
    let sample = |index: usize| -> Lines {
        let position = index as f32 * BINS as f32 / segments as f32;
        let i0 = position.floor() as usize % BINS;
        let i1 = (i0 + 1) % BINS;
        let t = position - position.floor();
        let a = lines.get(i0).copied().unwrap_or_default();
        let b = lines.get(i1).copied().unwrap_or_default();
        Lines {
            near: a.near + (b.near - a.near) * t,
            mid: a.mid + (b.mid - a.mid) * t,
            far: a.far + (b.far - a.far) * t,
        }
    };
    for bin in 0..segments {
        let a0 = bin as f32 * std::f32::consts::TAU / segments as f32;
        let a1 = (bin + 1) as f32 * std::f32::consts::TAU / segments as f32;
        let lines = sample(bin);
        // Pass 1: nearest occluding ridge, filled black to the ground.
        push_quad(a0, a1, base, lines.near, black);
        // Pass 2: taller terrain visible above the first occluder.
        if lines.mid > lines.near + min_gap {
            push_quad(a0, a1, lines.near, lines.mid, grey);
        }
        // Pass 3: distant summits beyond the mid ridge, whitecapped.
        if lines.far > lines.mid + min_gap {
            push_quad(a0, a1, lines.mid, lines.far, white);
        }
    }
    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    let triangles = indices.len() / 3;
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
    (mesh, triangles)
}

/// Capture sequence used by the S1.5 gate. Runs only with HORIZON_CAPTURE=1.
fn horizon_capture(
    mut commands: Commands,
    time: Res<Time>,
    mut state: Local<CaptureState>,
    mut clock: ResMut<SimClock>,
    mut weather: ResMut<Weather>,
    mut camera: Query<(&mut Transform, &mut Projection), With<FlyCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    if !capture_enabled() || !horizon_enabled() {
        return;
    }
    let poses = capture_poses();
    let elapsed = time.elapsed_secs();
    if elapsed < 2.5 {
        return;
    }
    if state.index >= poses.len() {
        if state.finished_at == 0.0 {
            state.finished_at = elapsed;
        } else if elapsed - state.finished_at > 1.0 {
            let _ = exit.write(AppExit::Success);
        }
        return;
    }
    let pose = poses[state.index];
    if state.entered == 0.0 {
        state.entered = elapsed;
        clock.minute_of_day = pose.minute as f64;
        // Every horizon pose renders in a real weather state. The open-sky
        // poses take a SUNBREAK (labelled), never a forced clear noon.
        weather.auto = false;
        if pose.overcast {
            weather.force(WeatherKind::Overcast);
        } else {
            weather.force(WeatherKind::Sunbreak);
        }
        info!(
            "HORIZON pose={} weather={} (labelled)",
            pose.name,
            if pose.overcast {
                "overcast"
            } else {
                "sunbreak"
            }
        );
        if let Ok((mut transform, mut projection)) = camera.single_mut() {
            let base = Vec3::new(HORIZON_WORLD_OFFSET + pose.x, pose.y, pose.z);
            transform.translation = base;
            let yaw = pose.bearing.to_radians();
            let forward = Vec3::new(yaw.sin(), pose.pitch.to_radians().tan(), -yaw.cos());
            let direction = forward.normalize_or_zero();
            transform.rotation = Quat::from_euler(
                EulerRot::YXZ,
                (-direction.x).atan2(-direction.z),
                direction.y.asin(),
                0.0,
            );
            if let Projection::Perspective(perspective) = &mut *projection {
                perspective.fov = pose.fov;
            }
        }
    }
    if elapsed - state.entered >= 1.6 {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("captures")
            .join(pose.name);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        state.index += 1;
        state.entered = 0.0;
    }
}

#[derive(Default)]
struct CaptureState {
    index: usize,
    entered: f32,
    finished_at: f32,
}

#[derive(Clone, Copy)]
struct Pose {
    name: &'static str,
    x: f32,
    z: f32,
    y: f32,
    bearing: f32,
    pitch: f32,
    fov: f32,
    minute: f32,
    overcast: bool,
}

fn capture_poses() -> Vec<Pose> {
    // Elliott Bay pose: world -5532.7, 245.2 from lon -122.409 / lat 47.603.
    let bay_x = -5532.7;
    let bay_z = 245.2;
    vec![
        Pose {
            name: "horizon_rainier_noon.png",
            x: bay_x,
            z: bay_z,
            y: 24.0,
            bearing: 150.5,
            pitch: 1.5,
            fov: 0.55,
            minute: 12.0 * 60.0,
            overcast: false,
        },
        Pose {
            name: "horizon_rainier_downtown.png",
            x: 0.0,
            z: 0.0,
            y: 90.0,
            bearing: 150.5,
            pitch: 1.4,
            fov: 0.55,
            minute: 12.0 * 60.0,
            overcast: false,
        },
        Pose {
            name: "horizon_rainier_dawn.png",
            x: bay_x,
            z: bay_z,
            y: 24.0,
            bearing: 150.5,
            pitch: 1.5,
            fov: 0.55,
            minute: 5.0 * 60.0 + 30.0,
            overcast: false,
        },
        Pose {
            name: "horizon_rainier_dusk.png",
            x: bay_x,
            z: bay_z,
            y: 24.0,
            bearing: 150.5,
            pitch: 1.5,
            fov: 0.55,
            minute: 20.0 * 60.0 + 30.0,
            overcast: false,
        },
        Pose {
            name: "horizon_olympics_west.png",
            x: bay_x,
            z: bay_z,
            y: 24.0,
            bearing: 278.0,
            pitch: 0.9,
            fov: 0.75,
            minute: 12.0 * 60.0,
            overcast: false,
        },
        Pose {
            name: "horizon_cascades_east.png",
            x: 0.0,
            z: 0.0,
            y: 600.0,
            bearing: 88.0,
            pitch: 0.8,
            fov: 0.75,
            minute: 12.0 * 60.0,
            overcast: false,
        },
        Pose {
            name: "horizon_cascade_zoom.png",
            x: 0.0,
            z: 0.0,
            y: 600.0,
            bearing: 92.0,
            pitch: 1.1,
            fov: 0.28,
            minute: 12.0 * 60.0,
            overcast: false,
        },
        Pose {
            name: "horizon_rainier_overcast.png",
            x: bay_x,
            z: bay_z,
            y: 24.0,
            bearing: 150.5,
            pitch: 1.5,
            fov: 0.55,
            minute: 12.0 * 60.0,
            overcast: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_grid() -> Vec<u16> {
        let bytes = std::fs::read(asset_path(HORIZON_PATH)).expect("horizon grid");
        bytes
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect()
    }

    fn load_profiles() -> Vec<u16> {
        let bytes = std::fs::read(asset_path(PROFILES_PATH)).expect("profiles");
        bytes
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect()
    }

    #[test]
    fn packed_cells_decode_as_contracted() {
        let grid = load_grid();
        assert_eq!(grid.len(), GRID * GRID);
        // Rainier summit cell (row 283, col 243): 4,393.0 m, snow.
        let rainier = grid[283 * GRID + 243];
        let elevation = (rainier >> 2) as f32 * 0.5;
        assert!((elevation - 4390.0).abs() <= 3.0, "rainier {elevation}");
        assert_eq!(rainier & 0b01, 1, "rainier snow bit");
        // Cascade forest cell with tree bit.
        let forest = grid[0];
        assert_eq!(forest & 0b10, 0b10, "forest tree bit");
        // Offshore cell clean.
        let offshore = grid[40];
        assert_eq!(offshore, 0, "offshore pack");
    }

    #[test]
    fn profiles_match_bruteforce_raycasts() {
        let profiles = load_profiles();
        let grid = load_grid();
        let center = (GRID / 2) as f32;
        for bin in (0..BINS).step_by(17) {
            let angle = bin as f32 * std::f32::consts::TAU / BINS as f32;
            for sample in [0usize, 37, 120, 260, 399] {
                let distance = (sample + 1) as f32 * PROFILE_STEP_M;
                let x = angle.sin() * distance;
                let z = -angle.cos() * distance;
                let col = center + x / GRID_SPACING_M - 0.5;
                let row = center + z / GRID_SPACING_M - 0.5;
                let c0 = col.floor() as isize;
                let r0 = row.floor() as isize;
                if c0 < 0 || r0 < 0 || c0 + 1 >= GRID as isize || r0 + 1 >= GRID as isize {
                    continue;
                }
                let fc = col - c0 as f32;
                let fr = row - r0 as f32;
                let idx =
                    |r: isize, c: isize| (grid[r as usize * GRID + c as usize] >> 2) as f32 * 0.5;
                let expected = idx(r0, c0) * (1.0 - fc) * (1.0 - fr)
                    + idx(r0, c0 + 1) * fc * (1.0 - fr)
                    + idx(r0 + 1, c0) * (1.0 - fc) * fr
                    + idx(r0 + 1, c0 + 1) * fc * fr;
                let stored = profiles[bin * SAMPLES + sample] as f32 * 0.5;
                assert!(
                    (stored - expected).abs() <= 1.0,
                    "bin {bin} sample {sample}: {stored} vs {expected}"
                );
            }
        }
    }

    #[test]
    fn rainier_is_the_far_line_apex_near_its_bearing() {
        let profiles = load_profiles();
        let lines = band_lines(&profiles, 250.0);
        let (bin, line) = lines
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.far.total_cmp(&b.1.far))
            .unwrap();
        let bearing = bin as f32 * 360.0 / BINS as f32;
        assert!((bearing - 152.5).abs() <= 2.0, "far apex bearing {bearing}");
        let expected = (4392.9_f32 - 250.0).atan2(94_200.0);
        assert!(
            (line.far - expected).abs() <= 0.3_f32.to_radians(),
            "far apex angle {} vs {}",
            line.far.to_degrees(),
            expected.to_degrees()
        );
    }
}
