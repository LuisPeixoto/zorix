//! When a schedule fires next (ADR-0034). Pure: the clock is a parameter, and the time zone's own rules (daylight saving
//! included) decide what "08:00" means.

use std::str::FromStr;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use zorix_core::{Timestamp, When};

/// The time zone named by a schedule, or the computer's own, or UTC when that cannot be known.
pub fn zone(name: Option<&str>) -> Result<Tz, String> {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => {
            name.parse::<Tz>().map_err(|_| format!("“{name}” is not a time zone (use a name such as Europe/Lisbon)."))
        }
        None => Ok(iana_time_zone::get_timezone().ok().and_then(|n| n.parse::<Tz>().ok()).unwrap_or(Tz::UTC)),
    }
}

fn to_utc(at: Timestamp) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(at.millis()).unwrap_or_default()
}

fn stamp<T: TimeZone>(at: DateTime<T>) -> Timestamp {
    Timestamp(at.timestamp_millis())
}

/// A local date and time as an instant. An hour that happens twice (the clocks go back) takes the first; one that does not exist
/// (the clocks go forward) moves to the first hour that does.
fn resolve(zone: Tz, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Tz>> {
    let mut naive = date.and_time(time);
    for _ in 0..3 {
        if let Some(found) = zone.from_local_datetime(&naive).earliest() {
            return Some(found);
        }
        naive += Duration::hours(1);
    }
    None
}

fn parse_time(text: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(text, "%H:%M").map_err(|_| format!("“{text}” is not a time (use HH:MM)."))
}

/// The first fire strictly after `after`, or none when the schedule never fires again.
pub fn next_fire(when: &When, timezone: Option<&str>, after: Timestamp) -> Result<Option<Timestamp>, String> {
    match when {
        When::Every { minutes } => {
            let step = i64::from(*minutes) * 60_000;
            if step <= 0 {
                return Err("An interval is at least one minute.".into());
            }
            // Aligned to the epoch, so a restart does not shift the beat.
            Ok(Some(Timestamp((after.millis().div_euclid(step) + 1) * step)))
        }
        When::Daily { times, days } => {
            let zone = zone(timezone)?;
            let mut times: Vec<NaiveTime> = times.iter().map(|t| parse_time(t)).collect::<Result<_, _>>()?;
            times.sort();
            let local = to_utc(after).with_timezone(&zone);
            for offset in 0..9 {
                let date = local.date_naive() + Duration::days(offset);
                let weekday = date.weekday().number_from_monday();
                if !days.is_empty() && !days.iter().any(|d| d.number() == weekday) {
                    continue;
                }
                for time in &times {
                    if let Some(candidate) = resolve(zone, date, *time)
                        && stamp(candidate) > after
                    {
                        return Ok(Some(stamp(candidate)));
                    }
                }
            }
            Ok(None)
        }
        When::Cron { expr } => {
            if expr.split_whitespace().count() != 5 {
                return Err("A cron expression has five fields: minute hour day-of-month month day-of-week.".into());
            }
            let cron = croner::Cron::from_str(expr).map_err(|e| format!("“{expr}” is not a cron expression: {e}"))?;
            let zone = zone(timezone)?;
            let from = to_utc(after).with_timezone(&zone);
            Ok(cron.find_next_occurrence(&from, false).ok().map(stamp))
        }
    }
}

/// The start of the local day (00:00 in the zone) that contains `at`.
pub fn day_start(at: Timestamp, timezone: Option<&str>) -> Result<Timestamp, String> {
    let zone = zone(timezone)?;
    let local = to_utc(at).with_timezone(&zone);
    let midnight = resolve(zone, local.date_naive(), NaiveTime::MIN).ok_or("That day has no midnight.")?;
    Ok(stamp(midnight))
}

/// The start of the local week (Monday 00:00 in the zone) that contains `at`.
pub fn week_start(at: Timestamp, timezone: Option<&str>) -> Result<Timestamp, String> {
    let zone = zone(timezone)?;
    let local = to_utc(at).with_timezone(&zone);
    let monday = local.date_naive() - Duration::days(i64::from(local.weekday().num_days_from_monday()));
    let start = resolve(zone, monday, NaiveTime::MIN).ok_or("That week has no Monday midnight.")?;
    Ok(stamp(start))
}

/// A local day as `YYYY-MM-DD`.
pub fn local_date(at: Timestamp, timezone: Option<&str>) -> Result<String, String> {
    let zone = zone(timezone)?;
    Ok(to_utc(at).with_timezone(&zone).format("%Y-%m-%d").to_string())
}

/// The next `HH:MM` after `after`, for a wait that ends at a time of day.
pub fn next_time_of_day(time: &str, timezone: Option<&str>, after: Timestamp) -> Result<Timestamp, String> {
    let when = When::Daily { times: vec![time.to_string()], days: vec![] };
    next_fire(&when, timezone, after)?.ok_or_else(|| "That time of day never comes.".into())
}

/// Every fire in `(from, to]`, for counting the ones missed while nothing was running. Capped, so a long absence stays cheap.
pub fn fires_between(
    when: &When,
    timezone: Option<&str>,
    from: Timestamp,
    to: Timestamp,
    cap: usize,
) -> Result<Vec<Timestamp>, String> {
    let mut out = Vec::new();
    let mut at = from;
    while out.len() < cap {
        match next_fire(when, timezone, at)? {
            Some(next) if next <= to => {
                out.push(next);
                at = next;
            }
            _ => break,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use zorix_core::Day;

    use super::*;

    fn at(zone: &str, y: i32, m: u32, d: u32, h: u32, min: u32) -> Timestamp {
        let tz: Tz = zone.parse().unwrap();
        stamp(tz.with_ymd_and_hms(y, m, d, h, min, 0).unwrap())
    }

    fn weekdays(times: &[&str]) -> When {
        When::Daily {
            times: times.iter().map(|t| t.to_string()).collect(),
            days: vec![Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri],
        }
    }

    #[test]
    fn the_week_starts_on_monday_at_midnight_in_the_zone_and_the_day_at_midnight() {
        // Wednesday 2026-11-18 15:30 in Lisbon (UTC+0 in November).
        let wed = at("Europe/Lisbon", 2026, 11, 18, 15, 30);
        assert_eq!(week_start(wed, Some("Europe/Lisbon")).unwrap(), at("Europe/Lisbon", 2026, 11, 16, 0, 0));
        assert_eq!(day_start(wed, Some("Europe/Lisbon")).unwrap(), at("Europe/Lisbon", 2026, 11, 18, 0, 0));
        // Monday 00:00 itself starts its own week; Sunday night belongs to the week before.
        let monday = at("Europe/Lisbon", 2026, 11, 16, 0, 0);
        assert_eq!(week_start(monday, Some("Europe/Lisbon")).unwrap(), monday);
        let sunday = at("Europe/Lisbon", 2026, 11, 15, 23, 59);
        assert_eq!(week_start(sunday, Some("Europe/Lisbon")).unwrap(), at("Europe/Lisbon", 2026, 11, 9, 0, 0));
        assert_eq!(local_date(wed, Some("Europe/Lisbon")).unwrap(), "2026-11-18");
        // The zone decides the day: 23:30 in Lisbon is already the next day in Tokyo.
        let late = at("Europe/Lisbon", 2026, 11, 18, 23, 30);
        assert_eq!(local_date(late, Some("Asia/Tokyo")).unwrap(), "2026-11-19");
    }

    #[test]
    fn weekdays_skip_the_weekend() {
        let zone = "Europe/Lisbon";
        // 2026-11-20 is a Friday.
        let friday_noon = at(zone, 2026, 11, 20, 12, 0);
        let next = next_fire(&weekdays(&["08:00"]), Some(zone), friday_noon).unwrap().unwrap();
        assert_eq!(next, at(zone, 2026, 11, 23, 8, 0), "the next fire is Monday");
        let before = at(zone, 2026, 11, 23, 7, 59);
        assert_eq!(next_fire(&weekdays(&["08:00"]), Some(zone), before).unwrap().unwrap(), next);
        // Exactly at the time, the next one is the following day: strictly after.
        let exact = next_fire(&weekdays(&["08:00"]), Some(zone), next).unwrap().unwrap();
        assert_eq!(exact, at(zone, 2026, 11, 24, 8, 0));
    }

    #[test]
    fn several_times_a_day_are_in_order() {
        let zone = "UTC";
        let when = When::Daily { times: vec!["16:00".into(), "08:00".into()], days: vec![] };
        let noon = at(zone, 2026, 11, 20, 12, 0);
        assert_eq!(next_fire(&when, Some(zone), noon).unwrap().unwrap(), at(zone, 2026, 11, 20, 16, 0));
        let evening = at(zone, 2026, 11, 20, 18, 0);
        assert_eq!(next_fire(&when, Some(zone), evening).unwrap().unwrap(), at(zone, 2026, 11, 21, 8, 0));
    }

    #[test]
    fn the_hour_follows_the_time_zone_across_daylight_saving() {
        let zone = "Europe/Lisbon";
        // Clocks go back on 2026-10-25 and forward on 2026-03-29: 08:00 local is 07:00 UTC in summer and 08:00 UTC in winter.
        let daily = When::Daily { times: vec!["08:00".into()], days: vec![] };
        let summer = next_fire(&daily, Some(zone), at("UTC", 2026, 7, 1, 0, 0)).unwrap().unwrap();
        assert_eq!(summer, at("UTC", 2026, 7, 1, 7, 0));
        let winter = next_fire(&daily, Some(zone), at("UTC", 2026, 12, 1, 0, 0)).unwrap().unwrap();
        assert_eq!(winter, at("UTC", 2026, 12, 1, 8, 0));
    }

    #[test]
    fn an_hour_that_does_not_exist_moves_to_the_first_that_does() {
        let zone = "Europe/Lisbon";
        // On 2026-03-29 at 01:00 the clocks jump to 02:00, so 01:30 does not exist that day.
        let daily = When::Daily { times: vec!["01:30".into()], days: vec![] };
        let next = next_fire(&daily, Some(zone), at("UTC", 2026, 3, 29, 0, 0)).unwrap().unwrap();
        assert_eq!(next, at("Europe/Lisbon", 2026, 3, 29, 2, 30));
    }

    #[test]
    fn intervals_keep_their_beat_across_restarts() {
        let every_15 = When::Every { minutes: 15 };
        let a = next_fire(&every_15, None, Timestamp(10 * 60_000)).unwrap().unwrap();
        assert_eq!(a, Timestamp(15 * 60_000));
        assert_eq!(next_fire(&every_15, None, a).unwrap().unwrap(), Timestamp(30 * 60_000));
        assert_eq!(next_fire(&every_15, None, Timestamp(14 * 60_000 + 59_999)).unwrap().unwrap(), a);
    }

    #[test]
    fn cron_runs_in_its_time_zone() {
        let zone = "America/New_York";
        let cron = When::Cron { expr: "30 9 * * 1-5".into() };
        let next = next_fire(&cron, Some(zone), at(zone, 2026, 11, 20, 10, 0)).unwrap().unwrap();
        assert_eq!(next, at(zone, 2026, 11, 23, 9, 30));
        assert!(next_fire(&When::Cron { expr: "nonsense".into() }, None, Timestamp(0)).is_err());
        assert!(next_fire(&When::Cron { expr: "* * * * * *".into() }, None, Timestamp(0)).is_err());
    }

    #[test]
    fn a_bad_zone_or_time_is_explained() {
        let daily = When::Daily { times: vec!["08:00".into()], days: vec![] };
        assert!(next_fire(&daily, Some("Mars/Olympus"), Timestamp(0)).unwrap_err().contains("not a time zone"));
        let bad = When::Daily { times: vec!["8am".into()], days: vec![] };
        assert!(next_fire(&bad, Some("UTC"), Timestamp(0)).is_err());
        assert!(zone(None).is_ok());
    }

    #[test]
    fn the_fires_in_a_gap_are_counted_and_capped() {
        let zone = "UTC";
        let daily = When::Daily { times: vec!["08:00".into()], days: vec![] };
        let from = at(zone, 2026, 11, 1, 9, 0);
        let to = at(zone, 2026, 11, 6, 9, 0);
        assert_eq!(fires_between(&daily, Some(zone), from, to, 100).unwrap().len(), 5);
        assert_eq!(fires_between(&daily, Some(zone), from, to, 3).unwrap().len(), 3);
        assert!(fires_between(&daily, Some(zone), to, from, 10).unwrap().is_empty());
    }

    #[test]
    fn a_wait_until_a_time_of_day() {
        let zone = "UTC";
        let at_noon = at(zone, 2026, 11, 20, 12, 0);
        assert_eq!(next_time_of_day("18:00", Some(zone), at_noon).unwrap(), at(zone, 2026, 11, 20, 18, 0));
        assert_eq!(next_time_of_day("06:00", Some(zone), at_noon).unwrap(), at(zone, 2026, 11, 21, 6, 0));
    }
}
