//! `remind-me` — the deterministic handler that proves the thesis.
//!
//! A note says "remind me to <task> [<when>]". The Construct parses the intent,
//! extracts an optional due date/time with plain rules (no NLP model), and writes
//! a structured reminder back into the note. **No LLM is ever invoked** — this is
//! the handler that makes "most of your agent calls didn't need to be model calls"
//! visible and checkable.
//!
//! Date parsing is deliberately scoped to the common phrasings (today/tonight,
//! tomorrow, "in N days/weeks/...", weekdays, ISO dates, "at H[:MM][am|pm]").
//! ponytail: hand-rolled rather than pulling an NLP-date crate; widen the keyword
//! set here if real notes need more forms — don't reach for a model.

use super::{ERROR_KEY, RUN_KEY, STATUS_KEY};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Weekday};
use construct_obsidian::frontmatter::Note;

/// Frontmatter keys the reminder handler writes.
pub const REMINDER_KEY: &str = "construct_reminder";
pub const REMINDER_DUE_KEY: &str = "construct_reminder_due";

#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    /// The thing to be reminded about, with any trailing time phrase stripped.
    pub task: String,
    /// Resolved due instant, if a date/time was understood.
    pub due: Option<DateTime<Local>>,
}

/// Parse a reminder out of note text relative to `now`. Returns `None` only when
/// no reminder instruction can be found at all.
pub fn parse_reminder(body: &str, now: DateTime<Local>) -> Option<Reminder> {
    let instruction = extract_instruction(body)?;
    let (task, due) = split_task_and_due(&instruction, now);
    let task = task.trim().trim_end_matches(['.', ',']).trim().to_string();
    if task.is_empty() {
        return None;
    }
    Some(Reminder { task, due })
}

/// Find the reminder instruction text: the part after "remind me to|about|:" (or
/// "remind me") on the first line that mentions it. Falls back to a bare
/// "reminder: X" line. Case-insensitive.
fn extract_instruction(body: &str) -> Option<String> {
    for raw in body.lines() {
        let line = raw.trim().trim_start_matches(['-', '*', '#', ' ']).trim();
        let lower = line.to_lowercase();
        if let Some(idx) = lower.find("remind me") {
            let after = &line[idx + "remind me".len()..];
            let after_lower = after.to_lowercase();
            for lead in [" to ", " about ", ": ", " that "] {
                if let Some(stripped) = after_lower.strip_prefix(lead) {
                    let start = after.len() - stripped.len();
                    return Some(strip_inline_tags(&after[start..]));
                }
            }
            // "remind me <X>" with no connector word.
            let rest = after.trim_start_matches([':', ' ']);
            if !rest.trim().is_empty() {
                return Some(strip_inline_tags(rest));
            }
        }
        if let Some(rest) = lower.strip_prefix("reminder:") {
            let start = line.len() - rest.len();
            return Some(strip_inline_tags(&line[start..]));
        }
    }
    None
}

/// Drop trailing inline #tags (e.g. the trigger tag) from an instruction line.
fn strip_inline_tags(s: &str) -> String {
    s.split_whitespace()
        .filter(|tok| !tok.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split an instruction into (task, due). Scans for a time phrase introduced by
/// a connector (on/at/by/in/this/next/tomorrow/...) and resolves it against `now`.
fn split_task_and_due(
    instruction: &str,
    now: DateTime<Local>,
) -> (String, Option<DateTime<Local>>) {
    let words: Vec<&str> = instruction.split_whitespace().collect();
    // Try every suffix starting position; take the EARLIEST one that parses to a
    // due instant, so "call mom tomorrow at 5pm" splits task="call mom".
    for start in 0..words.len() {
        // Only treat as a time phrase if it begins with a recognized connector or keyword,
        // so we don't swallow task words like "to buy milk".
        if !starts_with_time_cue(&words[start..]) {
            continue;
        }
        let phrase = words[start..].join(" ");
        if let Some(due) = parse_due(&phrase, now) {
            let task = words[..start].join(" ");
            return (task, Some(due));
        }
    }
    (instruction.to_string(), None)
}

/// Does this token slice begin with a word that could introduce a time?
fn starts_with_time_cue(words: &[&str]) -> bool {
    let Some(first) = words.first() else {
        return false;
    };
    let w = first
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    matches!(
        w.as_str(),
        "on" | "at" | "by" | "in" | "this" | "next" | "today" | "tonight" | "tomorrow"
    ) || parse_weekday(&w).is_some()
        || NaiveDate::parse_from_str(&w, "%Y-%m-%d").is_ok()
}

/// Resolve a time phrase like "tomorrow at 5pm" / "in 3 days" / "next monday" /
/// "2026-07-01 at 09:00" to a concrete local instant. Returns None if nothing parses.
pub fn parse_due(phrase: &str, now: DateTime<Local>) -> Option<DateTime<Local>> {
    let lower = phrase.to_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    let today = now.date_naive();

    // --- time-of-day component ("at H[:MM][am|pm]" or a bare "5pm") ---
    let time = parse_time(&tokens);

    // --- date component ---
    let mut date: Option<NaiveDate> = None;
    let mut tonight = false;

    // "in N unit(s)" (days/weeks/months/hours/minutes).
    if let Some(pos) = tokens.iter().position(|t| *t == "in") {
        if let (Some(nstr), Some(unit)) = (tokens.get(pos + 1), tokens.get(pos + 2)) {
            if let Ok(n) = nstr.parse::<i64>() {
                let unit = unit.trim_end_matches('s');
                match unit {
                    "minute" | "min" => return Some(now + Duration::minutes(n)),
                    "hour" | "hr" => return Some(now + Duration::hours(n)),
                    "day" => date = Some(today + Duration::days(n)),
                    "week" => date = Some(today + Duration::weeks(n)),
                    "month" => date = Some(add_months(today, n)),
                    _ => {}
                }
            }
        }
    }

    if date.is_none() {
        for (i, tok) in tokens.iter().enumerate() {
            match *tok {
                "today" => date = Some(today),
                "tonight" => {
                    date = Some(today);
                    tonight = true;
                }
                "tomorrow" => date = Some(today + Duration::days(1)),
                "next" => {
                    if let Some(next) = tokens.get(i + 1) {
                        if *next == "week" {
                            date = Some(today + Duration::weeks(1));
                        } else if let Some(wd) = parse_weekday(next) {
                            date = Some(next_weekday(today, wd));
                        }
                    }
                }
                _ => {
                    if let Some(wd) = parse_weekday(tok) {
                        date = Some(next_weekday(today, wd));
                    } else if let Ok(d) = NaiveDate::parse_from_str(tok, "%Y-%m-%d") {
                        date = Some(d);
                    }
                }
            }
            if date.is_some() {
                break;
            }
        }
    }

    match (date, time) {
        (Some(d), Some(t)) => local_at(d, t),
        (Some(d), None) => {
            let t = if tonight {
                NaiveTime::from_hms_opt(20, 0, 0)?
            } else {
                NaiveTime::from_hms_opt(9, 0, 0)?
            };
            local_at(d, t)
        }
        // Bare time with no date → today if still future, else tomorrow.
        (None, Some(t)) => {
            let candidate = local_at(today, t)?;
            if candidate > now {
                Some(candidate)
            } else {
                local_at(today + Duration::days(1), t)
            }
        }
        (None, None) => None,
    }
}

/// Parse "at 5pm" / "at 15:30" / a bare "5pm" / "9am" from tokens.
fn parse_time(tokens: &[&str]) -> Option<NaiveTime> {
    // Prefer the token right after "at"; else scan for any clocky token.
    let at_pos = tokens.iter().position(|t| *t == "at");
    let candidates: Vec<&&str> = match at_pos {
        Some(p) => tokens.get(p + 1).into_iter().collect(),
        None => tokens.iter().collect(),
    };
    for tok in candidates {
        if let Some(t) = parse_clock_token(tok) {
            return Some(t);
        }
    }
    None
}

/// "5pm", "5:30pm", "9am", "15:00", "8" (→ 08:00).
fn parse_clock_token(tok: &str) -> Option<NaiveTime> {
    let t = tok.trim_end_matches([',', '.']);
    let (body, ampm) = if let Some(b) = t.strip_suffix("am") {
        (b, Some(false))
    } else if let Some(b) = t.strip_suffix("pm") {
        (b, Some(true))
    } else {
        (t, None)
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (body.parse::<u32>().ok()?, 0),
    };
    // A bare integer with no am/pm and no colon is only a time if it came after "at"
    // — but we already gate bare-token scanning to clocky shapes via am/pm/colon.
    if ampm.is_none() && !body.contains(':') {
        // Require am/pm or HH:MM to avoid reading "buy 2 apples" as a time.
        return None;
    }
    let h = match ampm {
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    if h > 23 || m > 59 {
        return None;
    }
    NaiveTime::from_hms_opt(h, m, 0)
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    match s.trim_end_matches([',', '.']) {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" | "tues" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" | "thurs" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

/// Nearest strictly-future date with the given weekday (today→same weekday = +7).
fn next_weekday(from: NaiveDate, target: Weekday) -> NaiveDate {
    let cur = from.weekday().num_days_from_monday() as i64;
    let tgt = target.num_days_from_monday() as i64;
    let mut delta = tgt - cur;
    if delta <= 0 {
        delta += 7;
    }
    from + Duration::days(delta)
}

/// Add `n` calendar months to a date, clamping the day to the target month's length.
fn add_months(d: NaiveDate, n: i64) -> NaiveDate {
    let total = (d.year() as i64) * 12 + (d.month0() as i64) + n;
    let year = total.div_euclid(12) as i32;
    let month0 = total.rem_euclid(12) as u32;
    let mut day = d.day();
    loop {
        if let Some(out) = NaiveDate::from_ymd_opt(year, month0 + 1, day) {
            return out;
        }
        day -= 1; // clamp Feb 30 → Feb 28/29, etc.
        if day == 0 {
            return d; // unreachable in practice
        }
    }
}

/// Combine a local date + time into a DateTime<Local>, handling DST gaps/folds.
fn local_at(d: NaiveDate, t: NaiveTime) -> Option<DateTime<Local>> {
    match Local.from_local_datetime(&d.and_time(t)) {
        chrono::LocalResult::Single(dt) => Some(dt),
        chrono::LocalResult::Ambiguous(dt, _) => Some(dt),
        chrono::LocalResult::None => Some(Local.from_utc_datetime(&d.and_time(t))),
    }
}

/// Render the human-readable reminder block. The "no model call" line makes the
/// thesis visible right in the note.
pub fn render_reminder(r: &Reminder, captured: NaiveDate) -> String {
    let due_line = match &r.due {
        Some(dt) => format!("- **Due:** {}", dt.format("%Y-%m-%d %H:%M (%a)")),
        None => "- **Due:** (no date parsed)".to_string(),
    };
    format!(
        "**⏰ Reminder:** {task}\n{due}\n- **Captured:** {captured}\n- _Handled deterministically — no model call._",
        task = r.task,
        due = due_line,
    )
}

/// Apply the reminder to the note text: managed block at top, frontmatter, done.
/// Pure transform — the whole handler is deterministic.
pub fn apply_reminder(
    text: &str,
    r: &Reminder,
    captured: NaiveDate,
    done_tag: Option<&str>,
) -> String {
    let mut note = Note::parse(text);
    note.body = construct_obsidian::block::upsert_named_at_top(
        &note.body,
        "reminder",
        &render_reminder(r, captured),
    );
    note.set_str(REMINDER_KEY, &r.task);
    if let Some(dt) = &r.due {
        note.set_str(REMINDER_DUE_KEY, &dt.to_rfc3339());
    }
    note.set_str(STATUS_KEY, "done");
    note.remove(RUN_KEY);
    note.remove(ERROR_KEY);
    if let Some(tag) = done_tag {
        if !note.body.contains(&format!("#{tag}")) {
            note.body = format!("{}\n#{}\n", note.body.trim_end(), tag);
        }
    }
    note.to_string()
}

/// The note is tagged `remind-me` but has no "remind me to …" line. That's not an
/// error — complete gracefully and say so in the note (a managed block, so it's
/// replaced cleanly if a real reminder is added later).
pub fn apply_no_reminder(text: &str, captured: NaiveDate) -> String {
    let mut note = Note::parse(text);
    note.body = construct_obsidian::block::upsert_named_at_top(
        &note.body,
        "reminder",
        &format!(
            "**⏰ Reminder:** none found\n- _No \"remind me to …\" line in this note — nothing scheduled._\n- _Checked {captured} · handled deterministically — no model call._"
        ),
    );
    note.set_str(STATUS_KEY, "done");
    note.remove(RUN_KEY);
    note.remove(ERROR_KEY);
    note.remove(REMINDER_KEY);
    note.remove(REMINDER_DUE_KEY);
    note.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    // A Monday at 10:00, for stable relative-date math.
    fn now() -> DateTime<Local> {
        at(2026, 6, 22, 10, 0) // 2026-06-22 is a Monday
    }

    #[test]
    fn parses_plain_task_no_date() {
        let r = parse_reminder("remind me to call the dentist", now()).unwrap();
        assert_eq!(r.task, "call the dentist");
        assert!(r.due.is_none());
    }

    #[test]
    fn parses_tomorrow() {
        let r = parse_reminder("remind me to water the plants tomorrow", now()).unwrap();
        assert_eq!(r.task, "water the plants");
        assert_eq!(r.due.unwrap(), at(2026, 6, 23, 9, 0));
    }

    #[test]
    fn parses_tomorrow_at_time() {
        let r = parse_reminder("Remind me to submit the report tomorrow at 5pm", now()).unwrap();
        assert_eq!(r.task, "submit the report");
        assert_eq!(r.due.unwrap(), at(2026, 6, 23, 17, 0));
    }

    #[test]
    fn parses_in_n_days() {
        let r = parse_reminder("remind me to renew the domain in 3 days", now()).unwrap();
        assert_eq!(r.task, "renew the domain");
        assert_eq!(r.due.unwrap(), at(2026, 6, 25, 9, 0));
    }

    #[test]
    fn parses_in_n_hours() {
        let r = parse_reminder("remind me to check the oven in 2 hours", now()).unwrap();
        assert_eq!(r.due.unwrap(), at(2026, 6, 22, 12, 0));
    }

    #[test]
    fn parses_next_weekday() {
        // now is Monday; "next friday" → that week's Friday (the 26th).
        let r = parse_reminder("remind me to file taxes next friday", now()).unwrap();
        assert_eq!(r.task, "file taxes");
        assert_eq!(
            r.due.unwrap().date_naive(),
            NaiveDate::from_ymd_opt(2026, 6, 26).unwrap()
        );
    }

    #[test]
    fn parses_iso_date() {
        let r = parse_reminder("remind me to renew passport on 2026-09-01", now()).unwrap();
        assert_eq!(r.task, "renew passport");
        assert_eq!(
            r.due.unwrap().date_naive(),
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()
        );
    }

    #[test]
    fn bare_time_today_or_tomorrow() {
        // 5pm is still ahead of 10:00 → today.
        let r = parse_reminder("remind me to stretch at 5pm", now()).unwrap();
        assert_eq!(r.due.unwrap(), at(2026, 6, 22, 17, 0));
        // 8am already passed → rolls to tomorrow.
        let r2 = parse_reminder("remind me to stretch at 8am", now()).unwrap();
        assert_eq!(r2.due.unwrap(), at(2026, 6, 23, 8, 0));
    }

    #[test]
    fn tonight_defaults_to_evening() {
        let r = parse_reminder("remind me to take out the bins tonight", now()).unwrap();
        assert_eq!(r.due.unwrap(), at(2026, 6, 22, 20, 0));
    }

    #[test]
    fn reminder_colon_form_and_tag_stripped() {
        let r = parse_reminder("reminder: book flights #theconstruct/remind-me", now()).unwrap();
        assert_eq!(r.task, "book flights");
    }

    #[test]
    fn no_instruction_returns_none() {
        assert!(parse_reminder("just some unrelated note text", now()).is_none());
    }

    #[test]
    fn apply_no_reminder_completes_cleanly() {
        let claimed = super::super::apply_claim("just a note", "run-1");
        let out = apply_no_reminder(&claimed, now().date_naive());
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("done"));
        assert!(note.get_str(RUN_KEY).is_none());
        assert!(note.get_str(ERROR_KEY).is_none());
        assert!(out.contains("none found"));
    }

    #[test]
    fn does_not_eat_task_words_as_time() {
        // "to buy 2 apples" — the bare "2" must not be read as a time.
        let r = parse_reminder("remind me to buy 2 apples", now()).unwrap();
        assert_eq!(r.task, "buy 2 apples");
        assert!(r.due.is_none());
    }

    #[test]
    fn apply_writes_block_and_frontmatter_done() {
        let claimed = super::super::apply_claim("remind me to call mom tomorrow", "run-1");
        let r = parse_reminder("remind me to call mom tomorrow", now()).unwrap();
        let out = apply_reminder(&claimed, &r, now().date_naive(), Some("theconstruct/done"));
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("done"));
        assert!(note.get_str(RUN_KEY).is_none());
        assert_eq!(note.get_str(REMINDER_KEY).as_deref(), Some("call mom"));
        assert!(note.get_str(REMINDER_DUE_KEY).is_some());
        assert!(out.contains("⏰ Reminder:"));
        assert!(out.contains("no model call"));
        assert!(out.contains("#theconstruct/done"));
    }

    #[test]
    fn add_months_clamps_end_of_month() {
        let jan31 = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        assert_eq!(
            add_months(jan31, 1),
            NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
        );
    }
}
