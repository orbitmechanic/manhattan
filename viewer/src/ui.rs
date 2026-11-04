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
use crate::stationing::{
    CivilCoordinate, StationToggle, Stationing, StationingConfig, StationingState, toggle_station,
};

#[derive(Component)]
pub struct StatusText;

#[derive(Component)]
pub struct ControlsText;

#[derive(Component)]
pub struct StationText;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct StationButton(pub StationToggle);

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

fn station_button(base: &mut ChildSpawnerCommands, label: &str, toggle: StationToggle) {
    let mut entity = base.spawn((
        Button,
        StationButton(toggle),
        Hovered::default(),
        Node {
            min_width: px(88),
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
                font_size: FontSize::Px(12.0),
                ..default()
            },
            TextColor(Color::srgb(0.88, 0.93, 1.0)),
        ));
    });
    entity.observe(
        move |_: On<Activate>, mut config: ResMut<StationingConfig>| {
            toggle_station(&mut config, toggle);
        },
    );
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

    // stationing / civil-coordinate panel
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(230),
                right: px(14),
                width: px(300),
                padding: UiRect::all(px(10)),
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.015, 0.025, 0.05, 0.85)),
            BorderColor::all(Color::srgba(0.40, 0.58, 0.82, 0.55)),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("STATIONING / CIVIL COORDINATES"),
                TextFont {
                    font_size: FontSize::Px(12.0),
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
                station_button(row, "lines [L]", StationToggle::Lines);
                station_button(row, "ticks [T]", StationToggle::Ticks);
                station_button(row, "coords [C]", StationToggle::Coords);
                station_button(row, "set [G]", StationToggle::Set);
            });
            p.spawn((
                StationText,
                Text::new("Station array loading…"),
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgba(0.82, 0.88, 0.97, 0.95)),
            ));
        });

    // bottom-left help
    commands.spawn((
        ControlsText,
        Text::new(
            "FLY: W A S D  |  Q/E down/up  |  hold RMB to look  |  Shift boost  |  wheel speed  |  arrows look\n\
             TIME: [ / ] day  |  - / = hour  |  ; / ' minute  |  , / . month  |  Space play/pause  |  B reverse  |  R reset\n\
             STATIONING: L lines  |  T 100-ft ticks  |  C coordinates  |  G imagery / building fronts",
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

pub fn update_station_text(
    config: Res<StationingConfig>,
    stationing: Res<Stationing>,
    state: Res<StationingState>,
    mut text: Query<&mut Text, With<StationText>>,
) {
    let Some(mut text) = text.iter_mut().next() else {
        return;
    };
    let selected = stationing.set(config.set);
    let coords = state.coords(config.set);
    let lines_state = if config.show_lines { "ON" } else { "OFF" };
    let ticks_state = if config.show_ticks { "ON" } else { "OFF" };
    let coords_state = if config.show_coords { "ON" } else { "OFF" };
    let mut out = format!(
        "set {} [G]   lines {lines_state}   ticks {ticks_state}\n\
         coords {coords_state}   {} lines / {:.1} mi   array {}\n\
         OFF: negative left / positive right (up-station)\n",
        config.set.label(),
        selected.lines.len(),
        selected.total_length_ft / 5280.0,
        coords.len()
    );
    if config.show_coords && !coords.is_empty() {
        let mut nearest: Vec<&CivilCoordinate> = coords.iter().collect();
        nearest.sort_by(|a, b| a.plan_distance_ft.total_cmp(&b.plan_distance_ft));
        for c in nearest.iter().take(3) {
            out.push_str(&format!(
                "{}  STA {:.1}  OFF {:+.1}  EL {:+.1}\n",
                c.line_id, c.station_ft, c.offset_ft, c.elevation_ft
            ));
        }
        if let Some(c) = nearest.first() {
            out.push_str(&format!(
                "nearest tangent STA {:.1} @ EL {:.1} ft   dSTA {:+.1}\n",
                c.tangent_station_ft, c.tangent_elevation_ft, c.station_from_tangent_ft
            ));
        }
    } else if !config.show_coords {
        out.push_str("camera civil-coordinate array hidden\n");
    }
    text.0 = out;
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
