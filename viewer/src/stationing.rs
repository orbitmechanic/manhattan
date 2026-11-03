//! Street-centreline stationing and civil coordinates.
//!
//! `station_lines.json` is generated from the block geometry + satellite road
//! hints by `scripts/build_station_lines.py`.  Each line carries station ticks
//! every 100 ft, plan tangent/arc/reverse-curve metadata, and a vertical tangent
//! profile.  The viewer draws the lines/ticks on demand and computes a civil
//! coordinate array for the camera against every line.

use bevy::prelude::*;
use serde::Deserialize;

use crate::camera::FlyCamera;

pub const FEET_PER_METER: f32 = 3.280_84;
const COORD_UPDATE_SECS: f32 = 0.15;

static STATION_DATA: &str = include_str!("../assets/models/station_lines.json");

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct PlanCurve {
    pub start_station_ft: f32,
    pub end_station_ft: f32,
    pub radius_ft: f32,
    pub turn_deg: f32,
    pub direction: String,
    pub reverse: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct VerticalCurve {
    #[serde(default)]
    pub start_station_ft: f32,
    #[serde(default)]
    pub end_station_ft: f32,
    #[serde(default)]
    pub length_ft: f32,
    #[serde(default)]
    pub k_value: f32,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct VerticalProfile {
    #[serde(default)]
    pub tangent_points_ft: Vec<f32>,
    #[serde(default)]
    pub grades: Vec<f32>,
    #[serde(default)]
    pub curves: Vec<VerticalCurve>,
    #[serde(default)]
    pub start_elevation_ft: f32,
    #[serde(default)]
    pub end_elevation_ft: f32,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct StationLine {
    pub id: String,
    pub points: Vec<[f32; 2]>,
    pub length_ft: f32,
    pub tick_interval_ft: f32,
    pub ticks_ft: Vec<f32>,
    pub tick_points: Vec<[f32; 2]>,
    pub plan_curves: Vec<PlanCurve>,
    pub vertical: VerticalProfile,
}

impl StationLine {
    /// Flat vertical tangent interpolation for the current profile.
    pub fn elevation_ft(&self, station_ft: f32) -> f32 {
        let start_station = self
            .vertical
            .tangent_points_ft
            .first()
            .copied()
            .unwrap_or(0.0);
        let grade = self.vertical.grades.first().copied().unwrap_or(0.0);
        self.vertical.start_elevation_ft + grade * (station_ft - start_station)
    }

    pub fn tangent_station(&self, station_ft: f32) -> (f32, f32) {
        let mut best = self
            .vertical
            .tangent_points_ft
            .first()
            .copied()
            .unwrap_or(0.0);
        let mut best_d = (station_ft - best).abs();
        for &tp in &self.vertical.tangent_points_ft {
            let d = (station_ft - tp).abs();
            if d < best_d {
                best = tp;
                best_d = d;
            }
        }
        (best, self.elevation_ft(best))
    }
}

#[derive(Deserialize)]
struct StationFile {
    line_count: usize,
    total_length_ft: f32,
    lines: Vec<StationLine>,
}

#[derive(Resource)]
pub struct Stationing {
    pub lines: Vec<StationLine>,
    pub line_count: usize,
    pub total_length_ft: f32,
}

impl Default for Stationing {
    fn default() -> Self {
        let parsed: StationFile =
            serde_json::from_str(STATION_DATA).expect("station_lines.json is invalid");
        Self {
            lines: parsed.lines,
            line_count: parsed.line_count,
            total_length_ft: parsed.total_length_ft,
        }
    }
}

#[derive(Resource, Clone, Debug)]
pub struct StationingConfig {
    pub show_lines: bool,
    pub show_ticks: bool,
    pub show_coords: bool,
}

impl Default for StationingConfig {
    fn default() -> Self {
        Self {
            show_lines: false,
            show_ticks: false,
            show_coords: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StationToggle {
    Lines,
    Ticks,
    Coords,
}

#[derive(Clone, Debug)]
pub struct CivilCoordinate {
    pub line_index: usize,
    pub line_id: String,
    /// Station along the line, measured from its start.
    pub station_ft: f32,
    /// Negative is left of the up-station direction; positive is right.
    pub offset_ft: f32,
    /// Camera elevation minus the line elevation at the nearest station.
    pub elevation_ft: f32,
    /// Plan distance from the camera to the centreline.
    pub plan_distance_ft: f32,
    /// Nearest vertical tangent point and its elevation.
    pub tangent_station_ft: f32,
    pub tangent_elevation_ft: f32,
    /// Signed station distance from that tangent and elevation difference from
    /// the tangent elevation to the camera.
    pub station_from_tangent_ft: f32,
    pub elevation_from_tangent_ft: f32,
}

#[derive(Resource, Default)]
pub struct StationingState {
    pub coords: Vec<CivilCoordinate>,
    last_update: f32,
}

impl StationingState {
    pub fn nearest(&self) -> Option<&CivilCoordinate> {
        self.coords
            .iter()
            .min_by(|a, b| a.plan_distance_ft.total_cmp(&b.plan_distance_ft))
    }
}

/// Project `p` onto a line in plan and return (station_ft, signed_offset_ft,
/// plan_distance_ft).  The sign is negative left / positive right of the
/// up-station direction.
pub fn project_line(line: &StationLine, p: Vec2) -> (f32, f32, f32) {
    let mut best_dist = f32::INFINITY;
    let mut best_station = 0.0;
    let mut best_offset = 0.0;
    let mut station_m = 0.0;
    for i in 0..line.points.len().saturating_sub(1) {
        let a = Vec2::new(line.points[i][0], line.points[i][1]);
        let b = Vec2::new(line.points[i + 1][0], line.points[i + 1][1]);
        let ab = b - a;
        let len2 = ab.length_squared();
        if len2 < 1e-9 {
            continue;
        }
        let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
        let q = a + ab * t;
        let d = p.distance(q);
        if d < best_dist {
            let seg_len = len2.sqrt();
            best_dist = d;
            best_station = (station_m + seg_len * t) * FEET_PER_METER;
            let dir = ab / seg_len;
            let right = Vec2::new(-dir.y, dir.x);
            best_offset = (p - q).dot(right) * FEET_PER_METER;
        }
        station_m += len2.sqrt();
    }
    (best_station, best_offset, best_dist * FEET_PER_METER)
}

pub fn update_civil_coordinates(
    time: Res<Time>,
    camera: Single<&Transform, With<FlyCamera>>,
    stationing: Res<Stationing>,
    mut state: ResMut<StationingState>,
) {
    let now = time.elapsed_secs();
    if !state.coords.is_empty() && now - state.last_update < COORD_UPDATE_SECS {
        return;
    }
    state.last_update = now;
    let p = Vec2::new(camera.translation.x, camera.translation.z);
    let camera_elevation_ft = camera.translation.y * FEET_PER_METER;
    state.coords.clear();
    state.coords.reserve(stationing.lines.len());
    for (line_index, line) in stationing.lines.iter().enumerate() {
        if line.points.len() < 2 {
            continue;
        }
        let (station_ft, offset_ft, plan_distance_ft) = project_line(line, p);
        let line_elevation = line.elevation_ft(station_ft);
        let (tangent_station_ft, tangent_elevation_ft) = line.tangent_station(station_ft);
        state.coords.push(CivilCoordinate {
            line_index,
            line_id: line.id.clone(),
            station_ft,
            offset_ft,
            elevation_ft: camera_elevation_ft - line_elevation,
            plan_distance_ft,
            tangent_station_ft,
            tangent_elevation_ft,
            station_from_tangent_ft: station_ft - tangent_station_ft,
            elevation_from_tangent_ft: camera_elevation_ft - tangent_elevation_ft,
        });
    }
}

pub fn draw_stationing(
    mut gizmos: Gizmos,
    config: Res<StationingConfig>,
    stationing: Res<Stationing>,
) {
    if config.show_lines {
        let color = Color::srgb(1.0, 0.68, 0.18);
        for line in &stationing.lines {
            if line.points.len() < 2 {
                continue;
            }
            gizmos.linestrip(
                line.points.iter().map(|p| Vec3::new(p[0], 0.22, p[1])),
                color,
            );
        }
    }
    if config.show_ticks {
        let color = Color::srgba(0.28, 0.86, 1.0, 0.92);
        for line in &stationing.lines {
            for p in &line.tick_points {
                gizmos.line(
                    Vec3::new(p[0], -3.0, p[1]),
                    Vec3::new(p[0], 30.0, p[1]),
                    color,
                );
            }
        }
    }
}

pub fn handle_station_keys(keys: Res<ButtonInput<KeyCode>>, mut config: ResMut<StationingConfig>) {
    if keys.just_pressed(KeyCode::KeyL) {
        config.show_lines = !config.show_lines;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        config.show_ticks = !config.show_ticks;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        config.show_coords = !config.show_coords;
    }
}

pub fn toggle_station(config: &mut StationingConfig, toggle: StationToggle) {
    match toggle {
        StationToggle::Lines => config.show_lines = !config.show_lines,
        StationToggle::Ticks => config.show_ticks = !config.show_ticks,
        StationToggle::Coords => config.show_coords = !config.show_coords,
    }
}

pub fn configure_station_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    config.line.width = 1.5;
    config.depth_bias = -0.05;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> StationLine {
        StationLine {
            id: "TEST".into(),
            points: vec![[0.0, 0.0], [100.0, 0.0]],
            length_ft: 100.0 * FEET_PER_METER,
            tick_interval_ft: 100.0,
            ticks_ft: vec![0.0, 100.0],
            tick_points: vec![[0.0, 0.0], [30.48, 0.0]],
            plan_curves: vec![],
            vertical: VerticalProfile {
                tangent_points_ft: vec![0.0, 100.0 * FEET_PER_METER],
                grades: vec![0.0],
                curves: vec![],
                start_elevation_ft: 0.0,
                end_elevation_ft: 0.0,
            },
        }
    }

    #[test]
    fn station_and_offset_sign() {
        let l = line();
        let (station, offset, distance) = project_line(&l, Vec2::new(15.0, 4.0));
        assert!((station - 15.0 * FEET_PER_METER).abs() < 1e-2);
        assert!(offset > 0.0, "right side should be positive: {offset}");
        assert!((distance - 4.0 * FEET_PER_METER).abs() < 1e-2);
        let (_, left, _) = project_line(&l, Vec2::new(15.0, -4.0));
        assert!(left < 0.0, "left side should be negative: {left}");
    }

    #[test]
    fn station_data_loads() {
        let s = Stationing::default();
        assert!(s.line_count > 1_000, "line count {}", s.line_count);
        assert_eq!(s.line_count, s.lines.len());
        assert!(s.lines.iter().all(|l| l.points.len() >= 2));
        assert!(s.lines.iter().any(|l| !l.plan_curves.is_empty()));
    }
}
