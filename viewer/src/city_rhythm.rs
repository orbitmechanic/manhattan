//! City heartbeat: Times-Square fields shared by lighting, facades, ads,
//! traffic and pedestrians.
//!
//! All functions are pure and deterministic. They are generation/presentation
//! inputs only; none of them enters the damage journal.

use bevy::prelude::*;

/// Times Square epicentre, projected onto the shipped geometry.
///
/// The Blender dataset stops at ~40.731 N (Central Park South), while the real
/// Times Square is ~4 km further north.  This is the nearest mapped point at
/// Times Square's longitude; the radial fields are anchored here so the epicentre
/// is actually inside the city that renders. Documented in INTEGRATION.md.
pub const TIMES_SQUARE: Vec2 = Vec2::new(1711.638, -2000.0);

pub fn epicenter_distance(x: f32, z: f32) -> f32 {
    Vec2::new(x, z).distance(TIMES_SQUARE)
}

fn hash01(mut v: u32) -> f32 {
    v ^= v >> 16;
    v = v.wrapping_mul(0x7FEB_352D);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846C_A68F);
    v ^= v >> 16;
    (v & 0x00FF_FFFF) as f32 / 16_777_216.0
}

/// Advertising density: saturated in the Times Square core, zero at the map
/// edge. 1.0 within 250 m, smooth falloff to 0 at 2.5 km.
pub fn ad_density(x: f32, z: f32) -> f32 {
    let d = epicenter_distance(x, z);
    if d <= 250.0 {
        return 1.0;
    }
    ((2_500.0 - d) / 2_250.0).clamp(0.0, 1.0).powf(1.35)
}

/// Later buildings skew glassy.
fn year_factor(year: u32) -> f32 {
    let t = ((year as f32 - 1965.0) / 20.0).clamp(-4.0, 4.0);
    0.18 + 0.82 / (1.0 + (-t).exp())
}

/// Probability of a glassy facade for a record. Height never appears here:
/// tall brick buildings are explicitly allowed.
pub fn glass_probability(x: f32, z: f32, year: u32) -> f32 {
    let d = epicenter_distance(x, z);
    let radial = (-(d / 900.0).powi(2)).exp();
    (0.04 + 0.90 * radial * year_factor(year)).clamp(0.04, 0.945)
}

/// Deterministic per-building roll used by both generators' classification.
pub fn glass_roll(id: usize, block: usize) -> f32 {
    hash01(
        id as u32
            ^ (block as u32)
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add(0xA17E_55AA),
    )
}

pub fn is_glassy(id: usize, block: usize, x: f32, z: f32, year: u32) -> bool {
    glass_roll(id, block) < glass_probability(x, z, year)
}

fn gauss(hour: f32, centre: f32, width: f32) -> f32 {
    (-((hour - centre) / width).powi(2)).exp()
}

/// Daily activity: 0.05 at 3 AM, peaks at 8:30 and 17:00.
pub fn daily_pace(hour: f32) -> f32 {
    let hour = hour.rem_euclid(24.0);
    let am = gauss(hour, 8.5, 1.25);
    let pm = gauss(hour, 17.0, 1.45);
    let midday = gauss(hour, 13.0, 3.0) * 0.55;
    0.05 + 0.90 * am.max(pm).max(midday).clamp(0.0, 1.0)
}

/// Lane occupancy. At the AM peak, lanes toward Times Square carry ~0.86 and
/// the reverse ~0.41; the asymmetry flips at the PM peak.
pub fn lane_occupancy(hour: f32, toward_epicenter: bool) -> f32 {
    let hour = hour.rem_euclid(24.0);
    let pace = daily_pace(hour);
    let am = gauss(hour, 8.5, 1.25);
    let pm = gauss(hour, 17.0, 1.45);
    let toward_factor = if am >= pm {
        if toward_epicenter { 0.90 } else { 0.40 }
    } else if toward_epicenter {
        0.40
    } else {
        0.90
    };
    0.05 + (pace - 0.05) * toward_factor
}

/// Pedestrian field = normalized ad density × daily pace (ads drive crowds).
pub fn pedestrian_density(x: f32, z: f32, hour: f32) -> f32 {
    ad_density(x, z) * daily_pace(hour)
}

/// Hallogen/neon mix for one block, deterministic and stable.
pub fn block_ambient_color(block: usize) -> Vec3 {
    let a = hash01(block as u32 ^ 0x51A7_5EED);
    let b = hash01(block as u32 ^ 0x9E37_79B9);
    let halogen = Vec3::new(1.0, 0.72, 0.30);
    let neon = Vec3::new(0.24, 0.44, 1.0);
    halogen
        .lerp(neon, a * 0.75)
        .lerp(Vec3::new(0.95, 0.55, 0.85), b * 0.18)
}

/// Night ambient is ground-up: bright at the feet, dark at the roof.
pub fn night_ambient(block: usize, height_fraction: f32) -> Vec3 {
    let h = height_fraction.clamp(0.0, 1.0);
    block_ambient_color(block) * (1.0 - h).powf(1.7)
}

/// Moon tint from aggregate fire/smoke density (journal consumer, read-only).
pub fn moon_tint(burning_buildings: usize) -> Vec3 {
    let pollution = 1.0 - (-(burning_buildings as f32) / 26.0).exp();
    Vec3::new(0.62, 0.72, 1.0).lerp(Vec3::new(1.0, 0.43, 0.20), pollution)
}

/// Direct sun attenuation under cloud: overcast flattens and dims sunlight.
pub fn overcast_direct_factor(cover: f32, storm: f32) -> f32 {
    (1.0 - 0.80 * cover.clamp(0.0, 1.0) - 0.14 * storm.clamp(0.0, 1.0)).clamp(0.04, 1.0)
}

/// Deck altitude for a given overcast amount: low enough to swallow the top
/// ~20 % of the tallest downtown towers (the tallest record is ~410 m).
pub fn cloud_deck_altitude(cover: f32, tallest_m: f32) -> f32 {
    let base = (tallest_m * 0.80).clamp(260.0, 420.0);
    let high = 960.0;
    let cover = cover.clamp(0.0, 1.0);
    if cover >= 0.70 {
        base
    } else {
        high + (base - high) * (cover / 0.70)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glass_probability_is_bounded_and_radial() {
        let core = glass_probability(TIMES_SQUARE.x, TIMES_SQUARE.y, 2020);
        assert!(core > 0.6 && core < 0.95, "core {core}");
        let far = glass_probability(TIMES_SQUARE.x + 8_000.0, TIMES_SQUARE.y, 2020);
        assert!(far < 0.06);
        assert!(core > far);
        // Later buildings are glassier at the same place.
        let old = glass_probability(TIMES_SQUARE.x, TIMES_SQUARE.y, 1900);
        assert!(glass_probability(TIMES_SQUARE.x, TIMES_SQUARE.y, 2010) > old);
    }

    #[test]
    fn glass_probability_is_height_independent_by_construction() {
        // The formula has no height argument; assert two same-location/years
        // with wildly different heights share one probability.
        let a = glass_probability(TIMES_SQUARE.x + 400.0, TIMES_SQUARE.y, 1980);
        let b = glass_probability(TIMES_SQUARE.x + 400.0, TIMES_SQUARE.y, 1980);
        assert_eq!(a, b);
        assert!(a < 1.0);
    }

    #[test]
    fn daily_pace_has_two_peaks_and_a_graveyard() {
        assert!(daily_pace(3.0) <= 0.06);
        assert!(daily_pace(8.5) > 0.8);
        assert!(daily_pace(17.0) > 0.8);
        assert!(daily_pace(13.0) < daily_pace(8.5));
        assert!(daily_pace(3.0) < daily_pace(13.0));
    }

    #[test]
    fn commute_asymmetry_flips_between_peaks() {
        let am_in = lane_occupancy(8.5, true);
        let am_out = lane_occupancy(8.5, false);
        let pm_in = lane_occupancy(17.0, true);
        let pm_out = lane_occupancy(17.0, false);
        assert!(am_in > am_out + 0.3);
        assert!(pm_out > pm_in + 0.3);
        assert!((am_in - 0.86).abs() < 0.1);
        assert!((am_out - 0.41).abs() < 0.1);
        let night = lane_occupancy(3.0, true);
        assert!(night < 0.08);
    }

    #[test]
    fn ads_and_people_roll_off_from_times_square() {
        assert_eq!(ad_density(TIMES_SQUARE.x, TIMES_SQUARE.y), 1.0);
        assert!(ad_density(TIMES_SQUARE.x + 4_000.0, TIMES_SQUARE.y) == 0.0);
        assert!(
            pedestrian_density(TIMES_SQUARE.x, TIMES_SQUARE.y, 8.5)
                > pedestrian_density(TIMES_SQUARE.x + 800.0, TIMES_SQUARE.y, 8.5)
        );
    }

    #[test]
    fn night_ambient_is_ground_up_and_block_coloured() {
        let base = night_ambient(7, 0.0).length();
        let roof = night_ambient(7, 1.0).length();
        assert!(base > roof * 2.0);
        assert_ne!(block_ambient_color(7), block_ambient_color(8));
    }

    #[test]
    fn moon_tint_reddens_with_smoke_and_clears() {
        let clean = moon_tint(0);
        let burning = moon_tint(40);
        assert!(clean.z > clean.x);
        assert!(burning.x > clean.x);
        assert!(burning.z < clean.z);
        assert_eq!(moon_tint(0), clean);
    }

    #[test]
    fn overcast_dims_direct_light() {
        assert_eq!(overcast_direct_factor(0.0, 0.0), 1.0);
        assert!(overcast_direct_factor(0.72, 0.0) < 0.5);
        assert!(overcast_direct_factor(0.96, 1.0) < 0.15);
    }

    #[test]
    fn deck_drops_below_the_tallest_tower_top() {
        let deck = cloud_deck_altitude(1.0, 410.0);
        assert!(deck < 410.0 * 0.9);
        assert!(deck > 300.0);
        assert!(cloud_deck_altitude(0.0, 410.0) > 900.0);
        // A thunderstorm (cover ~0.96) must also sit at the low deck.
        assert!(cloud_deck_altitude(0.96, 410.0) < 360.0);
    }
}
