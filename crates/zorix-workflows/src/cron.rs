//! Five-field cron expressions (ADR-0041): minute, hour, day of month, month and day of week.
//!
//! A field is a list of items, and an item is `*`, a number or a name, a range `a-b`, and a step `*/n` or `a-b/n`. Months and days of the
//! week may be written as names (`jan`, `mon`), in any case, and Sunday is 0 or 7. When the day of month and the day of week are both
//! restricted, a day matches if either does; when one is `*`, the other decides. Nothing else is accepted: no seconds, no `L`, `W`, `#`,
//! `?` or `@daily`, so what a schedule means is what these lines say.
//!
//! This module only says whether a minute of the calendar matches. Finding the next one, across daylight saving, is in `schedule`.

use jiff::civil::DateTime;

/// A parsed expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cron {
    minute: [bool; 60],
    hour: [bool; 24],
    day: [bool; 32],
    month: [bool; 13],
    /// Sunday is 0.
    weekday: [bool; 7],
    day_is_star: bool,
    weekday_is_star: bool,
    /// A schedule with a `*` or a step in its minute or hour repeats through the day, so it keeps its beat through an hour that happens
    /// twice; one made only of fixed times ("at 01:30") fires once.
    repeats: bool,
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// What a field is: its name for messages, the smallest and largest value, and the names it may use (the first name is `first`).
struct Field {
    name: &'static str,
    min: u32,
    max: u32,
    names: &'static [&'static str],
    first: u32,
}

const MINUTE: Field = Field { name: "minute", min: 0, max: 59, names: &[], first: 0 };
const HOUR: Field = Field { name: "hour", min: 0, max: 23, names: &[], first: 0 };
const DAY: Field = Field { name: "day of month", min: 1, max: 31, names: &[], first: 1 };
const MONTH: Field = Field { name: "month", min: 1, max: 12, names: &MONTHS, first: 1 };
const WEEKDAY: Field = Field { name: "day of week", min: 0, max: 7, names: &WEEKDAYS, first: 0 };

fn value(field: &Field, text: &str) -> Result<u32, String> {
    if let Some(at) = field.names.iter().position(|n| n.eq_ignore_ascii_case(text)) {
        return Ok(field.first + at as u32);
    }
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) || text.len() > 3 {
        return Err(format!(
            "“{text}” is not a {} (use {} to {}{}).",
            field.name,
            field.min,
            field.max,
            if field.names.is_empty() { "" } else { ", or a name" }
        ));
    }
    let number: u32 = text.parse().map_err(|_| format!("“{text}” is not a number."))?;
    if number < field.min || number > field.max {
        return Err(format!("{number} is outside the {} (use {} to {}).", field.name, field.min, field.max));
    }
    Ok(number)
}

/// The values a field lists, with the largest value of the field's own range (Sunday is 7 in the weekday field, and is folded onto 0).
fn parse_field(field: &Field, text: &str) -> Result<Vec<u32>, String> {
    let mut values = Vec::new();
    for item in text.split(',') {
        if item.is_empty() {
            return Err(format!("The {} has an empty item.", field.name));
        }
        if item.contains(['#', '?'])
            || ((item.ends_with('L') || item.ends_with('W') || item == "L" || item == "W")
                && value(field, item).is_err())
        {
            return Err(format!(
                "“{item}” is not supported: use numbers, names, ranges (1-5), lists (1,3) and steps (*/15) in the five fields."
            ));
        }
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step
                    .parse()
                    .ok()
                    .filter(|s| (1..=999).contains(s))
                    .ok_or_else(|| format!("“{step}” is not a step (use a whole number from 1)."))?;
                (range, Some(step))
            }
            None => (item, None),
        };
        let upper = |text: &str| -> Result<u32, String> {
            // `sun` at the end of a range is the seventh day, so `mon-sun` is the whole week.
            if field.name == "day of week" && text.eq_ignore_ascii_case("sun") { Ok(7) } else { value(field, text) }
        };
        let (low, high) = if range == "*" {
            (field.min, if field.name == "day of week" { 6 } else { field.max })
        } else if let Some((a, b)) = range.split_once('-') {
            let (a, b) = (value(field, a)?, upper(b)?);
            if a > b {
                return Err(format!("The range “{range}” goes backwards."));
            }
            (a, b)
        } else {
            let single = value(field, range)?;
            if step.is_some() {
                return Err(format!(
                    "“{item}” needs a range or a * before the step (for example {single}-{}/{}).",
                    field.max,
                    step.unwrap_or(1)
                ));
            }
            (single, single)
        };
        let step = step.unwrap_or(1);
        let mut v = low;
        while v <= high {
            values.push(v);
            v += step;
        }
    }
    Ok(values)
}

impl Cron {
    /// Read an expression.
    pub fn parse(expr: &str) -> Result<Self, String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err("A cron expression has five fields: minute hour day-of-month month day-of-week.".into());
        }
        let bad = |why: String| format!("“{}” is not a cron expression: {why}", expr.trim());
        let mut cron = Cron {
            minute: [false; 60],
            hour: [false; 24],
            day: [false; 32],
            month: [false; 13],
            weekday: [false; 7],
            day_is_star: fields[2] == "*",
            weekday_is_star: fields[4] == "*",
            repeats: fields[0]
                .split(',')
                .chain(fields[1].split(','))
                .any(|item| item.starts_with('*') || item.contains('/')),
        };
        for v in parse_field(&MINUTE, fields[0]).map_err(bad)? {
            cron.minute[v as usize] = true;
        }
        for v in parse_field(&HOUR, fields[1]).map_err(bad)? {
            cron.hour[v as usize] = true;
        }
        for v in parse_field(&DAY, fields[2]).map_err(bad)? {
            cron.day[v as usize] = true;
        }
        for v in parse_field(&MONTH, fields[3]).map_err(bad)? {
            cron.month[v as usize] = true;
        }
        for v in parse_field(&WEEKDAY, fields[4]).map_err(bad)? {
            cron.weekday[(v % 7) as usize] = true;
        }
        Ok(cron)
    }

    pub(crate) fn month_matches(&self, at: &DateTime) -> bool {
        self.month[at.month() as usize]
    }

    /// Whether the day of this date is one of the schedule's, with the rule for a day of month and a day of week together.
    pub(crate) fn day_matches(&self, at: &DateTime) -> bool {
        let day = self.day[at.day() as usize];
        let weekday = self.weekday[at.weekday().to_sunday_zero_offset() as usize];
        match (self.day_is_star, self.weekday_is_star) {
            (true, true) => true,
            (true, false) => weekday,
            (false, true) => day,
            (false, false) => day || weekday,
        }
    }

    pub(crate) fn hour_matches(&self, at: &DateTime) -> bool {
        self.hour[at.hour() as usize]
    }

    /// The next minute of the hour at or after `minute` that the schedule has.
    pub(crate) fn next_minute_in_hour(&self, minute: i8) -> Option<i8> {
        (minute.max(0)..60).find(|m| self.minute[*m as usize])
    }

    pub(crate) fn minute_matches(&self, at: &DateTime) -> bool {
        self.minute[at.minute() as usize]
    }

    /// Every field at once.
    pub(crate) fn matches(&self, at: &DateTime) -> bool {
        self.month_matches(at) && self.day_matches(at) && self.hour_matches(at) && self.minute_matches(at)
    }

    /// Whether the schedule keeps its beat through an hour that the clocks repeat (an hourly job does; a job at 01:30 fires once).
    pub(crate) fn repeats_through_the_day(&self) -> bool {
        self.repeats
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    fn at(y: i16, m: i8, d: i8, h: i8, min: i8) -> DateTime {
        date(y, m, d).at(h, min, 0, 0)
    }

    #[test]
    fn the_classic_forms_are_read() {
        for good in [
            "* * * * *",
            "*/5 * * * *",
            "0 8 * * 1-5",
            "0 8 * * mon-fri",
            "0 8 * * MON-FRI",
            "0 9 * * 1,3,5",
            "0 0 1 */3 *",
            "10-50/10 * * * *",
            "0 0 * * 7",
            "0 0 * * 0,7",
            "00 08 * * *",
            "  0   8  *  *  * ",
            "0\t8 * * *",
            "0 0 * jan-mar *",
            "0 0 29 feb *",
            "0 0 1-31/2 * *",
            "0 0 * * 1,1,1",
            "0 0 * * mon-sun",
            "0 5-7 * * 7",
        ] {
            assert!(Cron::parse(good).is_ok(), "{good:?}: {:?}", Cron::parse(good));
        }
    }

    #[test]
    fn what_is_not_a_classic_expression_is_refused_with_a_reason() {
        for (bad, why) in [
            ("", "five fields"),
            ("* * * *", "five fields"),
            ("* * * * * *", "five fields"),
            ("@daily", "five fields"),
            ("0 0 L * *", "not supported"),
            ("0 0 * * 5L", "not supported"),
            ("0 0 * * 1#1", "not supported"),
            ("0 0 15W * *", "not supported"),
            ("0 0 ? * 1", "not supported"),
            ("*/0 * * * *", "not a step"),
            ("5/15 * * * *", "before the step"),
            ("61 * * * *", "outside"),
            ("0 24 * * *", "outside"),
            ("0 0 0 * *", "outside"),
            ("0 0 32 * *", "outside"),
            ("0 0 * 13 *", "outside"),
            ("0 0 * * 8", "outside"),
            ("0 0 ,1 * *", "empty item"),
            ("0 0 1- * *", "not a day of month"),
            ("0 0 5-1 * *", "backwards"),
            ("a b c d e", "is not a"),
            ("0 0 * * mon-xyz", "not a day of week"),
            ("0 0 * jan/2 *", "before the step"),
            ("0 0 1/2 * *", "before the step"),
        ] {
            let error = Cron::parse(bad).unwrap_err();
            assert!(error.contains(why), "{bad:?} gave {error:?}, which should say “{why}”");
        }
    }

    #[test]
    fn steps_start_at_the_beginning_of_their_field() {
        let cron = Cron::parse("*/20 */10 */10 */4 */2").unwrap();
        assert!(cron.minute[0] && cron.minute[20] && cron.minute[40] && !cron.minute[10]);
        assert!(cron.hour[0] && cron.hour[10] && cron.hour[20] && !cron.hour[5]);
        assert!(cron.day[1] && cron.day[11] && cron.day[21] && cron.day[31] && !cron.day[10], "days start at 1");
        assert!(cron.month[1] && cron.month[5] && cron.month[9] && !cron.month[4], "months start at 1");
        assert_eq!(
            (0..7).filter(|d| cron.weekday[*d]).collect::<Vec<_>>(),
            vec![0, 2, 4, 6],
            "weekdays start at Sunday and stop at Saturday"
        );
    }

    #[test]
    fn sunday_is_zero_or_seven_and_names_work_in_any_case() {
        let sunday = Cron::parse("0 0 * * 7").unwrap();
        let other = Cron::parse("0 0 * * sUn").unwrap();
        assert_eq!(sunday, other);
        assert!(Cron::parse("0 0 * * 6-7").unwrap().weekday[0]);
        let weekend = Cron::parse("0 0 * * sat,sun").unwrap();
        assert!(
            weekend.day_matches(&at(2026, 3, 7, 0, 0))
                && weekend.day_matches(&at(2026, 3, 8, 0, 0))
                && !weekend.day_matches(&at(2026, 3, 9, 0, 0))
        );
    }

    #[test]
    fn a_day_of_month_and_a_day_of_week_together_match_either_and_a_star_leaves_it_to_the_other() {
        // 2026-03-13 is a Friday; the 6th is a Friday too; the 9th is a Monday.
        let either = Cron::parse("0 0 13 * 5").unwrap();
        assert!(
            either.day_matches(&at(2026, 3, 13, 0, 0))
                && either.day_matches(&at(2026, 3, 6, 0, 0))
                && either.day_matches(&at(2026, 3, 13, 0, 0))
        );
        assert!(!either.day_matches(&at(2026, 3, 9, 0, 0)));
        let by_day = Cron::parse("0 0 13 * *").unwrap();
        assert!(by_day.day_matches(&at(2026, 3, 13, 0, 0)) && !by_day.day_matches(&at(2026, 3, 6, 0, 0)));
        let by_weekday = Cron::parse("0 0 * * 5").unwrap();
        assert!(by_weekday.day_matches(&at(2026, 3, 6, 0, 0)) && !by_weekday.day_matches(&at(2026, 3, 9, 0, 0)));
        // A step is not a star: `*/10` with a weekday is "either".
        let stepped = Cron::parse("0 0 */10 * 1").unwrap();
        assert!(stepped.day_matches(&at(2026, 3, 11, 0, 0)) && stepped.day_matches(&at(2026, 3, 9, 0, 0)));
    }

    #[test]
    fn an_hour_field_that_repeats_is_told_from_one_that_is_fixed() {
        for repeating in ["0 * * * *", "0 */2 * * *", "0 1-5/2 * * *", "0 1,*/6 * * *", "*/30 1-3 * * *"] {
            assert!(Cron::parse(repeating).unwrap().repeats_through_the_day(), "{repeating}");
        }
        for fixed in ["30 1 * * *", "0 1,13 * * *", "0 8-18 * * *", "0 0 * * *", "15,45 2 * * *"] {
            assert!(!Cron::parse(fixed).unwrap().repeats_through_the_day(), "{fixed}");
        }
    }
}
