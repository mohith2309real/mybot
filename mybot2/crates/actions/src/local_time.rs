//! Dates and times.

use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday};
use serde_json::json;

use crate::{Action, local, n, n_or, s, s_or};

const C: &str = "Dates & Times";
const D: (&str, &str, &str, bool) = ("date", "string", "A date: 2026-10-05, 10/05/2026, or 'today'", true);

pub fn parse_date(t: &str) -> Result<NaiveDate, String> {
    let t = t.trim();
    match t.to_lowercase().as_str() {
        "today" | "now" => return Ok(Local::now().date_naive()),
        "tomorrow" => return Ok(Local::now().date_naive() + Duration::days(1)),
        "yesterday" => return Ok(Local::now().date_naive() - Duration::days(1)),
        _ => {}
    }
    for f in ["%Y-%m-%d", "%Y/%m/%d", "%m/%d/%Y", "%d.%m.%Y", "%B %d, %Y", "%b %d, %Y", "%d %B %Y", "%d %b %Y", "%Y%m%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(t, f) {
            return Ok(d);
        }
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(t) {
        return Ok(dt.date_naive());
    }
    Err(format!("could not read \"{t}\" as a date (try 2026-10-05)"))
}

fn parse_dt(t: &str) -> Result<NaiveDateTime, String> {
    let t = t.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(t) {
        return Ok(dt.naive_utc());
    }
    for f in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(d) = NaiveDateTime::parse_from_str(t, f) {
            return Ok(d);
        }
    }
    parse_date(t).map(|d| d.and_hms_opt(0, 0, 0).unwrap())
}

fn offset(t: &str) -> Result<FixedOffset, String> {
    let t = t.trim().to_uppercase().replace("UTC", "").replace("GMT", "");
    if t.is_empty() {
        return Ok(FixedOffset::east_opt(0).unwrap());
    }
    let sign = if t.starts_with('-') { -1 } else { 1 };
    let body = t.trim_start_matches(['+', '-']);
    let (h, m) = body.split_once(':').map(|(a, b)| (a, b)).unwrap_or((body, "0"));
    let secs = sign * (h.parse::<i32>().map_err(|_| format!("bad offset {t}"))? * 3600 + m.parse::<i32>().unwrap_or(0) * 60);
    FixedOffset::east_opt(secs).ok_or_else(|| format!("bad offset {t}"))
}

fn add_business_days(mut d: NaiveDate, mut n: i64) -> NaiveDate {
    let step = if n >= 0 { 1 } else { -1 };
    while n != 0 {
        d += Duration::days(step);
        if !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
            n -= step;
        }
    }
    d
}

pub fn defs() -> Vec<Action> {
    vec![
        local("date_now", C, "Current date and time (local and UTC)", &[],
            |_| { let l = Local::now(); Ok(format!("local: {}\nutc: {}\nunix: {}", l.format("%A %Y-%m-%d %H:%M:%S %:z"), Utc::now().format("%Y-%m-%d %H:%M:%S"), l.timestamp())) },
            (json!({}), "unix:")),
        local("date_add", C, "Add days, weeks, months or years to a date", &[D, ("amount", "integer", "How many (negative to subtract)", true), ("unit", "string", "days, weeks, months or years", true)],
            |v| {
                let d = parse_date(s(v, "date")?)?; let a = n(v, "amount")? as i64;
                let out = match s(v, "unit")?.trim_end_matches('s') {
                    "day" => d + Duration::days(a), "week" => d + Duration::weeks(a),
                    "month" => d.checked_add_months(chrono::Months::new(a.unsigned_abs() as u32)).filter(|_| a >= 0).or_else(|| d.checked_sub_months(chrono::Months::new(a.unsigned_abs() as u32))).ok_or("out of range")?,
                    "year" => d.with_year(d.year() + a as i32).or_else(|| NaiveDate::from_ymd_opt(d.year() + a as i32, d.month(), 28)).ok_or("out of range")?,
                    other => return Err(format!("unknown unit {other}")),
                };
                Ok(out.format("%Y-%m-%d (%A)").to_string())
            },
            (json!({"date": "2026-01-31", "amount": 1, "unit": "months"}), "2026-02-28")),
        local("date_diff", C, "Days (and weeks, months) between two dates", &[("from", "string", "Start date", true), ("to", "string", "End date", true)],
            |v| { let (a, b) = (parse_date(s(v, "from")?)?, parse_date(s(v, "to")?)?); let d = (b - a).num_days(); Ok(format!("{d} days\n{:.1} weeks\n~{:.1} months", d as f64 / 7.0, d as f64 / 30.4375)) },
            (json!({"from": "2026-01-01", "to": "2026-03-01"}), "59 days")),
        local("day_of_week", C, "Which weekday a date falls on", &[D], |v| Ok(parse_date(s(v, "date")?)?.format("%A").to_string()), (json!({"date": "2026-10-05"}), "Monday")),
        local("date_format", C, "Reformat a date (strftime pattern like %d %B %Y)", &[D, ("pattern", "string", "e.g. %d %B %Y, %m/%d/%y, %A", true)],
            |v| { let d = parse_date(s(v, "date")?)?; let p = s(v, "pattern")?; let mut out = String::new(); use std::fmt::Write; write!(out, "{}", d.format(p)).map_err(|_| "bad pattern".to_string())?; Ok(out) },
            (json!({"date": "2026-10-05", "pattern": "%d %B %Y"}), "05 October 2026")),
        local("unix_to_date", C, "Convert a Unix timestamp (seconds or ms) to a date", &[("timestamp", "number", "Unix time", true)],
            |v| { let t = n(v, "timestamp")? as i64; let secs = if t.abs() > 100_000_000_000 { t / 1000 } else { t }; let d = DateTime::from_timestamp(secs, 0).ok_or("out of range")?; Ok(format!("utc: {}\nlocal: {}", d.format("%Y-%m-%d %H:%M:%S"), d.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S %:z"))) },
            (json!({"timestamp": 1700000000}), "utc: 2023-11-14 22:13:20")),
        local("date_to_unix", C, "Convert a date/time (UTC) to a Unix timestamp", &[("datetime", "string", "e.g. 2026-10-05 14:30", true)],
            |v| Ok(Utc.from_utc_datetime(&parse_dt(s(v, "datetime")?)?).timestamp().to_string()),
            (json!({"datetime": "2023-11-14 22:13:20"}), "1700000000")),
        local("timezone_convert", C, "Convert a time between UTC offsets (e.g. +05:30 to -04:00)", &[("datetime", "string", "e.g. 2026-10-05 09:00", true), ("from_offset", "string", "e.g. +05:30 or UTC", true), ("to_offset", "string", "e.g. -04:00", true)],
            |v| {
                let local_dt = parse_dt(s(v, "datetime")?)?; let (f, t) = (offset(s(v, "from_offset")?)?, offset(s(v, "to_offset")?)?);
                let at = f.from_local_datetime(&local_dt).single().ok_or("ambiguous time")?;
                Ok(at.with_timezone(&t).format("%Y-%m-%d %H:%M (%:z)").to_string())
            },
            (json!({"datetime": "2026-10-05 09:00", "from_offset": "+05:30", "to_offset": "UTC"}), "2026-10-05 03:30 (+00:00)")),
        local("business_days_between", C, "Working days (Mon–Fri) between two dates, excluding the start", &[("from", "string", "Start date", true), ("to", "string", "End date", true)],
            |v| { let (mut a, b) = (parse_date(s(v, "from")?)?, parse_date(s(v, "to")?)?); let mut c = 0; while a < b { a += Duration::days(1); if !matches!(a.weekday(), Weekday::Sat | Weekday::Sun) { c += 1; } } Ok(format!("{c} business days")) },
            (json!({"from": "2026-10-02", "to": "2026-10-09"}), "5 business days")),
        local("add_business_days", C, "Add working days (Mon–Fri) to a date", &[D, ("days", "integer", "Working days to add", true)],
            |v| Ok(add_business_days(parse_date(s(v, "date")?)?, n(v, "days")? as i64).format("%Y-%m-%d (%A)").to_string()),
            (json!({"date": "2026-10-09", "days": 1}), "2026-10-12 (Monday)")),
        local("week_number", C, "ISO week number and year of a date", &[D],
            |v| { let w = parse_date(s(v, "date")?)?.iso_week(); Ok(format!("week {} of {}", w.week(), w.year())) },
            (json!({"date": "2026-01-01"}), "week 1 of 2026")),
        local("days_until", C, "Days from today until a date", &[D],
            |v| { let d = (parse_date(s(v, "date")?)? - Local::now().date_naive()).num_days(); Ok(if d >= 0 { format!("{d} days to go") } else { format!("{} days ago", -d) }) },
            (json!({"date": "today"}), "0 days to go")),
        local("age_from_birthdate", C, "Age in years (and days to the next birthday)", &[("birthdate", "string", "Date of birth", true), ("on", "string", "As of this date (default today)", false)],
            |v| {
                let b = parse_date(s(v, "birthdate")?)?; let on = parse_date(s_or(v, "on", "today"))?;
                let mut age = on.year() - b.year(); if (on.month(), on.day()) < (b.month(), b.day()) { age -= 1; }
                let mut next = NaiveDate::from_ymd_opt(on.year(), b.month(), b.day().min(28)).unwrap_or(on); if next < on { next = next.with_year(on.year() + 1).unwrap_or(next); }
                Ok(format!("{age} years\nnext birthday in {} days", (next - on).num_days()))
            },
            (json!({"birthdate": "2000-05-10", "on": "2026-05-09"}), "25 years")),
        local("month_calendar", C, "A text calendar for a month", &[("year", "integer", "Year", true), ("month", "integer", "Month 1–12", true)],
            |v| {
                let (y, m) = (n(v, "year")? as i32, n(v, "month")? as u32); let first = NaiveDate::from_ymd_opt(y, m, 1).ok_or("bad month")?;
                let days = (first.checked_add_months(chrono::Months::new(1)).unwrap() - first).num_days() as u32;
                let mut out = format!("{}\nMo Tu We Th Fr Sa Su\n", first.format("%B %Y")); let pad = first.weekday().num_days_from_monday();
                out.push_str(&"   ".repeat(pad as usize));
                for d in 1..=days { out.push_str(&format!("{d:>2} ")); if (pad + d) % 7 == 0 { out.push('\n'); } }
                Ok(out.trim_end().to_string())
            },
            (json!({"year": 2026, "month": 10}), "October 2026")),
        local("is_leap_year", C, "Whether a year is a leap year", &[("year", "integer", "Year", true)],
            |v| { let y = n(v, "year")? as i32; Ok(if NaiveDate::from_ymd_opt(y, 2, 29).is_some() { format!("{y} is a leap year") } else { format!("{y} is not a leap year") }) },
            (json!({"year": 2028}), "2028 is a leap year")),
        local("cron_next_runs", C, "The next times a cron schedule fires", &[("schedule", "string", "5-field cron, e.g. 30 9 * * 1-5", true), ("count", "integer", "How many (default 5)", false)],
            |v| { let c = mybot_core::cron::Cron::parse(s(v, "schedule")?)?; Ok(format!("{}\n{}", c.describe(), c.upcoming(Local::now(), n_or(v, "count", 5.0) as usize).iter().map(|t| t.format("%a %Y-%m-%d %H:%M").to_string()).collect::<Vec<_>>().join("\n"))) },
            (json!({"schedule": "0 9 * * 1-5", "count": 2}), "every weekday at 09:00")),
        local("duration_format", C, "Turn seconds into h/m/s (or parse 1h30m into seconds)", &[("value", "string", "Seconds, or a duration like 1h30m", true)],
            |v| {
                let t = s(v, "value")?.trim();
                if let Ok(secs) = t.parse::<f64>() { let s2 = secs as u64; return Ok(format!("{}h {}m {}s", s2 / 3600, s2 % 3600 / 60, s2 % 60)); }
                let mut total = 0u64; let mut cur = String::new();
                for c in t.chars() { if c.is_ascii_digit() { cur.push(c); } else { let x: u64 = cur.parse().unwrap_or(0); cur.clear(); total += x * match c { 'd' => 86400, 'h' => 3600, 'm' => 60, 's' => 1, _ => 0 }; } }
                Ok(format!("{total} seconds"))
            },
            (json!({"value": "1h30m"}), "5400 seconds")),
    ]
}
