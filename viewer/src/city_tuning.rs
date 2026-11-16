//! Reviewer-facing tuning controls for the merged city, persisted to TOML.
//!
//! The standalone demo had these on its panel; the host lost them. Values are
//! loaded from `viewer/tuning.toml` at startup and saved with F6/F7.

use std::path::Path;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const TUNING_PATH: &str = "tuning.toml";

#[derive(Resource, Clone, Debug, Serialize, Deserialize)]
pub struct CityTuning {
    pub explosive_yield_kg: f32,
    pub projectile_mass_kg: f32,
    pub projectile_velocity_mps: f32,
    pub restoration_rate_m3s: f32,
    pub grip_max_g: f32,
    pub fire_stage_pace: f32,
    pub admission_budget_ms: f32,
}

impl Default for CityTuning {
    fn default() -> Self {
        Self {
            explosive_yield_kg: 5.0,
            projectile_mass_kg: 1_500.0,
            projectile_velocity_mps: 42.0,
            restoration_rate_m3s: 900.0,
            grip_max_g: 3.5,
            fire_stage_pace: 1.0,
            admission_budget_ms: 6.0,
        }
    }
}

impl CityTuning {
    pub fn load() -> Self {
        std::fs::read_to_string(TUNING_PATH)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> bool {
        toml::to_string_pretty(self)
            .ok()
            .and_then(|text| std::fs::write(TUNING_PATH, text).ok())
            .is_some()
    }
}

pub fn setup_tuning(mut commands: Commands) {
    let tuning = CityTuning::load();
    commands.insert_resource(tuning);
}

pub fn tuning_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut tuning: ResMut<CityTuning>,
    admission: Option<ResMut<destructibles_bevy::impact::Admission>>,
    destructibles: Option<ResMut<destructibles_bevy::tuning::Tuning>>,
) {
    if let Some(mut admission) = admission {
        admission.budget_ms = tuning.admission_budget_ms.max(0.5);
    }
    if let Some(mut tuning_res) = destructibles {
        tuning_res.grip_max_g = tuning.grip_max_g;
    }
    if keys.just_pressed(KeyCode::F6) {
        let _ = tuning.save();
    }
    if keys.just_pressed(KeyCode::F7) {
        *tuning = CityTuning::load();
    }
    let step = if keys.pressed(KeyCode::ShiftLeft) {
        10.0
    } else {
        1.0
    };
    if keys.just_pressed(KeyCode::ArrowUp) {
        tuning.explosive_yield_kg = (tuning.explosive_yield_kg + step).min(500.0);
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        tuning.explosive_yield_kg = (tuning.explosive_yield_kg - step).max(0.0);
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        tuning.projectile_velocity_mps =
            (tuning.projectile_velocity_mps + step * 10.0).min(2_000.0);
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        tuning.projectile_velocity_mps = (tuning.projectile_velocity_mps - step * 10.0).max(1.0);
    }
    let _ = Path::new(TUNING_PATH);
}

pub fn tuning_panel(
    tuning: Res<CityTuning>,
    mut text: Query<&mut Text, With<crate::instrument::InstrumentText>>,
    mut last: Local<String>,
) {
    let line = format!(
        "TUNE yield {:.1}kg mass {:.0}kg v {:.0}m/s restore {:.0}m3/s grip {:.1}g fire {:.2} admit {:.1}ms  F6 save/F7 load",
        tuning.explosive_yield_kg,
        tuning.projectile_mass_kg,
        tuning.projectile_velocity_mps,
        tuning.restoration_rate_m3s,
        tuning.grip_max_g,
        tuning.fire_stage_pace,
        tuning.admission_budget_ms
    );
    if *last == line {
        return;
    }
    *last = line.clone();
    for mut body in &mut text {
        if !body.0.contains("TUNE ") {
            body.0.push('\n');
            body.0.push_str(&line);
        } else if let Some(index) = body.0.find("\nTUNE ") {
            body.0.replace_range(index + 1.., &line);
        }
    }
}
