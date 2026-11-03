//! Calendar, clock and low-precision solar/lunar ephemeris.
//!
//! The model is Manhattan, so all calculations use New York City's coordinates
//! and Eastern Standard Time (UTC-5).  Daylight-saving time is deliberately
//! ignored: a viewer's clock is "local standard time", which keeps the demo
//! deterministic.

use std::f64::consts::PI;

pub const NYC_LAT: f32 = 40.7580;
pub const NYC_LON: f32 = -73.9855;
/// Local time = UTC + offset. EST is UTC-5.
pub const TZ_OFFSET_MIN: i32 = -300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    pub const fn new(year: i32, month: u32, day: u32) -> Self {
        Self { year, month, day }
    }

    pub fn is_leap(year: i32) -> bool {
        (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
    }

    pub fn days_in_month(year: i32, month: u32) -> u32 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if Self::is_leap(year) => 29,
            2 => 28,
            _ => 30,
        }
    }

    /// Julian day number at 00:00 UT (Gregorian calendar).
    pub fn julian_day(self) -> i64 {
        let a = (14 - self.month as i64) / 12;
        let y = self.year as i64 + 4800 - a;
        let m = self.month as i64 + 12 * a - 3;
        self.day as i64 + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045
    }

    pub fn from_julian_day(jdn: i64) -> Self {
        let a = jdn + 32044;
        let b = (4 * a + 3) / 146097;
        let c = a - 146097 * b / 4;
        let d = (4 * c + 3) / 1461;
        let e = c - 1461 * d / 4;
        let m = (5 * e + 2) / 153;
        let day = e - (153 * m + 2) / 5 + 1;
        let month = m + 3 - 12 * (m / 10);
        let year = 100 * b + d - 4800 + m / 10;
        Self {
            year: year as i32,
            month: month as u32,
            day: day as u32,
        }
    }

    pub fn add_days(self, delta: i32) -> Self {
        Self::from_julian_day(self.julian_day() + delta as i64)
    }

    pub fn add_months(self, delta: i32) -> Self {
        let mut total = self.year * 12 + self.month as i32 - 1 + delta;
        let year = total.div_euclid(12);
        total = total.rem_euclid(12);
        let month = total as u32 + 1;
        let day = self.day.min(Self::days_in_month(year, month));
        Self { year, month, day }
    }

    pub fn day_of_year(self) -> u32 {
        let mut d = self.day;
        for m in 1..self.month {
            d += Self::days_in_month(self.year, m);
        }
        d
    }

    pub fn weekday(self) -> &'static str {
        // 2026-01-01 was a Thursday; use JDN modulo 7 for any date.
        const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        // JDN 0 was a Monday.
        NAMES[(self.julian_day().rem_euclid(7)) as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DateTime {
    pub date: Date,
    /// Local standard time, in minutes since midnight (0..1440).
    pub minute_of_day: f64,
}

impl DateTime {
    pub fn new(date: Date, hour: u32, minute: u32) -> Self {
        Self {
            date,
            minute_of_day: (hour * 60 + minute) as f64,
        }
    }

    pub fn hour(&self) -> u32 {
        (self.minute_of_day / 60.0).floor().rem_euclid(24.0) as u32
    }

    pub fn minute(&self) -> u32 {
        self.minute_of_day.floor().rem_euclid(60.0) as u32
    }

    pub fn julian_day_utc(&self) -> f64 {
        self.date.julian_day() as f64 + (self.minute_of_day - TZ_OFFSET_MIN as f64) / 1440.0
    }

    pub fn label(&self) -> String {
        format!(
            "{}-{:02}-{:02} ({})  {:02}:{:02}",
            self.date.year,
            self.date.month,
            self.date.day,
            self.date.weekday(),
            self.hour(),
            self.minute()
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SkyPosition {
    /// Degrees above the horizon (negative when set).
    pub altitude_deg: f32,
    /// Compass bearing: 0 = north, 90 = east, 180 = south, 270 = west.
    pub azimuth_deg: f32,
}

/// NOAA solar position (accurate to a few arcminutes for this purpose).
pub fn sun_position(dt: &DateTime, lat_deg: f32, lon_deg: f32) -> SkyPosition {
    let jd = dt.julian_day_utc();
    let t = (jd - 2_451_545.0) / 36_525.0;

    let l0 = (280.46646 + t * (36_000.769_83 + t * 0.0003032)).rem_euclid(360.0);
    let m_deg = 357.52911 + t * (35_999.050_29 - 0.0001537 * t);
    let m = m_deg.to_radians();
    let c = m.sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + (2.0 * m).sin() * (0.019993 - 0.000101 * t)
        + (3.0 * m).sin() * 0.000289;
    let true_long = l0 + c;
    let omega = (125.04 - 1934.136 * t).to_radians();
    let lambda = (true_long - 0.00569 - 0.00478 * omega.sin()).to_radians();

    let eps0 = 23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let eps = (eps0 + 0.00256 * omega.cos()).to_radians();
    let decl = (eps.sin() * lambda.sin()).asin();

    let y = (eps / 2.0).tan().powi(2);
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let l0_rad = l0.to_radians();
    let eqtime = 4.0
        * (y * (2.0 * l0_rad).sin() - 2.0 * e * m.sin()
            + 4.0 * e * y * m.sin() * (2.0 * l0_rad).cos()
            - 0.5 * y * y * (4.0 * l0_rad).sin()
            - 1.25 * e * e * (2.0 * m).sin())
        .to_degrees();

    // UTC minutes since midnight, then NOAA's true solar time.
    let utc_min = (dt.minute_of_day - TZ_OFFSET_MIN as f64).rem_euclid(1440.0);
    let true_solar = (utc_min + eqtime + 4.0 * lon_deg as f64).rem_euclid(1440.0);
    let ha = (true_solar / 4.0 - 180.0).to_radians();
    let lat = (lat_deg as f64).to_radians();

    let alt = (lat.sin() * decl.sin() + lat.cos() * decl.cos() * ha.cos()).asin();
    let az = ha
        .sin()
        .atan2(ha.cos() * lat.sin() - decl.tan() * lat.cos())
        .to_degrees()
        + 180.0;

    SkyPosition {
        altitude_deg: alt.to_degrees() as f32,
        azimuth_deg: az.rem_euclid(360.0) as f32,
    }
}

/// Low-precision lunar position (Meeus ch. 47) plus illuminated fraction.
pub fn moon_position(dt: &DateTime, lat_deg: f32, lon_deg: f32) -> (SkyPosition, f32) {
    let jd = dt.julian_day_utc();
    let t = (jd - 2_451_545.0) / 36_525.0;

    let l = (218.3164477 + 481_267.88123421 * t - 0.0015786 * t * t).to_radians();
    let d = (297.8501921 + 445_267.1114034 * t - 0.0018819 * t * t).to_radians();
    let m = (357.5291092 + 35_999.050_290_9 * t - 0.0001536 * t * t).to_radians();
    let mp = (134.9633964 + 477_198.8675055 * t + 0.0087414 * t * t).to_radians();
    let f = (93.2720950 + 483_202.0175233 * t - 0.0036539 * t * t).to_radians();

    let lambda = l
        + (6.289 * mp.sin()
            + 1.274 * (2.0 * d - mp).sin()
            + 0.658 * (2.0 * d).sin()
            + 0.214 * (2.0 * mp).sin()
            - 0.186 * m.sin()
            - 0.114 * (2.0 * f).sin())
        .to_radians();
    let beta = (5.128 * f.sin()
        + 0.280 * (mp + f).sin()
        + 0.277 * (mp - f).sin()
        + 0.173 * (2.0 * d - f).sin()
        + 0.055 * (2.0 * d - mp + f).sin()
        + 0.046 * (2.0 * d - mp - f).sin()
        + 0.033 * (2.0 * d + mp).sin()
        + 0.017 * (2.0 * mp - f).sin())
    .to_radians();

    let eps = 23.4392911_f64.to_radians();
    let sin_dec = beta.sin() * eps.cos() + beta.cos() * eps.sin() * lambda.sin();
    let dec = sin_dec.asin();
    let ra = (lambda.sin() * eps.cos() - beta.tan() * eps.sin()).atan2(lambda.cos());

    let gmst = (280.46061837 + 360.98564736629 * (jd - 2_451_545.0) + 0.000387933 * t * t)
        .rem_euclid(360.0);
    let lst = (gmst + lon_deg as f64).to_radians();
    let ha = lst - ra;
    let lat = (lat_deg as f64).to_radians();

    let alt = (lat.sin() * dec.sin() + lat.cos() * dec.cos() * ha.cos()).asin();
    let az = ha
        .sin()
        .atan2(ha.cos() * lat.sin() - dec.tan() * lat.cos())
        .to_degrees()
        + 180.0;

    // Synodic age -> illuminated fraction: 0 = new, 0.5 = full moon.
    let age = (jd - 2_451_550.1).rem_euclid(29.530_588_853);
    let phase = age / 29.530_588_853;
    let illumination = 0.5 * (1.0 - (2.0 * PI * phase).cos());

    (
        SkyPosition {
            altitude_deg: alt.to_degrees() as f32,
            azimuth_deg: az.rem_euclid(360.0) as f32,
        },
        illumination as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime {
        DateTime::new(Date::new(y, mo, d), h, mi)
    }

    #[test]
    fn summer_noon_is_high() {
        let p = sun_position(&dt(2026, 6, 21, 12, 0), NYC_LAT, NYC_LON);
        assert!(
            p.altitude_deg > 60.0 && p.altitude_deg < 80.0,
            "summer noon altitude {}",
            p.altitude_deg
        );
        assert!((p.azimuth_deg - 180.0).abs() < 25.0, "az {}", p.azimuth_deg);
    }

    #[test]
    fn winter_noon_is_lower() {
        let p = sun_position(&dt(2026, 12, 21, 12, 0), NYC_LAT, NYC_LON);
        assert!(
            p.altitude_deg > 18.0 && p.altitude_deg < 32.0,
            "winter noon altitude {}",
            p.altitude_deg
        );
    }

    #[test]
    fn midnight_sun_is_below_horizon() {
        let p = sun_position(&dt(2026, 6, 21, 0, 30), NYC_LAT, NYC_LON);
        assert!(
            p.altitude_deg < -10.0,
            "midnight altitude {}",
            p.altitude_deg
        );
    }

    #[test]
    fn night_sun_is_below_horizon() {
        let p = sun_position(&dt(2026, 11, 1, 22, 0), NYC_LAT, NYC_LON);
        assert!(p.altitude_deg < -5.0, "night altitude {}", p.altitude_deg);
    }

    #[test]
    fn moon_is_in_range() {
        let (p, illum) = moon_position(&dt(2026, 11, 1, 22, 0), NYC_LAT, NYC_LON);
        assert!(p.altitude_deg >= -90.0 && p.altitude_deg <= 90.0);
        assert!((0.0..=1.0).contains(&illum), "illum {illum}");
    }

    #[test]
    fn date_arithmetic_wraps() {
        assert_eq!(Date::new(2026, 1, 31).add_days(1), Date::new(2026, 2, 1));
        assert_eq!(Date::new(2026, 12, 31).add_days(1), Date::new(2027, 1, 1));
        assert_eq!(Date::new(2024, 2, 28).add_days(1), Date::new(2024, 2, 29));
        assert_eq!(Date::new(2025, 2, 28).add_days(1), Date::new(2025, 3, 1));
    }

    #[test]
    fn month_arithmetic_clamps_day() {
        assert_eq!(Date::new(2026, 1, 31).add_months(1), Date::new(2026, 2, 28));
        assert_eq!(
            Date::new(2026, 12, 15).add_months(1),
            Date::new(2027, 1, 15)
        );
        assert_eq!(
            Date::new(2026, 1, 15).add_months(-1),
            Date::new(2025, 12, 15)
        );
    }

    #[test]
    fn julian_roundtrip() {
        for d in [
            Date::new(2000, 1, 1),
            Date::new(2026, 11, 1),
            Date::new(2024, 2, 29),
        ] {
            assert_eq!(Date::from_julian_day(d.julian_day()), d);
        }
    }
}
