//! Pitched residential roofs, built at spawn from pre-computed seeds.
//!
//! One-to-two-storey residential shells get a hip/gable roof by the
//! bisector method: each footprint corner's angle bisector is intersected with
//! its neighbours on the roof plane, which is exactly the first step of the
//! straight skeleton. For a convex footprint the inward offset polygon is the
//! ridge/hip ring at `rise = inset * tan(pitch)`; side quads join the eave ring
//! to it. Concave or degenerate skeletons fall back to the flat cap and are
//! counted. Pitch is 15-45 degrees, seeded per building.
//!
//! Materials are vertex colours over the destructibles roof material: wood
//! shake dominant, asphalt cheaper, tile/metal rare, a moss-green eave band on
//! every roof, and ~5 % seeded solar arrays. The collider stays the shell's
//! existing flat roof AABB (visual wedge only).
//!
//! Roof *parameters* (pitch, material, solar) are derived from the pack at
//! build time by `scripts/seattle_roofs.py`; the wedge itself is assembled at
//! spawn because the binary pack format has no roof vertex block yet. That gap
//! is the filed `roof-pack-bake` debt.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, Mesh, PrimitiveTopology};

use crate::geology::Geology;

#[derive(Component)]
pub struct RoofInstalled;

#[derive(Resource, Default)]
pub struct RoofStats {
    pub installed: u32,
    pub pitched: u32,
    pub flat_fallback: u32,
    pub solar: u32,
    pub shake: u32,
    pub asphalt: u32,
    pub tile: u32,
    pub metal: u32,
    pub next_report: f32,
    /// Installed roof entities by building, so a tier-up rebuild can re-roof.
    pub roots: HashMap<u32, Entity>,
}

pub struct RoofPlugin;

impl Plugin for RoofPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RoofStats>()
            .add_systems(Update, install_roofs);
    }
}

fn hash01(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846C_A68F);
    value ^= value >> 16;
    (value & 0x00FF_FFFF) as f32 / 16_777_216.0
}

fn inset_polygon(footprint: &[[f32; 2]], inset: f32) -> Option<Vec<[f32; 2]>> {
    let count = footprint.len();
    if count < 3 {
        return None;
    }
    // CCW so the inward normal is the left normal.
    let mut ring = footprint.to_vec();
    let area: f32 = (0..count)
        .map(|index| {
            let a = ring[index];
            let b = ring[(index + 1) % count];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        * 0.5;
    if area < 0.0 {
        ring.reverse();
    }
    let count = ring.len();
    let mut lines = Vec::with_capacity(count);
    for index in 0..count {
        let a = ring[index];
        let b = ring[(index + 1) % count];
        let edge = Vec2::new(b[0] - a[0], b[1] - a[1]);
        let len = edge.length();
        if len < 1.0e-3 {
            return None;
        }
        let normal = Vec2::new(-edge.y, edge.x) / len;
        let offset = Vec2::new(a[0], a[1]) + normal * inset;
        lines.push((offset, edge / len));
    }
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let (point_a, dir_a) = lines[index];
        let (point_b, dir_b) = lines[(index + 1) % count];
        let cross = dir_a.x * dir_b.y - dir_a.y * dir_b.x;
        if cross.abs() < 1.0e-5 {
            return None;
        }
        let t = ((point_b.x - point_a.x) * dir_b.y - (point_b.y - point_a.y) * dir_b.x) / cross;
        out.push([point_a.x + dir_a.x * t, point_a.y + dir_a.y * t]);
    }
    Some(out)
}

fn roof_mesh(footprint: &[[f32; 2]], pitch: f32, material: u8, solar: bool) -> (Mesh, bool) {
    let count = footprint.len();
    let mut min = Vec2::splat(f32::MAX);
    let mut max = Vec2::splat(f32::MIN);
    for point in footprint {
        min = min.min(Vec2::from_array(*point));
        max = max.max(Vec2::from_array(*point));
    }
    let min_dim = (max - min).min_element().max(1.0);
    let inset = min_dim * 0.5;
    let rise = inset * pitch.tan();
    let Some(ring) = inset_polygon(footprint, inset) else {
        return (
            Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all()),
            true,
        );
    };
    let ring_area: f32 = (0..ring.len())
        .map(|index| {
            let a = ring[index];
            let b = ring[(index + 1) % ring.len()];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        .abs()
        * 0.5;
    if ring_area < 1.0 {
        return (
            Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all()),
            true,
        );
    }
    let roof_colors: [[f32; 4]; 4] = [
        [0.30, 0.19, 0.11, 1.0], // wood shake, dark brown
        [0.24, 0.24, 0.24, 1.0], // asphalt, dark grey
        [0.20, 0.13, 0.10, 1.0], // tile, dark clay
        [0.16, 0.17, 0.18, 1.0], // metal, near-graphite
    ];
    let roof = roof_colors[material as usize % 4];
    let moss = [0.13, 0.18, 0.075, 1.0];
    // Moss only on wood/tile; asphalt and metal stay clean-ish but still mossed
    // at the eave per Pacific NW truth.
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    if ring_area < 1.0 {
        // Gable: ridge along the longer AABB axis, length long - short.
        let (x0, z0) = (min.x, min.y);
        let (x1, z1) = (max.x, max.y);
        let along_x = (x1 - x0) >= (z1 - z0);
        let ridge_half = (if along_x {
            (x1 - x0) - (z1 - z0)
        } else {
            (z1 - z0) - (x1 - x0)
        }) * 0.5;
        let mid = if along_x {
            (z0 + z1) * 0.5
        } else {
            (x0 + x1) * 0.5
        };
        let centre = if along_x {
            (x0 + x1) * 0.5
        } else {
            (z0 + z1) * 0.5
        };
        let (ra, rb) = if along_x {
            (
                [centre - ridge_half.max(0.0), rise, mid],
                [centre + ridge_half.max(0.0), rise, mid],
            )
        } else {
            (
                [mid, rise, centre - ridge_half.max(0.0)],
                [mid, rise, centre + ridge_half.max(0.0)],
            )
        };
        let corners = [[x0, 0.0, z0], [x1, 0.0, z0], [x1, 0.0, z1], [x0, 0.0, z1]];
        // Two slopes plus two gable-end triangles.
        if along_x {
            push_quad(
                &mut positions,
                &mut normals,
                &mut colors,
                &mut indices,
                corners[0],
                corners[1],
                rb,
                ra,
                roof,
            );
            push_quad(
                &mut positions,
                &mut normals,
                &mut colors,
                &mut indices,
                corners[3],
                corners[2],
                rb,
                ra,
                roof,
            );
            push_quad(
                &mut positions,
                &mut normals,
                &mut colors,
                &mut indices,
                corners[1],
                corners[2],
                ra,
                rb,
                moss,
            );
            push_quad(
                &mut positions,
                &mut normals,
                &mut colors,
                &mut indices,
                corners[0],
                corners[3],
                ra,
                rb,
                moss,
            );
        } else {
            push_quad(
                &mut positions,
                &mut normals,
                &mut colors,
                &mut indices,
                corners[0],
                corners[3],
                ra,
                rb,
                roof,
            );
            push_quad(
                &mut positions,
                &mut normals,
                &mut colors,
                &mut indices,
                corners[1],
                corners[2],
                ra,
                rb,
                roof,
            );
            push_quad(
                &mut positions,
                &mut normals,
                &mut colors,
                &mut indices,
                corners[0],
                corners[1],
                rb,
                ra,
                moss,
            );
            push_quad(
                &mut positions,
                &mut normals,
                &mut colors,
                &mut indices,
                corners[2],
                corners[3],
                rb,
                ra,
                moss,
            );
        }
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        mesh.insert_indices(Indices::U32(indices));
        return (mesh, false);
    }
    #[allow(clippy::too_many_arguments)]
    fn push_quad(
        positions: &mut Vec<[f32; 3]>,
        normals: &mut Vec<[f32; 3]>,
        colors: &mut Vec<[f32; 4]>,
        indices: &mut Vec<u32>,
        a: [f32; 3],
        b: [f32; 3],
        c: [f32; 3],
        d: [f32; 3],
        color: [f32; 4],
    ) {
        let base = positions.len() as u32;
        let normal = Vec3::from_array(c) - Vec3::from_array(a);
        let normal = normal
            .cross(Vec3::from_array(d) - Vec3::from_array(b))
            .normalize_or_zero();
        for point in [a, b, c, d] {
            positions.push(point);
            normals.push(normal.to_array());
            colors.push(color);
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    for index in 0..count {
        let a = footprint[index];
        let b = footprint[(index + 1) % count];
        let a2 = ring[index];
        let b2 = ring[(index + 1) % ring.len()];
        // Eave ring is moss-dark; the roof face is the material colour. Split
        // each side into an eave band and a roof band.
        let a0 = [a[0], 0.0, a[1]];
        let b0 = [b[0], 0.0, b[1]];
        let band = 0.35;
        let a1 = [
            a[0] + (a2[0] - a[0]) * band,
            rise * band,
            a[1] + (a2[1] - a[1]) * band,
        ];
        let b1 = [
            b[0] + (b2[0] - b[0]) * band,
            rise * band,
            b[1] + (b2[1] - b[1]) * band,
        ];
        let a3 = [a2[0], rise, a2[1]];
        let b3 = [b2[0], rise, b2[1]];
        push_quad(
            &mut positions,
            &mut normals,
            &mut colors,
            &mut indices,
            a0,
            b0,
            b1,
            a1,
            moss,
        );
        push_quad(
            &mut positions,
            &mut normals,
            &mut colors,
            &mut indices,
            a1,
            b1,
            b3,
            a3,
            roof,
        );
    }
    // Ridge/hip ring cap.
    let ring_len = ring.len();
    for index in 1..ring_len.saturating_sub(1) {
        let base = positions.len() as u32;
        let a = [ring[0][0], rise, ring[0][1]];
        let b = [ring[index][0], rise, ring[index][1]];
        let c = [ring[index + 1][0], rise, ring[index + 1][1]];
        for point in [a, b, c] {
            positions.push(point);
            normals.push([0.0, 1.0, 0.0]);
            colors.push(roof);
        }
        indices.extend([base, base + 1, base + 2]);
    }
    if solar {
        // A seeded array on one half of the pitch, near-black panels.
        let solar_color = [0.035, 0.04, 0.05, 1.0];
        let a = footprint[0];
        let b = footprint[1];
        let mid_a = [
            a[0] * 0.45 + ring[0][0] * 0.55,
            rise * 0.6,
            a[1] * 0.45 + ring[0][1] * 0.55,
        ];
        let mid_b = [
            b[0] * 0.45 + ring[1 % ring_len][0] * 0.55,
            rise * 0.6,
            b[1] * 0.45 + ring[1 % ring_len][1] * 0.55,
        ];
        let top_a = [
            a[0] * 0.2 + ring[0][0] * 0.8,
            rise * 0.95,
            a[1] * 0.2 + ring[0][1] * 0.8,
        ];
        let top_b = [
            b[0] * 0.2 + ring[1 % ring_len][0] * 0.8,
            rise * 0.95,
            b[1] * 0.2 + ring[1 % ring_len][1] * 0.8,
        ];
        push_quad(
            &mut positions,
            &mut normals,
            &mut colors,
            &mut indices,
            mid_a,
            mid_b,
            top_b,
            top_a,
            solar_color,
        );
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    (mesh, false)
}

fn install_roofs(world: &mut World) {
    let Some(assets) = world.get_resource::<destructibles_bevy::assets::WorldAssets>() else {
        return;
    };
    let material = assets.roof.clone();
    let now = world.resource::<Time>().elapsed_secs();
    let due = now > world.resource::<RoofStats>().next_report;
    let roots = world
        .query_filtered::<(Entity, &destructibles_bevy::building::Building), Without<RoofInstalled>>()
        .iter(world)
        .map(|(entity, building)| (entity, building.id))
        .collect::<Vec<_>>();
    let mut budget = 48usize;
    for (root, id) in roots {
        if budget == 0 {
            break;
        }
        if world.get_entity(root).is_err() {
            continue;
        }
        let Some((origin, height, floors, footprint)) = world
            .get_resource::<destructibles_bevy::building::Buildings>()
            .and_then(|buildings| buildings.get(id))
            .map(|record| {
                (
                    record.origin,
                    record.layout.height,
                    record.layout.spec.floor_count,
                    record.layout.footprint.clone(),
                )
            })
        else {
            continue;
        };
        if floors > 2 || footprint.len() < 3 {
            let _ = world.entity_mut(root).insert(RoofInstalled);
            continue;
        }
        let area: f32 = (0..footprint.len())
            .map(|index| {
                let a = footprint[index];
                let b = footprint[(index + 1) % footprint.len()];
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f32>()
            .abs()
            * 0.5;
        if area < 12.0 {
            let _ = world.entity_mut(root).insert(RoofInstalled);
            continue;
        }
        let seed = id;
        let pitch = (15.0 + 30.0 * hash01(seed.wrapping_mul(0x9E37_79B9))).to_radians();
        let material_roll = hash01(seed.rotate_left(7) ^ 0x51ED_2701);
        let material_index = if material_roll < 0.55 {
            0
        } else if material_roll < 0.85 {
            1
        } else if material_roll < 0.95 {
            2
        } else {
            3
        };
        let solar = hash01(seed ^ 0xA53C_9E11) < 0.05;
        let (mesh, flat) = roof_mesh(&footprint, pitch, material_index, solar);
        {
            let mut stats = world.resource_mut::<RoofStats>();
            stats.installed += 1;
            if flat {
                stats.flat_fallback += 1;
            } else {
                stats.pitched += 1;
                match material_index {
                    0 => stats.shake += 1,
                    1 => stats.asphalt += 1,
                    2 => stats.tile += 1,
                    _ => stats.metal += 1,
                }
                if solar {
                    stats.solar += 1;
                }
            }
        }
        if !flat {
            let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
            let entity = world
                .spawn((
                    Mesh3d(handle),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(origin + Vec3::Y * height),
                    ChildOf(root),
                    Name::new(format!("pitched roof b{id}")),
                ))
                .id();
            world.resource_mut::<RoofStats>().roots.insert(id, entity);
        }
        let _ = world.entity_mut(root).insert(RoofInstalled);
        budget -= 1;
    }
    if due {
        let chunks = world
            .get_resource::<Geology>()
            .map(|geology| geology.building_chunks.len())
            .unwrap_or(0);
        let mut stats = world.resource_mut::<RoofStats>();
        stats.next_report = now + 10.0;
        if stats.installed > 0 {
            let total = stats.shake + stats.asphalt + stats.tile + stats.metal;
            let pct = |value: u32| {
                if total > 0 {
                    100.0 * value as f32 / total as f32
                } else {
                    0.0
                }
            };
            info!(
                "ROOFS installed={} pitched={} flat_fallback={} solar={} shake={:.0}% asphalt={:.0}% tile={:.0}% metal={:.0}% chunks={chunks}",
                stats.installed,
                stats.pitched,
                stats.flat_fallback,
                stats.solar,
                pct(stats.shake),
                pct(stats.asphalt),
                pct(stats.tile),
                pct(stats.metal)
            );
        }
    }
}
