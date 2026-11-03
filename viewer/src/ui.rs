//! On-screen calendar/clock controls and status readout.

use bevy::{
    picking::hover::Hovered,
    prelude::*,
    ui_widgets::{Activate, Button},
};

use crate::blocks::BlockLights;
use crate::camera::{FlyCamera, MIN_CAMERA_HEIGHT};
use crate::clock::{ClockAction, SimClock};
use crate::environment::{Environment, fog_cull_distance};

#[derive(Component)]
pub struct StatusText;

#[derive(Component)]
pub struct ControlsText;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockButton(pub ClockAction);

fn button(base: &mut ChildSpawnerCommands, label: &str, action: ClockAction) {
    let mut entity = base.spawn((
        Button,
        ClockButton(action),
        Hovered::default(),
        Node {
            min_width: px(64),
            height: px(28),
            padding: UiRect::horizontal(px(8)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.12, 0.17, 0.25, 0.92)),
        BorderColor::all(Color::srgba(0.45, 0.62, 0.82, 0.75)),
    ));
    entity.with_children(|p| {
        p.spawn((
            Text::new(label),
            TextFont {
                font_size: FontSize::Px(13.0),
                ..default()
            },
            TextColor(Color::srgb(0.88, 0.93, 1.0)),
        ));
    });
    entity.observe(move |_: On<Activate>, mut clock: ResMut<SimClock>| {
        clock.apply(action);
    });
}

pub fn spawn_ui(mut commands: Commands) {
    // top-left status panel
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(14),
                left: px(14),
                padding: UiRect::all(px(12)),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.015, 0.025, 0.05, 0.82)),
            BorderColor::all(Color::srgba(0.40, 0.58, 0.82, 0.55)),
        ))
        .with_children(|p| {
            p.spawn((
                StatusText,
                Text::new("MANHATTAN"),
                TextFont {
                    font_size: FontSize::Px(17.0),
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.97, 1.0)),
            ));
        });

    // top-right calendar / clock panel
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(14),
                right: px(14),
                width: px(300),
                padding: UiRect::all(px(10)),
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.015, 0.025, 0.05, 0.82)),
            BorderColor::all(Color::srgba(0.40, 0.58, 0.82, 0.55)),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("CALENDAR / CLOCK"),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                TextColor(Color::srgb(0.62, 0.76, 0.95)),
            ));
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: px(6),
                row_gap: px(6),
                ..default()
            })
            .with_children(|row| {
                button(row, "- 1 day", ClockAction::Day(-1));
                button(row, "+ 1 day", ClockAction::Day(1));
                button(row, "- 1 hr", ClockAction::Hour(-1));
                button(row, "+ 1 hr", ClockAction::Hour(1));
                button(row, "- 5 min", ClockAction::Minute(-5));
                button(row, "+ 5 min", ClockAction::Minute(5));
                button(row, "- 1 mo", ClockAction::Month(-1));
                button(row, "+ 1 mo", ClockAction::Month(1));
                button(row, "play / pause", ClockAction::TogglePlay);
                button(row, "reverse", ClockAction::Reverse);
                button(row, "reset", ClockAction::Reset);
            });
        });

    // bottom-left help
    commands.spawn((
        ControlsText,
        Text::new(
            "FLY: W A S D  |  Q/E down/up  |  hold RMB to look  |  Shift boost  |  wheel speed  |  arrows look\n\
             TIME: [ / ] day  |  - / = hour  |  ; / ' minute  |  , / . month  |  Space play/pause  |  B reverse  |  R reset",
        ),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        TextColor(Color::srgba(0.80, 0.86, 0.95, 0.92)),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            left: px(14),
            padding: UiRect::all(px(8)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.015, 0.025, 0.05, 0.70)),
    ));
}

pub fn handle_calendar_keys(keys: Res<ButtonInput<KeyCode>>, mut clock: ResMut<SimClock>) {
    if keys.just_pressed(KeyCode::BracketLeft) {
        clock.apply(ClockAction::Day(-1));
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        clock.apply(ClockAction::Day(1));
    }
    if keys.just_pressed(KeyCode::Minus) {
        clock.apply(ClockAction::Hour(-1));
    }
    if keys.just_pressed(KeyCode::Equal) {
        clock.apply(ClockAction::Hour(1));
    }
    if keys.just_pressed(KeyCode::Semicolon) {
        clock.apply(ClockAction::Minute(-5));
    }
    if keys.just_pressed(KeyCode::Quote) {
        clock.apply(ClockAction::Minute(5));
    }
    if keys.just_pressed(KeyCode::Comma) {
        clock.apply(ClockAction::Month(-1));
    }
    if keys.just_pressed(KeyCode::Period) {
        clock.apply(ClockAction::Month(1));
    }
    if keys.just_pressed(KeyCode::Space) {
        clock.apply(ClockAction::TogglePlay);
    }
    if keys.just_pressed(KeyCode::KeyB) {
        clock.apply(ClockAction::Reverse);
    }
    if keys.just_pressed(KeyCode::KeyR) {
        clock.apply(ClockAction::Reset);
    }
}

pub fn update_status(
    clock: Res<SimClock>,
    env: Res<Environment>,
    blocks: Res<BlockLights>,
    camera: Single<(&Transform, &FlyCamera), With<FlyCamera>>,
    mut status: Query<&mut Text, With<StatusText>>,
) {
    let Some(mut text) = status.iter_mut().next() else {
        return;
    };
    let phase = if env.moon_illumination > 0.92 {
        "full".to_string()
    } else if env.moon_illumination > 0.55 {
        "gibbous".to_string()
    } else if env.moon_illumination > 0.15 {
        "quarter".to_string()
    } else {
        "crescent".to_string()
    };
    let (transform, fly) = *camera;
    let pos = transform.translation;
    let cull = fog_cull_distance(env.fog_visibility);
    text.0 = format!(
        "MANHATTAN  -  {}\n\
         {}   {}\n\
         Sun   alt {:+.1} deg   az {:05.1}\n\
         Moon  alt {:+.1} deg   az {:05.1}   {} {:.0}%\n\
         Twilight {:0.2}  ({:.0} min)   lit {}/{}\n\
         Penumbra #{}   {} flicker / {} fade\n\
         FULL FLOOR GEOMETRY  1.53M tris   cull {:.1} km\n\
         Fog vis {:.1} km   Camera  x {:.0}  y {:.0}  z {:.0}   (floor {:.1})\n\
         {} x{:.0}   speed {:.0} u/s",
        if env.night { "NIGHT" } else { "DAY" },
        clock.date_label(),
        clock.time_label(),
        env.sun.altitude_deg,
        env.sun.azimuth_deg,
        env.moon.altitude_deg,
        env.moon.azimuth_deg,
        phase,
        env.moon_illumination * 100.0,
        blocks.progress,
        blocks.twilight_span_min,
        blocks.lit_count,
        blocks.block_count,
        blocks.penumbra_id,
        blocks.flicker_count,
        blocks.fade_count,
        cull / 1000.0,
        env.fog_visibility / 1000.0,
        pos.x,
        pos.y,
        pos.z,
        MIN_CAMERA_HEIGHT,
        if clock.playing {
            if clock.speed < 0.0 {
                "PLAY REV"
            } else {
                "PLAY FWD"
            }
        } else if clock.speed < 0.0 {
            "|| REV"
        } else {
            "|| FWD"
        },
        clock.speed.abs(),
        fly.speed,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::Date;
    use bevy::app::Update;

    fn key_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(ButtonInput::<KeyCode>::default());
        app.insert_resource(SimClock::default());
        app.add_systems(Update, handle_calendar_keys);
        app
    }

    #[test]
    fn bracket_right_advances_the_calendar() {
        let mut app = key_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::BracketRight);
        app.update();
        let clock = app.world().resource::<SimClock>();
        assert_eq!(clock.date, Date::new(2026, 6, 22));
    }

    #[test]
    fn equal_advances_the_clock() {
        let mut app = key_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Equal);
        app.update();
        let clock = app.world().resource::<SimClock>();
        assert_eq!(clock.datetime().hour(), 13);
    }

    #[test]
    fn button_activate_changes_the_clock() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(SimClock::default());
        app.add_systems(Startup, spawn_ui);
        app.update();
        let entity = {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<ClockButton>>();
            q.iter(app.world())
                .find(|e| {
                    app.world()
                        .get::<ClockButton>(*e)
                        .is_some_and(|b| b.0 == ClockAction::Day(1))
                })
                .expect("day + button")
        };
        app.world_mut().trigger(Activate { entity });
        let clock = app.world().resource::<SimClock>();
        assert_eq!(clock.date, Date::new(2026, 6, 22));
    }

    #[test]
    fn space_toggles_playback() {
        let mut app = key_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        app.update();
        assert!(app.world().resource::<SimClock>().playing);
    }
}
