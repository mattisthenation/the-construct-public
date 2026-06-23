//! Injectable wall-clock so idle/schedule logic is testable without sleeping.
use chrono::{DateTime, Local};

/// A source of the current local time. Real code uses `SystemClock`; tests use a fixed clock.
pub trait Clock: Send + Sync {
    fn now_local(&self) -> DateTime<Local>;
}

/// Production clock backed by the OS.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_local(&self) -> DateTime<Local> {
        Local::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// A clock pinned to a fixed instant, for deterministic tests.
    struct FixedClock(DateTime<Local>);
    impl Clock for FixedClock {
        fn now_local(&self) -> DateTime<Local> {
            self.0
        }
    }

    #[test]
    fn fixed_clock_returns_its_instant() {
        let t = Local.with_ymd_and_hms(2026, 6, 2, 13, 30, 0).unwrap();
        let c = FixedClock(t);
        assert_eq!(c.now_local(), t);
    }

    #[test]
    fn system_clock_is_monotonic_ish() {
        let a = SystemClock.now_local();
        let b = SystemClock.now_local();
        assert!(b >= a);
    }
}
