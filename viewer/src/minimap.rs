//! Round lower-right minimap and flight instrument dials.
//!
//! The map is generated on the CPU into two UI textures: a building/street
//! footprint layer that is regenerated only when the player moves, and a
//! transparent instrument layer with the FOV rays, Mach thermometer, velocity
//! triangle and acceleration rim.  This keeps the map crisp without adding a
//! render pass or custom material.
//!
//! The map texture is north-up.  A [`UiTransform`] rotation on the map image
//! node turns it heading-up, while the instrument texture and telemetry stay
//! fixed to the screen; the FOV rays therefore always point straight up.
//!
//! Streets come from OpenStreetMap (names + real geometry) converted into the
//! model's Web Mercator frame.  The anchor below is the one stored by
//! BlenderGIS in `manhattan.blend` (EPSG:3857, zoom 15): world (0, 0) is
//! longitude −74.00087591003654, latitude 40.713460624392276, and one world
//! unit is one mercator metre.

use std::f32::consts::TAU;

use bevy::{
    asset::RenderAssetUsages,
    image::{Image, ImageSampler},
    math::Rot2,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use serde::Deserialize;

use crate::{camera::FlyCamera, motion::MotionState, storefront::StorefrontData};

const MAP_TEX: usize = 512;
/// World-space radius of the circular map, in mercator metres.
const MAP_RADIUS_WORLD: f32 = 650.0;
const MAP_CENTER: f32 = MAP_TEX as f32 * 0.5;
const MAP_RADIUS: f32 = MAP_TEX as f32 * 0.5 - 8.0;
const ACCEL_RADIUS: f32 = MAP_RADIUS - 8.0;
const VELOCITY_RADIUS: f32 = MAP_RADIUS - 30.0;
/// 7 o'clock in screen coordinates: atan2(y, x) with y down.
const ZERO_ANGLE: f32 = 2.094_395_2;
/// Zero at 7 o'clock, maximum at 5 o'clock: a 300° clockwise sweep.
const DIAL_SWEEP: f32 = 5.235_987_7;
/// Sea-level Mach 1, used by the telemetry and the 0..Mach 2 scale.
const MACH_1_MPS: f32 = 343.0;
const MAX_MACH: f32 = 2.0;
const MAX_VELOCITY_MPH: f32 = MAX_MACH * MACH_1_MPS * 2.236_936;
const MAX_ACCEL_G: f32 = 4.0;
const MAP_UI_SIZE: f32 = 360.0;
const MAP_UPDATE_SECS: f32 = 0.20;
const MAP_MOVE_EPS_SQ: f32 = 4.0 * 4.0;
const OVERLAY_UPDATE_SECS: f32 = 0.10;
const TEXT_UPDATE_SECS: f32 = 0.12;

/// Web Mercator radius of the Earth.
const EARTH_RADIUS: f64 = 6_378_137.0;
/// BlenderGIS scene origin from `manhattan.blend`.
const GEO_ANCHOR_X: f64 = -8_237_739.824_561_493;
const GEO_ANCHOR_Y: f64 = 4_970_168.600_037_185;

const STREET_DATA: &str = include_str!("../assets/models/street_network.json");

#[derive(Resource)]
struct MinimapImages {
    map: Handle<Image>,
    overlay: Handle<Image>,
}

/// Rotates with the camera heading (north-up texture shown heading-up).
#[derive(Component)]
struct MinimapMapImage;

#[derive(Component, Clone, Copy)]
enum MinimapText {
    Telemetry,
    LatLon,
    Street,
}

#[derive(Resource, Default)]
struct NearestStreet(String);

#[derive(Deserialize)]
struct StreetFile {
    streets: Vec<StreetRecord>,
}

#[derive(Deserialize)]
struct StreetRecord {
    name: String,
    highway: String,
    /// Flat `[x, z, x, z, ...]` in mercator metres.
    points: Vec<i32>,
}

struct Street {
    name: String,
    highway: String,
    points: Vec<Vec2>,
    min: Vec2,
    max: Vec2,
}

#[derive(Resource)]
struct StreetNetwork {
    streets: Vec<Street>,
}

impl StreetNetwork {
    fn load() -> Self {
        let file: StreetFile =
            serde_json::from_str(STREET_DATA).expect("street_network.json is invalid");
        let mut streets = Vec::with_capacity(file.streets.len());
        for record in file.streets {
            let mut points = Vec::with_capacity(record.points.len() / 2);
            let mut min = Vec2::splat(f32::INFINITY);
            let mut max = Vec2::splat(f32::NEG_INFINITY);
            for pair in record.points.as_chunks::<2>().0 {
                let point = Vec2::new(pair[0] as f32, pair[1] as f32);
                min = min.min(point);
                max = max.max(point);
                points.push(point);
            }
            if points.len() < 2 {
                continue;
            }
            streets.push(Street {
                name: record.name,
                highway: record.highway,
                points,
                min,
                max,
            });
        }
        Self { streets }
    }
}

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StreetNetwork>()
            .init_resource::<NearestStreet>()
            .add_systems(PostStartup, setup_minimap)
            .add_systems(
                Update,
                (
                    update_minimap_heading,
                    update_minimap_map,
                    update_minimap_overlay,
                    update_minimap_text,
                )
                    .chain(),
            );
    }
}

impl FromWorld for StreetNetwork {
    fn from_world(_world: &mut World) -> Self {
        Self::load()
    }
}

fn make_image(data: Vec<u8>) -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: MAP_TEX as u32,
            height: MAP_TEX as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.sampler = ImageSampler::linear();
    image
}

fn inside_circle(x: i32, y: i32) -> bool {
    let dx = x as f32 + 0.5 - MAP_CENTER;
    let dy = y as f32 + 0.5 - MAP_CENTER;
    dx * dx + dy * dy <= MAP_RADIUS * MAP_RADIUS
}

fn put(data: &mut [u8], x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= MAP_TEX as i32 || y >= MAP_TEX as i32 || !inside_circle(x, y) {
        return;
    }
    let index = (y as usize * MAP_TEX + x as usize) * 4;
    data[index..index + 4].copy_from_slice(&color);
}

fn draw_line(data: &mut [u8], a: Vec2, b: Vec2, color: [u8; 4], thickness: i32) {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = a.x + dx * t;
        let y = a.y + dy * t;
        let xi = x.round() as i32;
        let yi = y.round() as i32;
        for ox in -thickness..=thickness {
            for oy in -thickness..=thickness {
                put(data, xi + ox, yi + oy, color);
            }
        }
    }
}

fn draw_ring(data: &mut [u8], radius: f32, width: f32, color: [u8; 4]) {
    let steps = (radius * 8.0).max(360.0) as i32;
    for i in 0..steps {
        let a = i as f32 / steps as f32 * TAU;
        let r0 = (radius - width).max(0.0);
        let r1 = radius + width;
        let mut r = r0;
        while r <= r1 {
            put(
                data,
                (MAP_CENTER + a.cos() * r).round() as i32,
                (MAP_CENTER + a.sin() * r).round() as i32,
                color,
            );
            r += 1.0;
        }
    }
}

fn draw_arc_band(data: &mut [u8], inner: f32, outer: f32, start: f32, end: f32, color: [u8; 4]) {
    let span = (end - start).abs();
    if span < 0.001 {
        return;
    }
    let steps = (span * 64.0).max(8.0) as i32;
    for i in 0..=steps {
        let a = start + (end - start) * i as f32 / steps as f32;
        let mut r = inner;
        while r <= outer {
            put(
                data,
                (MAP_CENTER + a.cos() * r).round() as i32,
                (MAP_CENTER + a.sin() * r).round() as i32,
                color,
            );
            r += 1.0;
        }
    }
}

fn draw_dot(data: &mut [u8], center: Vec2, radius: f32, color: [u8; 4]) {
    let r = radius.ceil() as i32;
    for y in -r..=r {
        for x in -r..=r {
            if (x * x + y * y) as f32 <= radius * radius {
                put(data, center.x as i32 + x, center.y as i32 + y, color);
            }
        }
    }
}

fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let sign = |p1: Vec2, p2: Vec2, p3: Vec2| {
        (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y)
    };
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

fn draw_triangle(data: &mut [u8], points: [Vec2; 3], color: [u8; 4]) {
    let min_x = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let max_x = points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let min_y = points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let max_y = points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    for y in min_y.floor() as i32..=max_y.ceil() as i32 {
        for x in min_x.floor() as i32..=max_x.ceil() as i32 {
            if point_in_triangle(
                Vec2::new(x as f32 + 0.5, y as f32 + 0.5),
                points[0],
                points[1],
                points[2],
            ) {
                put(data, x, y, color);
            }
        }
    }
}

fn world_to_map(world: Vec2, center: Vec2) -> Vec2 {
    Vec2::new(
        MAP_CENTER + (world.x - center.x) / MAP_RADIUS_WORLD * MAP_RADIUS,
        MAP_CENTER + (world.y - center.y) / MAP_RADIUS_WORLD * MAP_RADIUS,
    )
}

/// Convert model world `(x, z)` to longitude/latitude.  `x` is east and `z`
/// is south in the Web Mercator frame described at the top of this file.
fn world_to_lon_lat(x: f32, z: f32) -> (f64, f64) {
    let mercator_x = GEO_ANCHOR_X + x as f64;
    let mercator_y = GEO_ANCHOR_Y - z as f64;
    let lon = (mercator_x / EARTH_RADIUS).to_degrees();
    let lat =
        (2.0 * (mercator_y / EARTH_RADIUS).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
    (lon, lat)
}

fn street_thickness(highway: &str) -> i32 {
    match highway {
        "motorway" | "trunk" | "primary" => 2,
        "motorway_link" | "trunk_link" | "primary_link" | "secondary" | "tertiary" => 1,
        "residential" | "unclassified" | "living_street" | "pedestrian" => 1,
        _ => 0,
    }
}

fn point_segment_distance_sq(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < 1e-6 {
        return (point - a).length_squared();
    }
    let t = ((point - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    (point - (a + ab * t)).length_squared()
}

fn nearest_named_street(center: Vec2, streets: &[Street]) -> Option<String> {
    let mut best = f32::INFINITY;
    let mut name = None;
    for street in streets {
        if street.name.is_empty() {
            continue;
        }
        if street.max.x < center.x - MAP_RADIUS_WORLD
            || street.min.x > center.x + MAP_RADIUS_WORLD
            || street.max.y < center.y - MAP_RADIUS_WORLD
            || street.min.y > center.y + MAP_RADIUS_WORLD
        {
            continue;
        }
        for pair in street.points.windows(2) {
            let d = point_segment_distance_sq(center, pair[0], pair[1]);
            if d < best {
                best = d;
                name = Some(street.name.clone());
            }
        }
    }
    name
}

fn render_map_layer(
    data: &mut [u8],
    center: Vec2,
    buildings: &[crate::storefront::BuildingMeta],
    streets: &[Street],
) -> Option<String> {
    // Dark slate ground so the black street lines read clearly.
    for y in 0..MAP_TEX as i32 {
        for x in 0..MAP_TEX as i32 {
            put(data, x, y, [26, 30, 37, 255]);
        }
    }

    // Real OpenStreetMap streets, drawn minor-first so the major roads finish
    // on top.  Bounding boxes reject almost every street outside the circle.
    let black = [0, 0, 0, 255];
    for street in streets.iter().rev() {
        if street.max.x < center.x - MAP_RADIUS_WORLD
            || street.min.x > center.x + MAP_RADIUS_WORLD
            || street.max.y < center.y - MAP_RADIUS_WORLD
            || street.min.y > center.y + MAP_RADIUS_WORLD
        {
            continue;
        }
        let thickness = street_thickness(&street.highway);
        for pair in street.points.windows(2) {
            draw_line(
                data,
                world_to_map(pair[0], center),
                world_to_map(pair[1], center),
                black,
                thickness,
            );
        }
    }

    let outline = [148, 153, 160, 255];
    for building in buildings {
        let centroid = Vec2::new(building.centroid[0], building.centroid[1]);
        if centroid.distance_squared(center) > (MAP_RADIUS_WORLD + 80.0).powi(2) {
            continue;
        }
        if building.perimeter.len() < 3 {
            continue;
        }
        for i in 0..building.perimeter.len() {
            let a = Vec2::from_array(building.perimeter[i]);
            let b = Vec2::from_array(building.perimeter[(i + 1) % building.perimeter.len()]);
            draw_line(
                data,
                world_to_map(a, center),
                world_to_map(b, center),
                outline,
                0,
            );
        }
    }

    nearest_named_street(center, streets)
}

fn render_overlay_layer(data: &mut [u8], velocity_mph: f32, accel_g: f32) {
    let center = Vec2::splat(MAP_CENTER);
    let ring = [132, 162, 194, 235];
    let minor = [155, 177, 200, 225];
    let major = [225, 238, 250, 245];
    let green = [54, 236, 118, 235];
    let cyan = [74, 220, 255, 240];
    let red = [255, 72, 58, 245];

    draw_ring(data, MAP_RADIUS, 2.0, ring);
    draw_ring(data, VELOCITY_RADIUS, 1.5, [104, 154, 188, 220]);
    draw_ring(data, ACCEL_RADIUS, 1.8, [118, 132, 148, 235]);

    // Graded ticks over the 300° scale that runs from 7 o'clock to 5 o'clock.
    let ticks = 20;
    for index in 0..=ticks {
        let fraction = index as f32 / ticks as f32;
        let angle = ZERO_ANGLE + fraction * DIAL_SWEEP;
        let long = index % 5 == 0;
        let r0 = if long {
            ACCEL_RADIUS - 11.0
        } else {
            ACCEL_RADIUS - 6.0
        };
        let r1 = ACCEL_RADIUS + 3.0;
        let color = if long { major } else { minor };
        let dir = Vec2::new(angle.cos(), angle.sin());
        draw_line(data, center + dir * r0, center + dir * r1, color, 0);
    }

    // Field of view rays: the map rotates with the camera, so the rays are
    // fixed pointing up.
    let half_fov = 35.0_f32.to_radians();
    let up = -std::f32::consts::FRAC_PI_2;
    for offset in [-half_fov, half_fov] {
        let angle = up + offset;
        let dir = Vec2::new(angle.cos(), angle.sin());
        draw_line(
            data,
            center,
            center + dir * (VELOCITY_RADIUS * 0.72),
            [45, 190, 96, 210],
            0,
        );
    }

    // Velocity thermometer, filled clockwise from the 7 o'clock zero.  Mach 2
    // (the fastest aircraft speed) lands exactly on the 5 o'clock tick.
    let velocity_fraction = (velocity_mph / MAX_VELOCITY_MPH).clamp(0.0, 1.0);
    let velocity_angle = ZERO_ANGLE + velocity_fraction * DIAL_SWEEP;
    draw_arc_band(
        data,
        VELOCITY_RADIUS - 7.0,
        VELOCITY_RADIUS + 2.0,
        ZERO_ANGLE,
        velocity_angle,
        cyan,
    );
    let vdir = Vec2::new(velocity_angle.cos(), velocity_angle.sin());
    let tangent = Vec2::new(-vdir.y, vdir.x);
    let tip = center + vdir * (VELOCITY_RADIUS + 10.0);
    let base = center + vdir * (VELOCITY_RADIUS - 5.0);
    draw_triangle(
        data,
        [tip, base + tangent * 6.0, base - tangent * 6.0],
        cyan,
    );

    // Acceleration: red arc and dot on the outer rim, using the same scale
    // geometry as the velocity dial.
    let accel_fraction = (accel_g / MAX_ACCEL_G).clamp(0.0, 1.0);
    let accel_angle = ZERO_ANGLE + accel_fraction * DIAL_SWEEP;
    draw_arc_band(
        data,
        ACCEL_RADIUS - 2.0,
        ACCEL_RADIUS + 3.0,
        ZERO_ANGLE,
        accel_angle,
        red,
    );
    let adir = Vec2::new(accel_angle.cos(), accel_angle.sin());
    draw_dot(data, center + adir * ACCEL_RADIUS, 4.0, red);

    // Zero marker at 7 o'clock and the Mach 2 end tick at 5 o'clock.
    let zero_dir = Vec2::new(ZERO_ANGLE.cos(), ZERO_ANGLE.sin());
    draw_line(
        data,
        center + zero_dir * (ACCEL_RADIUS - 14.0),
        center + zero_dir * (ACCEL_RADIUS + 7.0),
        [230, 235, 240, 240],
        1,
    );
    let max_dir = Vec2::new(
        (ZERO_ANGLE + DIAL_SWEEP).cos(),
        (ZERO_ANGLE + DIAL_SWEEP).sin(),
    );
    draw_line(
        data,
        center + max_dir * (ACCEL_RADIUS - 14.0),
        center + max_dir * (ACCEL_RADIUS + 7.0),
        [230, 235, 240, 240],
        1,
    );

    // Player position and a thin center ring.
    draw_ring(data, 9.0, 1.0, [120, 160, 130, 200]);
    draw_dot(data, center, 5.0, green);
    draw_dot(data, center, 2.0, [220, 255, 230, 255]);
    // Draw the acceleration marker last so the zero tick never hides it when
    // acceleration is exactly zero.
    draw_dot(data, center + adir * ACCEL_RADIUS, 5.0, red);
}

fn render_overlay_initial() -> Vec<u8> {
    vec![0; MAP_TEX * MAP_TEX * 4]
}

fn dial_label_position(angle: f32) -> (f32, f32) {
    let ui_scale = MAP_UI_SIZE / MAP_TEX as f32;
    let radius = (ACCEL_RADIUS - 26.0) * ui_scale;
    (
        MAP_UI_SIZE * 0.5 + angle.cos() * radius,
        MAP_UI_SIZE * 0.5 + angle.sin() * radius,
    )
}

fn setup_minimap(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    buildings: Res<StorefrontData>,
    streets: Res<StreetNetwork>,
    camera: Single<&Transform, With<FlyCamera>>,
) {
    let center = Vec2::new(camera.translation.x, camera.translation.z);
    let mut map_data = render_overlay_initial();
    let nearest = render_map_layer(
        &mut map_data,
        center,
        buildings.buildings(),
        &streets.streets,
    );
    let map = images.add(make_image(map_data));
    let overlay = images.add(make_image(render_overlay_initial()));
    commands.insert_resource(MinimapImages {
        map: map.clone(),
        overlay: overlay.clone(),
    });
    commands.insert_resource(NearestStreet(nearest.unwrap_or_default()));

    commands
        .spawn((
            Name::new("Minimap"),
            Node {
                position_type: PositionType::Absolute,
                right: px(16),
                bottom: px(14),
                width: px(MAP_UI_SIZE),
                height: px(MAP_UI_SIZE),
                ..default()
            },
        ))
        .with_children(|parent| {
            parent.spawn((
                MinimapMapImage,
                UiTransform::default(),
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
                ImageNode::new(map),
                ZIndex(1),
            ));
            parent.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
                ImageNode::new(overlay),
                ZIndex(5),
            ));

            parent.spawn((
                MinimapText::Telemetry,
                Text::new("X —   Y —   Z —\nV — mph  — kph  M —\nA — G"),
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgba(0.90, 0.95, 1.0, 0.98)),
                TextShadow::default(),
                TextLayout::no_wrap(),
                ZIndex(10),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(55),
                    top: px(62),
                    width: px(250),
                    ..default()
                },
            ));

            parent.spawn((
                MinimapText::LatLon,
                Text::new("LAT — N\nLON — W"),
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgba(0.86, 0.93, 1.0, 0.98)),
                TextShadow::default(),
                TextLayout::justify(Justify::Right),
                ZIndex(10),
                Node {
                    position_type: PositionType::Absolute,
                    right: px(62),
                    bottom: px(54),
                    width: px(108),
                    ..default()
                },
            ));

            parent.spawn((
                MinimapText::Street,
                Text::new("ST  —"),
                TextFont {
                    font_size: FontSize::Px(10.0),
                    ..default()
                },
                TextColor(Color::srgba(0.72, 0.92, 0.82, 0.98)),
                TextShadow::default(),
                TextLayout::no_wrap(),
                ZIndex(10),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(55),
                    top: px(108),
                    width: px(250),
                    ..default()
                },
            ));

            // The zero tick is at 7 o'clock; the label sits just inside it.
            let (zero_x, zero_y) = dial_label_position(ZERO_ANGLE);
            parent.spawn((
                Text::new("0"),
                TextFont {
                    font_size: FontSize::Px(9.0),
                    ..default()
                },
                TextColor(Color::srgba(0.86, 0.90, 0.96, 0.95)),
                TextShadow::default(),
                ZIndex(10),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(zero_x - 3.0),
                    top: px(zero_y - 5.0),
                    ..default()
                },
            ));

            // Mach 1 is halfway round the 300° sweep, at 12 o'clock.
            let (m1_x, m1_y) = dial_label_position(ZERO_ANGLE + DIAL_SWEEP * 0.5);
            parent.spawn((
                Text::new("M1"),
                TextFont {
                    font_size: FontSize::Px(9.0),
                    ..default()
                },
                TextColor(Color::srgba(0.80, 0.88, 0.96, 0.92)),
                TextShadow::default(),
                ZIndex(10),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(m1_x - 7.0),
                    top: px(m1_y - 5.0),
                    ..default()
                },
            ));
        });
}

fn minimap_rotation(yaw: f32) -> Rot2 {
    // Camera yaw is positive to the left.  Facing west (yaw +90°) needs the
    // map's north rotated clockwise so west is at the top; UiTransform's
    // positive rotation is clockwise, so the yaw can be used directly.
    Rot2::radians(yaw)
}

fn update_minimap_heading(
    camera: Single<&FlyCamera, With<FlyCamera>>,
    map: Single<&mut UiTransform, With<MinimapMapImage>>,
) {
    let mut map = map.into_inner();
    map.rotation = minimap_rotation(camera.yaw);
}

#[allow(clippy::too_many_arguments)]
fn update_minimap_map(
    time: Res<Time>,
    camera: Single<&Transform, With<FlyCamera>>,
    buildings: Res<StorefrontData>,
    streets: Res<StreetNetwork>,
    nearest: Option<ResMut<NearestStreet>>,
    images: Option<Res<MinimapImages>>,
    mut assets: ResMut<Assets<Image>>,
    mut last: Local<Option<(Vec2, f32)>>,
) {
    let Some(images) = images else {
        return;
    };
    let center = Vec2::new(camera.translation.x, camera.translation.z);
    let now = time.elapsed_secs();
    if let Some((last_center, last_time)) = *last
        && (now - last_time < MAP_UPDATE_SECS
            || center.distance_squared(last_center) < MAP_MOVE_EPS_SQ)
    {
        return;
    }
    *last = Some((center, now));
    let Some(mut image) = assets.get_mut(&images.map) else {
        return;
    };
    let Some(data) = image.data.as_mut() else {
        return;
    };
    data.fill(0);
    let nearest_name = render_map_layer(data, center, buildings.buildings(), &streets.streets);
    if let Some(mut nearest) = nearest {
        nearest.0 = nearest_name.unwrap_or_default();
    }
}

fn update_minimap_overlay(
    time: Res<Time>,
    motion: Res<MotionState>,
    images: Option<Res<MinimapImages>>,
    mut assets: ResMut<Assets<Image>>,
    mut last: Local<f32>,
) {
    let Some(images) = images else {
        return;
    };
    let now = time.elapsed_secs();
    if now - *last < OVERLAY_UPDATE_SECS {
        return;
    }
    *last = now;
    let Some(mut image) = assets.get_mut(&images.overlay) else {
        return;
    };
    let Some(data) = image.data.as_mut() else {
        return;
    };
    data.fill(0);
    let mph = motion.speed * 2.236_936;
    render_overlay_layer(data, mph, motion.accel_g);
}

fn update_minimap_text(
    time: Res<Time>,
    camera: Single<&Transform, With<FlyCamera>>,
    motion: Res<MotionState>,
    nearest: Res<NearestStreet>,
    mut last: Local<f32>,
    mut texts: Query<(&MinimapText, &mut Text)>,
) {
    let now = time.elapsed_secs();
    if now - *last < TEXT_UPDATE_SECS {
        return;
    }
    *last = now;
    let position = camera.translation;
    let mph = motion.speed * 2.236_936;
    let kph = motion.speed * 3.6;
    let mach = motion.speed / MACH_1_MPS;
    let (lon, lat) = world_to_lon_lat(position.x, position.z);
    for (kind, mut text) in &mut texts {
        text.0 = match kind {
            MinimapText::Telemetry => format!(
                "X {:>6.0}   Y {:>5.0}   Z {:>6.0}\n\
                 V {:>5.0} mph  {:>5.0} kph  M {:.3}\n\
                 A {:.2} G",
                position.x, position.y, position.z, mph, kph, mach, motion.accel_g,
            ),
            MinimapText::LatLon => format!("LAT {:.5} N\nLON {:.5} W", lat.abs(), lon.abs()),
            MinimapText::Street => {
                if nearest.0.is_empty() {
                    "ST  —".to_string()
                } else {
                    format!("ST  {}", nearest.0)
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_matches_the_blend_scene_anchor() {
        let (lon, lat) = world_to_lon_lat(0.0, 0.0);
        assert!((lon - -74.000_875_910_036_54).abs() < 1e-9, "lon {lon}");
        assert!((lat - 40.713_460_624_392_276).abs() < 1e-9, "lat {lat}");
    }

    #[test]
    fn map_rotation_puts_the_heading_at_the_top() {
        use std::f32::consts::FRAC_PI_2;
        // Facing west should rotate the map clockwise so west is up.
        let west = minimap_rotation(FRAC_PI_2);
        assert!(west.sin > 0.99, "west heading should rotate clockwise");
        // Facing east should rotate it counter-clockwise so east is up.
        let east = minimap_rotation(-FRAC_PI_2);
        assert!(
            east.sin < -0.99,
            "east heading should rotate counter-clockwise"
        );
    }

    #[test]
    fn mach_two_sits_at_five_oclock() {
        let max_angle = ZERO_ANGLE + DIAL_SWEEP;
        // 5 o'clock is 60° clockwise from +x in screen coordinates.
        assert!((max_angle - (std::f32::consts::FRAC_PI_3 + TAU)).abs() < 1e-4);
        let dir = Vec2::new(max_angle.cos(), max_angle.sin());
        assert!(dir.x > 0.49 && dir.y > 0.85, "dir {dir:?}");
    }

    #[test]
    fn street_network_has_names_and_geometry() {
        let network = StreetNetwork::load();
        assert!(network.streets.len() > 4_000);
        let named = network
            .streets
            .iter()
            .filter(|street| !street.name.is_empty())
            .count();
        assert!(named > 3_000, "named streets: {named}");
        assert!(
            network
                .streets
                .iter()
                .any(|street| street.name == "Broadway")
        );
        for street in network.streets.iter().take(500) {
            assert!(street.points.len() >= 2);
            assert!(street.min.x <= street.max.x && street.min.y <= street.max.y);
        }
    }

    #[test]
    fn nearest_street_finds_broadway() {
        // Broadway passes close to the model origin near City Hall.
        let network = StreetNetwork::load();
        let name = nearest_named_street(Vec2::new(0.0, 0.0), &network.streets);
        assert!(name.is_some(), "no street near the origin");
    }
}
