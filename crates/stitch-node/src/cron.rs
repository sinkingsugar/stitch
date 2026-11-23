//! When a recurring schedule fires (design §2.3): a five-field cron
//! expression (`minute hour day-of-month month day-of-week`), read in a
//! time zone, or a plain interval (`@every 10m`).
//!
//! Each field takes `*`, a number, a range `a-b`, a step `*/n` or `a-b/n`,
//! and lists of these (`1,15,30-40/5`); months and days take their English
//! names' first three letters (`jan`, `mon`), and Sunday is 0 or 7. As in
//! cron, when both the day of the month and the day of the week are
//! restricted, a day matching either fires. `@hourly`, `@daily`
//! (`@midnight`), `@weekly`, `@monthly` and `@yearly` (`@annually`) stand
//! for their usual expressions.
//!
//! Times are civil times in the zone: a time skipped by a daylight-saving
//! change fires at the first instant after the gap, and a time that occurs
//! twice fires once, at its first occurrence.

use std::fmt;
use std::str::FromStr;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

/// When a recurring schedule fires.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Every {
  Cron(Cron),
  /// A fixed interval from the last firing.
  Interval(SignedDuration),
}

impl Every {
  /// The first firing time strictly after `after`, read in `tz`.
  pub fn next_after(&self, after: Timestamp, tz: &TimeZone) -> Option<Timestamp> {
    match self {
      Every::Cron(c) => c.next_after(after, tz),
      Every::Interval(d) => after.checked_add(*d).ok(),
    }
  }
}

impl FromStr for Every {
  type Err = String;

  fn from_str(s: &str) -> Result<Self, String> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("@every") {
      let d = interval(rest.trim())?;
      return Ok(Every::Interval(d));
    }
    s.parse().map(Every::Cron)
  }
}

/// `90s`, `10m`, `2h`, `1d`, or a bare number of minutes; at least a
/// minute.
pub fn interval(s: &str) -> Result<SignedDuration, String> {
  let s = s.trim();
  let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
  let (n, unit) = s.split_at(split);
  let n: i64 = n
    .parse()
    .map_err(|_| format!("`{s}`: an interval is a number and a unit: 90s, 10m, 2h, 1d"))?;
  let secs = match unit.trim() {
    "s" | "sec" | "secs" => n,
    "" | "m" | "min" | "mins" => n * 60,
    "h" | "hour" | "hours" => n * 3600,
    "d" | "day" | "days" => n * 86400,
    u => return Err(format!("`{u}`: the units are s, m, h and d")),
  };
  if secs < 60 {
    return Err(format!("`{s}`: an interval is a minute at least"));
  }
  Ok(SignedDuration::from_secs(secs))
}

/// A parsed five-field cron expression.
#[derive(Clone, PartialEq, Eq)]
pub struct Cron {
  minutes: u64,
  hours: u32,
  /// Bits 1 to 31.
  days: u32,
  /// Bits 1 to 12.
  months: u16,
  /// Bits 0 (Sunday) to 6.
  weekdays: u8,
  days_any: bool,
  weekdays_any: bool,
  text: String,
}

impl fmt::Debug for Cron {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Cron({})", self.text)
  }
}

impl fmt::Display for Cron {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&self.text)
  }
}

const MONTHS: [&str; 12] = [
  "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// One field's values as bits, and whether it was `*` (unrestricted).
fn field(
  text: &str,
  what: &str,
  min: u32,
  max: u32,
  names: &[&str],
  name_base: u32,
) -> Result<(u64, bool), String> {
  let value = |s: &str| -> Result<u32, String> {
    let lower = s.to_ascii_lowercase();
    if let Some(i) = names.iter().position(|n| *n == lower) {
      return Ok(i as u32 + name_base);
    }
    let n: u32 = s
      .parse()
      .map_err(|_| format!("`{s}` is not a {what} (in `{text}`)"))?;
    if n < min || n > max {
      return Err(format!(
        "{what} {n} is out of range: {min} to {max} (in `{text}`)"
      ));
    }
    Ok(n)
  };
  let mut bits = 0u64;
  let mut any = false;
  for part in text.split(',') {
    if part.is_empty() {
      return Err(format!("an empty item in the {what} field `{text}`"));
    }
    let (range, step) = match part.split_once('/') {
      Some((r, s)) => {
        let s: u32 = s
          .parse()
          .ok()
          .filter(|s| *s > 0)
          .ok_or_else(|| format!("`{s}` is not a step (in `{text}`)"))?;
        (r, s)
      }
      None => (part, 1),
    };
    let (lo, hi) = if range == "*" {
      if step == 1 {
        any = true;
      }
      (min, max)
    } else if let Some((a, b)) = range.split_once('-') {
      let (a, b) = (value(a)?, value(b)?);
      if a > b {
        return Err(format!("`{range}` runs backwards (in `{text}`)"));
      }
      (a, b)
    } else {
      let a = value(range)?;
      // `5/15`: from 5 to the end, as cron reads it.
      (a, if step > 1 { max } else { a })
    };
    let mut v = lo;
    while v <= hi {
      bits |= 1 << v;
      v += step;
    }
  }
  Ok((bits, any))
}

impl FromStr for Cron {
  type Err = String;

  fn from_str(s: &str) -> Result<Self, String> {
    let text = s.trim();
    let expanded = match text.to_ascii_lowercase().as_str() {
      "@hourly" => "0 * * * *",
      "@daily" | "@midnight" => "0 0 * * *",
      "@weekly" => "0 0 * * 0",
      "@monthly" => "0 0 1 * *",
      "@yearly" | "@annually" => "0 0 1 1 *",
      _ if text.starts_with('@') => {
        return Err(format!(
          "`{text}`: the shorthands are @hourly, @daily, @weekly, @monthly, @yearly and @every <interval>"
        ));
      }
      _ => text,
    };
    let parts: Vec<&str> = expanded.split_whitespace().collect();
    let [mi, h, dom, mo, dow] = parts.as_slice() else {
      return Err(format!(
        "`{text}`: a cron expression has five fields: minute hour day-of-month month day-of-week"
      ));
    };
    let (minutes, _) = field(mi, "minute", 0, 59, &[], 0)?;
    let (hours, _) = field(h, "hour", 0, 23, &[], 0)?;
    let (days, days_any) = field(dom, "day of the month", 1, 31, &[], 0)?;
    let (months, _) = field(mo, "month", 1, 12, &MONTHS, 1)?;
    let (mut weekdays, weekdays_any) = field(dow, "day of the week", 0, 7, &WEEKDAYS, 0)?;
    // 7 is Sunday too.
    if weekdays & (1 << 7) != 0 {
      weekdays = (weekdays | 1) & 0x7f;
    }
    Ok(Cron {
      minutes,
      hours: hours as u32,
      days: days as u32,
      months: months as u16,
      weekdays: weekdays as u8,
      days_any,
      weekdays_any,
      text: text.to_owned(),
    })
  }
}

impl Cron {
  fn day_matches(&self, d: Date) -> bool {
    if self.months & (1 << d.month()) == 0 {
      return false;
    }
    let dom = self.days & (1 << d.day()) != 0;
    let dow = self.weekdays & (1 << d.weekday().to_sunday_zero_offset()) != 0;
    match (self.days_any, self.weekdays_any) {
      (true, true) => true,
      (false, true) => dom,
      (true, false) => dow,
      // Both restricted: either, as cron does.
      (false, false) => dom || dow,
    }
  }

  /// The first time strictly after `after` that the expression names,
  /// read in `tz`; `None` when there is none within eight years (`0 0 30
  /// 2 *`, the 30th of February).
  pub fn next_after(&self, after: Timestamp, tz: &TimeZone) -> Option<Timestamp> {
    let start = after.to_zoned(tz.clone()).datetime();
    let mut day = start.date();
    for _ in 0..(366 * 8) {
      if self.day_matches(day) {
        for h in 0..24i8 {
          if self.hours & (1 << h) == 0 {
            continue;
          }
          if day == start.date() && h < start.hour() {
            continue;
          }
          for m in 0..60i8 {
            if self.minutes & (1 << m) == 0 {
              continue;
            }
            let Ok(z) = day.at(h, m, 0, 0).to_zoned(tz.clone()) else {
              continue;
            };
            let t = z.timestamp();
            if t > after {
              return Some(t);
            }
          }
        }
      }
      day = day.tomorrow().ok()?;
    }
    None
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn at(s: &str, tz: &TimeZone) -> Timestamp {
    s.parse::<jiff::civil::DateTime>()
      .unwrap()
      .to_zoned(tz.clone())
      .unwrap()
      .timestamp()
  }

  fn next(expr: &str, after: &str, tz: &str) -> String {
    let tz = TimeZone::get(tz).unwrap();
    let c: Cron = expr.parse().unwrap();
    let t = c.next_after(at(after, &tz), &tz).unwrap();
    t.to_zoned(tz).datetime().to_string()
  }

  #[test]
  fn next_times() {
    let utc = "UTC";
    assert_eq!(
      next("*/15 * * * *", "2026-10-19T09:07", utc),
      "2026-10-19T09:15:00"
    );
    assert_eq!(
      next("0 9 * * *", "2026-10-19T09:00", utc),
      "2026-10-20T09:00:00"
    );
    assert_eq!(
      next("0 9 * * *", "2026-10-19T08:59:30", utc),
      "2026-10-19T09:00:00"
    );
    // Weekdays at 9: from a Friday evening to Monday.
    assert_eq!(
      next("0 9 * * 1-5", "2026-10-23T18:00", utc),
      "2026-10-26T09:00:00"
    );
    assert_eq!(
      next("0 9 * * mon-fri", "2026-10-23T18:00", utc),
      "2026-10-26T09:00:00"
    );
    // Sunday as 7.
    assert_eq!(
      next("30 6 * * 7", "2026-10-19T00:00", utc),
      "2026-10-25T06:30:00"
    );
    // The first of the month, across the year.
    assert_eq!(
      next("@monthly", "2026-12-15T00:00", utc),
      "2027-01-01T00:00:00"
    );
    // Day of the month or of the week when both are restricted.
    assert_eq!(
      next("0 0 13 * 5", "2026-10-19T00:00", utc),
      "2026-10-23T00:00:00"
    );
    assert_eq!(
      next("0 12 29 2 *", "2026-03-01T00:00", utc),
      "2028-02-29T12:00:00"
    );
    assert_eq!(
      next("5/20 * * * *", "2026-10-19T10:30", utc),
      "2026-10-19T10:45:00"
    );
  }

  #[test]
  fn time_zones_and_daylight_saving() {
    // 09:00 in Rome is 07:00 UTC in summer.
    let rome = TimeZone::get("Europe/Rome").unwrap();
    let c: Cron = "0 9 * * *".parse().unwrap();
    let t = c
      .next_after("2026-07-01T00:00:00Z".parse().unwrap(), &rome)
      .unwrap();
    assert_eq!(t.to_string(), "2026-07-01T07:00:00Z");
    // 02:30 does not exist on 2026-03-29 in Rome: fired after the gap.
    assert_eq!(
      next("30 2 * * *", "2026-03-29T00:00", "Europe/Rome"),
      "2026-03-29T03:30:00"
    );
    // 02:30 happens twice on 2026-10-25: once.
    let first = c_next("30 2 * * *", "2026-10-25T00:00:00Z", &rome);
    let second = c_next("30 2 * * *", &first.to_string(), &rome);
    assert_eq!(
      second.to_zoned(rome.clone()).datetime().to_string(),
      "2026-10-26T02:30:00"
    );
  }

  fn c_next(expr: &str, after: &str, tz: &TimeZone) -> Timestamp {
    expr
      .parse::<Cron>()
      .unwrap()
      .next_after(after.parse().unwrap(), tz)
      .unwrap()
  }

  #[test]
  fn bad_expressions_say_why() {
    for (e, says) in [
      ("* * * *", "five fields"),
      ("60 * * * *", "out of range"),
      ("* 24 * * *", "out of range"),
      ("* * 0 * *", "out of range"),
      ("*/0 * * * *", "not a step"),
      ("10-5 * * * *", "backwards"),
      ("* * * foo *", "not a month"),
      ("@sometimes", "shorthands"),
    ] {
      let err = e.parse::<Cron>().unwrap_err();
      assert!(err.contains(says), "{e}: {err}");
    }
    assert!(
      "0 0 30 2 *"
        .parse::<Cron>()
        .unwrap()
        .next_after(Timestamp::now(), &TimeZone::UTC)
        .is_none()
    );
  }

  #[test]
  fn intervals() {
    let e: Every = "@every 10m".parse().unwrap();
    let t: Timestamp = "2026-10-19T09:00:00Z".parse().unwrap();
    assert_eq!(
      e.next_after(t, &TimeZone::UTC).unwrap().to_string(),
      "2026-10-19T09:10:00Z"
    );
    assert_eq!(interval("2h").unwrap(), SignedDuration::from_hours(2));
    assert_eq!(interval("5").unwrap(), SignedDuration::from_mins(5));
    assert!(interval("30s").is_err());
    assert!(interval("3w").is_err());
  }
}
