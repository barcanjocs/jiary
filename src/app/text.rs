//! Plain text helpers: wrapping, plurals, times and durations.

/// Splits `text` into rows of at most `width` chars, breaking after the last
/// space that fits, or mid-word when a word is longer than a row. Every char
/// is kept, so the rows concatenate back to `text`. A trailing empty row is
/// added when the last one is full, so the cursor after it has a place to go.
pub(super) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let chars = text.chars().collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut start = 0;
    while chars.len() - start > width {
        let window = &chars[start..start + width];
        let fits_exactly = chars[start + width] == ' ';
        let end = match window.iter().rposition(|&c| c == ' ') {
            Some(space) if !fits_exactly => start + space + 1,
            _ => start + width,
        };
        rows.push(chars[start..end].iter().collect());
        start = end;
    }
    let last = chars[start..].iter().collect::<String>();
    let full = chars.len() - start == width;
    rows.push(last);
    if full {
        rows.push(String::new());
    }
    rows
}

/// "1 session", "3 sessions".
pub(super) fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

/// A time with its day when that isn't today: "17:42", "yesterday 17:42",
/// "Fri 17:42" within the week, "23 Oct 17:42" before.
pub(super) fn fmt_when(dt: &chrono::DateTime<chrono::Utc>) -> String {
    let local = dt.with_timezone(&chrono::Local);
    let days = (chrono::Local::now().date_naive() - local.date_naive()).num_days();
    let time = local.format("%H:%M");
    match days {
        ..=0 => time.to_string(),
        1 => format!("yesterday {time}"),
        2..=6 => format!("{} {time}", local.format("%a")),
        _ => format!("{} {time}", local.format("%-d %b")),
    }
}

pub(super) fn fmt_time(dt: &chrono::DateTime<chrono::Utc>) -> String {
    dt.with_timezone(&chrono::Local).format("%H:%M").to_string()
}

pub(super) fn fmt_duration(
    start: &chrono::DateTime<chrono::Utc>,
    end: &chrono::DateTime<chrono::Utc>,
) -> String {
    fmt_hms(*end - *start)
}

/// "1h 24m" or "9m" — per-session durations and the header total.
pub(super) fn fmt_hms(dur: chrono::Duration) -> String {
    let h = dur.num_hours();
    let m = dur.num_minutes() % 60;
    if h > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}m", m)
    }
}
