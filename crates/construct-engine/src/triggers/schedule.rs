//! Schedule-trigger decision core (pure). Decides whether a daily job is due,
//! including catch-up after downtime. No sleeping; the spawner (Plan 3) drives
//! the poll loop and supplies the clock + last-run timestamp.
use chrono::{DateTime, Local, NaiveTime, TimeZone};

/// The most recent firing instant at-or-before `now` for a job scheduled daily
/// at `daily_time` (local). E.g. if daily_time=01:00 and now=2026-06-02T00:30,
/// the most recent firing is 2026-06-01T01:00.
pub fn last_firing_at_or_before(now: DateTime<Local>, daily_time: NaiveTime) -> DateTime<Local> {
    // `.earliest()` (not `.single()`): for an ambiguous fall-back DST fold it
    // returns the earlier of the two valid instants; it returns None only when
    // the local time falls in a spring-forward gap. In the gap case we step back
    // a day rather than substituting `now` (which would make `today_fire <= now`
    // trivially true and spuriously double-fire on DST-transition day).
    let today_fire = Local
        .from_local_datetime(&now.date_naive().and_time(daily_time))
        .earliest();
    match today_fire {
        Some(tf) if tf <= now => tf,
        _ => {
            // today's firing is in the future (or in a DST gap) → last one was yesterday
            let yest = now.date_naive().pred_opt().unwrap_or(now.date_naive());
            Local
                .from_local_datetime(&yest.and_time(daily_time))
                .earliest()
                .unwrap_or(now)
        }
    }
}

/// True if the daily job is due to run now. `last_run` is the last successful
/// run instant (parsed from the store), or None if it has never run.
pub fn due(last_run: Option<DateTime<Local>>, daily_time: NaiveTime, now: DateTime<Local>) -> bool {
    let fire = last_firing_at_or_before(now, daily_time);
    match last_run {
        None => true,              // never run → run now (covers first launch)
        Some(last) => last < fire, // a firing instant has elapsed since last run
    }
}

/// Parse a `H:MM`/`HH:MM` 24-hour string into a `NaiveTime` (seconds = 0).
pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    NaiveTime::from_hms_opt(h, m, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }
    fn one_am() -> NaiveTime {
        NaiveTime::from_hms_opt(1, 0, 0).unwrap()
    }

    #[test]
    fn never_run_is_due() {
        assert!(due(None, one_am(), at(2026, 6, 2, 9, 0)));
    }

    #[test]
    fn not_due_when_already_ran_after_todays_firing() {
        // ran at 01:05 today; now is 09:00 today; next firing is tomorrow 01:00.
        let last = at(2026, 6, 2, 1, 5);
        assert!(!due(Some(last), one_am(), at(2026, 6, 2, 9, 0)));
    }

    #[test]
    fn catch_up_when_firing_missed() {
        // last ran yesterday 01:00; laptop asleep through today's 01:00;
        // now it is 09:00 today → today's 01:00 firing elapsed, so due.
        let last = at(2026, 6, 1, 1, 0);
        assert!(due(Some(last), one_am(), at(2026, 6, 2, 9, 0)));
    }

    #[test]
    fn not_due_before_first_firing_of_day_if_already_ran() {
        // ran yesterday 01:00; now is today 00:30 (before today's 01:00).
        let last = at(2026, 6, 1, 1, 0);
        assert!(!due(Some(last), one_am(), at(2026, 6, 2, 0, 30)));
    }

    #[test]
    fn parse_hhmm_parses_valid_times() {
        assert_eq!(parse_hhmm("01:00"), NaiveTime::from_hms_opt(1, 0, 0));
        assert_eq!(parse_hhmm("9:05"), NaiveTime::from_hms_opt(9, 5, 0));
        assert_eq!(parse_hhmm("23:59"), NaiveTime::from_hms_opt(23, 59, 0));
        assert_eq!(parse_hhmm("bad"), None);
        assert_eq!(parse_hhmm("25:00"), None);
    }

    #[test]
    fn last_firing_picks_yesterday_before_todays_time() {
        let now = at(2026, 6, 2, 0, 30);
        assert_eq!(
            last_firing_at_or_before(now, one_am()),
            at(2026, 6, 1, 1, 0)
        );
    }
}
