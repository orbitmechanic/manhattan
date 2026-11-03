#![allow(clippy::type_complexity)]
//! Manhattan fly-through viewer.
//!
//! Controls
//! --------
//! FPS fly camera: WASD, Q/E down/up, hold right mouse button to look,
//! Shift to boost, wheel to change speed.  The camera is hard-clamped to the
//! city ground line (y >= 3) and swept against the baked building height grid,
//! so it slides along walls instead of entering buildings.
//!
//! Calendar/clock: `[`/`]` day, `-`/`=` hour, `;`/`'` five minutes,
//! `,`/`.` month, Space play/pause, R reset.  The on-screen buttons drive the
//! same `SimClock` actions.
//!
//! The sun and moon are real directional lights; their altitude/azimuth come
//! from a solar/lunar ephemeris for New York.  When the sun drops below the
//! horizon the building materials switch to the generated night textures and
//! the lit windows/street lights become emissive.

use bevy::{
    asset::{AssetMetaCheck, AssetPlugin},
    camera::visibility::VisibilityRange,
    core_pipeline::tonemapping::Tonemapping,
    gltf::GltfAssetLabel,
    pbr::{DistanceFog, FogFalloff, MaterialPlugin},
    post_process::{
        bloom::Bloom,
        dof::{DepthOfField, DepthOfFieldMode},
    },
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
};
use manhattan_viewer::{
    astro::{Date, NYC_LAT, NYC_LON, moon_position, sun_position},
    blocks::{BlockLights, update_blocks},
    camera::{FlyCamera, MAX_PITCH, MIN_CAMERA_HEIGHT, MoveState, fly_camera_system, step_camera},
    city_material::{
        BuildingChunk, CityMaterial, CityMaterialSetup, setup_city_materials, update_city_materials,
    },
    clock::{ClockAction, SimClock},
    collision::{CAMERA_RADIUS, CollisionGrid},
    environment::{
        Environment, fog_cull_distance, setup_lighting, spawn_fog_banks, update_environment,
        update_fog_bank_materials, update_fog_banks,
    },
    ui::{handle_calendar_keys, spawn_ui, update_status},
};

#[derive(Resource, Clone)]
struct Capture {
    path: Option<String>,
    target_frame: u32,
    done: bool,
}

#[derive(Resource, Default)]
struct FrameCounter(u32);

#[derive(Resource, Default)]
struct SelfTest {
    path: Option<String>,
    stage: u32,
    camera_before: Vec3,
    camera_after: Vec3,
    clamp_y_before: f32,
    clamp_y_after: f32,
    noon_sun_alt: f32,
    night_sun_alt: f32,
    noon_mode: String,
    night_mode: String,
    hour_wrap_date: String,
    hour_wrap_hour: u32,
    noon_targets: usize,
    night_targets: usize,
    blocks_total: usize,
    building_blocks: usize,
    park_blocks: usize,
    flicker_blocks: usize,
    fade_blocks: usize,
    twilight_span_min: f32,
    cull_entities: usize,
    dof_active: bool,
    materials_ready: bool,
    collision_solid: bool,
    collision_free: bool,
    fog_visibility_km: f32,
    fog_cull_m: f32,
}

#[derive(Clone, Copy, Debug)]
enum ViewPreset {
    Overview,
    Mid,
    Street,
}

#[derive(Resource, Clone, Debug)]
struct Args {
    shot: Option<String>,
    date: Option<Date>,
    time: Option<(u32, u32)>,
    frames: u32,
    selftest: bool,
    report: Option<String>,
    view: ViewPreset,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            shot: None,
            date: None,
            time: None,
            frames: 600,
            selftest: false,
            report: None,
            view: ViewPreset::Overview,
        }
    }
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--shot" => args.shot = it.next(),
            "--frames" => {
                if let Some(v) = it.next() {
                    args.frames = v.parse().unwrap_or(300);
                }
            }
            "--selftest" => args.selftest = true,
            "--report" => args.report = it.next(),
            "--view" => {
                args.view = match it.next().as_deref() {
                    Some("street") => ViewPreset::Street,
                    Some("mid") => ViewPreset::Mid,
                    _ => ViewPreset::Overview,
                }
            }
            "--date" => {
                if let Some(v) = it.next() {
                    let parts: Vec<i32> = v.split('-').filter_map(|p| p.parse().ok()).collect();
                    if parts.len() == 3 {
                        args.date = Some(Date::new(parts[0], parts[1] as u32, parts[2] as u32));
                    }
                }
            }
            "--time" => {
                if let Some(v) = it.next() {
                    let parts: Vec<u32> = v.split(':').filter_map(|p| p.parse().ok()).collect();
                    if parts.len() >= 2 {
                        args.time = Some((parts[0], parts[1]));
                    }
                }
            }
            _ => {}
        }
    }
    args
}

fn main() {
    #[allow(unused_mut)]
    let mut args = parse_args();
    #[cfg(target_arch = "wasm32")]
    apply_web_args(&mut args);
    let mut clock = SimClock::default();
    if let Some(d) = args.date {
        clock.date = d;
    }
    if let Some((h, m)) = args.time {
        clock.set_time(h, m);
    }

    let capture = Capture {
        path: args.shot.clone(),
        target_frame: args.frames,
        done: false,
    };
    let selftest = SelfTest {
        path: args.report.clone().or_else(|| {
            if args.selftest {
                Some("selftest_report.json".to_string())
            } else {
                None
            }
        }),
        ..default()
    };

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Manhattan — day/night fly-through".to_string(),
                        resolution: (1440u32, 900u32).into(),
                        canvas: Some("#manhattan-canvas".into()),
                        fit_canvas_to_parent: false,
                        prevent_default_event_handling: true,
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(MaterialPlugin::<CityMaterial>::default())
        .insert_resource(ClearColor(Color::srgb(0.30, 0.50, 0.82)))
        .insert_resource(clock)
        .insert_resource(capture)
        .insert_resource(FrameCounter::default())
        .insert_resource(selftest)
        .insert_resource(Environment::default())
        .insert_resource(BlockLights::default())
        .insert_resource(CityMaterialSetup::default())
        .insert_resource(CollisionGrid::default())
        .insert_resource(args)
        .add_systems(
            Startup,
            (
                setup_lighting,
                spawn_camera,
                spawn_scene,
                spawn_fog_banks,
                spawn_ui,
            ),
        )
        .add_systems(
            Update,
            (
                tick_clock,
                handle_calendar_keys,
                fly_camera_system,
                update_fog_banks,
                update_environment,
                update_fog_bank_materials,
                update_dof,
                update_fog_culling,
                setup_city_materials,
                cull_web_buildings,
                hide_web_loading,
                update_blocks,
                update_city_materials,
                update_status,
                run_selftest,
                capture_system,
            )
                .chain(),
        )
        .run();
}

fn tick_clock(time: Res<Time>, mut clock: ResMut<SimClock>) {
    clock.tick(time.delta_secs() as f64);
}

#[cfg(target_arch = "wasm32")]
fn apply_web_args(args: &mut Args) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(location) = window.location().search() else {
        return;
    };
    for pair in location.trim_start_matches('?').split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");
        match key {
            "view" => {
                args.view = match value {
                    "street" => ViewPreset::Street,
                    "mid" => ViewPreset::Mid,
                    _ => ViewPreset::Overview,
                };
            }
            "time" => {
                let parts: Vec<u32> = value.split(':').filter_map(|p| p.parse().ok()).collect();
                if parts.len() >= 2 {
                    args.time = Some((parts[0], parts[1]));
                }
            }
            _ => {}
        }
    }
}

fn spawn_camera(mut commands: Commands, args: Res<Args>) {
    let (position, target) = match args.view {
        ViewPreset::Overview => (
            Vec3::new(3600.0, 2300.0, 4700.0),
            Vec3::new(1000.0, 90.0, -100.0),
        ),
        ViewPreset::Mid => (
            Vec3::new(2200.0, 420.0, 2400.0),
            Vec3::new(900.0, 80.0, -300.0),
        ),
        ViewPreset::Street => (
            Vec3::new(900.0, 80.0, 900.0),
            Vec3::new(1100.0, 150.0, -400.0),
        ),
    };
    let dir = (target - position).normalize();
    let yaw = (-dir.x).atan2(-dir.z);
    let pitch = dir.y.asin().clamp(-MAX_PITCH, MAX_PITCH);

    commands.spawn((
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Default,
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: 70f32.to_radians(),
            near: 1.0,
            far: 45_000.0,
            ..default()
        }),
        Transform::from_translation(position)
            .with_rotation(Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch)),
        FlyCamera {
            yaw,
            pitch,
            speed: 650.0,
        },
        AmbientLight {
            color: Color::srgb(0.55, 0.68, 0.9),
            brightness: 120.0,
            ..default()
        },
        DistanceFog {
            color: Color::srgb(0.28, 0.46, 0.76),
            directional_light_color: Color::srgba(1.0, 0.6, 0.3, 0.25),
            directional_light_exponent: 14.0,
            falloff: FogFalloff::Linear {
                start: 5_500.0,
                end: 34_000.0,
            },
        },
        Tonemapping::TonyMcMapface,
        Bloom {
            intensity: 0.16,
            ..Bloom::NATURAL
        },
        DepthOfField {
            mode: DepthOfFieldMode::Gaussian,
            focal_distance: 180.0,
            sensor_height: 2.5,
            aperture_f_stops: 1.0,
            max_circle_of_confusion_diameter: 18.0,
            max_depth: 7_000.0,
        },
    ));
}

fn spawn_scene(mut commands: Commands, asset_server: Res<AssetServer>) {
    for path in ["models/manhattan_ground.glb", "models/manhattan_city.glb"] {
        commands.spawn(WorldAssetRoot(
            asset_server.load(GltfAssetLabel::Scene(0).from_asset(path)),
        ));
    }
}

fn update_dof(
    camera: Single<(&Transform, &mut DepthOfField), With<FlyCamera>>,
    env: Res<Environment>,
) {
    let (transform, mut dof) = camera.into_inner();
    // Focus a little ahead of the camera, but deepen the field at altitude so
    // the overview stays readable.  Distant objects are clamped at the fog
    // visibility, so the blur saturates exactly where the fog takes over.
    let altitude = transform.translation.y.max(0.0);
    let alt = (altitude / 2_200.0).clamp(0.0, 1.0);
    dof.focal_distance = (altitude * 1.35 + 70.0).clamp(90.0, 3_200.0);
    // Deliberately non-physical 2.5 m sensor: at city scale a real 18 mm
    // sensor produces a sub-pixel circle of confusion, which is why the
    // original f/16 settings looked like no depth of field at all.
    dof.sensor_height = 2.5;
    dof.aperture_f_stops = 1.0 + 2.6 * alt;
    dof.max_circle_of_confusion_diameter = 18.0 - 9.0 * alt;
    dof.max_depth = env.fog_visibility.clamp(1_500.0, 12_000.0);
}

/// Cull the real building meshes at the fog distance.  Nothing is merged or
/// replaced with a box proxy; the fog simply hides geometry beyond the point
/// where it is no longer visible.
fn update_fog_culling(
    env: Res<Environment>,
    mut chunks: Query<&mut VisibilityRange, With<BuildingChunk>>,
) {
    let cull = fog_cull_distance(env.fog_visibility);
    for mut range in &mut chunks {
        range.start_margin = 0.0..0.0;
        range.end_margin = cull..(cull + 600.0);
    }
}

fn capture_system(
    mut commands: Commands,
    mut frame: ResMut<FrameCounter>,
    mut capture: ResMut<Capture>,
    mut exit: MessageWriter<AppExit>,
) {
    frame.0 += 1;
    let Some(path) = capture.path.clone() else {
        return;
    };
    if !capture.done && frame.0 >= capture.target_frame {
        capture.done = true;
        info!("Capturing {path} at frame {}", frame.0);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path))
            .observe(
                |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    exit.write(AppExit::Success);
                },
            );
    } else if capture.done && frame.0 > capture.target_frame + 240 {
        // Safety net: never leave a headless test hanging.
        exit.write(AppExit::error());
    }
}

#[allow(clippy::too_many_arguments)]
fn run_selftest(
    frame: Res<FrameCounter>,
    mut test: ResMut<SelfTest>,
    mut clock: ResMut<SimClock>,
    env: Res<Environment>,
    blocks: Res<BlockLights>,
    setup: Res<CityMaterialSetup>,
    culling: Query<&VisibilityRange>,
    dof: Query<&DepthOfField, With<FlyCamera>>,
    grid: Res<CollisionGrid>,
    camera: Single<&mut Transform, With<FlyCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = test.path.clone() else {
        return;
    };
    if frame.0 < 600 {
        return;
    }
    let mut camera = camera;
    match test.stage {
        0 => {
            test.camera_before = camera.translation;
            let mut cam = FlyCamera {
                yaw: 0.0,
                pitch: -0.6,
                speed: 650.0,
            };
            let mut t = Transform::from_translation(camera.translation)
                .with_rotation(camera.rotation)
                .with_scale(camera.scale);
            step_camera(
                &mut t,
                &mut cam,
                MoveState {
                    forward: 1.0,
                    ..default()
                },
                0.5,
            );
            test.camera_after = t.translation;
            test.clamp_y_before = -50.0;

            // Prove the baked height grid is loadable and that a swept move
            // into a real building stops outside it.
            if let Some(inside) = grid.find_solid(60.0) {
                test.collision_solid = grid.blocked(inside, CAMERA_RADIUS);
                let mut start = Vec3::new(inside.x - 200.0, 10.0, inside.z);
                for k in 0..48 {
                    let q = Vec3::new(inside.x - 8.0 * k as f32, 10.0, inside.z);
                    if !grid.blocked(q, CAMERA_RADIUS) {
                        start = q;
                        break;
                    }
                }
                let end = grid.resolve(start, Vec3::new(500.0, 0.0, 0.0), CAMERA_RADIUS);
                test.collision_free = !grid.blocked(end, CAMERA_RADIUS) && end.x < inside.x;
            }

            camera.translation.y = -50.0;
            test.stage = 1;
        }
        1 => {
            test.clamp_y_after = camera.translation.y;
            clock.set_date(2026, 6, 21);
            clock.set_time(23, 30);
            clock.apply(ClockAction::Hour(1));
            test.hour_wrap_date = format!("{:?}", clock.date);
            test.hour_wrap_hour = clock.datetime().hour();
            clock.set_date(2026, 6, 21);
            clock.set_time(12, 0);
            test.stage = 2;
        }
        2 => {
            let dt = clock.datetime();
            test.noon_sun_alt = sun_position(&dt, NYC_LAT, NYC_LON).altitude_deg;
            test.noon_mode = if env.night { "night" } else { "day" }.to_string();
            test.noon_targets = blocks.target_count;
            test.blocks_total = blocks.block_count;
            test.building_blocks = blocks.building_blocks;
            test.park_blocks = blocks.park_blocks;
            test.flicker_blocks = blocks.flicker_count;
            test.fade_blocks = blocks.fade_count;
            test.twilight_span_min = blocks.twilight_span_min;
            test.cull_entities = culling.iter().count();
            test.dof_active = !dof.is_empty();
            test.materials_ready = setup.done;
            test.fog_visibility_km = env.fog_visibility / 1000.0;
            test.fog_cull_m = fog_cull_distance(env.fog_visibility);
            clock.set_time(22, 0);
            test.stage = 3;
        }
        3 => {
            let dt = clock.datetime();
            test.night_sun_alt = sun_position(&dt, NYC_LAT, NYC_LON).altitude_deg;
            test.night_mode = if env.night { "night" } else { "day" }.to_string();
            test.night_targets = blocks.target_count;
            let (moon, illum) = moon_position(&dt, NYC_LAT, NYC_LON);
            let moved = test.camera_before.distance(test.camera_after);
            let clamped = test.clamp_y_after >= MIN_CAMERA_HEIGHT;
            let report = format!(
                "{{\n  \"camera\": {{\n    \"moved_units\": {:.2},\n    \"clamped_y\": {:.2},\n    \"floor\": {:.2},\n    \"clamp_ok\": {}\n  }},\n  \"clock\": {{\n    \"noon_sun_alt_deg\": {:.2},\n    \"night_sun_alt_deg\": {:.2},\n    \"noon_mode\": \"{}\",\n    \"night_mode\": \"{}\",\n    \"hour_wrap_date\": \"{}\",\n    \"hour_wrap_hour\": {}\n  }},\n  \"moon\": {{\n    \"alt_deg\": {:.2},\n    \"az_deg\": {:.2},\n    \"illumination\": {:.3}\n  }},\n  \"blocks\": {{\n    \"total\": {},\n    \"building_blocks\": {},\n    \"park_blocks\": {},\n    \"noon_targets\": {},\n    \"night_targets\": {},\n    \"flicker_blocks\": {},\n    \"fade_blocks\": {},\n    \"twilight_span_minutes\": {:.1}\n  }},\n  \"render\": {{\n    \"visibility_range_entities\": {},\n    \"fog_cull_m\": {:.0},\n    \"fog_visibility_km\": {:.1},\n    \"dof_active\": {},\n    \"materials_ready\": {}\n  }},\n  \"collision\": {{\n    \"grid_solid\": {},\n    \"resolved_free\": {}\n  }},\n  \"assertions\": {{\n    \"camera_moves\": {},\n    \"camera_above_ground\": {},\n    \"noon_is_day\": {},\n    \"night_is_night\": {},\n    \"hour_wrap_ok\": {},\n    \"collision_ok\": {},\n    \"fog_cull_ok\": {}\n  }}\n}}\n",
                moved,
                test.clamp_y_after,
                MIN_CAMERA_HEIGHT,
                clamped,
                test.noon_sun_alt,
                test.night_sun_alt,
                test.noon_mode,
                test.night_mode,
                test.hour_wrap_date,
                test.hour_wrap_hour,
                moon.altitude_deg,
                moon.azimuth_deg,
                illum,
                test.blocks_total,
                test.building_blocks,
                test.park_blocks,
                test.noon_targets,
                test.night_targets,
                test.flicker_blocks,
                test.fade_blocks,
                test.twilight_span_min,
                test.cull_entities,
                test.fog_cull_m,
                test.fog_visibility_km,
                test.dof_active,
                test.materials_ready,
                test.collision_solid,
                test.collision_free,
                moved > 1.0,
                clamped,
                test.noon_mode == "day",
                test.night_mode == "night",
                test.hour_wrap_date.ends_with("22 }") && test.hour_wrap_hour == 0,
                test.collision_solid && test.collision_free,
                test.fog_cull_m >= 2_500.0 && test.fog_cull_m <= 24_000.0
            );
            let _ = std::fs::write(&path, &report);
            println!("SELFTEST REPORT {path}\n{report}");
            let ok = moved > 1.0
                && clamped
                && test.noon_mode == "day"
                && test.night_mode == "night"
                && test.hour_wrap_date.ends_with("22 }")
                && test.hour_wrap_hour == 0
                && test.blocks_total > 1000
                && test.noon_targets == 0
                && test.night_targets == test.blocks_total
                && test.flicker_blocks + test.fade_blocks == test.blocks_total
                && test.twilight_span_min > 0.0
                && test.cull_entities >= 1_000
                && test.dof_active
                && test.materials_ready
                && test.collision_solid
                && test.collision_free;
            if ok {
                exit.write(AppExit::Success);
            } else {
                exit.write(AppExit::error());
            }
            test.stage = 4;
        }
        _ => {}
    }
}

fn hide_web_loading(setup: Res<CityMaterialSetup>, mut hidden: Local<bool>) {
    if !setup.done || *hidden {
        return;
    }
    *hidden = true;
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(window) = web_sys::window()
            && let Some(document) = window.document()
            && let Some(loader) = document.get_element_by_id("loading")
        {
            let _ = loader.set_class_name("loading hidden");
        }
    }
}

#[allow(unused_variables, unused_mut)]
fn cull_web_buildings(
    env: Res<Environment>,
    camera: Single<&Transform, With<FlyCamera>>,
    mut chunks: Query<(&GlobalTransform, &mut Visibility), With<BuildingChunk>>,
) {
    #[cfg(target_arch = "wasm32")]
    {
        let cull = fog_cull_distance(env.fog_visibility);
        for (transform, mut visibility) in &mut chunks {
            let distance = transform.translation().distance(camera.translation);
            *visibility = if distance.is_finite() && distance > cull {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
    }
}
