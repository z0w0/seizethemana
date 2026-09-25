// Tests for `land_colors`: producible-color parsing, fetch tables, and
// the off-color rank rules that back `deck suggest` and `deck cuts`.

use super::super::land_colors::*;
use crate::db::CardRow;

fn land(name: &str, type_line: &str, text: &str) -> CardRow {
    CardRow {
        name: name.to_string(),
        oracle_id: format!("oid-{name}"),
        mana_cost: String::new(),
        cmc: 0.0,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.to_string(),
        rarity: "rare".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: "tst".into(),
        collector_number: "1".into(),
        scryfall_id: format!("sid-{name}"),
        released_at: "2020-01-01".into(),
        game_changer: None,
    }
}

#[test]
fn fetch_table_covers_the_nine() {
    let heath = land(
        "Windswept Heath",
        "Land",
        "{T}, Pay 1 life: Search your library for a Plains or Forest card, put it onto the battlefield, then shuffle.",
    );
    let produced = land_producible_colors(&heath, "B");
    assert_eq!(
        produced,
        LandColors {
            letters: "WG".into(),
            any: false
        }
    );
    let delta = land(
        "Polluted Delta",
        "Land",
        "{T}, Pay 1 life: Search your library for an Island or Swamp card, put it onto the battlefield, then shuffle.",
    );
    let produced = land_producible_colors(&delta, "U");
    assert_eq!(produced.letters, "UB");
}

#[test]
fn basic_land_produces_its_color() {
    let swamp = land("Swamp", "Basic Land — Swamp", "");
    let produced = land_producible_colors(&swamp, "B");
    assert_eq!(produced.letters, "B");
}

#[test]
fn typed_dual_produces_both() {
    let dual = land(
        "Watery Grave",
        "Land — Swamp Island",
        "({T}: Add {U} or {B}.)",
    );
    let produced = land_producible_colors(&dual, "UB");
    assert!(produced.letters.contains('U') && produced.letters.contains('B'));
}

#[test]
fn any_color_land_matches_deck_colors() {
    let tower = land(
        "Command Tower",
        "Land",
        "{T}: Add one mana of any color in your commander's color identity.",
    );
    let produced = land_producible_colors(&tower, "UB");
    assert!(produced.any);
    assert_eq!(produced.letters, "UB");
}

#[test]
fn colorless_utility_land_is_empty() {
    let wastes = land("Wastes", "Basic Land", "{T}: Add {C}.");
    let produced = land_producible_colors(&wastes, "B");
    assert!(produced.letters.is_empty() && !produced.any);
    assert!(land_is_off_color(&wastes, "B"));
}

#[test]
fn generic_basic_fetch_lands_match_deck_colors() {
    // "basic land card" text names no type: the land fetches a basic of
    // every deck color (Myriad Landscape, Escape Tunnel), so it is
    // never off-color.
    let myriad = land(
        "Myriad Landscape",
        "Land",
        "{T}: Add {C}.\n{2}, {T}, Sacrifice Myriad Landscape: Search your library for up to two basic land cards.",
    );
    assert_eq!(land_rank(&myriad, "WB"), 0);
    assert!(!land_is_off_color(&myriad, "WB"));
    let escape = land(
        "Escape Tunnel",
        "Land",
        "{T}: Add {C}.\n{T}, Sacrifice Escape Tunnel: Search your library for a basic land card.",
    );
    assert!(!land_is_off_color(&escape, "WB"));
    // Evolving Wilds stays in the name table; its behavior is unchanged.
    let wilds = land(
        "Evolving Wilds",
        "Land",
        "{T}, Sacrifice Evolving Wilds: Search your library for a basic land card.",
    );
    assert!(!land_is_off_color(&wilds, "WB"));
    // Typed fetches still name their types: a Swamp/Forest fetcher in a
    // mono-W deck stays off-color.
    let typed = land(
        "Typed Fetch",
        "Land",
        "{T}, Sacrifice Typed Fetch: Search your library for a basic Swamp or Forest card.",
    );
    assert!(land_is_off_color(&typed, "W"));
    // Wastes never fetches: still colorless, still off-color in a B deck.
    let wastes = land("Wastes", "Basic Land", "{T}: Add {C}.");
    assert!(land_is_off_color(&wastes, "B"));
}

#[test]
fn off_color_rules_sort() {
    let heath = land(
        "Windswept Heath",
        "Land",
        "{T}, Pay 1 life: Search your library for a Plains or Forest card.",
    );
    // Mono-B deck: zero overlap → rank 2, off-color fetch (extra colors).
    assert_eq!(land_rank(&heath, "B"), 2);
    assert!(land_fetches_off_color(&heath, "B"));
    // Mono-W deck: full overlap but the Heath also fetches Forest —
    // a partial fetch ranks below basics (rank 1).
    assert_eq!(land_rank(&heath, "W"), 1);
    assert!(land_fetches_off_color(&heath, "W"));
    // UB deck: the Heath shares no colors with U-only → rank 2.
    assert_eq!(land_rank(&heath, "U"), 2);
    assert!(land_is_off_color(&heath, "B"), "zero overlap is off-color");
    // Non-land cards are never ranked.
    let bolt = land("Bolt", "Instant", "Deal 3 damage.");
    assert_eq!(land_rank(&bolt, "B"), 0);
}
