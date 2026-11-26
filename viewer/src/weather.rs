//! Lightweight dynamic weather: a slowly changing front, procedural cloud
//! layers, shader-driven rain/snow around the camera, wind and lightning.
//!
//! The system is deliberately cheap.  Clouds are four camera-following
//! circular decks drawn with a three-octave noise shader; precipitation is a
//! single open cylinder around the camera whose streaks are generated per
//! fragment.  No particle simulation or per-drop geometry is involved.

use std::f32::consts::TAU;

use bevy::{
    asset::RenderAssetUsages,
    light::NotShadowCaster,
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
    Drizzle,
    Rain,
    Storm,
    Snow,
    Sunbreak,
}

impl WeatherKind {
    /// The manual order (V key) walks the Seattle year: the lid, then the
    /// damp, then the wet, then the one window of sun, then back under.
    pub const CYCLE: [Self; 4] = [Self::Overcast, Self::Drizzle, Self::Rain, Self::Sunbreak];
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
            Self::Drizzle => "drizzle",
            Self::Rain => "rain",
            Self::Storm => "thunderstorm",
            Self::Snow => "snow",
            Self::Sunbreak => "sunbreak",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "clear" => Some(Self::Clear),
            "fair" => Some(Self::Fair),
            "overcast" | "cloudy" => Some(Self::Overcast),
            "drizzle" | "damp" => Some(Self::Drizzle),
            "rain" | "rainy" => Some(Self::Rain),
            "sunbreak" | "sun" => Some(Self::Sunbreak),
            "storm" | "thunder" | "thunderstorm" => Some(Self::Storm),
            "snow" | "snowy" => Some(Self::Snow),
            _ => None,
        }
    }

    fn cloud_target(self) -> f32 {
        match self {
            Self::Clear => 0.04,
            Self::Fair => 0.26,
            Self::Overcast => 0.82,
            Self::Drizzle => 0.86,
            Self::Rain => 0.90,
            Self::Storm => 0.97,
            Self::Snow => 0.90,
            // The lid opens: enough blue for the sun to throw a real shadow.
            Self::Sunbreak => 0.26,
        }
    }

    fn precip_target(self) -> f32 {
        match self {
            Self::Clear | Self::Fair | Self::Overcast | Self::Sunbreak => 0.0,
            Self::Drizzle => 0.30,
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
            Self::Fair => 12.0,
            Self::Sunbreak => 10.0,
            Self::Overcast => 18.0,
            Self::Drizzle => 20.0,
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
    /// Day-of-year the RNG was seeded from; the schedule is deterministic per
    /// day cycle, so two runs on the same date see the same fronts.
    seed_day: u32,
    /// 0..1 rain-soaked ground state (smoothed precipitation); the fire round
    /// will read it for suppression, the wetness pass will read it for albedo.
    pub wetness: f32,
    /// 0..1 patchiness of the deck: 0 is a sealed lid, 1 is a torn one with
    /// real holes. Seeded per day, drifts on the wind, and gates the shafts.
    pub holes: f32,
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
            target: WeatherKind::Overcast,
            cloud_cover: 0.82,
            precipitation: 0.0,
            storm: 0.0,
            snow: 0.0,
            wind: Vec2::new(28.0_f32.to_radians().cos(), 28.0_f32.to_radians().sin()),
            wind_kmh: 14.0,
            temperature_c: seasonal_temperature(Date::new(2026, 6, 21), 12.0 * 60.0),
            flash: 0.0,
            change_timer: 240.0,
            auto: true,
            seed_day: 0,
            wetness: 0.0,
            holes: 0.0,
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
            rng: 0x51A7_5EED,
            ..Default::default()
        };
        weather.seed_for(clock.date);
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
        self.change_timer = self.duration_for(self.target);
    }

    pub fn summary(&self) -> String {
        format!(
            "{}  cloud {:.0}%  holes {:.0}%  precip {:.0}%  wind {:.0} km/h  {:.0} C",
            self.target.label(),
            self.cloud_cover * 100.0,
            self.holes * 100.0,
            self.precipitation * 100.0,
            self.wind_kmh,
            self.temperature_c,
        )
    }

    /// Seed the front schedule from a date, so the day's weather is a property
    /// of the day, not of the session.
    pub fn seed_for(&mut self, date: Date) {
        let day = date.day_of_year();
        self.seed_day = day;
        self.rng = 0x51A7_5EED ^ (day.wrapping_mul(0x9E37_79B9)).rotate_left(7);
    }

    fn next_rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / 16_777_216.0
    }

    /// Seattle-credible transitions: the lid dominates, the damp follows it,
    /// rain is common, and a real sunbreak is a short, rare window. Snow only
    /// exists below freezing.
    fn pick_next(&mut self) -> WeatherKind {
        let r = self.next_rand();
        let cold = self.temperature_c < 2.0;
        if cold {
            return if r < 0.42 {
                WeatherKind::Overcast
            } else if r < 0.58 {
                WeatherKind::Snow
            } else if r < 0.70 {
                WeatherKind::Drizzle
            } else if r < 0.86 {
                WeatherKind::Sunbreak
            } else if r < 0.94 {
                WeatherKind::Rain
            } else {
                WeatherKind::Storm
            };
        }
        match self.target {
            // The lid stays shut most of the time; the damp slides in, and
            // every so often the cloud deck tears open for a few minutes.
            WeatherKind::Overcast | WeatherKind::Drizzle => {
                if r < 0.46 {
                    WeatherKind::Overcast
                } else if r < 0.68 {
                    WeatherKind::Drizzle
                } else if r < 0.86 {
                    WeatherKind::Rain
                } else if r < 0.97 {
                    WeatherKind::Sunbreak
                } else {
                    WeatherKind::Storm
                }
            }
            WeatherKind::Rain | WeatherKind::Storm => {
                if r < 0.34 {
                    WeatherKind::Overcast
                } else if r < 0.62 {
                    WeatherKind::Drizzle
                } else if r < 0.82 {
                    WeatherKind::Rain
                } else if r < 0.90 {
                    WeatherKind::Sunbreak
                } else {
                    WeatherKind::Storm
                }
            }
            // The window closes: back under the lid.
            WeatherKind::Sunbreak => {
                if r < 0.84 {
                    WeatherKind::Overcast
                } else {
                    WeatherKind::Clear
                }
            }
            WeatherKind::Clear => {
                if r < 0.52 {
                    WeatherKind::Overcast
                } else if r < 0.74 {
                    WeatherKind::Sunbreak
                } else {
                    WeatherKind::Drizzle
                }
            }
            WeatherKind::Fair => {
                if r < 0.50 {
                    WeatherKind::Overcast
                } else if r < 0.78 {
                    WeatherKind::Sunbreak
                } else {
                    WeatherKind::Drizzle
                }
            }
            WeatherKind::Snow => {
                if r < 0.55 {
                    WeatherKind::Overcast
                } else {
                    WeatherKind::Snow
                }
            }
        }
    }

    /// Minutes-scale duration of a front. Overcast runs long; a sunbreak is a
    /// window, not a season.
    fn duration_for(&mut self, kind: WeatherKind) -> f32 {
        let jitter = 0.75 + 0.5 * self.next_rand();
        match kind {
            WeatherKind::Overcast => 180.0 * jitter,
            WeatherKind::Drizzle => 150.0 * jitter,
            WeatherKind::Rain => 140.0 * jitter,
            WeatherKind::Storm => 90.0 * jitter,
            WeatherKind::Snow => 150.0 * jitter,
            WeatherKind::Sunbreak => 55.0 * (0.8 + 0.4 * self.next_rand()),
            WeatherKind::Clear => 120.0 * jitter,
            WeatherKind::Fair => 150.0 * jitter,
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
            WeatherKind::Sunbreak => 0.5,
            WeatherKind::Fair => -0.5,
            WeatherKind::Overcast => -1.5,
            WeatherKind::Drizzle => -2.0,
            WeatherKind::Rain => -3.0,
            WeatherKind::Storm => -5.0,
            WeatherKind::Snow => -4.0,
        };
        weather.temperature_c = approach(weather.temperature_c, base + cool, dt, 24.0);
    }

    // The schedule belongs to the DAY: cross midnight and the front lottery is
    // re-seeded, so a saved date always opens under the same sky.
    if clock.date.day_of_year() != weather.seed_day {
        weather.seed_for(clock.date);
    }
    if weather.auto {
        weather.change_timer -= dt;
        if weather.change_timer <= 0.0 {
            let next = weather.pick_next();
            weather.target = next;
            weather.change_timer = weather.duration_for(next);
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
    // Ground soak lags the rain and dries slowly; exposed as a field so the
    // fire round can suppress and the wetness pass can darken.
    weather.wetness = approach(weather.wetness, weather.precipitation, dt, 25.0);
    // Patchy cover: the lid is not a sheet. A seeded slow wave tears it open,
    // strongest under overcast (the cover is the master state; holes ride on
    // top), and a sunbreak is the extreme case.
    let tear = {
        let phase = weather.elapsed * 0.017 + weather.next_rand() * 0.15;
        let wave = (phase.sin() * 0.5 + 0.5).powf(2.2);
        let base = match weather.target {
            WeatherKind::Sunbreak => 0.95,
            WeatherKind::Clear => 0.6,
            WeatherKind::Fair => 0.45,
            WeatherKind::Overcast => 0.55,
            WeatherKind::Drizzle => 0.25,
            WeatherKind::Rain => 0.12,
            WeatherKind::Storm => 0.05,
            WeatherKind::Snow => 0.10,
        };
        // The lid closes in rain: holes only exist where the deck is torn.
        base * (0.35 + 0.65 * wave) * (1.0 - 0.85 * weather.precipitation)
    };
    weather.holes = approach(weather.holes, tear.clamp(0.0, 1.0), dt, 9.0);

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
    /// xyz = camera position; soft-particle fade near the deck.
    #[uniform(4)]
    pub camera: Vec4,
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
    /// Extra shader-space drift for this layer, independent of the weather wind.
    drift: Vec2,
    material: Handle<CloudMaterial>,
    /// Overcast-only low deck: altitude follows the data-driven deck height.
    low_deck: bool,
    max_opacity: f32,
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

/// Circular cloud deck.  A triangle fan with a full set of UVs so the shader
/// can fade the rim out; the square plane it replaces used to betray its edge
/// when the camera flew high above the city.
fn cloud_disc_mesh(radius: f32, segments: usize) -> Mesh {
    let mut positions = Vec::with_capacity(segments + 2);
    let mut normals = Vec::with_capacity(segments + 2);
    let mut uvs = Vec::with_capacity(segments + 2);
    let mut indices = Vec::with_capacity(segments * 3);
    positions.push([0.0, 0.0, 0.0]);
    normals.push([0.0, 1.0, 0.0]);
    uvs.push([0.5, 0.5]);
    for i in 0..=segments {
        let angle = i as f32 / segments as f32 * TAU;
        let (sin, cos) = angle.sin_cos();
        positions.push([cos * radius, 0.0, sin * radius]);
        normals.push([0.0, 1.0, 0.0]);
        uvs.push([0.5 + 0.5 * cos, 0.5 + 0.5 * sin]);
    }
    for i in 0..segments {
        let a = 1 + i as u32;
        let b = 1 + (i + 1) as u32;
        indices.extend([0, a, b]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

pub fn setup_weather(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CloudMaterial>>,
    mut precip_materials: ResMut<Assets<PrecipMaterial>>,
) {
    // Four independently drifting decks.  Each one is capped at 30% opacity
    // in the material; the layers are distinct altitudes/scales so they do
    // not read as one flat sheet.
    for (name, altitude, size, scale, drift) in [
        (
            "CloudDeckLow",
            960.0_f32,
            15_000.0_f32,
            380.0_f32,
            Vec2::new(12.0, -4.0),
        ),
        (
            "CloudDeckMidLow",
            1_340.0_f32,
            19_000.0_f32,
            520.0_f32,
            Vec2::new(-9.0, 7.0),
        ),
        (
            "CloudDeckMidHigh",
            1_740.0_f32,
            23_000.0_f32,
            680.0_f32,
            Vec2::new(6.0, 12.0),
        ),
        (
            "CloudDeckHigh",
            2_180.0_f32,
            27_000.0_f32,
            820.0_f32,
            Vec2::new(-3.0, -15.0),
        ),
    ] {
        let material = materials.add(CloudMaterial {
            data: Vec4::new(0.0, 0.3, 0.0, 0.0),
            lit: Vec4::new(0.7, 0.74, 0.80, 0.0),
            shade: Vec4::new(0.35, 0.38, 0.44, 0.0),
            extra: Vec4::new(0.0, scale, 0.0, 0.0),
            camera: Vec4::ZERO,
        });
        commands.spawn((
            Name::new(name),
            CloudLayer {
                altitude,
                scale,
                drift,
                material: material.clone(),
                low_deck: false,
                max_opacity: 0.30,
            },
            Mesh3d(meshes.add(cloud_disc_mesh(size * 0.5, 72))),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, altitude, 0.0),
            // Camera-following decks must never cast a directional shadow:
            // with sun shadows enabled that reads as a dark disc around the
            // camera (the iteration-21 traveling shadow).
            NotShadowCaster,
        ));
    }

    // Overcast-only low deck: low enough to swallow the top of downtown towers.
    let material = materials.add(CloudMaterial {
        data: Vec4::new(0.0, 0.0, 0.0, 0.0),
        lit: Vec4::new(0.7, 0.72, 0.76, 0.0),
        shade: Vec4::new(0.34, 0.36, 0.40, 0.0),
        extra: Vec4::new(0.0, 420.0, 0.0, 0.0),
        camera: Vec4::ZERO,
    });
    commands.spawn((
        Name::new("CloudDeckOvercastLow"),
        CloudLayer {
            altitude: 360.0,
            scale: 420.0,
            drift: Vec2::new(4.0, -6.0),
            material: material.clone(),
            low_deck: true,
            max_opacity: 0.62,
        },
        Mesh3d(meshes.add(cloud_disc_mesh(13_000.0 * 0.5, 72))),
        MeshMaterial3d(material),
        Transform::from_xyz(0.0, 360.0, 0.0),
        NotShadowCaster,
    ));

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
    // Night deck lighting: the city glows from below, so the cloud bases take
    // an amber cast over the dense districts and stay dark over the water.
    let glow = env.city_glow.min(2.0);
    let lit = sky * (0.96 + 0.22 * (1.0 - cover))
        + Vec3::new(0.20, 0.09, 0.02) * warm * (1.0 - cover)
        + Vec3::new(0.035, 0.045, 0.065) * (1.0 - daylight) * (1.0 - cover)
        + Vec3::new(0.20, 0.11, 0.045) * (1.0 - daylight) * glow * 0.45
        + Vec3::splat(flash * 0.8);
    let shade = sky * 0.46 + Vec3::new(0.018, 0.022, 0.030) * daylight + Vec3::splat(flash * 0.25);

    let deck_altitude = crate::city_rhythm::cloud_deck_altitude(cover, 410.0);
    for (mut transform, layer) in &mut cloud_layers {
        transform.translation.x = cam.x;
        transform.translation.z = cam.z;
        let layer_altitude = if layer.low_deck {
            deck_altitude
        } else {
            layer.altitude
        };
        transform.translation.y = layer_altitude;
        let altitude_fade = if cam.y > layer_altitude + 90.0 {
            0.30
        } else {
            1.0
        };
        let activation = if layer.low_deck {
            smoothstep(0.42, 0.72, cover)
        } else {
            1.0
        };
        if let Some(mut material) = cloud_materials.get_mut(&layer.material) {
            material.data = Vec4::new(
                seconds,
                cover,
                weather.wind.x * weather.wind_kmh * 0.02 + layer.drift.x,
                weather.wind.y * weather.wind_kmh * 0.02 + layer.drift.y,
            );
            // Hard ceiling: no individual cloud deck may exceed 30% opacity.
            let layer_opacity =
                ((0.70 + 0.30 * cover) * altitude_fade * activation).min(layer.max_opacity);
            material.lit = Vec4::new(lit.x, lit.y, lit.z, layer_opacity);
            material.camera = Vec4::new(cam.x, cam.y, cam.z, 0.0);
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

// ---------------------------------------------------------------------------
// Sunbeams / moonbeams v2: WORLD-ANCHORED shafts. The camera-child fan is
// deleted (Camera Ownership Law corollary: camera-attached overlays are for
// speed lines only). Each shaft is a long additive volume standing in the world
// at a hole in the deck, rotated onto the actual sun/moon vector, so the beam
// rakes across the terrain toward the horizon and converges by true geometry.
// ---------------------------------------------------------------------------

const SHAFT_COUNT: usize = 7;
const SHAFT_LOW_SUN_MIN: f32 = 2.0;
const SHAFT_LOW_SUN_MAX: f32 = 40.0;

#[derive(Asset, AsBindGroup, Debug, Clone, TypePath)]
pub struct BeamMaterial {
    /// x = time, y = strength 0..1, z = width scale, w unused
    #[uniform(0)]
    pub data: Vec4,
    /// rgb = tint, a = opacity
    #[uniform(1)]
    pub tint: Vec4,
}

impl Material for BeamMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/beams.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        // Blend, not Add: an additive shaft vanishes against a bright sky.
        // Blending composites the beam over sky, water, terrain and buildings
        // alike, which is the visibility the player requires.
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
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            // World volumes test against the city (so towers cut the beam) but
            // never write depth.
            depth.depth_compare = Some(CompareFunction::LessEqual);
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

#[derive(Component)]
pub struct LightShaft {
    index: usize,
    material: Handle<BeamMaterial>,
}

#[derive(Resource, Default)]
pub struct ShaftField {
    seeded_day: u32,
    /// Seeded per day: azimuth spread, radius, width and brightness of each hole.
    holes: Vec<Hole>,
}

#[derive(Clone, Copy, Default)]
struct Hole {
    azimuth: f32,
    radius: f32,
    width: f32,
    brightness: f32,
}

/// Unit-height crossed quads with a soft vertical gradient: the world volume.
fn beam_mesh() -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for plane in 0..2 {
        let angle = plane as f32 * std::f32::consts::FRAC_PI_2;
        let (sin, cos) = angle.sin_cos();
        let base = positions.len() as u32;
        for (x, v) in [(-0.5_f32, 0.0_f32), (0.5, 0.0), (0.5, 1.0), (-0.5, 1.0)] {
            positions.push([cos * x, v, sin * x]);
            normals.push([0.0, 1.0, 0.0]);
            uvs.push([if x < 0.0 { 0.0 } else { 1.0 }, v]);
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

pub fn setup_light_shafts(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BeamMaterial>>,
) {
    let mesh = meshes.add(beam_mesh());
    for index in 0..SHAFT_COUNT {
        let material = materials.add(BeamMaterial {
            data: Vec4::new(0.0, 0.0, 1.0, 0.0),
            tint: Vec4::new(1.0, 0.92, 0.78, 0.0),
        });
        commands.spawn((
            LightShaft {
                index,
                material: material.clone(),
            },
            Name::new(format!("light shaft {index}")),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::Hidden,
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_light_shafts(
    time: Res<Time>,
    clock: Res<SimClock>,
    env: Res<Environment>,
    weather: Res<Weather>,
    camera: Single<&Transform, With<FlyCamera>>,
    mut field: ResMut<ShaftField>,
    mut shafts: Query<(&LightShaft, &mut Transform, &mut Visibility), Without<FlyCamera>>,
    mut materials: ResMut<Assets<BeamMaterial>>,
    geology: Option<Res<crate::geology::Geology>>,
) {
    if field.seeded_day != clock.date.day_of_year() || field.holes.is_empty() {
        field.seeded_day = clock.date.day_of_year();
        let mut seed = 0x51A7_5EEDu32 ^ field.seeded_day.wrapping_mul(2_654_435_761);
        let mut rand = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as f32 / 16_777_216.0
        };
        field.holes = (0..SHAFT_COUNT)
            .map(|_| Hole {
                azimuth: rand() * std::f32::consts::TAU,
                radius: 220.0 + rand() * 620.0,
                width: 130.0 + rand() * 190.0,
                brightness: 0.5 + rand() * 0.5,
            })
            .collect();
    }

    let sun_alt = env.sun.altitude_deg;
    let moon_alt = env.moon.altitude_deg;
    let day = env.daylight > 0.35;
    let low_sun = (SHAFT_LOW_SUN_MIN..=SHAFT_LOW_SUN_MAX).contains(&sun_alt);
    let low_moon = env.moon_illumination >= 0.55
        && (SHAFT_LOW_SUN_MIN..=SHAFT_LOW_SUN_MAX).contains(&moon_alt);

    let (light_dir, strength, tint, probe) = if day && low_sun {
        (
            env.sun_dir,
            weather.holes,
            Vec3::new(1.0, 0.93, 0.78),
            std::env::var_os("SHAFT_PROBE").is_some(),
        )
    } else if !day && low_moon {
        (
            env.moon_dir,
            (env.moon_illumination - 0.5).clamp(0.0, 1.0) * 2.0,
            Vec3::new(0.64, 0.74, 1.0),
            false,
        )
    } else {
        (Vec3::ZERO, 0.0, Vec3::ZERO, false)
    };

    if std::env::var_os("BEAM_TRACE").is_some() {
        info!(
            "BEAMS {} sun_alt={:.1} moon_alt={:.1} illum={:.2} holes={:.2} shafts={}",
            if strength > 0.05 { "eligible" } else { "idle" },
            sun_alt,
            moon_alt,
            env.moon_illumination,
            weather.holes,
            field.holes.len()
        );
    }

    let active = strength > 0.05;
    let cam = camera.translation;
    // A crepuscular ray is WORLD geometry: it starts at the cloud deck and runs
    // ALONG the sun vector until it meets terrain — with a low sun that meeting
    // point is kilometres away on the horizon side, never at the camera's feet.
    let deck_height = 900.0_f32;
    let down = -light_dir.normalize_or_zero();
    let sin_alt = (-down.y).max(0.06);
    let mut landings = Vec::new();
    for (shaft, mut transform, mut visibility) in &mut shafts {
        let hole = field.holes[shaft.index % field.holes.len()];
        if !active {
            *visibility = Visibility::Hidden;
            continue;
        }
        let drift = time.elapsed_secs() * 0.01 + weather.wind.x * 0.6;
        // Landing site: seeded per hole, 2–16 km from the camera, biased to the
        // anti-solar half so the shaft rakes toward the viewer from the sun.
        let heading =
            f32::atan2(-light_dir.x, -light_dir.z) + (hole.azimuth - std::f32::consts::PI) * 0.6;
        let want = if probe {
            2_400.0
        } else {
            2_000.0 + hole.radius * 22.0 + drift * 40.0
        };
        let want = want.clamp(1_600.0, 16_000.0);
        // The ray can only fall as far as the deck allows: L·sin(alt) = deck.
        let max_reach = deck_height / sin_alt.tan().max(0.02);
        let distance = want.min(max_reach).max(1_600.0);
        let landing = Vec3::new(
            cam.x + heading.sin() * distance,
            0.0,
            cam.z + heading.cos() * distance,
        );
        let landing = match geology.as_ref() {
            Some(state) => Vec3::new(
                landing.x,
                state
                    .dem
                    .height(landing.x - crate::citypack::STREAM_WORLD_OFFSET, landing.z),
                landing.z,
            ),
            None => landing,
        };
        let length = (deck_height / sin_alt).min(distance / sin_alt.cos().max(0.1));
        let start = landing + Vec3::Y * (length * sin_alt);
        let along = (start - landing).normalize_or_zero();
        transform.translation = (start + landing) * 0.5;
        transform.rotation = Quat::from_rotation_arc(Vec3::Y, along);
        let width = if probe { 220.0 } else { hole.width * 1.8 };
        transform.scale = Vec3::new(width, length.max(50.0), width);
        *visibility = Visibility::Inherited;
        if let Some(mut material) = materials.get_mut(&shaft.material) {
            let level = if probe {
                1.0
            } else {
                strength * hole.brightness
            };
            material.data = Vec4::new(time.elapsed_secs(), level.clamp(0.0, 1.0), 1.0, 0.0);
            material.tint = Vec4::new(tint.x, tint.y, tint.z, 0.38);
        }
        landings.push(distance);
    }
    if std::env::var_os("BEAM_TRACE").is_some() && !landings.is_empty() {
        let near = landings.iter().filter(|d| **d < 1_609.0).count();
        let far = landings.iter().filter(|d| **d >= 1_609.0).count();
        let max = landings.iter().cloned().fold(0.0_f32, f32::max);
        info!(
            "BEAMS landing distances: ge1mi={far} lt1mi={near} max={:.1} km (deck {:.0} m, sun alt {:.1} deg)",
            max / 1000.0,
            deck_height,
            sun_alt
        );
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
        .add_plugins(MaterialPlugin::<BeamMaterial>::default())
        .init_resource::<ShaftField>()
        .add_systems(Startup, (setup_weather, setup_light_shafts))
        .add_systems(Update, update_light_shafts);
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
    fn cycling_walks_the_seattle_year() {
        let mut weather = Weather::default();
        weather.force(WeatherKind::Overcast);
        weather.cycle();
        assert_eq!(weather.target, WeatherKind::Drizzle);
        weather.cycle();
        assert_eq!(weather.target, WeatherKind::Rain);
        weather.cycle();
        assert_eq!(weather.target, WeatherKind::Sunbreak);
        weather.cycle();
        assert_eq!(weather.target, WeatherKind::Overcast);
    }

    #[test]
    fn overcast_is_the_nominal_state() {
        let weather = Weather::default();
        assert_eq!(weather.target, WeatherKind::Overcast);
        assert!(
            weather.cloud_cover > 0.75,
            "nominal lid was open: {}",
            weather.cloud_cover
        );
        assert_eq!(weather.precipitation, 0.0);
    }

    #[test]
    fn the_front_schedule_is_seeded_by_the_day() {
        let mut a = Weather::default();
        let mut b = Weather::default();
        a.seed_for(Date::new(2026, 3, 14));
        b.seed_for(Date::new(2026, 3, 14));
        a.force(WeatherKind::Overcast);
        b.force(WeatherKind::Overcast);
        for _ in 0..24 {
            let next_a = a.pick_next();
            let next_b = b.pick_next();
            assert_eq!(next_a, next_b, "same day must draw the same fronts");
            a.target = next_a;
            b.target = next_b;
        }
        let mut c = Weather::default();
        c.seed_for(Date::new(2026, 3, 15));
        c.force(WeatherKind::Overcast);
        c.pick_next();
        // A different day may coincidentally agree on one draw; over a run it
        // must diverge somewhere.
        let mut d = Weather::default();
        d.seed_for(Date::new(2026, 3, 14));
        d.force(WeatherKind::Overcast);
        let mut divergent = false;
        for _ in 0..24 {
            if c.pick_next() != d.pick_next() {
                divergent = true;
                break;
            }
        }
        assert!(
            divergent,
            "two different days shared a whole front schedule"
        );
    }
}
