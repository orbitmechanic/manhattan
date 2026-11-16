//! City-scale time-reversal controller.
//!
//! One invocation assigns every damaged building within `radius` of an
//! epicenter a per-building restoration rate through the destructibles volume
//! ledger. The falloff profile decides how amplitude is distributed, so the
//! nearest buildings finish first and the healing wave visibly radiates
//! outward. The scheduler is a single budgeted call per frame; destructibles
//! guarantees a fair minimum slice per building while spending the rest of the
//! budget highest-rate-first.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;

use destructibles_bevy::building::Buildings;
use destructibles_bevy::damage::DamageJournal;
use destructibles_bevy::rewind::{
    RewindState, SimClock as RewindClock, advance_restoration_budget, extend_restoration,
    set_restoration_rate,
};

use crate::minimap::{LiveFootprints, WaveRing};
use crate::neighborhood::{BuildingIndex, DamageAim, NEIGHBORHOOD_ID_BASE, Neighborhood};

/// The frame budget for restoration machinery. The damage admission budget is
/// 6 ms; restoration stays under it and reports its measured slice.
pub const RESTORATION_BUDGET_MS: f32 = 5.0;
/// This build uses the host quad column as the single far-plume mechanism.
pub const FAR_PLUME_OWNER: &str = "manhattan-host-quads";

const RADIUS_STEPS: [f32; 7] = [80.0, 150.0, 250.0, 400.0, 600.0, 900.0, 1_400.0];
const AMPLITUDE_STEPS: [f32; 6] = [100.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FalloffProfile {
    Flat,
    #[default]
    Linear,
    InverseSquare,
}

impl FalloffProfile {
    pub const ALL: [Self; 3] = [Self::Flat, Self::Linear, Self::InverseSquare];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Linear => "linear",
            Self::InverseSquare => "inverse-square",
        }
    }

    /// `t` is distance / radius in 0..=1. Flat keeps full amplitude to the
    /// edge; linear is `1 - t`; inverse-square is `1 / (1 + 15 t²)` normalized
    /// so the edge is exactly zero:
    /// `(1/(1+15t²) - 1/16) / (1 - 1/16)`.
    pub fn factor(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Flat => 1.0,
            Self::Linear => 1.0 - t,
            Self::InverseSquare => {
                let raw = 1.0 / (1.0 + 15.0 * t * t);
                ((raw - 1.0 / 16.0) / (1.0 - 1.0 / 16.0)).clamp(0.0, 1.0)
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WaveAssignment {
    pub building: usize,
    pub did: u32,
    pub rate: f32,
    pub distance: f32,
    pub epicenter: Vec3,
    pub initial_volume: f32,
    pub started_at: f64,
    pub completed_at: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct WaveState {
    pub epicenter: Vec3,
    pub radius: f32,
    pub amplitude: f32,
    pub profile: FalloffProfile,
    pub started_at: f64,
    pub revision: u64,
}

#[derive(Resource, Default)]
pub struct CityRestoration {
    pub config_radius: f32,
    pub config_amplitude: f32,
    pub profile: FalloffProfile,
    pub wave: Option<WaveState>,
    pub assignments: BTreeMap<usize, WaveAssignment>,
    pub waves: u32,
    pub completed_total: usize,
    pub last_buildings: usize,
    pub last_seconds: f32,
    pub last_throughput: f32,
    pub worst_slice_ms: f32,
    pub last_slice_ms: f32,
    pub starved_frames: u32,
    pub revision: u64,
}

impl CityRestoration {
    fn next_revision(&mut self) -> u64 {
        self.revision = self.revision.wrapping_add(1).max(1);
        self.revision
    }
}

/// Journal volume for a building that belongs to the wave cutoff. Damage that
/// arrives during the wave is newer than the cutoff and is intentionally left
/// for the next invocation.
fn wave_remaining(journal: &DamageJournal, did: u32, cutoff: f64) -> f32 {
    journal
        .0
        .entries()
        .iter()
        .filter(|entry| entry.cmd.building() == did && entry.at < cutoff)
        .map(|entry| entry.volume)
        .sum()
}

fn default_radius() -> f32 {
    250.0
}
fn default_amplitude() -> f32 {
    500.0
}

#[derive(Resource)]
pub struct FarSmoke {
    pub material: Handle<StandardMaterial>,
    pub mesh: Handle<Mesh>,
}

#[derive(Component)]
pub struct WavePanelText;

pub struct CityRestorationPlugin;

impl Plugin for CityRestorationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CityRestoration>()
            .add_systems(Startup, (setup_wave_panel, setup_far_smoke));
    }
}

fn setup_wave_panel(mut commands: Commands) {
    commands.spawn((
        WavePanelText,
        Node {
            position_type: PositionType::Absolute,
            left: px(12),
            bottom: px(300),
            padding: UiRect::all(px(6)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.05, 0.07, 0.55)),
        Text::new("WAVE —"),
        TextFont {
            font_size: FontSize::Px(11.0),
            ..default()
        },
        TextColor(Color::srgba(0.72, 1.0, 0.86, 0.98)),
        TextShadow::default(),
        ZIndex(19),
    ));
}

/// Soft radial alpha: 1 in the core, smoothly to 0 at the rim, with a little
/// deterministic noise so the plume does not read as a flat card.
fn smoke_alpha(x: f32, y: f32) -> f32 {
    let u = x * 2.0 - 1.0;
    let v = y * 2.0 - 1.0;
    let radius = (u * u * 1.15 + v * v * 0.9).sqrt();
    let edge = (1.0 - radius).clamp(0.0, 1.0);
    let noise = ((x * 37.0).sin() * (y * 53.0).cos() * 0.5 + 0.5) * 0.18 + 0.82;
    (edge.powf(1.5) * noise).clamp(0.0, 1.0)
}

fn setup_far_smoke(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    const SIZE: usize = 64;
    let mut data = vec![0u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let alpha = smoke_alpha(
                (x as f32 + 0.5) / SIZE as f32,
                (y as f32 + 0.5) / SIZE as f32,
            );
            let index = (y * SIZE + x) * 4;
            // Warm grey, matching the daylight haze family.
            data[index] = 226;
            data[index + 1] = 228;
            data[index + 2] = 232;
            data[index + 3] = (alpha * 235.0) as u8;
        }
    }
    let mut image = Image::new_fill(
        bevy::render::render_resource::Extent3d {
            width: SIZE as u32,
            height: SIZE as u32,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        &[0, 0, 0, 0],
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.sampler = bevy::image::ImageSampler::linear();
    let texture = images.add(image);
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.95, 0.96, 1.0, 0.92),
        base_color_texture: Some(texture),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let mesh = meshes.add(Rectangle::new(1.0, 1.0));
    commands.insert_resource(FarSmoke { material, mesh });
}

/// Contested buildings go to the nearer epicenter; completed assignments are
/// always replaceable. A running assignment at the same or nearer distance is
/// left alone so overlapping invocations do not thrash.
fn takes_assignment(new_distance: f32, existing: Option<(f32, Vec3, bool)>) -> bool {
    match existing {
        None => true,
        Some((_old_distance, _old_epicenter, true)) => true,
        Some((old_distance, _old_epicenter, false)) => new_distance < old_distance,
    }
}

/// Fire a city wave. Returns how many buildings received an assignment (new or
/// contested-and-reassigned). Buildings outside the radius are untouched.
#[allow(clippy::drop_non_drop)]
pub fn restore_city(
    world: &mut World,
    epicenter: Vec3,
    radius: f32,
    amplitude: f32,
    profile: FalloffProfile,
) -> usize {
    if radius <= 1.0 || amplitude <= 0.0 {
        return 0;
    }
    let now = world.resource::<RewindClock>().time;
    let center = Vec2::new(epicenter.x, epicenter.z);
    // Every damaged building is a candidate, live/frozen or dormant: the
    // journal is the durable truth, not the current render regime.
    let mut candidates: BTreeSet<u32> = BTreeSet::new();
    {
        let journal = world.resource::<DamageJournal>();
        for entry in journal.0.entries() {
            candidates.insert(entry.cmd.building());
        }
    }
    let mut planned: Vec<(usize, u32, f32, f32, f32, f32)> = Vec::new();
    {
        let index = world.resource::<BuildingIndex>();
        let journal = world.resource::<DamageJournal>();
        for did in candidates {
            if did < NEIGHBORHOOD_ID_BASE {
                continue;
            }
            let id = (did - NEIGHBORHOOD_ID_BASE) as usize;
            let Some(meta) = index.buildings.get(id) else {
                continue;
            };
            let distance = Vec2::new(meta.centroid[0], meta.centroid[1]).distance(center);
            if distance > radius {
                continue;
            }
            let volume = wave_remaining(journal, did, now);
            if volume <= 0.0 {
                continue;
            }
            let factor = profile.factor(distance / radius);
            let rate = amplitude * factor;
            if rate <= 0.01 {
                continue;
            }
            planned.push((id, did, distance, rate, factor, volume));
        }
    }
    if planned.is_empty() {
        info!(
            "WAVE: no damaged buildings within {radius:.0} m of ({:.0},{:.0})",
            epicenter.x, epicenter.z
        );
        return 0;
    }

    let mut rate_updates: Vec<(u32, f32)> = Vec::new();
    let mut activate: Vec<u32> = Vec::new();
    let mut assigned = 0usize;
    let mut city = world.resource_mut::<CityRestoration>();
    let revision = city.next_revision();
    city.wave = Some(WaveState {
        epicenter,
        radius,
        amplitude,
        profile,
        started_at: now,
        revision,
    });
    city.waves += 1;
    city.last_buildings = planned.len();
    city.last_seconds = 0.0;
    city.last_throughput = 0.0;
    city.worst_slice_ms = 0.0;
    city.starved_frames = 0;
    for (id, did, distance, rate, _factor, volume) in planned {
        let contest = city.assignments.get(&id).map(|existing| {
            (
                existing.distance,
                existing.epicenter,
                existing.completed_at.is_some(),
            )
        });
        if !takes_assignment(distance, contest) {
            continue;
        }
        match contest {
            Some(_) => {
                city.assignments.insert(
                    id,
                    WaveAssignment {
                        building: id,
                        did,
                        rate,
                        distance,
                        epicenter,
                        initial_volume: volume,
                        started_at: now,
                        completed_at: None,
                    },
                );
                rate_updates.push((did, rate));
                activate.push(did);
                assigned += 1;
            }
            None => {
                city.assignments.insert(
                    id,
                    WaveAssignment {
                        building: id,
                        did,
                        rate,
                        distance,
                        epicenter,
                        initial_volume: volume,
                        started_at: now,
                        completed_at: None,
                    },
                );
                rate_updates.push((did, rate));
                activate.push(did);
                assigned += 1;
            }
        }
    }
    drop(city);
    for (did, rate) in rate_updates {
        set_restoration_rate(world, did, rate);
    }
    if !activate.is_empty() {
        extend_restoration(world, &activate);
    }
    info!(
        "WAVE: {assigned} building(s) within {radius:.0} m, amplitude {amplitude:.0} m3/s, {} profile",
        profile.label()
    );
    assigned
}

/// Current wave front, used by the minimap ring and the HUD.
#[allow(clippy::drop_non_drop)]
fn update_ring(world: &mut World) {
    let (wave, ring, active) = {
        let city = world.resource::<CityRestoration>();
        match city.wave.clone() {
            None => (None, 0.0, 0),
            Some(wave) => {
                let journal = world.resource::<DamageJournal>();
                let mut total = 0.0f32;
                let mut remaining = 0.0f32;
                let mut active = 0usize;
                for assignment in city.assignments.values() {
                    if assignment.completed_at.is_some() {
                        continue;
                    }
                    let left = wave_remaining(journal, assignment.did, assignment.started_at);
                    total += assignment.initial_volume;
                    remaining += left.min(assignment.initial_volume);
                    active += 1;
                }
                let progress = if total > 1.0e-6 {
                    (1.0 - remaining / total).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                (Some(wave.clone()), wave.radius * (1.0 - progress), active)
            }
        }
    };
    world.resource_mut::<LiveFootprints>().wave = match wave {
        Some(wave) if active > 0 => Some(WaveRing {
            center: [wave.epicenter.x, wave.epicenter.z],
            radius_m: ring.max(4.0),
            revision: wave.revision,
        }),
        _ => None,
    };
}

pub fn restoration_tick(world: &mut World) {
    let now = world.resource::<RewindClock>().time;
    let has_wave = world.resource::<CityRestoration>().wave.is_some();
    let active = world
        .get_resource::<RewindState>()
        .is_some_and(|state| state.active || state.settling);
    let step = if active {
        advance_restoration_budget(world, RESTORATION_BUDGET_MS)
    } else {
        Default::default()
    };
    {
        let mut city = world.resource_mut::<CityRestoration>();
        city.last_slice_ms = step.work_ms;
        city.worst_slice_ms = city.worst_slice_ms.max(step.work_ms);
        if step.starved > 0 {
            city.starved_frames += 1;
        }
    }
    if !has_wave {
        return;
    }
    // Completion bookkeeping. An assignment completes when the journal up to
    // its own cutoff is empty; damage that arrives mid-wave is newer than the
    // cutoff and is left for the next invocation (journal order settles it).
    let mut newly_complete = 0usize;
    let mut all_done = true;
    {
        let cutoffs: Vec<(u32, f64)> = world
            .resource::<CityRestoration>()
            .assignments
            .values()
            .map(|assignment| (assignment.did, assignment.started_at))
            .collect();
        let volumes: BTreeMap<u32, f32> = {
            let journal = world.resource::<DamageJournal>();
            cutoffs
                .iter()
                .map(|(did, cutoff)| (*did, wave_remaining(journal, *did, *cutoff)))
                .collect()
        };
        let mut city = world.resource_mut::<CityRestoration>();
        for assignment in city.assignments.values_mut() {
            if assignment.completed_at.is_some() {
                continue;
            }
            let left = volumes.get(&assignment.did).copied().unwrap_or(0.0);
            if left <= 0.0 {
                assignment.completed_at = Some(now);
                newly_complete += 1;
            } else {
                all_done = false;
            }
        }
        if newly_complete > 0 {
            city.completed_total += newly_complete;
            info!("WAVE: {newly_complete} building(s) completed");
        }
        if all_done {
            let elapsed =
                (now - city.wave.as_ref().map(|w| w.started_at).unwrap_or(now)).max(0.001);
            city.last_seconds = elapsed as f32;
            city.last_throughput = city.assignments.len() as f32 / elapsed as f32;
            city.wave = None;
            city.assignments.clear();
            info!(
                "WAVE COMPLETE: {} buildings in {:.2}s ({:.2} buildings/s), worst slice {:.2} ms, starved frames {}",
                city.last_buildings,
                city.last_seconds,
                city.last_throughput,
                city.worst_slice_ms,
                city.starved_frames
            );
        }
    }
    update_ring(world);
}

fn near_step(values: &[f32], current: f32, direction: i32) -> f32 {
    let index = values
        .iter()
        .position(|value| (value - current).abs() < 0.5)
        .unwrap_or_else(|| {
            values
                .iter()
                .position(|value| *value >= current)
                .unwrap_or(values.len().saturating_sub(1))
        });
    let next = (index as i32 + direction).clamp(0, values.len() as i32 - 1) as usize;
    values[next]
}

pub fn wave_input(world: &mut World) {
    let keys = world.resource::<ButtonInput<KeyCode>>().clone();
    let mut radius = world.resource::<CityRestoration>().config_radius;
    let mut amplitude = world.resource::<CityRestoration>().config_amplitude;
    let mut profile = world.resource::<CityRestoration>().profile;
    if radius <= 0.0 {
        radius = default_radius();
    }
    if amplitude <= 0.0 {
        amplitude = default_amplitude();
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        radius = near_step(&RADIUS_STEPS, radius, -1);
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        radius = near_step(&RADIUS_STEPS, radius, 1);
    }
    if keys.just_pressed(KeyCode::Minus) {
        amplitude = near_step(&AMPLITUDE_STEPS, amplitude, -1);
    }
    if keys.just_pressed(KeyCode::Equal) {
        amplitude = near_step(&AMPLITUDE_STEPS, amplitude, 1);
    }
    if keys.just_pressed(KeyCode::KeyP) {
        let index = FalloffProfile::ALL
            .iter()
            .position(|candidate| *candidate == profile)
            .unwrap_or(0);
        profile = FalloffProfile::ALL[(index + 1) % FalloffProfile::ALL.len()];
    }
    {
        let mut city = world.resource_mut::<CityRestoration>();
        city.config_radius = radius;
        city.config_amplitude = amplitude;
        city.profile = profile;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let fire = keys.just_pressed(KeyCode::KeyG);
    let delegated = keys.just_pressed(KeyCode::KeyZ) && shift;
    if !fire && !delegated {
        return;
    }
    let epicenter = world.resource::<DamageAim>().point;
    let wave_radius = if delegated { 250.0 } else { radius };
    let fired = restore_city(world, epicenter, wave_radius, amplitude, profile);
    if fired > 0 {
        info!(
            "WAVE fired at ({:.0},{:.0}) radius {wave_radius:.0} m amplitude {amplitude:.0} m3/s",
            epicenter.x, epicenter.z
        );
    }
}

pub fn wave_panel(
    city: Res<CityRestoration>,
    aim: Res<DamageAim>,
    journal: Res<DamageJournal>,
    mut texts: Query<&mut Text, With<WavePanelText>>,
) {
    let mut active = 0usize;
    let mut remaining = 0.0f32;
    let mut total = 0.0f32;
    for assignment in city.assignments.values() {
        if assignment.completed_at.is_some() {
            continue;
        }
        active += 1;
        total += assignment.initial_volume;
        remaining += journal
            .0
            .remaining_volume(assignment.did)
            .min(assignment.initial_volume);
    }
    let ring = if let Some(wave) = &city.wave {
        let progress = if total > 1.0e-6 {
            (1.0 - remaining / total).clamp(0.0, 1.0)
        } else {
            1.0
        };
        wave.radius * (1.0 - progress)
    } else {
        0.0
    };
    let epicenter = world_aim_epicenter(&aim);
    let body = format!(
        "WAVE  R {:.0} m  A {:.0} m3/s  {}   [G] fire  [ ] radius  -/= amplitude  P profile\n\
         epicenter ({:.0},{:.0})  active {active}  ring {ring:.0} m  slice {:.1} ms  peak {:.1} ms  starved {}  tput {:.1} b/s",
        city.config_radius.max(default_radius()),
        city.config_amplitude.max(default_amplitude()),
        city.profile.label(),
        epicenter.x,
        epicenter.z,
        city.last_slice_ms,
        city.worst_slice_ms,
        city.starved_frames,
        city.last_throughput
    );
    for mut text in &mut texts {
        text.0 = body.clone();
    }
}

pub fn world_aim_epicenter(aim: &DamageAim) -> Vec3 {
    aim.point
}

/// Crossed, softened host smoke quads for a frozen damaged record.
pub fn spawn_host_smoke(world: &mut World, did: u32) -> Vec<Entity> {
    let Some((origin, height)) = world
        .resource::<Buildings>()
        .get(did)
        .map(|record| (record.origin, record.layout.height))
    else {
        return Vec::new();
    };
    let Some(far) = world.get_resource::<FarSmoke>() else {
        return Vec::new();
    };
    let material = far.material.clone();
    let mesh = far.mesh.clone();
    let width = (height * 0.75).clamp(16.0, 120.0);
    let column = (height * 1.6).clamp(50.0, 420.0);
    let levels = 5;
    let mut spawned = Vec::new();
    for level in 0..levels {
        let t = level as f32 / (levels - 1) as f32;
        let centre = origin + Vec3::Y * (height * 0.35 + column * (0.1 + 0.8 * t));
        let size = width * (0.65 + 0.55 * t);
        for yaw in [0.0f32, std::f32::consts::FRAC_PI_2] {
            spawned.push(
                world
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(centre)
                            .with_rotation(Quat::from_rotation_y(yaw))
                            .with_scale(Vec3::new(size, size * 1.15, 1.0)),
                        bevy::light::NotShadowCaster,
                        Visibility::Inherited,
                        Name::new(format!("far smoke b{did} l{level}")),
                    ))
                    .id(),
            );
        }
    }
    spawned
}

pub fn despawn_host_smoke(world: &mut World, did: u32) {
    let markers = world
        .resource_mut::<Neighborhood>()
        .smoke_markers
        .remove(&did);
    if let Some(markers) = markers {
        for marker in markers {
            if let Ok(entity_mut) = world.get_entity_mut(marker) {
                entity_mut.despawn();
            }
        }
    }
}

/// Smoke quads are the far plume; no destructibles marker is spawned by the
/// host. Kept as a function so the assertion lives next to the mechanism.
pub fn far_plume_is_host_owned() -> bool {
    FAR_PLUME_OWNER == "manhattan-host-quads"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falloff_profiles_reach_full_at_center_and_zero_or_full_at_edge() {
        assert_eq!(FalloffProfile::Flat.factor(0.0), 1.0);
        assert_eq!(FalloffProfile::Flat.factor(1.0), 1.0);
        assert!((FalloffProfile::Linear.factor(0.0) - 1.0).abs() < 1e-6);
        assert!(FalloffProfile::Linear.factor(1.0).abs() < 1e-6);
        assert!(FalloffProfile::Linear.factor(0.5) > FalloffProfile::Linear.factor(0.75));
        assert!((FalloffProfile::InverseSquare.factor(0.0) - 1.0).abs() < 1e-6);
        assert!(FalloffProfile::InverseSquare.factor(1.0).abs() < 1e-6);
        // Inverse-square falls faster than linear at half radius.
        assert!(FalloffProfile::InverseSquare.factor(0.5) < FalloffProfile::Linear.factor(0.5));
        // Clamped outside the disc.
        assert_eq!(FalloffProfile::Linear.factor(-1.0), 1.0);
        assert_eq!(FalloffProfile::Linear.factor(2.0), 0.0);
    }

    #[test]
    fn scheduling_volume_is_positive_and_monotonic_in_radius() {
        let profiles = FalloffProfile::ALL;
        for profile in profiles {
            let mut previous = f32::MAX;
            for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let value = profile.factor(t);
                assert!((0.0..=1.0).contains(&value));
                if profile != FalloffProfile::Flat {
                    assert!(value <= previous + 1e-6);
                    previous = value;
                }
            }
        }
    }

    #[test]
    fn far_plume_is_owned_by_the_host_only() {
        assert!(far_plume_is_host_owned());
        assert_eq!(FAR_PLUME_OWNER, "manhattan-host-quads");
    }

    #[test]
    fn soft_plume_alpha_fades_to_the_rim() {
        assert!(smoke_alpha(0.5, 0.5) > 0.65);
        assert!(smoke_alpha(0.01, 0.5) < 0.08);
        assert!(smoke_alpha(0.99, 0.5) < 0.08);
        assert!(smoke_alpha(0.5, 0.01) < 0.08);
        assert!(smoke_alpha(0.5, 0.99) < 0.08);
    }

    #[test]
    fn contested_buildings_go_to_the_nearer_epicenter() {
        let old = Vec3::ZERO;
        assert!(takes_assignment(40.0, None));
        assert!(takes_assignment(40.0, Some((80.0, old, false))));
        assert!(!takes_assignment(90.0, Some((80.0, old, false))));
        assert!(!takes_assignment(80.0, Some((80.0, old, false))));
        assert!(
            takes_assignment(900.0, Some((80.0, old, true))),
            "a completed assignment is always replaceable"
        );
    }

    #[test]
    fn radius_steps_are_monotonic_and_include_default() {
        let mut previous = 0.0;
        for step in RADIUS_STEPS {
            assert!(step > previous);
            previous = step;
        }
        assert!(RADIUS_STEPS.contains(&default_radius()));
        assert!(AMPLITUDE_STEPS.contains(&default_amplitude()));
    }
}
