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
const COORD_UPDATE_SECS: f32 = 0.25;
/// The HUD shows the three nearest lines; retain a few extra so the panel can
/// still choose the exact nearest three without keeping every line.
pub const MAX_COORDS_PER_SET: usize = 8;

static IMAGERY_DATA: &str = include_str!("../assets/models/station_lines_imagery.json");
static FRONTS_DATA: &str = include_str!("../assets/models/station_lines_fronts.json");
static COMPLETED_DATA: &str = include_str!("../assets/models/station_lines_completed.json");

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

#[derive(Clone, Debug)]
pub struct StationSet {
    pub lines: Vec<StationLine>,
    pub total_length_ft: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineSet {
    Imagery,
    Fronts,
    Completed,
}

impl LineSet {
    pub fn label(self) -> &'static str {
        match self {
            LineSet::Imagery => "IMAGERY",
            LineSet::Fronts => "BUILDING FRONTS",
            LineSet::Completed => "COMPLETED",
        }
    }

    pub fn next(self) -> Self {
        match self {
            LineSet::Imagery => LineSet::Fronts,
            LineSet::Fronts => LineSet::Completed,
            LineSet::Completed => LineSet::Imagery,
        }
    }
}

#[derive(Resource)]
pub struct Stationing {
    pub imagery: StationSet,
    pub fronts: StationSet,
    pub completed: StationSet,
}

impl Stationing {
    pub fn set(&self, set: LineSet) -> &StationSet {
        match set {
            LineSet::Imagery => &self.imagery,
            LineSet::Fronts => &self.fronts,
            LineSet::Completed => &self.completed,
        }
    }

    pub fn line_count(&self, set: LineSet) -> usize {
        self.set(set).lines.len()
    }
}

impl Default for Stationing {
    fn default() -> Self {
        let parse = |text: &str| {
            let parsed: StationFile =
                serde_json::from_str(text).expect("station_lines_*.json is invalid");
            (parsed.lines, parsed.total_length_ft, parsed.line_count)
        };
        let (imagery, imagery_len, imagery_count) = parse(IMAGERY_DATA);
        let (fronts, fronts_len, fronts_count) = parse(FRONTS_DATA);
        let (completed, completed_len, completed_count) = parse(COMPLETED_DATA);
        assert_eq!(imagery.len(), imagery_count);
        assert_eq!(fronts.len(), fronts_count);
        assert_eq!(completed.len(), completed_count);
        Self {
            imagery: StationSet {
                lines: imagery,
                total_length_ft: imagery_len,
            },
            fronts: StationSet {
                lines: fronts,
                total_length_ft: fronts_len,
            },
            completed: StationSet {
                lines: completed,
                total_length_ft: completed_len,
            },
        }
    }
}

#[derive(Resource, Clone, Debug)]
pub struct StationingConfig {
    pub show_lines: bool,
    pub show_ticks: bool,
    pub show_coords: bool,
    pub set: LineSet,
}

impl Default for StationingConfig {
    fn default() -> Self {
        Self {
            show_lines: false,
            show_ticks: false,
            show_coords: true,
            set: LineSet::Completed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StationToggle {
    Lines,
    Ticks,
    Coords,
    Set,
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
    pub imagery: Vec<CivilCoordinate>,
    pub fronts: Vec<CivilCoordinate>,
    pub completed: Vec<CivilCoordinate>,
}

impl StationingState {
    pub fn coords(&self, set: LineSet) -> &[CivilCoordinate] {
        match set {
            LineSet::Imagery => &self.imagery,
            LineSet::Fronts => &self.fronts,
            LineSet::Completed => &self.completed,
        }
    }

    pub fn nearest(&self, set: LineSet) -> Option<&CivilCoordinate> {
        self.coords(set)
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
    config: Res<StationingConfig>,
    camera: Single<&Transform, With<FlyCamera>>,
    stationing: Res<Stationing>,
    mut state: ResMut<StationingState>,
    mut last: Local<Option<(f32, Vec3)>>,
) {
    let now = time.elapsed_secs();
    let camera_position = camera.translation;
    let set = config.set;
    let switched = config.is_changed() || state.coords(set).is_empty();
    if !config.show_coords && !switched {
        return;
    }
    if !switched && let Some((last_time, last_position)) = *last {
        let moved = camera_position.distance_squared(last_position) > 25.0 * 25.0;
        if (moved && now - last_time < COORD_UPDATE_SECS) || (!moved && now - last_time < 5.0) {
            return;
        }
    }
    *last = Some((now, camera_position));

    let p = Vec2::new(camera_position.x, camera_position.z);
    let camera_elevation_ft = camera_position.y * FEET_PER_METER;
    let lines = &stationing.set(set).lines;
    let mut candidates: Vec<(f32, usize, f32, f32, f32, f32, f32)> =
        Vec::with_capacity(lines.len());
    for (line_index, line) in lines.iter().enumerate() {
        if line.points.len() < 2 {
            continue;
        }
        let (station_ft, offset_ft, plan_distance_ft) = project_line(line, p);
        let line_elevation = line.elevation_ft(station_ft);
        let (tangent_station_ft, tangent_elevation_ft) = line.tangent_station(station_ft);
        candidates.push((
            plan_distance_ft,
            line_index,
            station_ft,
            offset_ft,
            camera_elevation_ft - line_elevation,
            tangent_station_ft,
            tangent_elevation_ft,
        ));
    }
    if candidates.len() > MAX_COORDS_PER_SET {
        candidates.select_nth_unstable_by(MAX_COORDS_PER_SET, |a, b| a.0.total_cmp(&b.0));
        candidates.truncate(MAX_COORDS_PER_SET);
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let out: Vec<CivilCoordinate> = candidates
        .into_iter()
        .map(
            |(
                plan_distance_ft,
                line_index,
                station_ft,
                offset_ft,
                elevation_ft,
                tangent_station_ft,
                tangent_elevation_ft,
            )| CivilCoordinate {
                line_index,
                line_id: lines[line_index].id.clone(),
                station_ft,
                offset_ft,
                elevation_ft,
                plan_distance_ft,
                tangent_station_ft,
                tangent_elevation_ft,
                station_from_tangent_ft: station_ft - tangent_station_ft,
                elevation_from_tangent_ft: camera_elevation_ft - tangent_elevation_ft,
            },
        )
        .collect();
    match set {
        LineSet::Imagery => state.imagery = out,
        LineSet::Fronts => state.fronts = out,
        LineSet::Completed => state.completed = out,
    }
}

/// Squared distance from `c` to the 2D segment a-b, used to draw only the
/// parts of a centreline that fall inside the 1 km embellishment radius.
fn segment_distance_sq(c: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    let t = if len_sq <= f32::EPSILON {
        0.0
    } else {
        ((c - a).dot(ab) / len_sq).clamp(0.0, 1.0)
    };
    (a + ab * t).distance_squared(c)
}

pub fn draw_stationing(
    mut gizmos: Gizmos,
    config: Res<StationingConfig>,
    stationing: Res<Stationing>,
    camera: Single<&Transform, With<FlyCamera>>,
) {
    let selected = stationing.set(config.set);
    let c = Vec2::new(camera.translation.x, camera.translation.z);
    let r_sq = crate::detail::EMBELLISHMENT_RADIUS * crate::detail::EMBELLISHMENT_RADIUS;
    if config.show_lines {
        let color = match config.set {
            LineSet::Imagery => Color::srgb(0.20, 1.0, 0.48),
            LineSet::Fronts => Color::srgb(1.0, 0.68, 0.18),
            LineSet::Completed => Color::srgb(0.72, 0.52, 1.0),
        };
        for line in &selected.lines {
            for pair in line.points.windows(2) {
                let a = Vec2::from_array(pair[0]);
                let b = Vec2::from_array(pair[1]);
                if segment_distance_sq(c, a, b) > r_sq {
                    continue;
                }
                gizmos.line(Vec3::new(a.x, 0.22, a.y), Vec3::new(b.x, 0.22, b.y), color);
            }
        }
    }
    if config.show_ticks {
        let color = Color::srgba(0.28, 0.86, 1.0, 0.92);
        for line in &selected.lines {
            for p in &line.tick_points {
                let point = Vec2::new(p[0], p[1]);
                if point.distance_squared(c) > r_sq {
                    continue;
                }
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
    if keys.just_pressed(KeyCode::KeyG) {
        config.set = config.set.next();
    }
}

pub fn toggle_station(config: &mut StationingConfig, toggle: StationToggle) {
    match toggle {
        StationToggle::Lines => config.show_lines = !config.show_lines,
        StationToggle::Ticks => config.show_ticks = !config.show_ticks,
        StationToggle::Coords => config.show_coords = !config.show_coords,
        StationToggle::Set => config.set = config.set.next(),
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
    fn set_toggle_cycles() {
        let mut config = StationingConfig::default();
        assert_eq!(config.set, LineSet::Completed);
        toggle_station(&mut config, StationToggle::Set);
        assert_eq!(config.set, LineSet::Imagery);
        toggle_station(&mut config, StationToggle::Set);
        assert_eq!(config.set, LineSet::Fronts);
        toggle_station(&mut config, StationToggle::Set);
        assert_eq!(config.set, LineSet::Completed);
    }

    #[test]
    fn station_data_loads() {
        let s = Stationing::default();
        for set in [LineSet::Imagery, LineSet::Fronts, LineSet::Completed] {
            let data = s.set(set);
            assert!(data.lines.len() > 1_000, "{:?} line count", set);
            assert!(data.lines.iter().all(|l| l.points.len() >= 2));
            assert!(data.lines.iter().any(|l| !l.plan_curves.is_empty()));
        }
    }
}
