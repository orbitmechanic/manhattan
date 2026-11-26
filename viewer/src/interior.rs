//! Camera interior ban + sonic camera.
//!
//! Buildings are solid to the camera: a camera that would end a frame inside a
//! hull is deflected along the hull tangent (glancing, never a hard stop), with
//! the correction scaled by speed so a slow walk eases out and a mach pass
//! glances off. The collision grid covers Manhattan only, so Seattle needs its
//! own test against the live destructibles registry.
//!
//! The sonic camera is the Manhattan motion treatment reinstalled: one shared
//! `sonic_gain` curve drives the wind-rush volume and playback rate (the speed
//! cue), a small FOV pulse and a speed-scaled shake. Staged capture cameras are
//! never shaken — the Camera Ownership Law outranks the effect.

use bevy::prelude::*;

use crate::camera::FlyCamera;
use crate::camera_authority::StagedPose;
use crate::citypack::STREAM_WORLD_OFFSET;
use crate::motion::{MotionState, sonic_gain};

#[derive(Resource, Default)]
pub struct InteriorBanState {
    last_log: f32,
    last_building: u32,
    last_cost_log: f32,
}

pub struct InteriorPlugin;

impl Plugin for InteriorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InteriorBanState>()
            .init_resource::<InteriorProbeState>()
            .init_resource::<ProxyProbeState>()
            .init_resource::<ProxyGrid>()
            .add_systems(
                Update,
                (
                    interior_probe,
                    proxy_probe,
                    camera_interior_ban,
                    sonic_camera,
                    sonic_table,
                ),
            );
    }
}

/// INTERIOR_PROBE=1: teleport the camera inside the nearest resident shell so
/// the ban has a deliberate fly-through to log.
#[derive(Resource, Default)]
pub struct InteriorProbeState {
    done: bool,
}

fn interior_probe(world: &mut World) {
    if std::env::var_os("INTERIOR_PROBE").is_none() {
        return;
    }
    if world.resource::<InteriorProbeState>().done || world.resource::<Time>().elapsed_secs() < 6.0
    {
        return;
    }
    world.resource_mut::<InteriorProbeState>().done = true;
    let Some(camera) = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .map(|transform| transform.translation)
    else {
        return;
    };
    let target = world
        .resource::<destructibles_bevy::building::Buildings>()
        .all()
        .map(|record| (record.id, record.origin, record.layout.height))
        .min_by(|a, b| a.1.distance(camera).total_cmp(&b.1.distance(camera)));
    let Some((id, origin, height)) = target else {
        return;
    };
    if let Ok(mut transform) = world
        .query_filtered::<&mut Transform, With<FlyCamera>>()
        .single_mut(world)
    {
        transform.translation = origin + Vec3::Y * height * 0.3;
    }
    info!(
        "INTERIOR PROBE: entering b{id} at ({:.0},{:.0}) height {height:.0}",
        origin.x - STREAM_WORLD_OFFSET,
        origin.z
    );
}

/// Resolve a camera inside a proxy AABB. Speed 0 pushes straight out and
/// stops; speed adds a tangential glance. Returns the resolved position and
/// the outward normal.
pub fn box_escape(pos: Vec3, center: Vec3, half: Vec3, speed: f32) -> Option<(Vec3, Vec2)> {
    let local = pos - center;
    if local.x.abs() > half.x + 0.3 || local.z.abs() > half.z + 0.3 {
        return None;
    }
    if pos.y > center.y + half.y || pos.y < center.y - half.y {
        return None;
    }
    let dx = half.x + 0.6 - local.x.abs();
    let dz = half.z + 0.6 - local.z.abs();
    let (normal, depth) = if dx < dz {
        (Vec2::new(local.x.signum(), 0.0), dx)
    } else {
        (Vec2::new(0.0, local.z.signum()), dz)
    };
    let glance = (speed / 650.0).clamp(0.0, 1.0);
    let tangent = Vec2::new(-normal.y, normal.x) * (0.9 * glance);
    let resolved = Vec3::new(
        center.x + local.x + normal.x * (depth + 0.05) + tangent.x,
        pos.y,
        center.z + local.z + normal.y * (depth + 0.05) + tangent.y,
    );
    Some((resolved, normal))
}

/// SEATTLE_PROXY_PROBE=1: deliberately enter a far-merged bucket building
/// (boxes-only chunk) so the proxy AABB path has a real entry to log.
#[derive(Resource, Default)]
pub struct ProxyProbeState {
    done: bool,
}

fn proxy_probe(world: &mut World) {
    if std::env::var_os("SEATTLE_PROXY_PROBE").is_none() {
        return;
    }
    if world.resource::<ProxyProbeState>().done || world.resource::<Time>().elapsed_secs() < 9.0 {
        return;
    }
    let Some(cam) = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .map(|transform| transform.translation)
    else {
        return;
    };
    let target = {
        let Some(geology) = world.get_resource::<crate::geology::Geology>() else {
            return;
        };
        geology
            .proxy_colliders
            .iter()
            .filter_map(|(chunk, list)| {
                let node = geology.pack.nodes.get(*chunk)?;
                let distance = (node.center[0] - cam.x).hypot(node.center[1] - cam.z);
                (distance > 500.0)
                    .then(|| {
                        list.first()
                            .map(|entry| (*chunk, entry.0, entry.1, entry.2))
                    })
                    .flatten()
            })
            .next()
    };
    let Some((chunk, center, half, id)) = target else {
        return;
    };
    let inside = Vec3::new(center.x, (center.y - half.y * 0.5).max(3.0), center.z);
    if let Ok(mut transform) = world
        .query_filtered::<&mut Transform, With<FlyCamera>>()
        .single_mut(world)
    {
        transform.translation = inside;
    }
    world.resource_mut::<ProxyProbeState>().done = true;
    info!(
        "PROXY PROBE entering far bucket chunk={chunk} b{id} at ({:.1},{:.1},{:.1})",
        inside.x - crate::citypack::STREAM_WORLD_OFFSET,
        inside.y,
        inside.z
    );
}

/// Sector-grid broadphase over the proxy AABBs: 200 m cells, rebuilt only when
/// the collider set changes. A per-frame full scan would be a defect once the
/// far buckets carry thousands of boxes.
const PROXY_CELL: f32 = 200.0;

#[derive(Resource, Default)]
pub struct ProxyGrid {
    stamp: u64,
    cells: std::collections::HashMap<(i32, i32), Vec<(Vec3, Vec3, u32)>>,
    boxes: usize,
    rebuilds: u32,
    last_ms: f32,
    worst_ms: f32,
}

impl ProxyGrid {
    fn sync(&mut self, stamp: u64, source: &[(Vec3, Vec3, u32)]) {
        if self.stamp == stamp {
            return;
        }
        let started = crate::clock_compat::Instant::now();
        self.stamp = stamp;
        self.cells.clear();
        self.boxes = source.len();
        for entry in source {
            let key = cell_of(entry.0);
            self.cells.entry(key).or_default().push(*entry);
        }
        self.last_ms = started.elapsed().as_secs_f32() * 1000.0;
        self.worst_ms = self.worst_ms.max(self.last_ms);
        self.rebuilds += 1;
    }

    fn near(&self, position: Vec3) -> impl Iterator<Item = &(Vec3, Vec3, u32)> {
        let (cx, cz) = cell_of(position);
        (-1..=1)
            .flat_map(move |dx| (-1..=1).map(move |dz| (cx + dx, cz + dz)))
            .filter_map(|key| self.cells.get(&key))
            .flat_map(|list| list.iter())
    }
}

fn cell_of(position: Vec3) -> (i32, i32) {
    (
        (position.x / PROXY_CELL).floor() as i32,
        (position.z / PROXY_CELL).floor() as i32,
    )
}

static COLLIDER_DUMPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn camera_interior_ban(world: &mut World) {
    if world
        .get_resource::<StagedPose>()
        .is_some_and(|pose| pose.active)
    {
        return;
    }
    let Some(elapsed) = world.get_resource::<Time>().map(|time| time.elapsed_secs()) else {
        return;
    };
    let Some(position) = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .map(|transform| transform.translation)
    else {
        return;
    };
    let mut speed = world
        .get_resource::<MotionState>()
        .map(|motion| motion.speed)
        .unwrap_or(0.0);
    // Probe instrument: the non-interactive harness cannot hold a stick, so a
    // deliberate-entry probe may declare its entry speed. Logged as a probe.
    let probe_speed = std::env::var("SEATTLE_PROBE_SPEED").ok();
    if let Some(probe) = probe_speed
        .as_deref()
        .and_then(|text| text.parse::<f32>().ok())
    {
        speed = probe;
    }
    // One-off diagnostic dump of the proxy collider set (probe only).
    if elapsed > 8.0
        && std::env::var_os("SEATTLE_COLLIDER_DUMP").is_some()
        && !COLLIDER_DUMPED.swap(true, std::sync::atomic::Ordering::Relaxed)
        && let Some(geology) = world.get_resource::<crate::geology::Geology>()
    {
        let mut total = 0usize;
        for (chunk, list) in &geology.proxy_colliders {
            total += list.len();
            if let Some((center, half, id)) = list.first() {
                info!(
                    "PROXY COLLIDER chunk={chunk} b{id} center=({:.1},{:.1},{:.1}) half=({:.1},{:.1},{:.1}) n={}",
                    center.x - crate::citypack::STREAM_WORLD_OFFSET,
                    center.y,
                    center.z,
                    half.x,
                    half.y,
                    half.z,
                    list.len()
                );
            }
        }
        let shells = world
            .get_resource::<destructibles_bevy::building::Buildings>()
            .map(|buildings| {
                buildings
                    .all()
                    .map(|record| record.id)
                    .collect::<std::collections::HashSet<u32>>()
            })
            .unwrap_or_default();
        let free = geology
            .proxy_colliders
            .values()
            .flat_map(|list| list.iter())
            .find(|(_, _, id)| !shells.contains(id));
        info!(
            "PROXY COLLIDERS total={total} chunks={} shells={} free_sample={:?}",
            geology.proxy_colliders.len(),
            shells.len(),
            free.map(|(center, half, id)| (
                id,
                (center.x - crate::citypack::STREAM_WORLD_OFFSET).round(),
                center.y.round(),
                center.z.round(),
                half.round()
            ))
        );
    }
    let buildings = world.resource::<destructibles_bevy::building::Buildings>();
    let mut hit: Option<(u32, Vec2, Vec2)> = None;
    for record in buildings.all() {
        // Footprints are centroid-relative; the camera is world-space.
        let local = Vec2::new(position.x - record.origin.x, position.z - record.origin.z);
        let top = record.origin.y + record.layout.height;
        if position.y > top + 0.2 || position.y < record.origin.y - 2.0 {
            continue;
        }
        if !destructibles_core::math::point_in_polygon([local.x, local.y], &record.layout.footprint)
        {
            continue;
        }
        // Nearest hull edge: the deflection normal is the outward edge normal,
        // the tangent runs along the edge.
        let footprint = &record.layout.footprint;
        let mut best = (f32::MAX, Vec2::ZERO, Vec2::ZERO);
        for index in 0..footprint.len() {
            let a = Vec2::from(footprint[index]);
            let b = Vec2::from(footprint[(index + 1) % footprint.len()]);
            let edge = b - a;
            let len = edge.length().max(1.0e-4);
            let t = ((local - a).dot(edge) / (len * len)).clamp(0.0, 1.0);
            let point = a + edge * t;
            let distance = (local - point).length();
            if distance < best.0 {
                let tangent = edge / len;
                let normal = Vec2::new(-tangent.y, tangent.x);
                best = (distance, normal, tangent);
            }
        }
        hit = Some((record.id, best.1, best.2));
        break;
    }
    if hit.is_none() {
        // PROXY SOLIDITY: the box world is solid too. Per-building AABBs from
        // the per-chunk collider lists; cost measured and printed.
        let start = crate::clock_compat::Instant::now();
        let mut resolved = None;
        let mut boxes = 0usize;
        let stamp = world
            .get_resource::<crate::geology::Geology>()
            .map(|geology| geology.proxy_stamp)
            .unwrap_or(0);
        {
            let current = world.resource::<ProxyGrid>().stamp;
            if current != stamp {
                let snapshot = world
                    .get_resource::<crate::geology::Geology>()
                    .map(|geology| {
                        geology
                            .proxy_colliders
                            .values()
                            .flat_map(|list| list.iter().copied())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                world.resource_mut::<ProxyGrid>().sync(stamp, &snapshot);
            }
        }
        {
            let grid = world.resource::<ProxyGrid>();
            'outer: for (center, half, collider_id) in grid.near(position) {
                if (center.x - position.x).abs() > half.x + 2.0
                    || (center.z - position.z).abs() > half.z + 2.0
                {
                    continue;
                }
                boxes += 1;
                if let Some((new_pos, normal)) = box_escape(position, *center, *half, speed) {
                    resolved = Some((new_pos, normal, *collider_id));
                    break 'outer;
                }
            }
        }
        if let Some((new_pos, normal, collider_id)) = resolved {
            if let Ok(mut transform) = world
                .query_filtered::<&mut Transform, With<FlyCamera>>()
                .single_mut(world)
            {
                transform.translation = new_pos;
            }
            let elapsed = world.resource::<Time>().elapsed_secs();
            let mut state = world.resource_mut::<InteriorBanState>();
            if state.last_building != collider_id || elapsed - state.last_log > 1.0 {
                state.last_building = collider_id;
                state.last_log = elapsed;
                info!(
                    "CAMERA DEFLECT proxy b{collider_id} speed={speed:.0}{} resolution=({:.2},{:.2})",
                    if probe_speed.is_some() { " probe" } else { "" },
                    normal.x,
                    normal.y
                );
            }
        }
        let micros = start.elapsed().as_secs_f64() * 1.0e6;
        let elapsed = world.resource::<Time>().elapsed_secs();
        let mut state = world.resource_mut::<InteriorBanState>();
        if elapsed - state.last_cost_log > 2.0 {
            state.last_cost_log = elapsed;
            let total = world
                .get_resource::<ProxyGrid>()
                .map(|grid| grid.boxes)
                .unwrap_or(0);
            let (cells, rebuilds, worst_ms) = world
                .get_resource::<ProxyGrid>()
                .map(|grid| (grid.cells.len(), grid.rebuilds, grid.worst_ms))
                .unwrap_or((0, 0, 0.0));
            let sample = world
                .get_resource::<crate::geology::Geology>()
                .and_then(|geology| {
                    geology
                        .proxy_colliders
                        .values()
                        .flat_map(|list| list.iter())
                        .next()
                        .copied()
                });
            info!(
                "COLLISION COST proxy_boxes_total={total} sectors={cells} checked={boxes} query_us={micros:.0} rebuilds={rebuilds} worst_rebuild_ms={worst_ms:.1} sample={sample:?}"
            );
        }
        return;
    }
    let Some((building, normal, tangent)) = hit else {
        return;
    };
    // Glancing deflection: push out along the hull normal and carry the
    // tangential component forward, both scaled by speed.
    let glance = (speed / 650.0).clamp(0.15, 1.0);
    let push = 0.7 + 0.8 * glance;
    let slide = 0.5 * glance;
    if let Ok(mut transform) = world
        .query_filtered::<&mut Transform, With<FlyCamera>>()
        .single_mut(world)
    {
        transform.translation.x += normal.x * push + tangent.x * slide;
        transform.translation.z += normal.y * push + tangent.y * slide;
    }
    let mut state = world.resource_mut::<InteriorBanState>();
    if state.last_building != building || elapsed - state.last_log > 1.0 {
        state.last_building = building;
        state.last_log = elapsed;
        info!(
            "CAMERA DEFLECT b{building} speed={speed:.0} resolution=({:.2},{:.2}) glance={glance:.2}",
            normal.x, normal.y
        );
    }
}

fn sonic_camera(
    time: Res<Time>,
    motion: Res<MotionState>,
    staged: Res<StagedPose>,
    mut camera: Query<(&mut Transform, &mut Projection), With<FlyCamera>>,
) {
    let Ok((mut transform, mut projection)) = camera.single_mut() else {
        return;
    };
    let gain = if staged.active {
        0.0
    } else {
        sonic_gain(motion.speed)
    };
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.fov = (70.0 + 2.0 * gain).to_radians();
    }
    if gain <= 0.01 {
        return;
    }
    // The old 0.15 deg rotational shake at ~6 Hz read as the horizon warping
    // and the mountains moving at cruise. Keep only a whisper of shake.
    let t = time.elapsed_secs();
    let amp = 0.0004 * gain;
    transform.rotate_local_y((t * 37.0).sin() * amp);
    transform.rotate_local_x((t * 53.0).cos() * amp * 0.7);
    transform.translation.y += (t * 61.0).sin() * 0.012 * gain;
}

/// SONIC_TABLE=1: the gain curve at the gauntlet, measured rather than assumed.
fn sonic_table(
    time: Res<Time>,
    motion: Res<MotionState>,
    mut last: Local<f32>,
    mut printed: Local<bool>,
) {
    if std::env::var_os("SONIC_TABLE").is_none() {
        return;
    }
    if !*printed {
        *printed = true;
        // The gain curve at the gauntlet's scripted regimes; the flight camera
        // teleports between samples, so MotionState reads zero velocity.
        for speed in [15.0_f32, 30.0, 100.0, 650.0, 8_000.0] {
            let gain = sonic_gain(speed);
            info!(
                "SONIC CURVE speed={speed:.0} gain={gain:.3} wind_vol={:.3} wind_rate={:.2} fov={:.1} shake_deg={:.3}",
                0.02 + gain * 0.32,
                0.85 + 1.35 * gain,
                70.0 + 5.0 * gain,
                (0.0026 * gain).to_degrees()
            );
        }
    }
    let now = time.elapsed_secs();
    if now - *last < 0.5 {
        return;
    }
    *last = now;
    let gain = sonic_gain(motion.speed);
    info!(
        "SONIC speed={:.0} mach={:.2} gain={gain:.3} wind_vol={:.3} wind_rate={:.2} fov={:.1} shake_deg={:.3}",
        motion.speed,
        motion.mach,
        0.02 + gain * 0.32,
        0.85 + 1.35 * gain,
        70.0 + 2.0 * gain,
        (0.0004 * gain).to_degrees()
    );
}

#[cfg(test)]
mod solidity {
    use super::box_escape;
    use bevy::prelude::*;

    #[test]
    fn speed_zero_resolves_outward_and_stops() {
        let center = Vec3::new(10.0, 5.0, 0.0);
        let half = Vec3::new(5.0, 5.0, 4.0);
        let inside = Vec3::new(11.0, 5.0, 1.0);
        let (out, normal) = box_escape(inside, center, half, 0.0).expect("inside");
        assert!(out.x - center.x > half.x || out.z - center.z > half.z);
        // No tangential component at rest: the resolution is purely outward.
        assert!((out.z - inside.z).abs() < 0.2 || (out.x - inside.x).abs() < 0.2);
        assert!(normal.length() > 0.9);
        assert!(box_escape(out, center, half, 0.0).is_none(), "still inside");
    }

    #[test]
    fn speed_adds_a_glancing_deflection() {
        let center = Vec3::new(10.0, 5.0, 0.0);
        let half = Vec3::new(5.0, 5.0, 4.0);
        let inside = Vec3::new(11.0, 5.0, 1.0);
        let (slow, _) = box_escape(inside, center, half, 0.0).expect("inside");
        let (fast, _) = box_escape(inside, center, half, 650.0).expect("inside");
        assert!(fast.distance(slow) > 0.5, "no tangential glance at speed");
        assert!(
            box_escape(fast, center, half, 0.0).is_none(),
            "still inside"
        );
    }
}
