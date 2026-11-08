//! Round lower-right minimap and flight instrument dials.
//!
//! The map is generated on the CPU into two UI textures: a static-around-the-
//! current-position building footprint layer, and a transparent instrument
//! layer with the FOV rays, thermometer, velocity triangle and acceleration
//! rim.  This keeps the map crisp without adding a render pass or custom
//! material.

use std::f32::consts::TAU;

use bevy::{
    asset::RenderAssetUsages,
    image::{Image, ImageSampler},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

use crate::{
    astro::{NYC_LAT, NYC_LON},
    camera::FlyCamera,
    motion::MotionState,
    storefront::StorefrontData,
};

const MAP_TEX: usize = 512;
const MAP_RADIUS_WORLD: f32 = 650.0;
const MAP_CENTER: f32 = MAP_TEX as f32 * 0.5;
const MAP_RADIUS: f32 = MAP_TEX as f32 * 0.5 - 8.0;
const ACCEL_RADIUS: f32 = MAP_RADIUS - 8.0;
const VELOCITY_RADIUS: f32 = MAP_RADIUS - 30.0;
/// 7 o'clock in screen coordinates: atan2(y, x) with y down.
const ZERO_ANGLE: f32 = 2.094_395_2;
const MAX_VELOCITY_MPH: f32 = 5_000.0;
const MAX_ACCEL_G: f32 = 4.0;
const MAP_UI_SIZE: f32 = 360.0;
const MAP_UPDATE_SECS: f32 = 0.25;
const OVERLAY_UPDATE_SECS: f32 = 0.10;
const TEXT_UPDATE_SECS: f32 = 0.12;

#[derive(Resource)]
struct MinimapImages {
    map: Handle<Image>,
    overlay: Handle<Image>,
}

#[derive(Component, Clone, Copy)]
enum MinimapText {
    Telemetry,
    LatLon,
}

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, setup_minimap).add_systems(
            Update,
            (
                update_minimap_map,
                update_minimap_overlay,
                update_minimap_text,
            )
                .chain(),
        );
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

fn render_map_layer(data: &mut [u8], center: Vec2, buildings: &[crate::storefront::BuildingMeta]) {
    for y in 0..MAP_TEX as i32 {
        for x in 0..MAP_TEX as i32 {
            if inside_circle(x, y) {
                put(data, x, y, [5, 7, 9, 255]);
            }
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
}

fn render_overlay_layer(data: &mut [u8], yaw: f32, velocity_mph: f32, accel_g: f32) {
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

    // Graded ticks around the speed and acceleration rims.
    for index in 0..36 {
        let angle = index as f32 / 36.0 * TAU;
        let long = index % 6 == 0;
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

    // Field of view rays.
    let half_fov = 35.0_f32.to_radians();
    for offset in [-half_fov, half_fov] {
        let angle = yaw + std::f32::consts::PI + offset;
        let dir = Vec2::new(angle.sin(), angle.cos());
        draw_line(
            data,
            center,
            center + dir * (VELOCITY_RADIUS * 0.72),
            [45, 190, 96, 210],
            0,
        );
    }

    // Velocity thermometer, filled clockwise from the 7 o'clock zero.
    let velocity_fraction = (velocity_mph / MAX_VELOCITY_MPH).clamp(0.0, 1.0);
    let velocity_angle = ZERO_ANGLE + velocity_fraction * TAU;
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

    // Acceleration: red arc and dot on the outer rim.
    let accel_fraction = (accel_g / MAX_ACCEL_G).clamp(0.0, 1.0);
    let accel_angle = ZERO_ANGLE + accel_fraction * TAU;
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

    // Zero marker at 7 o'clock.
    let zero_dir = Vec2::new(ZERO_ANGLE.cos(), ZERO_ANGLE.sin());
    draw_line(
        data,
        center + zero_dir * (ACCEL_RADIUS - 14.0),
        center + zero_dir * (ACCEL_RADIUS + 7.0),
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

fn setup_minimap(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    buildings: Res<StorefrontData>,
    camera: Single<&Transform, With<FlyCamera>>,
) {
    let center = Vec2::new(camera.translation.x, camera.translation.z);
    let mut map_data = render_overlay_initial();
    render_map_layer(&mut map_data, center, buildings.buildings());
    let map = images.add(make_image(map_data));
    let overlay = images.add(make_image(render_overlay_initial()));
    commands.insert_resource(MinimapImages {
        map: map.clone(),
        overlay: overlay.clone(),
    });

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
                    left: px(42),
                    top: px(42),
                    width: px(280),
                    padding: UiRect::all(px(4)),
                    border_radius: BorderRadius::all(px(4)),
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
                    right: px(30),
                    bottom: px(26),
                    width: px(150),
                    padding: UiRect::all(px(4)),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
            ));

            // The zero tick is at 7 o'clock; the label sits just inside it.
            let zero = Vec2::new(ZERO_ANGLE.cos(), ZERO_ANGLE.sin());
            let ui_scale = MAP_UI_SIZE / MAP_TEX as f32;
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
                    left: px(MAP_UI_SIZE * 0.5 + zero.x * (ACCEL_RADIUS - 26.0) * ui_scale - 4.0),
                    top: px(MAP_UI_SIZE * 0.5 + zero.y * (ACCEL_RADIUS - 26.0) * ui_scale - 5.0),
                    ..default()
                },
            ));
        });
}

fn update_minimap_map(
    time: Res<Time>,
    camera: Single<&Transform, With<FlyCamera>>,
    buildings: Res<StorefrontData>,
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
        && now - last_time < MAP_UPDATE_SECS
        && center.distance_squared(last_center) < 25.0 * 25.0
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
    render_map_layer(data, center, buildings.buildings());
}

fn update_minimap_overlay(
    time: Res<Time>,
    camera: Single<&FlyCamera, With<FlyCamera>>,
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
    render_overlay_layer(data, camera.yaw, mph, motion.accel_g);
}

fn update_minimap_text(
    time: Res<Time>,
    camera: Single<&Transform, With<FlyCamera>>,
    motion: Res<MotionState>,
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
    let mach = motion.speed / 343.0;
    let lat = NYC_LAT - position.z / 111_320.0;
    let lon = NYC_LON + position.x / (111_320.0 * NYC_LAT.to_radians().cos());
    for (kind, mut text) in &mut texts {
        text.0 = match kind {
            MinimapText::Telemetry => format!(
                "X {:>6.0}   Y {:>5.0}   Z {:>6.0}\n\
                 V {:>5.0} mph  {:>5.0} kph  M {:.3}\n\
                 A {:.2} G",
                position.x, position.y, position.z, mph, kph, mach, motion.accel_g,
            ),
            MinimapText::LatLon => format!("LAT {:.5} N\nLON {:.5} W", lat.abs(), lon.abs()),
        };
    }
}
