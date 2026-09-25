// Tests for ManaBox purchase-price parsing (locale-rejection rules).

use super::parse_locale_price;

#[test]
fn plain_decimals_parse() {
    assert_eq!(parse_locale_price("12.50"), Some(12.50));
    assert_eq!(parse_locale_price("1"), Some(1.0));
    assert_eq!(parse_locale_price("0.99"), Some(0.99));
}

#[test]
fn single_comma_with_short_tail_is_decimal() {
    // "1,50" is a de-DE decimal comma, accepted as 1.50.
    assert_eq!(parse_locale_price("1,50"), Some(1.50));
    assert_eq!(parse_locale_price("12,3"), Some(12.3));
}

#[test]
fn three_digit_comma_tail_is_thousands_rejected() {
    // "1,234" is ambiguous (en-US thousands) and rejected outright.
    assert_eq!(parse_locale_price("1,234"), None);
    assert_eq!(parse_locale_price("1,234.50"), None);
}

#[test]
fn locale_thousands_dots_rejected() {
    // "1.234,50" (de-DE thousands dot + decimal comma) would read as
    // 1.234 — a 1000x error — so any value carrying both separators is
    // rejected and the row prices as 0 with a visible warning.
    assert_eq!(parse_locale_price("1.234,50"), None);
}

#[test]
fn single_thousands_dot_with_three_digit_tail_rejected() {
    // A lone "1.234" is 1234 in de-DE and 1.234 in en-US: ambiguous,
    // rejected the same way a thousands comma is.
    assert_eq!(parse_locale_price("1.234"), None);
    assert_eq!(parse_locale_price("12.345"), None);
    // A zero head can never be thousands notation: real decimals pass.
    assert_eq!(parse_locale_price("0.125"), Some(0.125));
    assert_eq!(parse_locale_price("0.123"), Some(0.123));
    // Real decimals keep parsing.
    assert_eq!(parse_locale_price("1.23"), Some(1.23));
    assert_eq!(parse_locale_price("1000.5"), Some(1000.5));
}

#[test]
fn garbage_rejected_without_panic() {
    assert_eq!(parse_locale_price(""), None);
    assert_eq!(parse_locale_price("n/a"), None);
    assert_eq!(parse_locale_price(",,"), None);
    assert_eq!(parse_locale_price("12a"), None);
}
