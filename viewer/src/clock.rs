//! Adjustable calendar + clock state.

use crate::astro::{Date, DateTime};
use bevy::prelude::Resource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockAction {
    Day(i32),
    Hour(i32),
    Minute(i32),
    Month(i32),
    TogglePlay,
    Reverse,
    Reset,
}

#[derive(Resource, Clone, Debug)]
pub struct SimClock {
    pub date: Date,
    /// Local standard time, in minutes since midnight. Kept fractional so the
    /// sun moves smoothly when time is accelerated.
    pub minute_of_day: f64,
    pub playing: bool,
    /// In-game minutes per real second.
    pub speed: f64,
}

pub const START_DATE: Date = Date::new(2026, 6, 21);
pub const START_MINUTE: f64 = 12.0 * 60.0;

impl Default for SimClock {
    fn default() -> Self {
        Self {
            date: START_DATE,
            minute_of_day: START_MINUTE,
            playing: false,
            speed: 6.0,
        }
    }
}

impl SimClock {
    pub fn datetime(&self) -> DateTime {
        DateTime {
            date: self.date,
            minute_of_day: self.minute_of_day.rem_euclid(1440.0),
        }
    }

    pub fn set_time(&mut self, hour: u32, minute: u32) {
        self.minute_of_day = (hour.min(23) * 60 + minute.min(59)) as f64;
    }

    pub fn set_date(&mut self, year: i32, month: u32, day: u32) {
        self.date = Date::new(year, month.clamp(1, 12), day.clamp(1, 31));
    }

    pub fn tick(&mut self, delta_secs: f64) {
        if !self.playing {
            return;
        }
        self.advance_minutes(delta_secs * self.speed);
    }

    pub fn advance_minutes(&mut self, minutes: f64) {
        let total = self.minute_of_day + minutes;
        let day_delta = (total / 1440.0).floor() as i64;
        self.minute_of_day = total.rem_euclid(1440.0);
        if day_delta != 0 {
            self.date = self.date.add_days(day_delta as i32);
        }
    }

    pub fn apply(&mut self, action: ClockAction) {
        match action {
            ClockAction::Day(d) => self.date = self.date.add_days(d),
            ClockAction::Month(d) => self.date = self.date.add_months(d),
            ClockAction::Hour(h) => self.advance_minutes(h as f64 * 60.0),
            ClockAction::Minute(m) => self.advance_minutes(m as f64),
            ClockAction::TogglePlay => self.playing = !self.playing,
            ClockAction::Reverse => {
                self.speed = -self.speed;
                if self.speed.abs() < 1.0 {
                    self.speed = 6.0;
                }
            }
            ClockAction::Reset => {
                self.date = START_DATE;
                self.minute_of_day = START_MINUTE;
                self.playing = false;
                self.speed = 6.0;
            }
        }
    }

    pub fn time_label(&self) -> String {
        let dt = self.datetime();
        format!("{:02}:{:02}", dt.hour(), dt.minute())
    }

    pub fn date_label(&self) -> String {
        let dt = self.datetime();
        format!(
            "{:04}-{:02}-{:02} {}",
            dt.date.year,
            dt.date.month,
            dt.date.day,
            dt.date.weekday()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_adjustment_wraps_day() {
        let mut c = SimClock::default();
        c.set_time(23, 30);
        c.apply(ClockAction::Hour(1));
        assert_eq!(c.date, Date::new(2026, 6, 22));
        assert_eq!(c.datetime().hour(), 0);
        assert_eq!(c.datetime().minute(), 30);
    }

    #[test]
    fn minute_adjustment_wraps_backwards() {
        let mut c = SimClock::default();
        c.set_time(0, 5);
        c.apply(ClockAction::Minute(-10));
        assert_eq!(c.date, Date::new(2026, 6, 20));
        assert_eq!(c.datetime().hour(), 23);
        assert_eq!(c.datetime().minute(), 55);
    }

    #[test]
    fn playing_advances_time() {
        let mut c = SimClock {
            playing: true,
            speed: 60.0,
            ..Default::default()
        };
        c.tick(2.0);
        assert_eq!(c.datetime().hour(), 14);
        assert_eq!(c.datetime().minute(), 0);
    }

    #[test]
    fn reverse_toggles_the_clock_direction() {
        let mut c = SimClock::default();
        c.apply(ClockAction::Reverse);
        assert!(c.speed < 0.0);
        c.apply(ClockAction::Reverse);
        assert!(c.speed > 0.0);
    }

    #[test]
    fn negative_tick_runs_time_backwards() {
        let mut c = SimClock {
            playing: true,
            speed: -60.0,
            ..Default::default()
        };
        c.set_time(12, 0);
        c.tick(2.0);
        assert_eq!(c.datetime().hour(), 10);
    }

    #[test]
    fn day_action_and_reset() {
        let mut c = SimClock::default();
        c.apply(ClockAction::Day(1));
        assert_eq!(c.date, Date::new(2026, 6, 22));
        c.apply(ClockAction::Reset);
        assert_eq!(c.date, START_DATE);
        assert_eq!(c.minute_of_day, START_MINUTE);
    }
}
