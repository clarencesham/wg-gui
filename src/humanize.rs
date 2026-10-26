use std::time::{SystemTime, UNIX_EPOCH};

const DAY: u64 = 86_400;
const WEEK: u64 = 7 * DAY;
const MONTH: u64 = 30 * DAY;
const YEAR: u64 = 365 * DAY;

pub fn time(timestamp: u64) -> String {
    if timestamp == 0 {
        return "never".to_string();
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if timestamp >= now {
        return "now".to_string();
    }

    let diff = now - timestamp;

    if diff < 1 {
        "now".to_string()
    } else if diff < 2 {
        "1 second ago".to_string()
    } else if diff < 60 {
        format!("{} seconds ago", diff)
    } else if diff < 120 {
        "1 minute ago".to_string()
    } else if diff < 3600 {
        format!("{} minutes ago", diff / 60)
    } else if diff < 7200 {
        "1 hour ago".to_string()
    } else if diff < DAY {
        format!("{} hours ago", diff / 3600)
    } else if diff < 2 * DAY {
        "1 day ago".to_string()
    } else if diff < WEEK {
        format!("{} days ago", diff / DAY)
    } else if diff < 2 * WEEK {
        "1 week ago".to_string()
    } else if diff < MONTH {
        format!("{} weeks ago", diff / WEEK)
    } else if diff < 2 * MONTH {
        "1 month ago".to_string()
    } else if diff < YEAR {
        format!("{} months ago", diff / MONTH)
    } else if diff < 18 * MONTH {
        "1 year ago".to_string()
    } else if diff < 2 * YEAR {
        "2 years ago".to_string()
    } else {
        format!("{} years ago", diff / YEAR)
    }
}

pub fn bytes(size: u64) -> String {
    const SIZES: [&str; 7] = ["B", "kB", "MB", "GB", "TB", "PB", "EB"];

    if size < 10 {
        return format!("{} B", size);
    }

    let base = 1000f64;
    let exponent = ((size as f64).ln() / base.ln()).floor();
    let suffix = SIZES[(exponent as usize).min(SIZES.len() - 1)];
    let value = (((size as f64) / base.powf(exponent)) * 10.0 + 0.5).floor() / 10.0;

    if value < 10.0 {
        format!("{:.1} {}", value, suffix)
    } else {
        format!("{:.0} {}", value, suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_formats_like_go_humanize() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(9), "9 B");
        assert_eq!(bytes(10), "10 B");
        assert_eq!(bytes(82854982), "83 MB");
        assert_eq!(bytes(1234), "1.2 kB");
        assert_eq!(bytes(1000), "1.0 kB");
    }

    #[test]
    fn time_zero_is_never() {
        assert_eq!(time(0), "never");
    }

    #[test]
    fn time_recent_is_now() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(time(now), "now");
    }
}
