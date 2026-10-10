//! Five-field cron (minute hour day-of-month month day-of-week), in local time.
//!
//! Supports `*`, `*/n`, ranges `a-b`, stepped ranges `a-b/n`, lists, month and
//! weekday names, and `7` as Sunday. When both day fields are restricted, a
//! day matches if *either* does — classic cron, which surprises people, so the
//! routines screen spells out the next few firings.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minutes: u64,
    hours: u64,
    dom: u64,
    months: u64,
    dow: u64,
    dom_any: bool,
    dow_any: bool,
    source: String,
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

fn name_value(s: &str, names: &[&str], offset: u32) -> Option<u32> {
    let l = s.to_ascii_lowercase();
    names.iter().position(|n| *n == l).map(|i| i as u32 + offset)
}

fn parse_field(field: &str, min: u32, max: u32, names: &[&str], name_offset: u32) -> Result<(u64, bool), String> {
    let mut bits = 0u64;
    let any = field == "*" || field == "?";
    for part in field.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().map_err(|_| format!("bad step in \"{part}\""))?),
            None => (part, 1),
        };
        if step == 0 {
            return Err(format!("step of 0 in \"{part}\""));
        }
        let value = |s: &str| -> Result<u32, String> {
            s.parse::<u32>().ok().or_else(|| name_value(s, names, name_offset)).ok_or_else(|| format!("\"{s}\" is not a valid value"))
        };
        let (lo, hi) = if range == "*" || range == "?" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (value(a)?, value(b)?)
        } else {
            let v = value(range)?;
            (v, if part.contains('/') { max } else { v })
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!("\"{part}\" is outside {min}-{max}"));
        }
        let mut v = lo;
        while v <= hi {
            bits |= 1 << v;
            v += step;
        }
    }
    Ok((bits, any))
}

impl Cron {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!("A schedule has 5 fields (minute hour day month weekday); \"{expr}\" has {}.", fields.len()));
        }
        let (minutes, _) = parse_field(fields[0], 0, 59, &[], 0)?;
        let (hours, _) = parse_field(fields[1], 0, 23, &[], 0)?;
        let (dom, dom_any) = parse_field(fields[2], 1, 31, &[], 0)?;
        let (months, _) = parse_field(fields[3], 1, 12, &MONTHS, 1)?;
        let (mut dow, dow_any) = parse_field(fields[4], 0, 7, &DAYS, 0)?;
        if dow & (1 << 7) != 0 {
            dow |= 1; // 7 is Sunday too
        }
        Ok(Self { minutes, hours, dom, months, dow, dom_any, dow_any, source: expr.to_string() })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    fn day_matches(&self, d: NaiveDate) -> bool {
        if self.months & (1 << d.month()) == 0 {
            return false;
        }
        let dom = self.dom & (1 << d.day()) != 0;
        let dow = self.dow & (1 << d.weekday().num_days_from_sunday()) != 0;
        match (self.dom_any, self.dow_any) {
            (true, true) => true,
            (true, false) => dow,
            (false, true) => dom,
            (false, false) => dom || dow,
        }
    }

    /// The first firing strictly after `after`.
    pub fn next_after(&self, after: DateTime<Local>) -> Option<DateTime<Local>> {
        let start = after + Duration::minutes(1);
        let mut date = start.date_naive();
        for day in 0..(366 * 5) {
            if self.day_matches(date) {
                for h in 0..24u32 {
                    if self.hours & (1 << h) == 0 {
                        continue;
                    }
                    for m in 0..60u32 {
                        if self.minutes & (1 << m) == 0 {
                            continue;
                        }
                        if day == 0 && (h, m) < (start.hour(), start.minute()) {
                            continue;
                        }
                        // A local time skipped by a DST jump does not exist;
                        // an ambiguous one fires at its first occurrence.
                        if let Some(t) = Local.from_local_datetime(&date.and_hms_opt(h, m, 0)?).earliest() {
                            return Some(t);
                        }
                    }
                }
            }
            date = date.succ_opt()?;
        }
        None
    }

    pub fn upcoming(&self, from: DateTime<Local>, n: usize) -> Vec<DateTime<Local>> {
        let mut out = Vec::new();
        let mut t = from;
        while out.len() < n {
            match self.next_after(t) {
                Some(next) => {
                    out.push(next);
                    t = next;
                }
                None => break,
            }
        }
        out
    }

    /// Has a firing come due in (last, now]?
    pub fn due_between(&self, last: DateTime<Local>, now: DateTime<Local>) -> bool {
        self.next_after(last).is_some_and(|t| t <= now)
    }

    /// "every weekday at 09:00"-style description for the common shapes.
    pub fn describe(&self) -> String {
        let f: Vec<&str> = self.source.split_whitespace().collect();
        let time = match (f[0].parse::<u32>(), f[1].parse::<u32>()) {
            (Ok(m), Ok(h)) => Some(format!("{h:02}:{m:02}")),
            _ => None,
        };
        match (time, f[2], f[3], f[4]) {
            (Some(t), "*", "*", "*") => format!("every day at {t}"),
            (Some(t), "*", "*", "1-5") => format!("every weekday at {t}"),
            (Some(t), "*", "*", d) if d.parse::<u32>().is_ok() => {
                let i = d.parse::<usize>().unwrap() % 7;
                format!("every {} at {t}", ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"][i])
            }
            (Some(t), d, "*", "*") if d.parse::<u32>().is_ok() => format!("on day {d} of every month at {t}"),
            _ if f[0].starts_with("*/") && f[1..] == ["*", "*", "*", "*"] => format!("every {} minutes", &f[0][2..]),
            (None, "*", "*", "*") if f[0] == "0" && f[1].starts_with("*/") => format!("every {} hours", &f[1][2..]),
            _ => format!("cron {}", self.source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.from_local_datetime(&NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()).unwrap()
    }

    #[test]
    fn parses_and_fires() {
        let c = Cron::parse("30 9 * * mon-fri").unwrap();
        // Saturday 2026-10-03 → Monday 2026-10-05 09:30
        assert_eq!(c.next_after(at(2026, 10, 3, 12, 0)).unwrap(), at(2026, 10, 5, 9, 30));
        assert_eq!(c.describe(), "cron 30 9 * * mon-fri");
        let c = Cron::parse("*/15 * * * *").unwrap();
        assert_eq!(c.next_after(at(2026, 1, 1, 10, 7)).unwrap(), at(2026, 1, 1, 10, 15));
        assert_eq!(c.describe(), "every 15 minutes");
        let c = Cron::parse("0 9 1 * *").unwrap();
        assert_eq!(c.next_after(at(2026, 1, 15, 0, 0)).unwrap(), at(2026, 2, 1, 9, 0));
        let c = Cron::parse("0 8 * * 7").unwrap(); // 7 = Sunday
        assert_eq!(c.next_after(at(2026, 10, 5, 0, 0)).unwrap(), at(2026, 10, 11, 8, 0));
        assert_eq!(Cron::parse("0 9 * * 1-5").unwrap().describe(), "every weekday at 09:00");
    }

    #[test]
    fn dom_or_dow() {
        // 13th of the month OR any Friday.
        let c = Cron::parse("0 0 13 * 5").unwrap();
        assert_eq!(c.next_after(at(2026, 10, 1, 0, 0)).unwrap(), at(2026, 10, 2, 0, 0)); // Fri 2 Oct
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "* * * *", "60 * * * *", "* 24 * * *", "*/0 * * * *", "0 0 32 * *", "x * * * *"] {
            assert!(Cron::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn due_window() {
        let c = Cron::parse("0 9 * * *").unwrap();
        assert!(c.due_between(at(2026, 1, 1, 8, 0), at(2026, 1, 1, 9, 0)));
        assert!(!c.due_between(at(2026, 1, 1, 9, 0), at(2026, 1, 1, 9, 30)));
    }
}
