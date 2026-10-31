//! The longer views' periods: calendar weeks (Monday first) and months,
//! counted back from the current one. Pure date arithmetic; the view
//! itself lives in app.rs.

use chrono::{Datelike, Duration, Months, NaiveDate};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    Week,
    Month,
}

impl Period {
    /// The first and last day of the period `offset` periods from the one
    /// holding `today` (0: this one, -1: the one before, …).
    pub fn days(self, offset: i32, today: NaiveDate) -> (NaiveDate, NaiveDate) {
        match self {
            Period::Week => {
                let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
                let first = monday + Duration::weeks(offset as i64);
                (first, first + Duration::days(6))
            }
            Period::Month => {
                let this_month = today.with_day(1).unwrap_or(today);
                let first = shift_months(this_month, offset);
                let last = shift_months(first, 1) - Duration::days(1);
                (first, last)
            }
        }
    }

    /// The title for the period from `first` to `last`, e.g. "WEEK · 26 Oct
    /// – 1 Nov 2026" or "OCTOBER 2026".
    pub fn title(self, first: NaiveDate, last: NaiveDate) -> String {
        match self {
            Period::Week if first.year() == last.year() => format!(
                "WEEK · {} – {}",
                first.format("%-d %b"),
                last.format("%-d %b %Y")
            ),
            Period::Week => format!(
                "WEEK · {} – {}",
                first.format("%-d %b %Y"),
                last.format("%-d %b %Y")
            ),
            Period::Month => first.format("%B %Y").to_string().to_uppercase(),
        }
    }
}

/// `date` moved by `months` (either way); `date` is always a 1st here, so
/// it never falls off the end of a month.
fn shift_months(date: NaiveDate, months: i32) -> NaiveDate {
    let shifted = if months >= 0 {
        date.checked_add_months(Months::new(months as u32))
    } else {
        date.checked_sub_months(Months::new(months.unsigned_abs()))
    };
    shifted.unwrap_or(date)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn weeks_run_monday_to_sunday() {
        // Saturday 31 Oct 2026.
        let today = d(2026, 10, 31);
        assert_eq!(
            Period::Week.days(0, today),
            (d(2026, 10, 26), d(2026, 11, 1))
        );
        assert_eq!(
            Period::Week.days(-1, today),
            (d(2026, 10, 19), d(2026, 10, 25))
        );
        // A Monday is its own week's first day.
        assert_eq!(Period::Week.days(0, d(2026, 10, 26)).0, d(2026, 10, 26));
    }

    #[test]
    fn months_run_from_the_first_to_the_last_day() {
        let today = d(2026, 10, 31);
        assert_eq!(
            Period::Month.days(0, today),
            (d(2026, 10, 1), d(2026, 10, 31))
        );
        assert_eq!(
            Period::Month.days(-1, today),
            (d(2026, 9, 1), d(2026, 9, 30))
        );
        // Across a year, and a leap February.
        assert_eq!(
            Period::Month.days(-11, d(2028, 1, 15)),
            (d(2027, 2, 1), d(2027, 2, 28))
        );
        assert_eq!(
            Period::Month.days(1, d(2028, 1, 15)),
            (d(2028, 2, 1), d(2028, 2, 29))
        );
    }

    #[test]
    fn titles_name_the_period() {
        let (first, last) = Period::Week.days(0, d(2026, 10, 31));
        assert_eq!(
            Period::Week.title(first, last),
            "WEEK · 26 Oct – 1 Nov 2026"
        );
        let (first, last) = Period::Week.days(0, d(2026, 12, 31));
        assert_eq!(
            Period::Week.title(first, last),
            "WEEK · 28 Dec 2026 – 3 Jan 2027"
        );
        let (first, last) = Period::Month.days(0, d(2026, 10, 31));
        assert_eq!(Period::Month.title(first, last), "OCTOBER 2026");
    }
}
