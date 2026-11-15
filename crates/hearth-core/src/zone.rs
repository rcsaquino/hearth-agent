//! The configured time zone and the local renderings derived from it.

use jiff::Timestamp;
use jiff::civil;
use jiff::tz::{Offset, TimeZone};

/// An IANA time zone, falling back to UTC when the name is unknown.
#[derive(Debug, Clone)]
pub struct Zone {
    name: String,
    zone: TimeZone,
}

/// A wall-clock reading with the offset in force at that instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub year: i16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub offset_seconds: i32,
}

impl Zone {
    /// Resolve an IANA name; an empty or unknown name is UTC.
    pub fn resolve(name: &str) -> Self {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Self::utc();
        }
        match TimeZone::get(trimmed) {
            Ok(zone) => Self {
                name: trimmed.to_owned(),
                zone,
            },
            Err(_) => Self::utc(),
        }
    }

    /// The UTC zone.
    pub fn utc() -> Self {
        Self {
            name: "UTC".to_owned(),
            zone: TimeZone::UTC,
        }
    }

    /// A fixed offset, named after that offset. This is a test seam.
    pub fn fixed(offset_seconds: i32) -> Self {
        let offset = Offset::from_seconds(offset_seconds).unwrap_or(Offset::UTC);
        Self {
            name: offset_name(offset.seconds()),
            zone: TimeZone::fixed(offset),
        }
    }

    /// The configured name, or `UTC` after a fallback.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The local reading of an instant.
    pub fn local(&self, micros: i64) -> Local {
        let timestamp = Timestamp::from_microsecond(micros).unwrap_or(Timestamp::UNIX_EPOCH);
        let zoned = timestamp.to_zoned(self.zone.clone());
        let datetime = zoned.datetime();
        Local {
            year: datetime.year(),
            month: datetime.month() as u8,
            day: datetime.day() as u8,
            hour: datetime.hour() as u8,
            minute: datetime.minute() as u8,
            second: datetime.second() as u8,
            offset_seconds: zoned.offset().seconds(),
        }
    }

    /// The ISO 8601 rendering of an instant in this zone.
    pub fn iso(&self, micros: i64) -> String {
        self.local(micros).iso()
    }

    /// The next instant at this zone's local wall-clock time, strictly after now.
    pub fn next_at(&self, now_micros: i64, hour: u8, minute: u8) -> i64 {
        let Ok(time) = civil::Time::new(hour as i8, minute as i8, 0, 0) else {
            return now_micros;
        };
        let now = self.local(now_micros);
        let mut date = civil::date(now.year, now.month as i8, now.day as i8);
        for _ in 0..2 {
            if let Ok(zoned) = self.zone.to_zoned(date.to_datetime(time)) {
                let candidate = zoned.timestamp().as_microsecond();
                if candidate > now_micros {
                    return candidate;
                }
            }
            let Ok(tomorrow) = date.tomorrow() else {
                break;
            };
            date = tomorrow;
        }
        now_micros
    }
}

impl Local {
    /// `YYYY-MM-DDThh:mm:ss+HH:MM`.
    pub fn iso(&self) -> String {
        let sign = if self.offset_seconds < 0 { '-' } else { '+' };
        let offset = self.offset_seconds.unsigned_abs();
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            offset / 3600,
            (offset % 3600) / 60
        )
    }

    /// `hh:mm:ss`.
    pub fn clock(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }

    /// `YYYY-MM-DD`.
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

fn offset_name(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let absolute = seconds.unsigned_abs();
    format!("{sign}{:02}:{:02}", absolute / 3600, (absolute % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_formatter_pads_every_part() {
        let local = Zone::fixed(-5 * 3600).local(0);
        assert_eq!(local.iso(), "1969-12-31T19:00:00-05:00");
        assert_eq!(local.clock(), "19:00:00");
        assert_eq!(local.date(), "1969-12-31");
        assert_eq!(Zone::utc().iso(0), "1970-01-01T00:00:00+00:00");
    }

    #[test]
    fn next_at_rolls_over_midnight() {
        let zone = Zone::fixed(8 * 3600);
        let before = 1_767_204_000_000_000;
        assert_eq!(zone.next_at(before, 3, 30), 1_767_209_400_000_000);
        let after = 1_767_211_200_000_000;
        assert_eq!(zone.next_at(after, 3, 30), 1_767_295_800_000_000);
    }
}
