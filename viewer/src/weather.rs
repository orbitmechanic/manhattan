//! Lightweight dynamic weather: a slowly changing front, procedural cloud
//! layers, shader-driven rain/snow around the camera, wind and lightning.
//!
//! The system is deliberately cheap.  Clouds are two camera-following planes
//! drawn with a three-octave noise shader; precipitation is a single open
//! cylinder around the camera whose streaks are generated per fragment.  No
//! particle simulation or per-drop geometry is involved.

use std::f32::consts::TAU;

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

use crate::{astro::Date, camera::FlyCamera, clock::SimClock, environment::Environment};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherKind {
    Clear,
    Fair,
    Overcast,
    Rain,
    Storm,
    Snow,
}

impl WeatherKind {
    pub const CYCLE: [Self; 5] = [
        Self::Clear,
        Self::Fair,
        Self::Overcast,
        Self::Rain,
        Self::Storm,
    ];
    pub const WINTER_CYCLE: [Self; 5] = [
        Self::Clear,
        Self::Fair,
        Self::Overcast,
        Self::Snow,
        Self::Storm,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Fair => "fair",
            Self::Overcast => "overcast",
            Self::Rain => "rain",
            Self::Storm => "thunderstorm",
            Self::Snow => "snow",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "clear" => Some(Self::Clear),
            "fair" => Some(Self::Fair),
            "overcast" | "cloudy" => Some(Self::Overcast),
            "rain" | "rainy" => Some(Self::Rain),
            "storm" | "thunder" | "thunderstorm" => Some(Self::Storm),
            "snow" | "snowy" => Some(Self::Snow),
            _ => None,
        }
    }

    fn cloud_target(self) -> f32 {
        match self {
            Self::Clear => 0.05,
            Self::Fair => 0.30,
            Self::Overcast => 0.72,
            Self::Rain => 0.86,
            Self::Storm => 0.96,
            Self::Snow => 0.90,
        }
    }

    fn precip_target(self) -> f32 {
        match self {
            Self::Clear | Self::Fair | Self::Overcast => 0.0,
            Self::Rain => 0.65,
            Self::Storm => 1.0,
            Self::Snow => 0.78,
        }
    }

    fn storm_target(self) -> f32 {
        match self {
            Self::Storm => 1.0,
            Self::Rain => 0.12,
            Self::Snow => 0.10,
            _ => 0.0,
        }
    }

    fn wind_target(self) -> f32 {
        match self {
            Self::Clear => 8.0,
            Self::Fair => 14.0,
            Self::Overcast => 20.0,
            Self::Rain => 27.0,
            Self::Storm => 55.0,
            Self::Snow => 21.0,
        }
    }
}

#[derive(Resource)]
pub struct Weather {
    /// The front the weather is currently moving towards.
    pub target: WeatherKind,
    pub cloud_cover: f32,
    pub precipitation: f32,
    pub storm: f32,
    pub snow: f32,
    pub wind: Vec2,
    pub wind_kmh: f32,
    pub temperature_c: f32,
    /// Lightning brightness, 0..1, decays over a few frames.
    pub flash: f32,
    /// Seconds until the next automatic front, when `auto` is set.
    pub change_timer: f32,
    pub auto: bool,
    /// Set when a preset asks for a fixed temperature (for example snow in
    /// July); the normal seasonal curve is then left alone.
    manual_temperature: bool,
    elapsed: f32,
    wind_angle: f32,
    flash_timer: f32,
    rng: u32,
}

impl Default for Weather {
    fn default() -> Self {
        Self {
            target: WeatherKind::Fair,
            cloud_cover: 0.30,
            precipitation: 0.0,
            storm: 0.0,
            snow: 0.0,
            wind: Vec2::new(28.0_f32.to_radians().cos(), 28.0_f32.to_radians().sin()),
            wind_kmh: 14.0,
            temperature_c: seasonal_temperature(Date::new(2026, 6, 21), 12.0 * 60.0),
            flash: 0.0,
            change_timer: 150.0,
            auto: true,
            manual_temperature: false,
            elapsed: 0.0,
            wind_angle: 28.0_f32.to_radians(),
            flash_timer: 5.0,
            rng: 0x51A7_5EED,
        }
    }
}

impl Weather {
    /// Build a weather resource for the CLI / URL preset, e.g. `--weather rain`.
    pub fn from_preset(preset: Option<&str>, clock: &SimClock) -> Self {
        let mut weather = Self {
            temperature_c: seasonal_temperature(clock.date, clock.minute_of_day),
            ..Default::default()
        };
        if let Some(kind) = preset.and_then(WeatherKind::from_name) {
            weather.auto = false;
            weather.force(kind);
        }
        weather
    }

    /// Snap directly to a front (used by presets and tests).
    pub fn force(&mut self, kind: WeatherKind) {
        self.target = kind;
        self.cloud_cover = kind.cloud_target();
        self.precipitation = kind.precip_target();
        self.storm = kind.storm_target();
        self.snow = if kind == WeatherKind::Snow { 1.0 } else { 0.0 };
        self.wind_kmh = kind.wind_target();
        self.wind = Vec2::new(self.wind_angle.cos(), self.wind_angle.sin());
        self.flash = 0.0;
        self.flash_timer = 4.0;
        if kind == WeatherKind::Snow {
            self.temperature_c = self.temperature_c.min(-1.5);
            self.manual_temperature = true;
        }
    }

    pub fn cycle(&mut self) {
        let order: &[WeatherKind] = if self.temperature_c < 2.0 {
            &WeatherKind::WINTER_CYCLE
        } else {
            &WeatherKind::CYCLE
        };
        let index = order
            .iter()
            .position(|kind| *kind == self.target)
            .unwrap_or(0);
        self.target = order[(index + 1) % order.len()];
        self.change_timer = 210.0;
    }

    pub fn summary(&self) -> String {
        format!(
            "{}  cloud {:.0}%  precip {:.0}%  wind {:.0} km/h  {:.0} C",
            self.target.label(),
            self.cloud_cover * 100.0,
            self.precipitation * 100.0,
            self.wind_kmh,
            self.temperature_c,
        )
    }

    fn next_rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / 16_777_216.0
    }

    fn pick_next(&mut self) -> WeatherKind {
        let r = self.next_rand();
        if self.temperature_c < 2.0 {
            if r < 0.22 {
                WeatherKind::Clear
            } else if r < 0.52 {
                WeatherKind::Fair
            } else if r < 0.70 {
                WeatherKind::Overcast
            } else if r < 0.88 {
                WeatherKind::Snow
            } else {
                WeatherKind::Storm
            }
        } else if r < 0.26 {
            WeatherKind::Clear
        } else if r < 0.58 {
            WeatherKind::Fair
        } else if r < 0.74 {
            WeatherKind::Overcast
        } else if r < 0.90 {
            WeatherKind::Rain
        } else {
            WeatherKind::Storm
        }
    }
}

pub fn seasonal_temperature(date: Date, minute_of_day: f64) -> f32 {
    let day = date.day_of_year() as f32;
    let annual = 13.0 + 13.0 * ((day - 205.0) * TAU / 365.0).cos();
    let hour = (minute_of_day / 60.0) as f32;
    let diurnal = 4.5 * ((hour - 9.0) * TAU / 24.0).sin();
    annual + diurnal
}

fn approach(current: f32, target: f32, dt: f32, tau: f32) -> f32 {
    current + (target - current) * (1.0 - (-dt / tau.max(0.01)).exp())
}

pub fn update_weather(time: Res<Time>, clock: Res<SimClock>, mut weather: ResMut<Weather>) {
    let dt = time.delta_secs().clamp(0.0, 0.25);
    weather.elapsed += dt;

    if !weather.manual_temperature {
        let base = seasonal_temperature(clock.date, clock.minute_of_day);
        let cool = match weather.target {
            WeatherKind::Clear => 0.0,
            WeatherKind::Fair => -0.5,
            WeatherKind::Overcast => -1.5,
            WeatherKind::Rain => -3.0,
            WeatherKind::Storm => -5.0,
            WeatherKind::Snow => -4.0,
        };
        weather.temperature_c = approach(weather.temperature_c, base + cool, dt, 24.0);
    }

    if weather.auto {
        weather.change_timer -= dt;
        if weather.change_timer <= 0.0 {
            weather.target = weather.pick_next();
            weather.change_timer = 150.0 + weather.next_rand() * 170.0;
        }
    }

    let target = weather.target;
    let cold = weather.temperature_c < 1.5;
    let mut precip_target = target.precip_target();
    let mut snow_target = if precip_target > 0.05 && cold {
        1.0
    } else {
        0.0
    };
    if target == WeatherKind::Snow {
        precip_target = target.precip_target();
        snow_target = 1.0;
    }

    weather.cloud_cover = approach(weather.cloud_cover, target.cloud_target(), dt, 16.0);
    weather.precipitation = approach(weather.precipitation, precip_target, dt, 11.0);
    weather.storm = approach(weather.storm, target.storm_target(), dt, 22.0);
    weather.snow = approach(weather.snow, snow_target, dt, 14.0);

    // Wind slowly veers and gusts; the same value slants the precipitation
    // and drifts the cloud shader.
    weather.wind_angle += dt * 0.012;
    let gust = 1.0 + 0.10 * (weather.elapsed * 0.37).sin() + 0.05 * (weather.elapsed * 0.113).sin();
    weather.wind_kmh = approach(weather.wind_kmh, target.wind_target() * gust, dt, 12.0);
    weather.wind = Vec2::new(weather.wind_angle.cos(), weather.wind_angle.sin());

    // Lightning: isolated flashes while the storm is strong.
    weather.flash = (weather.flash - dt * 2.8).max(0.0);
    if weather.storm > 0.42 {
        weather.flash_timer -= dt;
        if weather.flash_timer <= 0.0 {
            weather.flash = 1.0;
            let window = (15.0 - 7.0 * weather.storm).max(3.0);
            weather.flash_timer = 2.4 + weather.next_rand() * window;
        }
    } else {
        weather.flash_timer = weather.flash_timer.max(1.5);
    }
}

pub fn handle_weather_keys(keys: Res<ButtonInput<KeyCode>>, mut weather: ResMut<Weather>) {
    if keys.just_pressed(KeyCode::KeyV) {
        weather.cycle();
    }
}

#[derive(Asset, AsBindGroup, Debug, Clone, TypePath)]
pub struct CloudMaterial {
    #[uniform(0)]
    pub data: Vec4,
    #[uniform(1)]
    pub lit: Vec4,
    #[uniform(2)]
    pub shade: Vec4,
    #[uniform(3)]
    pub extra: Vec4,
}

impl Material for CloudMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/clouds.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[derive(Asset, AsBindGroup, Debug, Clone, TypePath)]
pub struct PrecipMaterial {
    #[uniform(0)]
    pub data: Vec4,
    #[uniform(1)]
    pub center: Vec4,
    #[uniform(2)]
    pub tint: Vec4,
}

impl Material for PrecipMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/precipitation.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn enable_prepass() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        // Rain and snow should fall in front of the whole scene, including
        // foreground vehicles and walls; the volume is a lens overlay rather
        // than a solid object, so it neither tests nor writes depth.
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_compare = Some(CompareFunction::Always);
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

#[derive(Component)]
pub struct CloudLayer {
    altitude: f32,
    scale: f32,
    material: Handle<CloudMaterial>,
}

#[derive(Component)]
pub struct PrecipVolume {
    material: Handle<PrecipMaterial>,
}

/// Open-ended cylinder with outward radial normals.  The fragment shader needs
/// no caps; skipping them halves the triangle count and avoids a visible disc
/// when the camera looks straight up.
fn precipitation_mesh(radius: f32, half_height: f32, segments: usize) -> Mesh {
    let mut positions = Vec::with_capacity((segments + 1) * 2);
    let mut normals = Vec::with_capacity((segments + 1) * 2);
    let mut indices = Vec::with_capacity(segments * 6);
    for i in 0..=segments {
        let angle = i as f32 / segments as f32 * TAU;
        let (sin, cos) = angle.sin_cos();
        let normal = [cos, 0.0, sin];
        positions.push([cos * radius, -half_height, sin * radius]);
        positions.push([cos * radius, half_height, sin * radius]);
        normals.push(normal);
        normals.push(normal);
    }
    for i in 0..segments {
        let base = (i * 2) as u32;
        indices.extend([base, base + 2, base + 1, base + 1, base + 2, base + 3]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

pub fn setup_weather(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CloudMaterial>>,
    mut precip_materials: ResMut<Assets<PrecipMaterial>>,
) {
    // Low broken cloud deck and a higher, thinner layer.  Both follow the
    // camera horizontally so their edges are never visible; the noise itself
    // stays fixed in world space so the clouds do not swim.
    for (name, altitude, size, scale) in [
        ("CloudDeckLow", 1_180.0_f32, 15_000.0_f32, 430.0_f32),
        ("CloudDeckHigh", 2_050.0_f32, 23_000.0_f32, 760.0_f32),
    ] {
        let material = materials.add(CloudMaterial {
            data: Vec4::new(0.0, 0.3, 0.0, 0.0),
            lit: Vec4::new(0.7, 0.74, 0.80, 0.0),
            shade: Vec4::new(0.35, 0.38, 0.44, 0.0),
            extra: Vec4::new(0.0, scale, 0.0, 0.0),
        });
        commands.spawn((
            Name::new(name),
            CloudLayer {
                altitude,
                scale,
                material: material.clone(),
            },
            Mesh3d(meshes.add(Plane3d::default().mesh().size(size, size))),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, altitude, 0.0),
        ));
    }

    let material = precip_materials.add(PrecipMaterial {
        data: Vec4::ZERO,
        center: Vec4::new(0.0, 0.0, 0.0, 65.0),
        tint: Vec4::new(0.7, 0.78, 0.9, 0.0),
    });
    commands.spawn((
        Name::new("Precipitation"),
        PrecipVolume {
            material: material.clone(),
        },
        Mesh3d(meshes.add(precipitation_mesh(65.0, 70.0, 64))),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        Visibility::Hidden,
    ));
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[allow(clippy::too_many_arguments)]
pub fn update_weather_visuals(
    time: Res<Time>,
    env: Res<Environment>,
    weather: Res<Weather>,
    camera: Single<&Transform, With<FlyCamera>>,
    mut cloud_layers: Query<
        (&mut Transform, &CloudLayer),
        (Without<FlyCamera>, Without<PrecipVolume>),
    >,
    mut precip: Query<
        (&mut Transform, &mut Visibility, &PrecipVolume),
        (Without<CloudLayer>, Without<FlyCamera>),
    >,
    mut cloud_materials: ResMut<Assets<CloudMaterial>>,
    mut precip_materials: ResMut<Assets<PrecipMaterial>>,
) {
    let cam = camera.translation;
    let seconds = time.elapsed_secs();
    let flash = weather.flash;
    let daylight = env.daylight;
    let cover = weather.cloud_cover.clamp(0.0, 1.0);
    let sky = env.sky_rgb;

    // Warm light near sunrise/sunset still catches the cloud bases.
    let warm = smoothstep(-6.0, 2.0, env.sun.altitude_deg)
        * (1.0 - smoothstep(2.0, 22.0, env.sun.altitude_deg));
    let lit = sky * (0.96 + 0.22 * (1.0 - cover))
        + Vec3::new(0.20, 0.09, 0.02) * warm * (1.0 - cover)
        + Vec3::new(0.035, 0.045, 0.065) * (1.0 - daylight) * (1.0 - cover)
        + Vec3::splat(flash * 0.8);
    let shade = sky * 0.46 + Vec3::new(0.018, 0.022, 0.030) * daylight + Vec3::splat(flash * 0.25);

    for (mut transform, layer) in &mut cloud_layers {
        transform.translation.x = cam.x;
        transform.translation.z = cam.z;
        transform.translation.y = layer.altitude;
        let altitude_fade = if cam.y > layer.altitude + 90.0 {
            0.30
        } else {
            1.0
        };
        if let Some(mut material) = cloud_materials.get_mut(&layer.material) {
            material.data = Vec4::new(
                seconds,
                cover,
                weather.wind.x * weather.wind_kmh * 0.02,
                weather.wind.y * weather.wind_kmh * 0.02,
            );
            material.lit = Vec4::new(lit.x, lit.y, lit.z, (0.62 + 0.38 * cover) * altitude_fade);
            material.shade = Vec4::new(shade.x, shade.y, shade.z, 1.0);
            material.extra = Vec4::new(flash, layer.scale, 0.0, 0.0);
        }
    }

    // Precipitation only matters near the street; hide the volume high above
    // the city where the camera would otherwise fly through a floating tube.
    let precip_visible = weather.precipitation > 0.02 && cam.y < 750.0;
    for (mut transform, mut visibility, volume) in &mut precip {
        transform.translation = cam;
        *visibility = if precip_visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let Some(mut material) = precip_materials.get_mut(&volume.material) {
            let snow = weather.snow.clamp(0.0, 1.0);
            material.data = Vec4::new(
                seconds,
                weather.precipitation,
                snow,
                weather.wind.x * weather.wind_kmh / 75.0,
            );
            material.center = Vec4::new(cam.x, cam.y, cam.z, 65.0);
            material.tint = Vec4::new(
                0.66 + 0.28 * snow,
                0.74 + 0.22 * snow,
                0.86 + 0.13 * snow,
                (0.48 + 0.32 * snow)
                    * (0.55 + 0.45 * weather.precipitation)
                    * 1.35
                    * (0.70 + 0.30 * env.daylight),
            );
        }
    }
}

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            MaterialPlugin::<CloudMaterial>::default(),
            MaterialPlugin::<PrecipMaterial>::default(),
        ))
        .init_resource::<Weather>()
        .add_systems(Startup, setup_weather);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seasonal_curve_is_cold_in_winter_and_warm_in_summer() {
        let winter = seasonal_temperature(Date::new(2026, 1, 15), 12.0 * 60.0);
        let summer = seasonal_temperature(Date::new(2026, 7, 15), 12.0 * 60.0);
        assert!(winter < 5.0, "winter was {winter}");
        assert!(summer > 20.0, "summer was {summer}");
    }

    #[test]
    fn forcing_rain_sets_precipitation_and_clouds() {
        let mut weather = Weather::default();
        weather.force(WeatherKind::Rain);
        assert!(weather.cloud_cover > 0.8);
        assert!(weather.precipitation > 0.5);
        assert_eq!(weather.snow, 0.0);
    }

    #[test]
    fn cold_precipitation_turns_to_snow() {
        let mut weather = Weather::default();
        weather.force(WeatherKind::Snow);
        assert_eq!(weather.snow, 1.0);
        assert!(weather.temperature_c < 0.0);
    }

    #[test]
    fn cycling_moves_to_the_next_front() {
        let mut weather = Weather::default();
        weather.force(WeatherKind::Clear);
        weather.cycle();
        assert_eq!(weather.target, WeatherKind::Fair);
    }
}
