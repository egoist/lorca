//! When a routine runs: an interval (`every 30m`, `every 2h`, `every 1d`), a five-field cron
//! expression (`0 9 * * 1-5`), or one date and time (`once 2026-10-12 09:00`), read in the
//! routine's IANA timezone; or a time before or after calendar events (`15m before events`),
//! which the events' own times place. `describe` says it in words the way Grok Bot's routine
//! panel does; `next_after` finds the next firing.

use chrono::{Datelike, TimeZone, Timelike};
use chrono_tz::Tz;

/// An IANA timezone by name, or what to pass instead.
pub fn timezone(name: &str) -> Result<Tz, String> {
    name.trim().parse().map_err(|_| format!("Unknown timezone {name:?}. Use an IANA timezone such as America/New_York, Asia/Singapore, or UTC."))
}

/// This Device's IANA timezone, which a routine keeps from its creation on; UTC when the system
/// names none Lorca knows.
pub fn local_timezone() -> String {
    iana_time_zone::get_timezone().ok().and_then(|name| timezone(&name).ok()).unwrap_or(Tz::UTC).to_string()
}

/// The shortest gap between two runs of one routine.
pub const MIN_INTERVAL_SECS: i64 = 5 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schedule {
    /// Every so many seconds, counted from the last run (or from when the routine was armed).
    Every(i64),
    Cron(Cron),
    /// One date and time on the routine's clock: the routine runs once and is done.
    Once(chrono::NaiveDateTime),
    /// So many minutes before calendar events start, or after they end; the events place it.
    Events(EventOffset),
}

/// Where a run falls against a calendar event: `minutes` before it starts, or after it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventOffset {
    pub minutes: u32,
    pub after: bool,
}

impl EventOffset {
    /// The longest an event schedule reaches before or after its event.
    pub const MAX_MINUTES: u32 = 24 * 60;

    /// When the run for an event that starts at `start` and ends at `end` is due.
    pub fn due(&self, start: i64, end: i64) -> i64 {
        if self.after {
            end + self.minutes as i64 * 60
        } else {
            start - self.minutes as i64 * 60
        }
    }
}

/// A five-field cron expression as bit sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minutes: u64,
    hours: u32,
    /// Bits 1..=31.
    days_of_month: u32,
    /// Bits 1..=12.
    months: u16,
    /// Bits 0..=6, Sunday is 0.
    days_of_week: u8,
    dom_restricted: bool,
    dow_restricted: bool,
    normalized: String,
}

const MONTH_NAMES: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const DAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const DAY_LABELS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Reads a schedule, or says what is wrong with it.
pub fn parse(text: &str) -> Result<Schedule, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Give a schedule: every 30m, every 2h, every 1d, or a cron expression like 0 9 * * 1-5.".into());
    }
    let lowered = text.to_lowercase();
    if let Some(rest) = lowered.strip_prefix("every ").or_else(|| lowered.strip_prefix('@')) {
        return parse_every(rest.trim());
    }
    if let Some(rest) = lowered.strip_prefix("once ") {
        return parse_once(rest.trim());
    }
    if lowered.contains("event") {
        return parse_events(&lowered);
    }
    let fields: Vec<&str> = text.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "Could not read the schedule {text:?}. Use every 30m, every 2h, every 1d, or five cron fields (minute hour day-of-month month day-of-week) like 0 9 * * 1-5."
        ));
    }
    let minutes = parse_field(fields[0], 0, 59, &[])?;
    let hours = parse_field(fields[1], 0, 23, &[])?;
    let days_of_month = parse_field(fields[2], 1, 31, &[])?;
    let months = parse_field(fields[3], 1, 12, &MONTH_NAMES)?;
    let mut days_of_week = parse_field(fields[4], 0, 7, &DAY_NAMES)?;
    // 7 is Sunday too.
    if days_of_week & (1 << 7) != 0 {
        days_of_week = (days_of_week | 1) & !(1 << 7);
    }
    // More firings within an hour than one every five minutes.
    if minutes.count_ones() as i64 > 3600 / MIN_INTERVAL_SECS {
        return Err("That runs too often. A routine runs at most every five minutes.".into());
    }
    Ok(Schedule::Cron(Cron {
        minutes,
        hours: hours as u32,
        days_of_month: days_of_month as u32,
        months: months as u16,
        days_of_week: days_of_week as u8,
        dom_restricted: fields[2] != "*",
        dow_restricted: fields[4] != "*",
        normalized: fields.join(" "),
    }))
}

/// A schedule that comes round again, for what is shared or set up from the marketplace: a
/// one-time date or a time around events belongs to one account's own day or calendar.
pub fn parse_repeating(text: &str) -> Result<Schedule, String> {
    let schedule = parse(text)?;
    if !schedule.repeats() {
        return Err("A shared routine repeats: use every 30m, every 2h, every 1d, or a cron expression.".into());
    }
    Ok(schedule)
}

/// `30m`, `2h`, `1d`, `30 minutes`, `2 hours`, `day`, `hour`, `hourly`, `daily`.
fn parse_every(rest: &str) -> Result<Schedule, String> {
    let compact: String = rest.chars().filter(|c| !c.is_whitespace()).collect();
    let (amount, unit) = match compact.as_str() {
        "hour" | "hourly" => (1, "h"),
        "day" | "daily" => (1, "d"),
        "minute" => (1, "m"),
        _ => {
            let digits: String = compact.chars().take_while(|c| c.is_ascii_digit()).collect();
            let unit = &compact[digits.len()..];
            let amount: i64 = digits.parse().map_err(|_| format!("Could not read the interval {rest:?}. Use every 30m, every 2h, or every 1d."))?;
            (amount, unit)
        }
    };
    let multiplier = match unit {
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        _ => return Err(format!("Could not read the interval {rest:?}. Use every 30m, every 2h, or every 1d.")),
    };
    let secs = amount.checked_mul(multiplier).ok_or("That interval is longer than a year.")?;
    if secs < MIN_INTERVAL_SECS {
        return Err("That runs too often. A routine runs at most every five minutes.".into());
    }
    if secs > 366 * 86_400 {
        return Err("That interval is longer than a year. Use a cron expression for a yearly date.".into());
    }
    Ok(Schedule::Every(secs))
}

/// `2026-10-12 09:00`, `2026-10-12T09:00`, `2026-10-12 9:00 am`: a date and a time of day.
fn parse_once(rest: &str) -> Result<Schedule, String> {
    let problem = || format!("Could not read the time {rest:?}. Give a date and a 24-hour time, like once 2026-10-12 09:00, on the routine's clock.");
    // The date has no letters, so a `t` can only stand between it and the time.
    let rest = rest.trim_start_matches("at ").replace(" at ", " ").replacen('t', " ", 1).replace("am", " am").replace("pm", " pm");
    let mut parts = rest.split_whitespace();
    let date = parts.next().and_then(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()).ok_or_else(problem)?;
    let clock = parts.next().ok_or_else(problem)?;
    let suffix = parts.next();
    if parts.next().is_some() {
        return Err(problem());
    }
    let (hour, minute) = clock.split_once(':').and_then(|(h, m)| Some((h.parse::<u32>().ok()?, m.get(..2)?.parse::<u32>().ok()?))).ok_or_else(problem)?;
    let hour = match suffix {
        None => hour,
        Some("am") if (1..=12).contains(&hour) => hour % 12,
        Some("pm") if (1..=12).contains(&hour) => hour % 12 + 12,
        Some(_) => return Err(problem()),
    };
    let time = chrono::NaiveTime::from_hms_opt(hour, minute, 0).ok_or_else(problem)?;
    Ok(Schedule::Once(date.and_time(time)))
}

/// `15m before events`, `1h before events`, `at events`, `10m after events`, `when events end`.
fn parse_events(text: &str) -> Result<Schedule, String> {
    let problem = || format!("Could not read {text:?}. Use 15m before events, at events, or 10m after events.");
    let words: String = text.replace("each ", "").replace("the ", "").replace("every ", "").replace("events", "event").split_whitespace().collect::<Vec<_>>().join(" ");
    let words = words.trim_end_matches(" ends").trim_end_matches(" end").trim_end_matches(" starts").trim_end_matches(" start");
    let (amount, after) = match words {
        "at event" | "when event" | "at start of event" | "before event" => ("", false),
        "at end of event" | "after event" => ("", true),
        _ => match words.strip_suffix(" before event").map(|amount| (amount, false)).or_else(|| words.strip_suffix(" after event").map(|amount| (amount, true))) {
            Some(found) => found,
            None => return Err(problem()),
        },
    };
    // "when events end" lost its last word above.
    let after = after || (words == "when event" && text.contains(" end"));
    let minutes = if amount.is_empty() {
        0
    } else {
        let compact: String = amount.chars().filter(|c| !c.is_whitespace()).collect();
        let digits: String = compact.chars().take_while(|c| c.is_ascii_digit()).collect();
        let value: u32 = digits.parse().map_err(|_| problem())?;
        match &compact[digits.len()..] {
            "m" | "min" | "mins" | "minute" | "minutes" => value,
            "h" | "hr" | "hrs" | "hour" | "hours" => value.checked_mul(60).ok_or_else(problem)?,
            _ => return Err(problem()),
        }
    };
    if minutes > EventOffset::MAX_MINUTES {
        return Err("Keep a time around events within a day of them.".into());
    }
    Ok(Schedule::Events(EventOffset { minutes, after }))
}

/// One cron field into a bit set: `*`, `*/n`, `a`, `a-b`, `a-b/n`, names, and lists of those.
fn parse_field(field: &str, min: u32, max: u32, names: &[&str]) -> Result<u64, String> {
    let mut bits: u64 = 0;
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!("Empty entry in the cron field {field:?}."));
        }
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, step.parse::<u32>().ok().filter(|s| *s > 0).ok_or_else(|| format!("Bad step in {part:?}."))?),
            None => (part, 1),
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (parse_value(a, min, max, names)?, parse_value(b, min, max, names)?)
        } else {
            let value = parse_value(range, min, max, names)?;
            if step > 1 {
                (value, max)
            } else {
                (value, value)
            }
        };
        if lo > hi {
            return Err(format!("The range {range:?} runs backwards."));
        }
        let mut value = lo;
        while value <= hi {
            bits |= 1 << value;
            value += step;
        }
    }
    Ok(bits)
}

fn parse_value(text: &str, min: u32, max: u32, names: &[&str]) -> Result<u32, String> {
    let text = text.trim().to_lowercase();
    if let Some(index) = names.iter().position(|n| text.starts_with(n) && text.len() <= 9) {
        return Ok(index as u32 + min);
    }
    let value: u32 = text.parse().map_err(|_| format!("Could not read {text:?} in the cron expression."))?;
    if value < min || value > max {
        return Err(format!("{value} is out of range ({min}-{max}) in the cron expression."));
    }
    Ok(value)
}

impl Schedule {
    /// The first time this schedule fires strictly after `after` (unix seconds), a cron read in
    /// `zone`: the same instant on every Device. A minute that daylight saving skips does not
    /// fire that day; one it repeats fires once, at its earlier instant. An interval counts
    /// elapsed seconds. `None` for an unknown zone or when nothing matches in two years.
    pub fn next_after(&self, after: i64, zone: &str) -> Option<i64> {
        let zone = timezone(zone).ok()?;
        match self {
            Schedule::Every(secs) => after.checked_add(*secs),
            Schedule::Cron(cron) => cron.next_after(after, zone),
            Schedule::Once(_) => self.once_at(zone).filter(|at| *at > after),
            Schedule::Events(_) => None,
        }
    }

    /// The instant of a one-time schedule on `zone`'s clock. A time that daylight saving skips
    /// runs at the same clock time an hour later; one it repeats, at its earlier instant.
    fn once_at(&self, zone: Tz) -> Option<i64> {
        let Schedule::Once(local) = self else { return None };
        zone.from_local_datetime(local).earliest().or_else(|| zone.from_local_datetime(&(*local + chrono::Duration::hours(1))).earliest()).map(|at| at.timestamp())
    }

    /// The instant a one-time schedule runs, read in `zone`.
    pub fn once_instant(&self, zone: &str) -> Option<i64> {
        self.once_at(timezone(zone).ok()?)
    }

    /// An interval or a cron expression: a schedule that comes round again.
    pub fn repeats(&self) -> bool {
        matches!(self, Schedule::Every(_) | Schedule::Cron(_))
    }

    /// The schedule in words: "Every 15 minutes", "Weekdays at 9:00 AM".
    pub fn describe(&self) -> String {
        match self {
            Schedule::Every(secs) => describe_interval(*secs),
            Schedule::Cron(cron) => cron.describe(),
            Schedule::Once(at) => format!("Once on {} at {}", at.format("%Y-%m-%d"), clock(at.hour(), at.minute())),
            Schedule::Events(offset) => describe_events(*offset, None),
        }
    }

    /// The canonical spelling, for storage.
    pub fn canonical(&self) -> String {
        match self {
            Schedule::Every(secs) => {
                if secs % 86_400 == 0 {
                    format!("every {}d", secs / 86_400)
                } else if secs % 3600 == 0 {
                    format!("every {}h", secs / 3600)
                } else {
                    format!("every {}m", secs / 60)
                }
            }
            Schedule::Cron(cron) => cron.normalized.clone(),
            Schedule::Once(at) => format!("once {}", at.format("%Y-%m-%d %H:%M")),
            Schedule::Events(EventOffset { minutes, after }) => format!("{minutes}m {} events", if *after { "after" } else { "before" }),
        }
    }
}

/// A time around calendar events in words, naming the events that match when only some do:
/// "15 minutes before each event", "When events matching “Customer” end".
pub fn describe_events(offset: EventOffset, matching: Option<&str>) -> String {
    let span = |minutes: u32| match minutes {
        m if m % 60 == 0 && m / 60 == 1 => "1 hour".to_string(),
        m if m % 60 == 0 => format!("{} hours", m / 60),
        1 => "1 minute".to_string(),
        m => format!("{m} minutes"),
    };
    match (matching, offset.after, offset.minutes) {
        (None, false, 0) => "When each event starts".into(),
        (None, true, 0) => "When each event ends".into(),
        (None, false, m) => format!("{} before each event", span(m)),
        (None, true, m) => format!("{} after each event ends", span(m)),
        (Some(words), false, 0) => format!("When events matching “{words}” start"),
        (Some(words), true, 0) => format!("When events matching “{words}” end"),
        (Some(words), false, m) => format!("{} before events matching “{words}”", span(m)),
        (Some(words), true, m) => format!("{} after events matching “{words}” end", span(m)),
    }
}

fn describe_interval(secs: i64) -> String {
    let plural = |n: i64, unit: &str| if n == 1 { format!("Every {unit}") } else { format!("Every {n} {unit}s") };
    if secs % 86_400 == 0 {
        plural(secs / 86_400, "day")
    } else if secs % 3600 == 0 {
        plural(secs / 3600, "hour")
    } else {
        plural(secs / 60, "minute")
    }
}

impl Cron {
    fn next_after(&self, after: i64, zone: Tz) -> Option<i64> {
        let mut date = zone.timestamp_opt(after, 0).single()?.date_naive();
        let horizon = after.checked_add(2 * 366 * 86_400)?;
        for _ in 0..=733 {
            let month_ok = self.months & (1 << date.month()) != 0;
            let dom_ok = self.days_of_month & (1 << date.day()) != 0;
            let dow_ok = self.days_of_week & (1 << date.weekday().num_days_from_sunday()) != 0;
            let day_ok = match (self.dom_restricted, self.dow_restricted) {
                (true, true) => dom_ok || dow_ok,
                (true, false) => dom_ok,
                (false, true) => dow_ok,
                (false, false) => true,
            };
            if month_ok && day_ok {
                for hour in (0..24).filter(|hour| self.hours & (1 << hour) != 0) {
                    for minute in (0..60).filter(|minute| self.minutes & (1 << minute) != 0) {
                        let local = date.and_hms_opt(hour, minute, 0)?;
                        if let Some(at) = zone.from_local_datetime(&local).earliest().map(|at| at.timestamp()) {
                            if at > after && at <= horizon {
                                return Some(at);
                            }
                        }
                    }
                }
            }
            date = date.succ_opt()?;
        }
        None
    }

    fn describe(&self) -> String {
        let minutes: Vec<u32> = (0..60).filter(|m| self.minutes & (1 << m) != 0).collect();
        let hours: Vec<u32> = (0..24).filter(|h| self.hours & (1 << h) != 0).collect();
        let doms: Vec<u32> = (1..=31).filter(|d| self.days_of_month & (1 << d) != 0).collect();
        let dows: Vec<u32> = (0..7).filter(|d| self.days_of_week & (1 << d) != 0).collect();
        let all_months = self.months & 0b1_1111_1111_1110 == 0b1_1111_1111_1110;
        let fallback = || format!("Cron {}", self.normalized);
        if !all_months {
            return fallback();
        }
        let every_day = !self.dom_restricted && (!self.dow_restricted || dows.len() == 7);
        let days = if every_day {
            None
        } else if self.dow_restricted && !self.dom_restricted {
            Some(match dows.as_slice() {
                [1, 2, 3, 4, 5] => "Weekdays".to_string(),
                [0, 6] => "Weekends".to_string(),
                [one] => format!("Every {}", DAY_LABELS[*one as usize]),
                many => join_words(&many.iter().map(|d| DAY_LABELS[*d as usize].to_string()).collect::<Vec<_>>()),
            })
        } else if self.dom_restricted && !self.dow_restricted {
            Some(format!("On the {} of every month", join_words(&doms.iter().map(|d| ordinal(*d)).collect::<Vec<_>>())))
        } else {
            return fallback();
        };
        // Times of day, or a rhythm within the day.
        let times: Option<String> = match (minutes.as_slice(), hours.as_slice()) {
            ([minute], hours) if hours.len() == 24 => {
                if days.is_some() {
                    None
                } else if *minute == 0 {
                    return "Every hour".into();
                } else {
                    return format!("Every hour at :{minute:02}");
                }
            }
            (minutes, hours) if hours.len() == 24 && days.is_none() && minutes.len() > 1 && evenly_spaced(minutes, 60) => {
                return format!("Every {} minutes", 60 / minutes.len() as u32);
            }
            ([0], hours) if hours.len() > 1 && days.is_none() && evenly_spaced(hours, 24) => {
                return format!("Every {} hours", 24 / hours.len() as u32);
            }
            ([minute], hours) if hours.len() <= 3 => Some(join_words(&hours.iter().map(|h| clock(*h, *minute)).collect::<Vec<_>>())),
            _ => None,
        };
        match (days, times) {
            (None, Some(times)) => format!("Every day at {times}"),
            (Some(days), Some(times)) => format!("{days} at {times}"),
            _ => fallback(),
        }
    }
}

/// `9:00 AM`, `5:30 PM`.
fn clock(hour: u32, minute: u32) -> String {
    let (h, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("{h}:{minute:02} {suffix}")
}

fn ordinal(n: u32) -> String {
    let suffix = match n % 100 {
        11..=13 => "th",
        _ => match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    };
    format!("{n}{suffix}")
}

/// "A", "A and B", "A, B, and C".
pub(crate) fn join_words(words: &[String]) -> String {
    match words {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        many => format!("{}, and {}", many[..many.len() - 1].join(", "), many[many.len() - 1]),
    }
}

/// `0, 15, 30, 45` within 60: a step that divides the cycle, starting at 0.
fn evenly_spaced(values: &[u32], cycle: u32) -> bool {
    if values.len() < 2 || values[0] != 0 || !cycle.is_multiple_of(values.len() as u32) {
        return false;
    }
    let step = cycle / values.len() as u32;
    values.iter().enumerate().all(|(i, v)| *v == i as u32 * step)
}

/// `today 9:00 AM`, `tomorrow 9:00 AM`, `Monday 9:00 AM`, `2026-10-01 9:00 AM`: when a run is
/// due, on the clock of `zone`.
pub fn when_label(at: i64, now: i64, zone: &str) -> String {
    let zone = timezone(zone).unwrap_or(Tz::UTC);
    let (Some(at), Some(now)) = (zone.timestamp_opt(at, 0).single(), zone.timestamp_opt(now, 0).single()) else { return String::new() };
    let time = clock(at.hour(), at.minute());
    match (at.date_naive() - now.date_naive()).num_days() {
        0 => format!("today {time}"),
        1 => format!("tomorrow {time}"),
        2..=6 => format!("{} {time}", DAY_LABELS[at.weekday().num_days_from_sunday() as usize]),
        _ => format!("{} {time}", at.format("%Y-%m-%d")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zone without daylight saving, so the plain cases read the same everywhere.
    const ZONE: &str = "Asia/Singapore";

    fn at(date: &str, clock: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(&format!("{date}T{clock}:00+08:00")).unwrap().timestamp()
    }

    fn utc(text: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(text).unwrap().timestamp()
    }

    #[test]
    fn intervals_parse_and_read_back() {
        assert_eq!(parse("every 30m").unwrap(), Schedule::Every(1800));
        assert_eq!(parse("every 2 hours").unwrap(), Schedule::Every(7200));
        assert_eq!(parse("Every day").unwrap(), Schedule::Every(86_400));
        assert_eq!(parse("@hourly").unwrap(), Schedule::Every(3600));
        assert_eq!(parse("every 30m").unwrap().describe(), "Every 30 minutes");
        assert_eq!(parse("every 1h").unwrap().describe(), "Every hour");
        assert_eq!(parse("every 3d").unwrap().describe(), "Every 3 days");
        assert_eq!(parse("every 90m").unwrap().canonical(), "every 90m");
        assert_eq!(parse("every 120m").unwrap().canonical(), "every 2h");
        assert!(parse("every 2m").unwrap_err().contains("too often"));
        assert!(parse("every fortnight").is_err());
        assert_eq!(parse("every 30m").unwrap().next_after(1000, ZONE), Some(2800));
    }

    #[test]
    fn cron_reads_in_words() {
        let words = |text: &str| parse(text).unwrap().describe();
        assert_eq!(words("0 9 * * *"), "Every day at 9:00 AM");
        assert_eq!(words("30 17 * * 1-5"), "Weekdays at 5:30 PM");
        assert_eq!(words("0 9 * * mon-fri"), "Weekdays at 9:00 AM");
        assert_eq!(words("0 10 * * 0,6"), "Weekends at 10:00 AM");
        assert_eq!(words("0 9 * * 1"), "Every Monday at 9:00 AM");
        assert_eq!(words("0 9 * * 1,3,5"), "Monday, Wednesday, and Friday at 9:00 AM");
        assert_eq!(words("0 9 1 * *"), "On the 1st of every month at 9:00 AM");
        assert_eq!(words("0 9 1,15 * *"), "On the 1st and 15th of every month at 9:00 AM");
        assert_eq!(words("0 9,17 * * *"), "Every day at 9:00 AM and 5:00 PM");
        assert_eq!(words("0 * * * *"), "Every hour");
        assert_eq!(words("15 * * * *"), "Every hour at :15");
        assert_eq!(words("*/15 * * * *"), "Every 15 minutes");
        assert_eq!(words("0 */6 * * *"), "Every 6 hours");
        assert_eq!(words("0 0 * * *"), "Every day at 12:00 AM");
        assert_eq!(words("5 4 * * 1-3"), "Monday, Tuesday, and Wednesday at 4:05 AM");
        assert_eq!(words("0 9 1 jan *"), "Cron 0 9 1 jan *");
        assert!(parse("* * * * *").unwrap_err().contains("too often"));
        assert!(parse("0 25 * * *").is_err());
        assert!(parse("0 9 * *").is_err());
        assert_eq!(parse("0  9 * * 1-5").unwrap().canonical(), "0 9 * * 1-5");
    }

    #[test]
    fn cron_finds_the_next_firing_in_its_zone() {
        // 2026-09-17 is a Thursday.
        let thursday_noon = at("2026-09-17", "12:00");
        let weekdays = parse("0 9 * * 1-5").unwrap();
        assert_eq!(weekdays.next_after(thursday_noon, ZONE), Some(at("2026-09-18", "09:00")));
        let friday_ten = at("2026-09-18", "10:00");
        assert_eq!(weekdays.next_after(friday_ten, ZONE), Some(at("2026-09-21", "09:00")), "skips the weekend");
        assert_eq!(weekdays.next_after(at("2026-09-18", "09:00"), ZONE), Some(at("2026-09-21", "09:00")), "strictly after");
        assert_eq!(weekdays.next_after(at("2026-09-18", "08:59"), ZONE), Some(at("2026-09-18", "09:00")));

        let quarter = parse("*/15 * * * *").unwrap();
        assert_eq!(quarter.next_after(at("2026-09-17", "12:07"), ZONE), Some(at("2026-09-17", "12:15")));
        assert_eq!(quarter.next_after(at("2026-09-17", "12:45"), ZONE), Some(at("2026-09-17", "13:00")));

        let first = parse("30 8 1 * *").unwrap();
        assert_eq!(first.next_after(thursday_noon, ZONE), Some(at("2026-10-01", "08:30")));
        let leap = parse("0 0 29 2 *").unwrap();
        assert_eq!(leap.next_after(thursday_noon, ZONE), Some(at("2028-02-29", "00:00")));
        // Both day fields set: either matches, as cron has it.
        let either = parse("0 9 15 * 1").unwrap();
        assert_eq!(either.next_after(at("2026-09-17", "12:00"), ZONE), Some(at("2026-09-21", "09:00")));
        assert_eq!(either.next_after(at("2026-10-13", "12:00"), ZONE), Some(at("2026-10-15", "09:00")));
    }

    #[test]
    fn due_labels_read_like_a_person() {
        let now = at("2026-09-17", "12:00");
        assert_eq!(when_label(at("2026-09-17", "17:30"), now, ZONE), "today 5:30 PM");
        assert_eq!(when_label(at("2026-09-18", "09:00"), now, ZONE), "tomorrow 9:00 AM");
        assert_eq!(when_label(at("2026-09-21", "09:00"), now, ZONE), "Monday 9:00 AM");
        assert_eq!(when_label(at("2026-10-01", "08:30"), now, ZONE), "2026-10-01 8:30 AM");
        // The day turns on the routine's clock: 9:00 AM tomorrow in New York is 9:00 PM in Singapore.
        assert_eq!(when_label(utc("2026-09-18T13:00:00Z"), now, "America/New_York"), "tomorrow 9:00 AM");
    }

    #[test]
    fn explicit_timezones_use_the_same_instant_on_every_device() {
        let morning = parse("0 9 * * *").unwrap();
        let after = utc("2026-10-08T00:00:00Z");
        assert_eq!(morning.next_after(after, "Asia/Singapore"), Some(utc("2026-10-08T01:00:00Z")));
        assert_eq!(morning.next_after(after, "America/New_York"), Some(utc("2026-10-08T13:00:00Z")));
        assert!(timezone("Mars/Base").is_err());
        assert!(timezone(&local_timezone()).is_ok());
        assert_eq!(morning.next_after(after, "Mars/Base"), None);
        assert_eq!(parse("every 1d").unwrap().next_after(after, "America/New_York"), Some(after + 86_400));
        assert!(parse("every 9223372036854775807d").is_err());
    }

    #[test]
    fn daylight_saving_gaps_skip_and_folds_fire_only_at_the_earlier_instant() {
        let zone = "America/New_York";
        let morning = parse("0 9 * * *").unwrap();
        assert_eq!(morning.next_after(utc("2026-03-07T14:00:00Z"), zone), Some(utc("2026-03-08T13:00:00Z")), "a 23-hour day");
        assert_eq!(morning.next_after(utc("2026-10-31T13:00:00Z"), zone), Some(utc("2026-11-01T14:00:00Z")), "a 25-hour day");
        let gap = parse("30 2 * * *").unwrap();
        assert_eq!(gap.next_after(utc("2026-03-08T00:00:00Z"), zone), Some(utc("2026-03-09T06:30:00Z")), "02:30 does not exist on March 8");
        let fold = parse("30 1 * * *").unwrap();
        let first = utc("2026-11-01T05:30:00Z");
        assert_eq!(fold.next_after(first - 60, zone), Some(first));
        assert_eq!(fold.next_after(first, zone), Some(utc("2026-11-02T06:30:00Z")), "the repeated 01:30 never fires again");
        assert_eq!(fold.next_after(utc("2026-11-01T06:00:00Z"), zone), Some(utc("2026-11-02T06:30:00Z")), "restarting inside the fold does not repeat it");
    }

    #[test]
    fn a_one_time_schedule_is_one_instant_on_the_routines_clock() {
        let once = parse("once 2026-10-12 09:00").unwrap();
        assert_eq!(once.canonical(), "once 2026-10-12 09:00");
        assert_eq!(once.describe(), "Once on 2026-10-12 at 9:00 AM");
        assert!(!once.repeats());
        for spelling in ["Once 2026-10-12T09:00", "once 2026-10-12 9:00 am", "once 2026-10-12 at 9:00", "once 2026-10-12 9:00am"] {
            assert_eq!(parse(spelling).unwrap(), once, "{spelling}");
        }
        assert_eq!(parse("once 2026-10-12 9:30 pm").unwrap().canonical(), "once 2026-10-12 21:30");
        assert!(parse("once tomorrow").is_err());
        assert!(parse("once 2026-10-12").is_err());
        assert!(parse("once 2026-10-12 25:00").is_err());
        let at = utc("2026-10-12T13:00:00Z");
        assert_eq!(once.next_after(at - 1, "America/New_York"), Some(at));
        assert_eq!(once.next_after(at, "America/New_York"), None, "it does not come round again");
        assert_eq!(once.once_instant("Asia/Singapore"), Some(utc("2026-10-12T01:00:00Z")));
        // 02:30 does not exist on March 8 in New York: it runs at 03:30.
        assert_eq!(parse("once 2026-03-08 02:30").unwrap().once_instant("America/New_York"), Some(utc("2026-03-08T07:30:00Z")));
    }

    #[test]
    fn times_around_events_read_back_in_words() {
        let words = |text: &str| parse(text).unwrap().describe();
        assert_eq!(parse("15m before events").unwrap(), Schedule::Events(EventOffset { minutes: 15, after: false }));
        assert_eq!(parse("1h before events").unwrap().canonical(), "60m before events");
        assert_eq!(parse("at events").unwrap().canonical(), "0m before events");
        assert_eq!(parse("when events end").unwrap().canonical(), "0m after events");
        assert_eq!(parse("10 minutes after each event").unwrap().canonical(), "10m after events");
        assert_eq!(words("15m before events"), "15 minutes before each event");
        assert_eq!(words("2h before events"), "2 hours before each event");
        assert_eq!(words("at events"), "When each event starts");
        assert_eq!(words("10m after events"), "10 minutes after each event ends");
        assert_eq!(describe_events(EventOffset { minutes: 15, after: false }, Some("Customer")), "15 minutes before events matching “Customer”");
        assert_eq!(describe_events(EventOffset { minutes: 0, after: true }, Some("Customer")), "When events matching “Customer” end");
        assert!(parse("2d before events").is_err());
        assert!(parse("25h before events").unwrap_err().contains("within a day"));
        assert!(parse("sometime around events").is_err());
        assert!(parse_repeating("15m before events").unwrap_err().contains("repeats"));
        assert!(parse_repeating("once 2030-01-01 09:00").is_err());
        assert!(parse_repeating("every 2h").is_ok());
        assert_eq!(parse("15m before events").unwrap().next_after(0, "UTC"), None, "the events place it");
        let offset = EventOffset { minutes: 15, after: false };
        assert_eq!(offset.due(10_000, 12_000), 9_100);
        assert_eq!(EventOffset { minutes: 10, after: true }.due(10_000, 12_000), 12_600);
    }

    #[test]
    fn explicit_cron_handles_month_boundaries_and_half_hour_dst() {
        let first = parse("30 8 1 * *").unwrap();
        assert_eq!(first.next_after(utc("2026-09-30T23:00:00Z"), "Asia/Singapore"), Some(utc("2026-10-01T00:30:00Z")));
        let fold = parse("45 1 * * *").unwrap();
        let zone = "Australia/Lord_Howe";
        let first = utc("2026-04-04T14:45:00Z");
        assert_eq!(fold.next_after(first - 60, zone), Some(first));
        assert_eq!(fold.next_after(first, zone), Some(utc("2026-04-05T15:15:00Z")));
    }
}
