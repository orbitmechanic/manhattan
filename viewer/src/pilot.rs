//! `--pilot` runtime: spawn the committed generated bundle only.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use destructibles_bevy::assets::WorldAssets;
use destructibles_bevy::building::{BuildingDetail, spawn_building_world_lod};
use destructibles_bevy::impact::apply_impact;
use destructibles_core::ImpactSpec;
use destructibles_core::family::MaterialFamily;
use destructibles_core::{BuildingSpec, FacadeStyle, core_for_style};
use serde::Deserialize;

use crate::camera::FlyCamera;

#[derive(Deserialize)]
struct Bundle {
    region: Region,
    records: Vec<Record>,
}

#[derive(Deserialize)]
struct Region {
    center: [f32; 2],
}

#[derive(Deserialize)]
struct Record {
    id: usize,
    origin: [f32; 3],
    height_m: f32,
    style: String,
    footprint: Vec<[f32; 2]>,
    floor_height_m: f32,
    floor_count: u32,
    #[serde(default)]
    storefronts: usize,
    #[serde(default)]
    ads: usize,
    fallback_box: Option<[f32; 3]>,
}

pub fn enabled() -> bool {
    std::env::var_os("MANHATTAN_PILOT").is_some()
}

fn style_from(label: &str) -> FacadeStyle {
    match label {
        "glass curtain wall" => FacadeStyle::GlassCurtainWall,
        "concrete grid" => FacadeStyle::ConcreteGrid,
        "brick brownstone" => FacadeStyle::BrickBrownstone,
        _ => FacadeStyle::BrickMidrise,
    }
}

pub fn spawn_pilot(world: &mut World) {
    if !enabled() {
        return;
    }
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/pilot/pilot_bundle.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        error!("PILOT: missing {}", path.display());
        return;
    };
    let Ok(bundle) = serde_json::from_str::<Bundle>(&text) else {
        error!("PILOT: invalid bundle");
        return;
    };
    let storefront_material =
        world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::srgb(0.75, 0.88, 0.92),
                emissive: LinearRgba::rgb(1.4, 1.6, 1.7),
                unlit: false,
                perceptual_roughness: 0.35,
                ..default()
            });
    let ad_material = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.25, 0.55),
            emissive: LinearRgba::rgb(2.2, 0.8, 1.4),
            unlit: false,
            ..default()
        });
    let started = std::time::Instant::now();
    let mut spawned = 0usize;
    for record in &bundle.records {
        if let Some(size) = record.fallback_box {
            let assets = world.resource::<WorldAssets>().clone();
            let mesh = world
                .resource_mut::<Assets<Mesh>>()
                .add(Cuboid::new(size[0], size[1], size[2]));
            let material = match style_from(&record.style) {
                FacadeStyle::BrickBrownstone => assets.brick.clone(),
                FacadeStyle::BrickMidrise => assets.brick.clone(),
                FacadeStyle::ConcreteGrid => assets.wall_concrete.clone(),
                FacadeStyle::GlassCurtainWall => assets.wall_glass.clone(),
            };
            world.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_xyz(record.origin[0], size[1] * 0.5, record.origin[2]),
                Name::new(format!("pilot fallback b{}", record.id)),
            ));
            spawned += 1;
            continue;
        }
        let style = style_from(&record.style);
        let family = match style {
            FacadeStyle::GlassCurtainWall | FacadeStyle::ConcreteGrid => MaterialFamily::SteelFrame,
            FacadeStyle::BrickBrownstone | FacadeStyle::BrickMidrise => {
                MaterialFamily::MasonryStone
            }
        };
        let spec = BuildingSpec {
            family,
            footprint: record.footprint.clone(),
            floor_height: record.floor_height_m,
            floor_count: record.floor_count,
            facade: style.material(),
            facade_style: style,
            windows: {
                let mut windows = style.windows();
                windows.height = (record.floor_height_m - 0.10).clamp(1.1, 3.0);
                windows.sill_height = 0.0;
                windows
            },
            core: core_for_style(style, &record.footprint, record.id as u32),
        };
        let _ = record.height_m;
        spawn_building_world_lod(
            world,
            record.id as u32,
            spec,
            Vec3::from_array(record.origin),
            BuildingDetail::Shell,
        );
        if !record.footprint.is_empty() {
            let minx = record
                .footprint
                .iter()
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min);
            let maxx = record
                .footprint
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max);
            let maxz = record
                .footprint
                .iter()
                .map(|p| p[1])
                .fold(f32::NEG_INFINITY, f32::max);
            let width = (maxx - minx).max(3.0);
            for i in 0..record.storefronts {
                let x = record.origin[0]
                    + minx
                    + width * (i as f32 + 0.5) / record.storefronts.max(1) as f32;
                let mesh = world.resource_mut::<Assets<Mesh>>().add(Cuboid::new(
                    width / record.storefronts.max(1) as f32 * 0.85,
                    2.8,
                    0.18,
                ));
                world.spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(storefront_material.clone()),
                    Transform::from_xyz(x, record.origin[1] + 1.5, record.origin[2] + maxz + 0.12),
                    Name::new(format!("pilot storefront b{}", record.id)),
                ));
            }
            for i in 0..record.ads {
                let x = record.origin[0] + minx + width * 0.5;
                let mesh = world
                    .resource_mut::<Assets<Mesh>>()
                    .add(Cuboid::new(2.2, 1.2, 0.12));
                world.spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(ad_material.clone()),
                    Transform::from_xyz(
                        x,
                        record.origin[1] + 7.0 + i as f32 * 1.4,
                        record.origin[2] + maxz + 0.16,
                    ),
                    Name::new(format!("pilot ad b{}", record.id)),
                ));
            }
        }
        spawned += 1;
    }
    if let Ok(mut camera) = world
        .query_filtered::<&mut Transform, With<FlyCamera>>()
        .single_mut(world)
    {
        camera.translation = Vec3::new(
            bundle.region.center[0],
            90.0,
            bundle.region.center[1] + 260.0,
        );
        camera.rotation = Quat::from_rotation_x(-0.25);
    }
    info!(
        "PILOT: spawned {spawned} shell buildings in {:.2} s",
        started.elapsed().as_secs_f32()
    );
}

/// Post-startup camera placement for the pilot bundle.
pub fn pilot_camera(mut done: Local<bool>, mut camera: Query<&mut Transform, With<FlyCamera>>) {
    if *done || !enabled() {
        return;
    }
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/pilot/pilot_bundle.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(bundle) = serde_json::from_str::<Bundle>(&text) else {
        return;
    };
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let aerial = std::env::var_os("PILOT_AERIAL").is_some();
    let height = if aerial { 320.0 } else { 8.0 };
    let offset = if aerial { 420.0 } else { 95.0 };
    transform.translation = Vec3::new(
        bundle.region.center[0],
        height,
        bundle.region.center[1] + offset,
    );
    let look = Vec3::new(
        bundle.region.center[0],
        if aerial { 40.0 } else { 12.0 },
        bundle.region.center[1],
    );
    let direction = (look - transform.translation).normalize_or_zero();
    let yaw = (-direction.x).atan2(-direction.z);
    let pitch = direction.y.asin();
    transform.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    *done = true;
}

#[derive(Default)]
pub struct PerfState {
    pose: usize,
    elapsed: f32,
    samples: Vec<f32>,
    settling: bool,
}

pub fn pilot_perf(
    time: Res<Time>,
    mut state: Local<PerfState>,
    mut camera: Query<&mut Transform, With<FlyCamera>>,
    mut clock: ResMut<crate::clock::SimClock>,
    meshes: Res<Assets<Mesh>>,
    mesh_query: Query<&Mesh3d>,
    mats: Query<&MeshMaterial3d<StandardMaterial>>,
    mut exit: MessageWriter<AppExit>,
) {
    if !enabled() || std::env::var_os("PILOT_PERF").is_none() {
        return;
    }
    let poses = [
        ("street_day", 12.0, 8.0, 95.0),
        ("aerial_day", 12.0, 320.0, 420.0),
        ("street_night", 0.0, 8.0, 95.0),
        ("aerial_night", 0.0, 320.0, 420.0),
    ];
    if state.pose >= poses.len() {
        let _ = exit.write(AppExit::Success);
        return;
    }
    let (name, hour, height, offset) = poses[state.pose];
    if state.elapsed == 0.0 {
        clock.minute_of_day = hour * 60.0;
        if let Ok(mut t) = camera.single_mut() {
            // region center from bundle
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/pilot/pilot_bundle.json");
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(b) = serde_json::from_str::<Bundle>(&text) {
                    t.translation =
                        Vec3::new(b.region.center[0], height, b.region.center[1] + offset);
                    let look = Vec3::new(
                        b.region.center[0],
                        if height > 100.0 { 40.0 } else { 12.0 },
                        b.region.center[1],
                    );
                    let d = (look - t.translation).normalize_or_zero();
                    t.rotation =
                        Quat::from_euler(EulerRot::YXZ, (-d.x).atan2(-d.z), d.y.asin(), 0.0);
                }
            }
        }
        state.settling = true;
    }
    state.elapsed += time.delta_secs();
    if state.settling {
        if state.elapsed >= 2.0 {
            state.settling = false;
            state.elapsed = 0.0;
            state.samples.clear();
        }
        return;
    }
    state.samples.push(time.delta_secs() * 1000.0);
    if state.elapsed >= 2.0 {
        let mut sorted = state.samples.clone();
        sorted.sort_by(f32::total_cmp);
        let avg = sorted.iter().sum::<f32>() / sorted.len().max(1) as f32;
        let p99 = sorted[((sorted.len() - 1) as f32 * 0.99) as usize];
        let groups = mats
            .iter()
            .map(|m| m.0.id())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let triangles: usize = mesh_query
            .iter()
            .filter_map(|m| meshes.get(&m.0))
            .map(|m| m.count_vertices() / 3)
            .sum();
        let entities = mesh_query.iter().count();
        println!(
            "PILOT PERF {name}: avg {avg:.2} ms p99 {p99:.2} ms draw_materials {groups} triangles {triangles} mesh_entities {entities}"
        );
        state.pose += 1;
        state.elapsed = 0.0;
        state.samples.clear();
    }
}

#[derive(Resource, Default)]
pub struct PilotDamageState {
    stage: u8,
    since: f32,
    id: Option<usize>,
}

pub fn pilot_damage(world: &mut World) {
    if !enabled() || std::env::var_os("PILOT_DAMAGE").is_none() {
        return;
    }
    let now = world.resource::<Time>().elapsed_secs();
    let mut state = std::mem::take(&mut *world.resource_mut::<PilotDamageState>());
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/pilot/pilot_bundle.json");
    match state.stage {
        0 => {
            if now < 4.0 {
                *world.resource_mut::<PilotDamageState>() = state;
                return;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                return;
            };
            let Ok(bundle) = serde_json::from_str::<Bundle>(&text) else {
                return;
            };
            let Some(record) = bundle.records.iter().find(|r| r.fallback_box.is_none()) else {
                return;
            };
            let id = record.id;
            let origin = Vec3::from_array(record.origin);
            if let Ok(mut camera) = world
                .query_filtered::<&mut Transform, With<FlyCamera>>()
                .single_mut(world)
            {
                let p = origin + Vec3::new(0.0, 8.0, 42.0);
                let look = origin + Vec3::Y * 12.0;
                let d = (look - p).normalize_or_zero();
                camera.translation = p;
                camera.rotation =
                    Quat::from_euler(EulerRot::YXZ, (-d.x).atan2(-d.z), d.y.asin(), 0.0);
            }
            destructibles_bevy::building::begin_detail_upgrade(
                world,
                id as u32,
                destructibles_bevy::building::DetailTier::Structural,
            );
            let started = std::time::Instant::now();
            for _ in 0..80 {
                let level = destructibles_bevy::building::advance_detail(world, id as u32, 50.0);
                if level >= destructibles_bevy::building::DetailTier::Structural {
                    break;
                }
            }
            info!(
                "PILOT DAMAGE: b{id} tiered to {:?} in {:.0} ms",
                destructibles_bevy::building::detail_level(world, id as u32),
                started.elapsed().as_secs_f32() * 1000.0
            );
            state.id = Some(id);
            state.stage = 1;
            state.since = now;
        }
        1 => {
            let id = state.id.unwrap_or(0);
            let origin = Vec3::new(0.0, 0.0, 0.0);
            let record = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<Bundle>(&t).ok())
                .and_then(|b| b.records.into_iter().find(|r| r.id == id));
            if let Some(r) = record {
                let o = Vec3::from_array(r.origin);
                let impact = ImpactSpec {
                    position: (o + Vec3::new(0.0, 6.0, 8.0)).to_array(),
                    radius: 0.0,
                    velocity: [0.0, -40.0, -300.0],
                    mass_kg: 20.0,
                    explosive_yield_kg_tnt: 0.0,
                };
                let started = std::time::Instant::now();
                apply_impact(world, id as u32, impact);
                info!(
                    "PILOT DAMAGE: bullet b{id} admitted in {:.1} ms journal={:.2}",
                    started.elapsed().as_secs_f32() * 1000.0,
                    world
                        .resource::<destructibles_bevy::damage::DamageJournal>()
                        .0
                        .head_for(id as u32)
                );
                let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("captures/pilot_damage_bullet.png");
                world
                    .spawn(Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(p));
            }
            let _ = origin;
            state.stage = 2;
            state.since = now;
        }
        2 => {
            if now - state.since < 2.0 {
                *world.resource_mut::<PilotDamageState>() = state;
                return;
            }
            let id = state.id.unwrap_or(0);
            let record = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<Bundle>(&t).ok())
                .and_then(|b| b.records.into_iter().find(|r| r.id == id));
            if let Some(r) = record {
                let o = Vec3::from_array(r.origin);
                let impact = ImpactSpec {
                    position: (o + Vec3::new(0.0, 3.0, 8.0)).to_array(),
                    radius: 0.0,
                    velocity: [0.0, 0.0, -42.0],
                    mass_kg: 1500.0,
                    explosive_yield_kg_tnt: 5.0,
                };
                apply_impact(world, id as u32, impact);
                info!(
                    "PILOT DAMAGE: car bomb b{id} journal={:.2}",
                    world
                        .resource::<destructibles_bevy::damage::DamageJournal>()
                        .0
                        .head_for(id as u32)
                );
                let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("captures/pilot_damage_carbomb.png");
                world
                    .spawn(Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(p));
            }
            state.stage = 3;
            state.since = now;
        }
        _ => {
            if now - state.since > 3.0 {
                world.write_message(AppExit::Success);
            }
        }
    }
    *world.resource_mut::<PilotDamageState>() = state;
}
