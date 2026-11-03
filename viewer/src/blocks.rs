//! Irregular city-block day/night response.
//!
//! The Blender art master is segmented from building footprints: blocks are
//! connected building clusters (8 m dilation) with boundaries in the streets,
//! plus large open-space/park blocks.  Every building/ground vertex carries its
//! block id in COLOR_0 alpha; this module decides each block's blend each frame.
//!
//! A block is either:
//!   * Flicker — snaps on/off with a fluorescent-tube burst, or
//!   * Fade — crossfades smoothly through its own twilight window.
//!
//! The style is re-randomised for every block at the start of every penumbra
//! (each dusk and each dawn, and in reverse time).

#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use serde::Deserialize;

use crate::astro::{Date, DateTime, NYC_LAT, NYC_LON, sun_position};
use crate::city_material::MAX_BLOCKS;
use crate::clock::SimClock;
use crate::environment::Environment;

pub const TWILIGHT_START_ALT: f32 = 1.0;
pub const TWILIGHT_END_ALT: f32 = -6.0;

/// 0 = full day, 1 = full night, driven by the sun's altitude.
pub fn twilight_progress(sun_altitude_deg: f32) -> f32 {
    ((TWILIGHT_START_ALT - sun_altitude_deg) / (TWILIGHT_START_ALT - TWILIGHT_END_ALT))
        .clamp(0.0, 1.0)
}

/// Actual dusk duration in minutes for the current date, from +1° to −6°.
pub fn twilight_span_minutes(date: Date) -> f32 {
    let mut prev = sun_position(
        &DateTime {
            date,
            minute_of_day: 0.0,
        },
        NYC_LAT,
        NYC_LON,
    )
    .altitude_deg;
    let mut start: Option<f64> = None;
    for m in 1..=1440 {
        let alt = sun_position(
            &DateTime {
                date,
                minute_of_day: m as f64,
            },
            NYC_LAT,
            NYC_LON,
        )
        .altitude_deg;
        if start.is_none() && prev > TWILIGHT_START_ALT && alt <= TWILIGHT_START_ALT {
            start = Some(m as f64);
        }
        if let Some(s) = start
            && prev > TWILIGHT_END_ALT
            && alt <= TWILIGHT_END_ALT
        {
            return (m as f64 - s) as f32;
        }
        prev = alt;
    }
    0.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionStyle {
    Flicker,
    Fade,
}

#[derive(Clone, Debug)]
pub struct BlockState {
    pub threshold: f32,
    pub style: TransitionStyle,
    pub fade_width: f32,
    pub blend: f32,
    pub lit: bool,
    pub target: bool,
    flicker_until: f32,
    next_toggle: f32,
    seed: u32,
}

#[derive(Deserialize)]
struct BlocksFile {
    block_count: usize,
    building_blocks: usize,
    park_blocks: usize,
}

#[derive(Resource)]
pub struct BlockLights {
    pub states: Vec<BlockState>,
    pub block_count: usize,
    pub building_blocks: usize,
    pub park_blocks: usize,
    pub progress: f32,
    pub penumbra_id: u32,
    salt: u32,
    was_in_penumbra: bool,
    pub lit_count: usize,
    pub target_count: usize,
    pub flicker_count: usize,
    pub fade_count: usize,
    pub twilight_span_min: f32,
    span_date: Date,
}

impl Default for BlockLights {
    fn default() -> Self {
        let parsed: BlocksFile = serde_json::from_str(include_str!("../assets/models/blocks.json"))
            .unwrap_or(BlocksFile {
                block_count: 1,
                building_blocks: 1,
                park_blocks: 0,
            });
        let count = parsed.block_count.min(MAX_BLOCKS);
        #[cfg(not(target_arch = "wasm32"))]
        let salt = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u32)
            .unwrap_or(0x1234_5678);
        #[cfg(target_arch = "wasm32")]
        let salt = 0x9E37_79B9u32;
        let states = (0..count)
            .map(|i| BlockState::new(i, salt))
            .collect::<Vec<_>>();
        Self {
            states,
            block_count: count,
            building_blocks: parsed.building_blocks,
            park_blocks: parsed.park_blocks,
            progress: 0.0,
            penumbra_id: 0,
            salt,
            was_in_penumbra: false,
            lit_count: 0,
            target_count: 0,
            flicker_count: 0,
            fade_count: 0,
            twilight_span_min: 0.0,
            span_date: Date::new(0, 1, 1),
        }
    }
}

fn hash_u32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

fn rand01(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*seed >> 8) as f32 / ((u32::MAX >> 8) as f32)
}

fn normal_sample(seed: &mut u32) -> f32 {
    let u1 = rand01(seed).max(1e-6);
    let u2 = rand01(seed);
    (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl BlockState {
    fn new(index: usize, penumbra: u32) -> Self {
        let mut s = Self {
            threshold: 0.5,
            style: TransitionStyle::Fade,
            fade_width: 0.1,
            blend: 0.0,
            lit: false,
            target: false,
            flicker_until: 0.0,
            next_toggle: 0.0,
            seed: 0,
        };
        s.randomize(index, penumbra);
        s
    }

    fn randomize(&mut self, index: usize, penumbra: u32) {
        let mut seed = hash_u32(
            (index as u32)
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add(penumbra.wrapping_mul(0x85EB_CA6B))
                .wrapping_add(0xC2B2_AE35),
        );
        self.threshold = (0.5 + 0.18 * normal_sample(&mut seed)).clamp(0.03, 0.97);
        self.style = if rand01(&mut seed) < 0.55 {
            TransitionStyle::Flicker
        } else {
            TransitionStyle::Fade
        };
        self.fade_width = 0.06 + rand01(&mut seed) * 0.12;
        self.seed = seed;
        self.blend = 0.0;
        self.lit = false;
        self.target = false;
        self.flicker_until = 0.0;
        self.next_toggle = 0.0;
    }

    fn fade_target(&self, progress: f32) -> f32 {
        let half = self.fade_width * 0.5;
        smoothstep(self.threshold - half, self.threshold + half, progress)
    }
}

impl BlockLights {
    fn randomize_penumbra(&mut self, progress: f32) {
        self.penumbra_id = self.penumbra_id.wrapping_add(1);
        for (i, state) in self.states.iter_mut().enumerate() {
            state.randomize(i, self.penumbra_id ^ self.salt);
            state.blend = if progress >= state.threshold {
                1.0
            } else {
                0.0
            };
            state.lit = state.blend > 0.5;
            state.target = state.lit;
        }
        info!(
            "New penumbra #{}: {} blocks re-randomised",
            self.penumbra_id,
            self.states.len()
        );
    }
}

fn step_flicker(state: &mut BlockState, target: bool, now: f32) {
    if target != state.target {
        state.target = target;
        let mut s = state.seed;
        state.flicker_until = now
            + if target {
                0.55 + rand01(&mut s) * 0.55
            } else {
                0.25 + rand01(&mut s) * 0.25
            };
        state.next_toggle = now;
        state.seed = s;
    }
    if now < state.flicker_until {
        if now >= state.next_toggle {
            state.lit = !state.lit;
            let mut s = state.seed;
            state.next_toggle = now + 0.04 + rand01(&mut s) * 0.08;
            state.seed = s;
        }
    } else {
        state.lit = target;
    }
}

pub fn update_blocks(
    time: Res<Time>,
    clock: Res<SimClock>,
    env: Res<Environment>,
    mut lights: ResMut<BlockLights>,
) {
    if lights.states.is_empty() {
        return;
    }

    if lights.span_date != clock.date {
        lights.twilight_span_min = twilight_span_minutes(clock.date);
        lights.span_date = clock.date;
    }

    let progress = twilight_progress(env.sun.altitude_deg);
    lights.progress = progress;
    let in_penumbra = progress > 0.0 && progress < 1.0;
    if in_penumbra && !lights.was_in_penumbra {
        lights.randomize_penumbra(progress);
    }
    lights.was_in_penumbra = in_penumbra;

    let now = time.elapsed_secs();
    let mut lit_count = 0;
    let mut target_count = 0;
    let mut flicker_count = 0;
    let mut fade_count = 0;

    for state in lights.states.iter_mut() {
        let target = progress >= state.threshold;
        state.target = target;
        match state.style {
            TransitionStyle::Flicker => {
                flicker_count += 1;
                step_flicker(state, target, now);
                state.blend = if state.lit { 1.0 } else { 0.0 };
            }
            TransitionStyle::Fade => {
                fade_count += 1;
                state.blend = state.fade_target(progress);
                state.lit = state.blend > 0.5;
            }
        }
        if state.lit {
            lit_count += 1;
        }
        if target {
            target_count += 1;
        }
    }
    lights.lit_count = lit_count;
    lights.target_count = target_count;
    lights.flicker_count = flicker_count;
    lights.fade_count = fade_count;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twilight_progress_endpoints() {
        assert_eq!(twilight_progress(20.0), 0.0);
        assert_eq!(twilight_progress(1.0), 0.0);
        assert!((twilight_progress(-2.5) - 0.5).abs() < 1e-5);
        assert_eq!(twilight_progress(-6.0), 1.0);
        assert_eq!(twilight_progress(-30.0), 1.0);
    }

    #[test]
    fn twilight_progress_is_monotonic() {
        let mut prev = 0.0;
        for alt in [5.0, 2.0, 0.0, -1.0, -2.0, -3.0, -4.0, -5.0, -6.0, -8.0] {
            let p = twilight_progress(alt);
            assert!(p >= prev);
            prev = p;
        }
    }

    #[test]
    fn thresholds_are_gaussian_and_in_range() {
        let lights = BlockLights::default();
        assert!(
            lights.block_count > 1000,
            "block count {}",
            lights.block_count
        );
        assert_eq!(lights.states.len(), lights.block_count);
        let mut samples = Vec::new();
        for s in &lights.states {
            assert!((0.03..=0.97).contains(&s.threshold));
            samples.push(s.threshold);
        }
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        assert!((mean - 0.5).abs() < 0.03, "mean {mean}");
    }

    #[test]
    fn styles_are_mixed_each_penumbra() {
        let mut lights = BlockLights::default();
        lights.randomize_penumbra(0.5);
        let flicker = lights
            .states
            .iter()
            .filter(|s| s.style == TransitionStyle::Flicker)
            .count();
        let fade = lights.states.len() - flicker;
        assert!(flicker > 100, "flicker {flicker}");
        assert!(fade > 100, "fade {fade}");
        // a new penumbra should change at least some styles/thresholds
        let before: Vec<f32> = lights.states.iter().map(|s| s.threshold).collect();
        lights.randomize_penumbra(0.5);
        let after: Vec<f32> = lights.states.iter().map(|s| s.threshold).collect();
        assert!(
            before
                .iter()
                .zip(after.iter())
                .filter(|(a, b)| (*a - *b).abs() > 1e-5)
                .count()
                > 100
        );
    }

    #[test]
    fn flicker_state_machine_on_and_off() {
        let mut state = BlockState::new(7, 1);
        state.style = TransitionStyle::Flicker;
        state.blend = 0.0;
        state.lit = false;
        state.target = false;
        let mut now = 1.0;
        step_flicker(&mut state, true, now);
        assert!(state.flicker_until > now);
        let mut toggles = 0;
        let mut prev = state.lit;
        while now < 5.0 {
            now += 0.02;
            step_flicker(&mut state, true, now);
            if state.lit != prev {
                toggles += 1;
                prev = state.lit;
            }
        }
        assert!(toggles >= 2, "toggles {toggles}");
        assert!(state.lit);
        step_flicker(&mut state, false, now);
        while now < 10.0 {
            now += 0.02;
            step_flicker(&mut state, false, now);
        }
        assert!(!state.lit);
    }

    #[test]
    fn fade_is_smooth_and_reversible() {
        let mut state = BlockState::new(9, 1);
        state.style = TransitionStyle::Fade;
        state.threshold = 0.5;
        state.fade_width = 0.1;
        let low = state.fade_target(0.4);
        let mid = state.fade_target(0.5);
        let high = state.fade_target(0.6);
        assert!(low < mid && mid < high);
        assert!((mid - 0.5).abs() < 1e-5);
        // reverse progress decreases the blend
        assert!(state.fade_target(0.45) < state.fade_target(0.55));
    }

    #[test]
    fn civil_twilight_span_is_plausible() {
        let summer = twilight_span_minutes(Date::new(2026, 6, 21));
        let winter = twilight_span_minutes(Date::new(2026, 12, 21));
        assert!((20.0..60.0).contains(&summer), "summer {summer}");
        assert!((15.0..50.0).contains(&winter), "winter {winter}");
        assert!(summer > winter, "summer {summer} winter {winter}");
    }
}
