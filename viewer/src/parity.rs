//! Configuration parity + representation audit.
//!
//! The player always launches bare `--seattle`. Any environment flag that
//! changes what is built or streamed is therefore a defect: instrumentation may
//! only add observation. This module prints the active configuration and, every
//! two seconds, an honest per-layer representation audit measured from the live
//! scene (shells, near proxies, merged batches, distinct baked tints, shipped
//! roof faces) so bare and instrumented runs can be compared like for like.

use std::collections::HashSet;

use bevy::prelude::*;

use crate::geology::Geology;

#[derive(Resource, Default)]
struct RepAudit {
    next_at: f32,
}

pub struct ParityPlugin;

impl Plugin for ParityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RepAudit>()
            .add_systems(Startup, print_config)
            .add_systems(Update, rep_audit);
    }
}

fn env(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

/// The canonical configuration line. `bare` means the player's verbatim
/// invocation; anything else is instrumentation and must not alter the world.
fn print_config() {
    let bare = !env("S2_GAUNTLET")
        && !env("FLIGHT_CAPTURE")
        && !env("EVIDENCE_CAPTURE")
        && !env("MURK_CAPTURE")
        && !env("GHOST_AERIAL")
        && !env("PEDESTRIAN_CAPTURE")
        && !env("TRAFFIC_CAPTURE")
        && !env("STOREFRONT_CAPTURE")
        && !env("SEATTLE_BOOT_CAPTURE");
    let world_flags = [
        ("SEATTLE_S2", env("SEATTLE_S2")),
        ("SEATTLE_S3", env("SEATTLE_S3")),
        ("SEATTLE_BOOT", env("SEATTLE_BOOT")),
        ("LANDMARKS_ALWAYS", env("LANDMARKS_ALWAYS")),
        ("PILOT_HORIZON", env("PILOT_HORIZON")),
        ("TREES_OFF", env("TREES_OFF")),
        ("S3_DAMAGE", env("S3_DAMAGE")),
        ("TRAFFIC_LEGACY", env("TRAFFIC_LEGACY")),
        (
            "TRAFFIC_CAP",
            std::env::var("TRAFFIC_CAP")
                .ok()
                .map(|_| true)
                .unwrap_or(false),
        ),
    ];
    let instrument = [
        ("S2_GAUNTLET", env("S2_GAUNTLET")),
        ("FLIGHT_CAPTURE", env("FLIGHT_CAPTURE")),
        ("EVIDENCE_CAPTURE", env("EVIDENCE_CAPTURE")),
        ("MURK_CAPTURE", env("MURK_CAPTURE")),
        ("PEDESTRIAN_CAPTURE", env("PEDESTRIAN_CAPTURE")),
        ("TRAFFIC_CAPTURE", env("TRAFFIC_CAPTURE")),
        ("STOREFRONT_CAPTURE", env("STOREFRONT_CAPTURE")),
        ("SEATTLE_BOOT_CAPTURE", env("SEATTLE_BOOT_CAPTURE")),
        ("REP_HUD", env("REP_HUD")),
    ];
    let world = world_flags
        .iter()
        .map(|(name, value)| match value {
            true => format!("{name}=1"),
            false => format!("{name}=0"),
        })
        .collect::<Vec<_>>()
        .join(" ");
    let inst = instrument
        .iter()
        .filter(|(_, on)| *on)
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(",");
    let command = std::env::args().collect::<Vec<_>>().join(" ");
    info!(
        "CONFIG {} cmd=[{command}] world[{world}] instrument[{inst}]",
        if bare { "bare" } else { "instrumented" }
    );
}

fn rep_audit(world: &mut World) {
    if world.get_resource::<RepAudit>().is_none() {
        return;
    }
    let now = world.resource::<Time>().elapsed_secs();
    if now < world.resource::<RepAudit>().next_at {
        return;
    }
    world.resource_mut::<RepAudit>().next_at = now + 2.0;
    let Some(geology) = world.get_resource::<Geology>() else {
        return;
    };
    let mut shells = 0usize;
    let mut proxies = 0usize;
    let mut proxy_roots = 0usize;
    let mut map_all: HashSet<u32> = HashSet::new();
    let mut map_shell: HashSet<u32> = HashSet::new();
    let mut entity_count = 0usize;
    for index in geology.building_chunks.keys() {
        for entity in world
            .get_resource::<Geology>()
            .and_then(|g| g.building_chunks.get(index))
            .into_iter()
            .flatten()
        {
            entity_count += 1;
            let building = world.get::<destructibles_bevy::building::Building>(*entity);
            match building {
                Some(building) => {
                    shells += 1;
                    map_shell.insert(building.id);
                    map_all.insert(building.id);
                }
                None => {
                    proxy_roots += 1;
                    // Distinct tints and representation kind from the proxy mesh.
                    if let Some(mesh3d) = world.get::<Mesh3d>(*entity)
                        && let Some(meshes) = world.get_resource::<Assets<Mesh>>()
                        && let Some(mesh) = meshes.get(&mesh3d.0)
                    {
                        proxies += mesh.count_vertices() / 16; // ~one box per 16 verts
                        if let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) =
                            mesh.attribute(Mesh::ATTRIBUTE_COLOR)
                        {
                            for color in colors.iter().step_by(16) {
                                map_all.insert(
                                    ((color[0] * 100.0) as u32) << 16
                                        | ((color[1] * 100.0) as u32) << 8
                                        | (color[2] * 100.0) as u32,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    let gables = crate::geology::SHIPPED_GABLES.load(std::sync::atomic::Ordering::Relaxed);
    let flats = crate::geology::SHIPPED_FLATS.load(std::sync::atomic::Ordering::Relaxed);
    let distinct = map_all.len().saturating_sub(map_shell.len());
    info!(
        "REP chunks={} entities={entity_count} shells={shells} proxy_roots={proxy_roots} proxy_boxes={proxies} tint_values={distinct} shipped_gables={gables} shipped_flats={flats}",
        geology.building_chunks.len()
    );
}
