//! When a schedule fires next (ADR-0034, ADR-0041). Pure: the clock is a parameter, and the time zone's own rules (daylight saving
//! included) decide what "08:00" means.
//!
//! Two rules for the clocks changing, one for each kind of schedule:
//!
//! - **At fixed times of the day** (`Daily`): an hour that happens twice (the clocks go back) takes the first; one that does not exist
//!   (the clocks go forward) moves forward an hour at a time to the first hour that does, so 02:30 becomes 03:30.
//! - **Cron**: a schedule of fixed times (`30 2 * * *`) fires, for a time the clocks jump over, at the first instant after the jump (02:30
//!   fires at 03:00), and once in an hour that happens twice (the first time). A schedule with a `*` or a step in its minute or hour (every
//!   hour, every 15 minutes, `*/30 1-3`) keeps its beat: it does not fire for a skipped minute, and fires every time through a repeated hour.

use jiff::civil::{DateTime, date};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{Span, Timestamp as Instant};
use zorix_core::{Timestamp, When};

use crate::cron::Cron;

/// The time zone named by a schedule, or the computer's own, or UTC when that cannot be known.
pub fn zone(name: Option<&str>) -> Result<TimeZone, String> {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => {
            TimeZone::get(name).map_err(|_| format!("“{name}” is not a time zone (use a name such as Europe/Lisbon)."))
        }
        None => Ok(TimeZone::try_system().unwrap_or(TimeZone::UTC)),
    }
}

fn instant(at: Timestamp) -> Result<Instant, String> {
    Instant::from_millisecond(at.millis()).map_err(|_| "That moment is outside the calendar.".to_string())
}

fn stamp(at: Instant) -> Timestamp {
    Timestamp(at.as_millisecond())
}

fn civil_of(at: Timestamp, zone: &TimeZone) -> Result<DateTime, String> {
    Ok(instant(at)?.to_zoned(zone.clone()).datetime())
}

/// A local date and time as an instant. An hour that happens twice (the clocks go back) takes the first; one that does not exist
/// (the clocks go forward) moves to the first hour that does.
fn resolve(zone: &TimeZone, civil: DateTime) -> Option<Timestamp> {
    let mut civil = civil;
    for _ in 0..3 {
        let ambiguous = zone.to_ambiguous_timestamp(civil);
        match ambiguous.offset() {
            AmbiguousOffset::Gap { .. } => civil = civil.checked_add(Span::new().hours(1)).ok()?,
            _ => return ambiguous.earlier().ok().map(stamp),
        }
    }
    None
}

fn parse_time(text: &str) -> Result<(i8, i8), String> {
    let bad = || format!("“{text}” is not a time (use HH:MM).");
    let (h, m) = text.split_once(':').ok_or_else(bad)?;
    if h.len() != 2 || m.len() != 2 {
        return Err(bad());
    }
    let (h, m): (i8, i8) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return Err(bad());
    }
    Ok((h, m))
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
            let mut times: Vec<(i8, i8)> = times.iter().map(|t| parse_time(t)).collect::<Result<_, _>>()?;
            times.sort();
            let local = civil_of(after, &zone)?;
            for offset in 0..9i64 {
                let day = local.date().checked_add(Span::new().days(offset)).map_err(|e| e.to_string())?;
                let weekday = day.weekday().to_monday_one_offset() as u8;
                if !days.is_empty() && !days.iter().any(|d| d.number() as u8 == weekday) {
                    continue;
                }
                for (hour, minute) in &times {
                    if let Some(candidate) = resolve(&zone, day.at(*hour, *minute, 0, 0))
                        && candidate > after
                    {
                        return Ok(Some(candidate));
                    }
                }
            }
            Ok(None)
        }
        When::Cron { expr } => {
            let cron = Cron::parse(expr)?;
            let zone = zone(timezone)?;
            Ok(next_cron(&cron, &zone, after)?)
        }
    }
}

/// How far ahead a cron expression is looked for: enough for `29 feb`, with the century years that are not leap years.
const LOOK_AHEAD_YEARS: i64 = 9;

fn offset_seconds(zone: &TimeZone, at: Instant) -> i32 {
    zone.to_offset(at).seconds()
}

/// The next instant after `after` at which the local clock shows a minute the expression has. Instants are walked, so the clocks going
/// back or forward are met as they happen; the walk jumps over whole months, days and hours that cannot match.
fn next_cron(cron: &Cron, zone: &TimeZone, after: Timestamp) -> Result<Option<Timestamp>, String> {
    let limit = after.millis().saturating_add(LOOK_AHEAD_YEARS * 366 * 86_400_000);
    let mut ms = after.millis().div_euclid(60_000) * 60_000 + 60_000;
    // Guard against a walk that does not move (it always moves forward by at least a minute, so this only bounds the work).
    let mut steps = 0u32;
    while ms <= limit {
        steps += 1;
        if steps > 2_000_000 {
            return Ok(None);
        }
        let now = Instant::from_millisecond(ms).map_err(|_| "That moment is outside the calendar.".to_string())?;
        let local = now.to_zoned(zone.clone()).datetime();

        // The clocks jumped forward to get here. A schedule of fixed times ("at 02:30") fires now for the time that was skipped; one that
        // repeats through the day (every hour, every 15 minutes) just carries on.
        let before = Instant::from_millisecond(ms - 60_000).map_err(|e| e.to_string())?;
        if !cron.repeats_through_the_day() && offset_seconds(zone, now) > offset_seconds(zone, before) {
            let skipped_from = before
                .to_zoned(zone.clone())
                .datetime()
                .checked_add(Span::new().minutes(1))
                .map_err(|e| e.to_string())?;
            let mut minute = skipped_from;
            let mut looked = 0;
            while minute < local && looked < 24 * 60 {
                if cron.matches(&minute) {
                    return Ok(Some(stamp(now)));
                }
                minute = minute.checked_add(Span::new().minutes(1)).map_err(|e| e.to_string())?;
                looked += 1;
            }
        }

        if !cron.month_matches(&local) {
            let first =
                date(local.year(), local.month(), 1).checked_add(Span::new().months(1)).map_err(|e| e.to_string())?;
            ms = boundary(zone, first.at(0, 0, 0, 0)).max(ms + 60_000);
            continue;
        }
        if !cron.day_matches(&local) {
            let next_day = local.date().tomorrow().map_err(|e| e.to_string())?;
            ms = boundary(zone, next_day.at(0, 0, 0, 0)).max(ms + 60_000);
            continue;
        }
        if !cron.hour_matches(&local) {
            ms += i64::from(60 - local.minute()) * 60_000;
            continue;
        }
        match cron.next_minute_in_hour(local.minute()) {
            Some(minute) if minute == local.minute() => {}
            Some(minute) => {
                ms += i64::from(minute - local.minute()) * 60_000;
                continue;
            }
            None => {
                ms += i64::from(60 - local.minute()) * 60_000;
                continue;
            }
        }

        // A match. In an hour the clocks repeat, the second time through only counts for a schedule that repeats through the day.
        let ambiguous = zone.to_ambiguous_timestamp(local);
        let second_time = ambiguous.is_ambiguous() && ambiguous.later().map(|later| later == now).unwrap_or(false);
        if second_time && !cron.repeats_through_the_day() {
            ms += 60_000;
            continue;
        }
        return Ok(Some(stamp(now)));
    }
    Ok(None)
}

/// The earliest instant at which the local clock shows `civil` or later (a time that does not exist is the first instant after the jump).
fn boundary(zone: &TimeZone, civil: DateTime) -> i64 {
    let ambiguous = zone.to_ambiguous_timestamp(civil);
    ambiguous.earlier().or_else(|_| ambiguous.compatible()).map(|t| t.as_millisecond()).unwrap_or(i64::MAX / 2)
}

/// The start of the local day (00:00 in the zone) that contains `at`.
pub fn day_start(at: Timestamp, timezone: Option<&str>) -> Result<Timestamp, String> {
    let zone = zone(timezone)?;
    let local = civil_of(at, &zone)?;
    resolve(&zone, local.date().at(0, 0, 0, 0)).ok_or_else(|| "That day has no midnight.".to_string())
}

/// The start of the local week (Monday 00:00 in the zone) that contains `at`.
pub fn week_start(at: Timestamp, timezone: Option<&str>) -> Result<Timestamp, String> {
    let zone = zone(timezone)?;
    let local = civil_of(at, &zone)?;
    let back = i64::from(local.weekday().to_monday_zero_offset());
    let monday = local.date().checked_add(Span::new().days(-back)).map_err(|e| e.to_string())?;
    resolve(&zone, monday.at(0, 0, 0, 0)).ok_or_else(|| "That week has no Monday midnight.".to_string())
}

/// A local day as `YYYY-MM-DD`.
pub fn local_date(at: Timestamp, timezone: Option<&str>) -> Result<String, String> {
    let zone = zone(timezone)?;
    let day = civil_of(at, &zone)?.date();
    Ok(format!("{:04}-{:02}-{:02}", day.year(), day.month(), day.day()))
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
    use zorix_core::Day;

    use super::*;

    fn at(zone: &str, y: i16, m: i8, d: i8, h: i8, min: i8) -> Timestamp {
        let tz = TimeZone::get(zone).unwrap();
        stamp(tz.to_ambiguous_timestamp(date(y, m, d).at(h, min, 0, 0)).earlier().unwrap())
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

    /// A tiny generator, so the expressions are the same on every run.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    fn random_field(rng: &mut Lcg, min: u64, max: u64) -> String {
        let value = |rng: &mut Lcg| min + rng.below(max - min + 1);
        match rng.below(6) {
            0 => "*".into(),
            1 => format!("*/{}", 1 + rng.below(max.max(4) / 2)),
            2 => format!("{}", value(rng)),
            3 => {
                let (a, b) = (value(rng), value(rng));
                format!("{}-{}", a.min(b), a.max(b))
            }
            4 => format!("{},{}", value(rng), value(rng)),
            _ => {
                let (a, b) = (value(rng), value(rng));
                format!("{}-{}/{}", a.min(b), a.max(b), 1 + rng.below(4))
            }
        }
    }

    #[test]
    fn the_next_fire_is_the_first_minute_that_matches_whatever_the_expression() {
        // In a zone with no clock changes, "next" must be what a minute-by-minute search finds. Many random expressions, three zones.
        let mut rng = Lcg(7);
        let mut checked = 0;
        for _ in 0..220 {
            let expr = format!(
                "{} {} {} {} {}",
                random_field(&mut rng, 0, 59),
                random_field(&mut rng, 0, 23),
                random_field(&mut rng, 1, 31),
                random_field(&mut rng, 1, 12),
                random_field(&mut rng, 0, 6)
            );
            let Ok(cron) = Cron::parse(&expr) else { continue };
            for zone in ["UTC", "Asia/Kolkata"] {
                let tz = TimeZone::get(zone).unwrap();
                let after = Timestamp(1_790_000_000_000 + rng.below(40 * 86_400) as i64 * 1_000);
                // The search, a minute at a time for up to 30 days.
                let mut ms = after.millis().div_euclid(60_000) * 60_000 + 60_000;
                let mut expected = None;
                for _ in 0..(30 * 24 * 60) {
                    let local = Instant::from_millisecond(ms).unwrap().to_zoned(tz.clone()).datetime();
                    if cron.matches(&local) {
                        expected = Some(Timestamp(ms));
                        break;
                    }
                    ms += 60_000;
                }
                let found = next_fire(&When::Cron { expr: expr.clone() }, Some(zone), after).unwrap();
                if let Some(expected) = expected {
                    assert_eq!(found, Some(expected), "{expr:?} in {zone} after {after:?}");
                    checked += 1;
                } else if let Some(found) = found {
                    // Nothing within 30 days by the slow search: the fast one may only have found something further away.
                    assert!(found.millis() > after.millis() + 30 * 86_400_000, "{expr:?} in {zone}: {found:?}");
                }
            }
        }
        assert!(checked > 150, "{checked} expressions were checked");
    }

    #[test]
    fn a_schedule_with_a_time_the_clocks_skip_fires_at_the_first_instant_after_and_one_that_repeats_just_carries_on() {
        let zone = "America/New_York";
        // Clocks go forward 02:00 -> 03:00 on 2026-03-08. 02:30 does not exist.
        let after = at(zone, 2026, 3, 8, 1, 50);
        let pinned = When::Cron { expr: "30 2 * * *".into() };
        assert_eq!(next_fire(&pinned, Some(zone), after).unwrap(), Some(at(zone, 2026, 3, 8, 3, 0)));
        let hourly = When::Cron { expr: "30 * * * *".into() };
        assert_eq!(next_fire(&hourly, Some(zone), after).unwrap(), Some(at(zone, 2026, 3, 8, 3, 30)));
    }

    #[test]
    fn an_hour_the_clocks_repeat_fires_once_for_a_fixed_time_and_every_time_for_a_schedule_that_repeats() {
        let zone = "Europe/Lisbon";
        // Clocks go back 02:00 WEST -> 01:00 WET on 2026-10-25: 01:30 happens twice (00:30 UTC and 01:30 UTC).
        let after = Timestamp(at("UTC", 2026, 10, 24, 12, 0).millis());
        let daily = When::Cron { expr: "30 1 * * *".into() };
        let fires = fires_between(&daily, Some(zone), after, at("UTC", 2026, 10, 27, 0, 0), 10).unwrap();
        assert_eq!(
            fires,
            vec![at("UTC", 2026, 10, 25, 0, 30), at("UTC", 2026, 10, 26, 1, 30)],
            "once on the day it happens twice"
        );
        let every_half_hour = When::Cron { expr: "*/30 1 * * *".into() };
        let fires = fires_between(
            &every_half_hour,
            Some(zone),
            at("UTC", 2026, 10, 25, 0, 0),
            at("UTC", 2026, 10, 25, 2, 0),
            10,
        )
        .unwrap();
        assert_eq!(
            fires,
            vec![at("UTC", 2026, 10, 25, 0, 30), at("UTC", 2026, 10, 25, 1, 0), at("UTC", 2026, 10, 25, 1, 30)],
            "01:30 and then 01:00 and 01:30 again, once the clocks have gone back"
        );
    }

    #[test]
    fn a_leap_day_is_found_years_ahead_and_an_impossible_date_never_comes() {
        let leap = When::Cron { expr: "0 0 29 2 *".into() };
        let next = next_fire(&leap, Some("UTC"), at("UTC", 2026, 3, 1, 0, 0)).unwrap().unwrap();
        assert_eq!(next, at("UTC", 2028, 2, 29, 0, 0));
        // The 30th of February does not exist.
        let never = When::Cron { expr: "0 0 30 2 *".into() };
        assert_eq!(next_fire(&never, Some("UTC"), at("UTC", 2026, 3, 1, 0, 0)).unwrap(), None);
    }
}
