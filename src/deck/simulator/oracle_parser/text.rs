//! Number-word and numeral extraction from Oracle text.

/// Cards drawn when a draw spell resolves: numerals and number words win
/// ("draw two cards", "draw seven cards"), otherwise 1.
pub(crate) fn draw_amount(text: &str) -> u32 {
    if !text.contains("draw ") && !text.contains("draws ") && !text.contains("investigate") {
        return 0;
    }
    for (word, n) in [
        ("ten", 10u32),
        ("nine", 9),
        ("eight", 8),
        ("seven", 7),
        ("six", 6),
        ("five", 5),
        ("four", 4),
        ("three", 3),
        ("two", 2),
        ("10", 10),
        ("9", 9),
        ("8", 8),
        ("7", 7),
        ("6", 6),
        ("5", 5),
        ("4", 4),
        ("3", 3),
        ("2", 2),
    ] {
        if text.contains(&format!("draw {word}")) || text.contains(&format!("draws {word}")) {
            return n;
        }
    }
    1
}

/// Number-word and numeral amounts for generic clauses ("scry 2",
/// "look at the top three cards"). Returns 1 when present but uncounted.
pub(crate) fn amount_after(text: &str, needle: &str) -> u32 {
    let mut from = 0;
    while let Some(rel) = text[from..].find(needle) {
        let tail = &text[from + rel + needle.len()..];
        let digits: String = tail
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<u32>() {
            return n;
        }
        for (word, n) in [
            ("ten", 10u32),
            ("nine", 9),
            ("eight", 8),
            ("seven", 7),
            ("six", 6),
            ("five", 5),
            ("four", 4),
            ("three", 3),
            ("two", 2),
            ("one", 1),
            ("a ", 1),
        ] {
            if tail.trim_start().starts_with(word) {
                return n;
            }
        }
        from += rel + needle.len();
    }
    1
}
