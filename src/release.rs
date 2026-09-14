// Release gating happens at ingest: `sync` skips never-released oracle
// cards (see `scryfall::should_ingest`), so the store only ever holds cards
// that are released or upcoming reprints. `released_at` refers to the
// oracle's latest recognized printing and is display-only. This module keeps
// the date parsing/checking helpers shared by ingest and display.

/// Today's date (local) as the YYYY-MM-DD string used by SQL comparisons.
pub fn today() -> String {
    chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string()
}

/// Core check on a raw date string.
pub fn is_date_unreleased(released_at: &str) -> bool {
    let Some(date) = parse_date(released_at) else {
        return false;
    };
    is_unreleased_date(date)
}

/// Date-level check shared by [`is_date_unreleased`] and the display helper.
fn is_unreleased_date(date: chrono::NaiveDate) -> bool {
    let today = chrono::Local::now().date_naive();
    date > today
}

/// Human phrase for a stored release date: "released 2020-01-01" for past
/// dates, "releases 2026-01-01" for future ones.
pub fn release_display(released_at: &str) -> String {
    match parse_date(released_at) {
        Some(date) if is_unreleased_date(date) => format!("releases {date}"),
        Some(date) => format!("released {date}"),
        None => "no release date".to_string(),
    }
}

fn parse_date(s: &str) -> Option<chrono::NaiveDate> {
    if s.len() != 10 || s.as_bytes()[4] != b'-' {
        return None;
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn future_dates_are_unreleased() {
        let next_year = (chrono::Local::now().date_naive() + chrono::Duration::days(365))
            .format("%Y-%m-%d")
            .to_string();
        assert!(is_date_unreleased(&next_year));
    }

    #[test]
    fn past_today_and_unknown_are_released() {
        assert!(!is_date_unreleased("2020-01-01"));
        let today = chrono::Local::now()
            .date_naive()
            .format("%Y-%m-%d")
            .to_string();
        // Released today is visible.
        assert!(!is_date_unreleased(&today));
        assert!(!is_date_unreleased(""));
        assert!(!is_date_unreleased("garbage"));
    }

    #[test]
    fn display_mentions_release() {
        assert!(release_display("2020-01-01").contains("2020-01-01"));
        assert!(release_display("").contains("no release date"));
    }
}
