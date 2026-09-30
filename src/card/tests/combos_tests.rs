// Tests for `stm card combos`: sorting, truncation, and rendering.

use super::*;
use crate::spellbook::{ComboPieceRow, ComboVariant};

fn variant(id: &str, popularity: Option<i64>) -> ComboVariant {
    ComboVariant {
        id: id.into(),
        produces: vec!["Win the game".into()],
        mana_value_needed: 2,
        bracket_tag: None,
        popularity,
        legalities: serde_json::from_str(r#"{"commander":true}"#).unwrap(),
    }
}

fn piece(name: &str, must_be_commander: bool) -> ComboPieceRow {
    ComboPieceRow {
        name: name.into(),
        ordinal: 0,
        zones: vec![],
        must_be_commander,
    }
}

/// Popularity ranks best first; ties break on variant id.
#[test]
fn rows_sort_by_popularity_then_id() {
    let mut rows = [
        (variant("b", Some(10)), vec![piece("A", false)]),
        (variant("a", Some(10)), vec![piece("B", false)]),
        (variant("c", None), vec![piece("C", false)]),
        (variant("d", Some(100)), vec![piece("D", false)]),
    ];
    sort_by_popularity(&mut rows);
    let ids: Vec<&str> = rows.iter().map(|(v, _)| v.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["d", "a", "b", "c"],
        "pop first, id breaks ties, unpopularity last"
    );
}

/// requires_commander on the CardCombo mirror reads the piece flags.
#[test]
fn card_combo_flags_commander_requirement() {
    let combo = CardCombo {
        requires_commander: crate::combos::requires_commander(&[piece("A", true)]),
        variant: variant("x", None),
        pieces: vec![piece("A", true)],
    };
    assert!(combo.requires_commander);
    let json = serde_json::to_value(combo.variant.popularity).unwrap();
    assert_eq!(json, serde_json::Value::Null);
}

/// Human rendering labels a commander combo and shows an empty produces
/// field without a placeholder.
#[test]
fn text_render_handles_missing_produces() {
    let mut v = variant("x", Some(5));
    v.produces.clear();
    v.bracket_tag = Some("S".into());
    let combo = CardCombo {
        requires_commander: true,
        variant: v,
        pieces: vec![piece("A", true)],
    };
    let line = combo_line(0, &combo, &crate::output::Styles::colorless());
    assert!(line.starts_with(" 1. A"), "{line}");
    assert!(line.contains("(commander)"), "commander label: {line}");
    assert!(line.contains("[S]"), "bracket tag: {line}");
    assert!(line.contains("pop 5"), "popularity: {line}");
    assert!(line.contains("legal: commander"), "legality note: {line}");
    // Empty produces leaves the arrow with nothing after it.
    assert!(line.contains("→  [S]"), "empty produces: {line}");
}
