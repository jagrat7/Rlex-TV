//! Numbers, durations and dates the way the pages show them.

/// `12:34` or `1:02:03`.
pub fn fmt_clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, total % 3600 / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `1.4 GiB`, `312 MiB`, `9.0 KiB`.
pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// A speed in MiB/s, as the engine reports it, as `2.3 MiB/s` (or
/// `640 KiB/s` below one).
pub fn fmt_speed(mib_per_second: f64) -> String {
    let speed = mib_per_second.max(0.0);
    if speed < 1.0 {
        format!("{:.0} KiB/s", speed * 1024.0)
    } else {
        format!("{speed:.1} MiB/s")
    }
}

/// An ISO date as `Jan 20, 2008`, or the string as it came if it isn't one.
pub fn fmt_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let date = iso.get(..10).unwrap_or(iso);
    let mut parts = date.split('-').map(str::parse::<u32>);
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Ok(year)), Some(Ok(month @ 1..=12)), Some(Ok(day @ 1..=31))) => {
            format!("{} {day}, {year}", MONTHS[month as usize - 1])
        }
        _ => date.to_string(),
    }
}

/// Minutes in a runtime such as `142 min`, `2h 22min` or `1 hr 56 min`.
pub fn runtime_minutes(runtime: &str) -> Option<u32> {
    let mut total = 0;
    let mut chars = runtime.chars().peekable();
    while let Some(c) = chars.next() {
        let Some(digit) = c.to_digit(10) else {
            continue;
        };
        let mut n = digit;
        while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
            n = n.checked_mul(10)?.checked_add(d)?;
            chars.next();
        }
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        // A number followed by `h` counts hours; anything else, minutes.
        let hours = chars.peek().is_some_and(|c| c.eq_ignore_ascii_case(&'h'));
        total += if hours { n.checked_mul(60)? } else { n };
    }
    (total > 0).then_some(total)
}

/// `S1 E3` from an episode label like `S1 E3 · Pilot`.
pub fn short_episode(label: &str) -> String {
    label.split(" · ").next().unwrap_or(label).to_string()
}

/// Minutes left of a video, rounded up, as `1h 59m left`.
pub fn fmt_time_left(position_secs: f64, duration_secs: f64) -> String {
    let left = ((duration_secs - position_secs) / 60.0).ceil().max(1.0) as u32;
    format!("{} left", fmt_minutes(left))
}

/// `49m`, or `2h 22m` once it's over an hour.
pub fn fmt_minutes(minutes: u32) -> String {
    let (hours, rest) = (minutes / 60, minutes % 60);
    if minutes <= 60 {
        format!("{minutes}m")
    } else if rest == 0 {
        format!("{hours}h")
    } else {
        format!("{hours}h {rest}m")
    }
}

/// A Cinemeta runtime in the `2h 22m` style, or as it came if it can't be read.
pub fn fmt_runtime(runtime: &str) -> String {
    runtime_minutes(runtime).map_or_else(|| runtime.to_string(), fmt_minutes)
}

#[cfg(test)]
mod tests {
    use super::{fmt_date, fmt_runtime, fmt_speed, fmt_time_left};

    #[test]
    fn formats_runtimes() {
        assert_eq!(fmt_runtime("49 min"), "49m");
        assert_eq!(fmt_runtime("60 min"), "60m");
        assert_eq!(fmt_runtime("61 min"), "1h 1m");
        assert_eq!(fmt_runtime("142 min"), "2h 22m");
        assert_eq!(fmt_runtime("120 min"), "2h");
        assert_eq!(fmt_runtime("2h 22min"), "2h 22m");
        assert_eq!(fmt_runtime("1 hr 56 min"), "1h 56m");
        assert_eq!(fmt_runtime("N/A"), "N/A");
    }

    #[test]
    fn formats_time_left() {
        assert_eq!(fmt_time_left(3000.0, 10140.0), "1h 59m left");
        assert_eq!(fmt_time_left(1200.0, 2900.0), "29m left");
        assert_eq!(fmt_time_left(2899.0, 2900.0), "1m left");
    }

    #[test]
    fn formats_speeds_given_in_mib() {
        assert_eq!(fmt_speed(2.34), "2.3 MiB/s");
        assert_eq!(fmt_speed(0.5), "512 KiB/s");
        assert_eq!(fmt_speed(0.0), "0 KiB/s");
    }

    #[test]
    fn formats_dates() {
        assert_eq!(fmt_date("2008-01-20T05:00:00.000Z"), "Jan 20, 2008");
        assert_eq!(fmt_date("2008"), "2008");
    }
}
