//! `--pilot` runtime: spawn the committed generated bundle only.

use bevy::prelude::*;
use destructibles_bevy::assets::WorldAssets;
use destructibles_bevy::building::spawn_building_world;
use destructibles_core::{BuildingSpec, FacadeStyle, core_for_style};
use destructibles_core::family::MaterialFamily;
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
            FacadeStyle::BrickBrownstone | FacadeStyle::BrickMidrise => MaterialFamily::MasonryStone,
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
        spawn_building_world(
            world,
            record.id as u32,
            spec,
            Vec3::from_array(record.origin),
        );
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
    info!("PILOT: spawned {spawned} generated buildings");
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
