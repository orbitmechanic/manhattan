//! Dynamic sun/moon lighting, sky colour and day/night material switching.

use bevy::{
    camera::visibility::Visibility,
    color::LinearRgba,
    pbr::{DistanceFog, FogFalloff},
    prelude::*,
};

use crate::astro::{NYC_LAT, NYC_LON, SkyPosition, moon_position, sun_position};
use crate::camera::FlyCamera;
use crate::clock::SimClock;

const SKY_DISTANCE: f32 = 16_000.0;

#[derive(Resource)]
pub struct Environment {
    pub sun: SkyPosition,
    pub moon: SkyPosition,
    pub moon_illumination: f32,
    pub sun_dir: Vec3,
    pub moon_dir: Vec3,
    /// 0 = full night, 1 = full day.
    pub daylight: f32,
    pub night: bool,
    /// Distance at which fog fully obscures the city, in world units.
    pub fog_visibility: f32,
    /// 0 = clear, 1 = dense; drives the fog-bank sprites.
    pub fog_strength: f32,
    /// Fog/sky tint used to keep the mist sheets consistent with the sky.
    pub fog_rgb: Vec3,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sun: SkyPosition::default(),
            moon: SkyPosition::default(),
            moon_illumination: 0.0,
            sun_dir: Vec3::ZERO,
            moon_dir: Vec3::ZERO,
            daylight: 1.0,
            night: false,
            fog_visibility: 7_000.0,
            fog_strength: 0.35,
            fog_rgb: Vec3::new(0.55, 0.68, 0.85),
        }
    }
}

#[derive(Component)]
pub struct SunLight;

#[derive(Component)]
pub struct MoonLight;

#[derive(Component)]
pub struct SkyRoot;

#[derive(Component)]
pub struct SunDisc;

#[derive(Component)]
pub struct MoonDisc;

#[derive(Component)]
pub struct Star;

#[derive(Component)]
pub struct FogBank {
    drift: Vec2,
    base_y: f32,
    phase: f32,
}

#[derive(Resource)]
pub struct FogBankMaterials(pub Vec<(Handle<StandardMaterial>, f32)>);

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn dir_from_alt_az(pos: SkyPosition) -> Vec3 {
    let alt = pos.altitude_deg.to_radians();
    let az = pos.azimuth_deg.to_radians();
    Vec3::new(az.sin() * alt.cos(), alt.sin(), -az.cos() * alt.cos())
}

pub fn setup_lighting(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Directional sun + moon. They are rotated every frame by update_environment.
    commands.spawn((
        SunLight,
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_rotation_x(-0.7)),
    ));
    // WebGL2 supports only one directional light per view.  The browser build
    // uses a single combined sun/moon fixture; native keeps both.
    #[cfg(not(target_arch = "wasm32"))]
    commands.spawn((
        MoonLight,
        DirectionalLight {
            illuminance: 0.0,
            shadow_maps_enabled: false,
            color: Color::srgb(0.62, 0.72, 1.0),
            ..default()
        },
        Transform::from_rotation(Quat::from_rotation_x(-1.0)),
    ));

    // Sky bodies live in a root that follows the camera, so they never get
    // closer/farther as the player flies across the city.
    let sun_mesh = meshes.add(Sphere::new(190.0).mesh().ico(4).unwrap());
    let moon_mesh = meshes.add(Sphere::new(135.0).mesh().ico(4).unwrap());
    let star_mesh = meshes.add(Sphere::new(28.0).mesh().ico(1).unwrap());

    let sun_material = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(120.0, 100.0, 62.0),
        unlit: true,
        ..default()
    });
    let moon_material = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(10.0, 12.0, 16.0),
        unlit: true,
        ..default()
    });
    let star_material = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(7.0, 7.5, 9.0),
        unlit: true,
        ..default()
    });

    commands
        .spawn((SkyRoot, Transform::default(), Visibility::default()))
        .with_children(|parent| {
            parent.spawn((
                SunDisc,
                Mesh3d(sun_mesh),
                MeshMaterial3d(sun_material),
                Transform::from_translation(Vec3::NEG_Z * SKY_DISTANCE),
            ));
            parent.spawn((
                MoonDisc,
                Mesh3d(moon_mesh),
                MeshMaterial3d(moon_material),
                Transform::from_translation(Vec3::NEG_Z * SKY_DISTANCE),
            ));

            // deterministic star field on the upper hemisphere
            let mut seed = 0x1234_5678u32;
            for _ in 0..260 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let u = (seed >> 8) as f32 / ((u32::MAX >> 8) as f32);
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = (seed >> 8) as f32 / ((u32::MAX >> 8) as f32);
                let y = 0.03 + u * 0.97;
                let r = (1.0 - y * y).max(0.0).sqrt();
                let theta = v * std::f32::consts::TAU;
                let pos = Vec3::new(r * theta.cos(), y, r * theta.sin()) * SKY_DISTANCE;
                let scale = 0.7 + (seed % 100) as f32 / 32.0;
                parent.spawn((
                    Star,
                    Mesh3d(star_mesh.clone()),
                    MeshMaterial3d(star_material.clone()),
                    Transform::from_translation(pos).with_scale(Vec3::splat(scale)),
                    Visibility::Hidden,
                ));
            }
        });
}

#[allow(clippy::too_many_arguments)]
#[cfg_attr(target_arch = "wasm32", allow(unused_variables, unused_mut))]
pub fn update_environment(
    time: Res<Time>,
    clock: Res<SimClock>,
    mut env: ResMut<Environment>,
    mut clear_color: ResMut<ClearColor>,
    camera: Single<
        (&Transform, &mut AmbientLight, &mut DistanceFog),
        (
            With<FlyCamera>,
            Without<SunLight>,
            Without<MoonLight>,
            Without<SkyRoot>,
            Without<SunDisc>,
            Without<MoonDisc>,
            Without<Star>,
        ),
    >,
    mut sun: Query<
        (&mut DirectionalLight, &mut Transform),
        (
            With<SunLight>,
            Without<MoonLight>,
            Without<FlyCamera>,
            Without<SkyRoot>,
            Without<SunDisc>,
            Without<MoonDisc>,
            Without<Star>,
        ),
    >,
    mut moon: Query<
        (&mut DirectionalLight, &mut Transform),
        (
            With<MoonLight>,
            Without<SunLight>,
            Without<FlyCamera>,
            Without<SkyRoot>,
            Without<SunDisc>,
            Without<MoonDisc>,
            Without<Star>,
        ),
    >,
    mut sky_root: Query<
        &mut Transform,
        (
            With<SkyRoot>,
            Without<FlyCamera>,
            Without<SunLight>,
            Without<MoonLight>,
            Without<SunDisc>,
            Without<MoonDisc>,
            Without<Star>,
        ),
    >,
    mut sun_disc: Query<
        (&mut Transform, &mut Visibility),
        (
            With<SunDisc>,
            Without<FlyCamera>,
            Without<SkyRoot>,
            Without<SunLight>,
            Without<MoonLight>,
            Without<MoonDisc>,
            Without<Star>,
        ),
    >,
    mut moon_disc: Query<
        (&mut Transform, &mut Visibility),
        (
            With<MoonDisc>,
            Without<FlyCamera>,
            Without<SkyRoot>,
            Without<SunLight>,
            Without<MoonLight>,
            Without<SunDisc>,
            Without<Star>,
        ),
    >,
    mut stars: Query<
        &mut Visibility,
        (
            With<Star>,
            Without<FlyCamera>,
            Without<SkyRoot>,
            Without<SunDisc>,
            Without<MoonDisc>,
        ),
    >,
) {
    let dt = clock.datetime();
    let sun_pos = sun_position(&dt, NYC_LAT, NYC_LON);
    let (moon_pos, moon_illum) = moon_position(&dt, NYC_LAT, NYC_LON);

    env.sun = sun_pos;
    env.moon = moon_pos;
    env.moon_illumination = moon_illum;
    env.sun_dir = dir_from_alt_az(sun_pos);
    env.moon_dir = dir_from_alt_az(moon_pos);
    env.daylight = smoothstep(-7.0, 9.0, sun_pos.altitude_deg);
    // Hysteresis: stay in the current mode until the sun is clearly up/down.
    // This stops the day/night texture set flickering at the horizon.
    if env.night {
        if sun_pos.altitude_deg > 1.0 {
            env.night = false;
        }
    } else if sun_pos.altitude_deg < -1.0 {
        env.night = true;
    }

    let (cam_transform, mut ambient, mut fog) = camera.into_inner();
    let camera_pos = cam_transform.translation;

    // ---- sky colour -----------------------------------------------------
    let day_sky = Vec3::new(0.30, 0.50, 0.82);
    let night_sky = Vec3::new(0.010, 0.018, 0.045);
    let sunset = Vec3::new(0.95, 0.36, 0.12);
    let t = smoothstep(-8.0, 12.0, sun_pos.altitude_deg);
    let mut sky = night_sky.lerp(day_sky, t);
    let sunset_glow = (-(sun_pos.altitude_deg / 5.5).powi(2)).exp()
        * (1.0 - smoothstep(4.0, 22.0, sun_pos.altitude_deg));
    sky += sunset * sunset_glow * 0.55;
    let sky_color = Color::srgb(sky.x, sky.y, sky.z);
    clear_color.0 = sky_color;

    // Variably dense city haze: lower altitude and night air are thicker, and
    // a slow weather wave opens and closes the sightline over a few minutes.
    // The same visibility value drives fog culling and the DOF depth, so
    // real geometry beyond the haze is not rendered.
    let altitude = camera_pos.y.max(0.0);
    let weather = (time.elapsed_secs() * 0.012 + 1.3).sin() * 0.5 + 0.5;
    let visibility =
        ((5_000.0 + altitude * 2.2) * (1.0 - 0.28 * env.daylight) * (0.72 + 0.42 * weather))
            .clamp(2_000.0, 22_000.0);
    env.fog_visibility = visibility;
    env.fog_strength = (1.0 - (visibility - 2_500.0) / 21_500.0).clamp(0.0, 1.0);
    env.fog_rgb = Vec3::new(sky.x * 0.92, sky.y * 0.94, sky.z * 1.02);
    fog.color = Color::srgb(env.fog_rgb.x, env.fog_rgb.y, env.fog_rgb.z);
    fog.falloff = FogFalloff::from_visibility_squared(visibility);

    // ---- ambient --------------------------------------------------------
    let ambient_color = Color::srgb(
        0.14 + 0.40 * env.daylight,
        0.18 + 0.50 * env.daylight,
        0.30 + 0.58 * env.daylight,
    );
    ambient.color = ambient_color;
    ambient.brightness = 55.0 + 75.0 * env.daylight;

    // ---- lights ---------------------------------------------------------
    let sun_up = sun_pos.altitude_deg.to_radians().sin().max(0.0);
    let sun_warmth = (1.0 - smoothstep(2.0, 24.0, sun_pos.altitude_deg)).clamp(0.0, 1.0);
    let moon_up = moon_pos.altitude_deg.to_radians().sin().max(0.0);
    #[cfg(target_arch = "wasm32")]
    {
        // One directional light only: use the sun when it is up, otherwise
        // repurpose the same fixture as moonlight.
        let (direction, illuminance, color) = if sun_up > 0.005 {
            (
                -env.sun_dir,
                16_000.0 * sun_up.powf(0.65),
                Color::srgb(
                    1.0,
                    0.58 + 0.38 * (1.0 - sun_warmth),
                    0.30 + 0.62 * (1.0 - sun_warmth),
                ),
            )
        } else {
            (
                -env.moon_dir,
                160.0 * moon_illum * moon_up.sqrt(),
                Color::srgb(0.62, 0.72, 1.0),
            )
        };
        for (mut light, mut transform) in &mut sun {
            light.illuminance = illuminance;
            light.color = color;
            transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        for (mut light, mut transform) in &mut sun {
            light.illuminance = 16_000.0 * sun_up.powf(0.65);
            light.color = Color::srgb(
                1.0,
                0.58 + 0.38 * (1.0 - sun_warmth),
                0.30 + 0.62 * (1.0 - sun_warmth),
            );
            transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, -env.sun_dir);
        }
        for (mut light, mut transform) in &mut moon {
            light.illuminance = 160.0 * moon_illum * moon_up.sqrt();
            transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, -env.moon_dir);
        }
    }

    // ---- sky bodies -----------------------------------------------------
    if let Ok(mut root) = sky_root.single_mut() {
        root.translation = camera_pos;
    }
    if let Ok((mut transform, mut vis)) = sun_disc.single_mut() {
        transform.translation = env.sun_dir * SKY_DISTANCE;
        *vis = if sun_pos.altitude_deg > -1.5 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    if let Ok((mut transform, mut vis)) = moon_disc.single_mut() {
        transform.translation = env.moon_dir * SKY_DISTANCE;
        *vis = if moon_pos.altitude_deg > -1.0 && moon_illum > 0.02 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for mut vis in &mut stars {
        *vis = if env.night {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Spawn large, soft, ground-hugging mist sheets over the city.  They add
/// spatial variation on top of the distance fog and break up the skyline.
pub fn spawn_fog_banks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let texture = asset_server.load("textures/fog_bank.png");
    let mesh = meshes.add(Rectangle::new(1.0, 1.0));
    let mut variants = Vec::new();
    let mut handles = Vec::new();
    for alpha in [0.045_f32, 0.065, 0.085] {
        let handle = materials.add(StandardMaterial {
            base_color: Color::srgba(0.78, 0.84, 0.93, alpha),
            base_color_texture: Some(texture.clone()),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        });
        variants.push(handle.clone());
        handles.push((handle, alpha));
    }
    commands.insert_resource(FogBankMaterials(handles));

    let mut seed = 0x9E37_79B9u32;
    let mut rnd = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / ((u32::MAX >> 8) as f32)
    };
    for i in 0..18 {
        let x = -1_800.0 + rnd() * 6_300.0;
        let z = -2_400.0 + rnd() * 5_000.0;
        let y = 18.0 + rnd() * 55.0;
        let width = 1_600.0 + rnd() * 1_800.0;
        let depth = 1_400.0 + rnd() * 1_600.0;
        let angle = rnd() * std::f32::consts::TAU;
        commands.spawn((
            FogBank {
                drift: Vec2::new(angle.cos(), angle.sin()) * (3.0 + rnd() * 6.0),
                base_y: y,
                phase: rnd() * std::f32::consts::TAU,
            },
            Mesh3d(mesh.clone()),
            MeshMaterial3d(variants[i % variants.len()].clone()),
            Transform::from_xyz(x, y, z)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::new(width, depth, 1.0)),
        ));
    }
}

pub fn update_fog_banks(time: Res<Time>, mut banks: Query<(&mut Transform, &mut FogBank)>) {
    let dt = time.delta_secs().clamp(0.0, 0.1);
    for (mut transform, mut bank) in &mut banks {
        transform.translation.x += bank.drift.x * dt;
        transform.translation.z += bank.drift.y * dt;
        bank.phase += dt * 0.05;
        transform.translation.y = bank.base_y + (bank.phase.sin() * 12.0);
        if transform.translation.x > 4_800.0 {
            transform.translation.x = -2_000.0;
        }
        if transform.translation.x < -2_000.0 {
            transform.translation.x = 4_800.0;
        }
        if transform.translation.z > 2_800.0 {
            transform.translation.z = -2_600.0;
        }
        if transform.translation.z < -2_600.0 {
            transform.translation.z = 2_800.0;
        }
    }
}

/// Keep the mist sheets tinted to the current sky and fade them in as the
/// haze thickens.
pub fn update_fog_bank_materials(
    env: Res<Environment>,
    banks: Res<FogBankMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let strength = 0.55 + 0.9 * env.fog_strength;
    for (handle, base_alpha) in &banks.0 {
        if let Some(mut material) = materials.get_mut(handle) {
            material.base_color = Color::srgba(
                env.fog_rgb.x,
                env.fog_rgb.y,
                env.fog_rgb.z,
                base_alpha * strength,
            );
        }
    }
}

/// Distance at which the fog is effectively opaque.  Real building geometry is
/// culled here; no proxy LOD is substituted.
pub fn fog_cull_distance(visibility: f32) -> f32 {
    (visibility * 1.05).clamp(2_500.0, 24_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fog_cull_tracks_visibility() {
        for visibility in [1_500.0, 3_000.0, 5_000.0, 10_000.0, 24_000.0] {
            let cull = fog_cull_distance(visibility);
            assert!(cull >= visibility.max(2_500.0));
            assert!(cull <= 24_000.0);
        }
        assert!(fog_cull_distance(10_000.0) > fog_cull_distance(5_000.0));
    }
}
