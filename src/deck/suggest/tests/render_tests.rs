// Render-layer tests for `deck suggest`: theme-word extraction and the
// JSON payload shape.

use super::super::Suggestion;
use crate::db::CardRow;

fn card(name: &str, type_line: &str) -> CardRow {
    CardRow {
        name: name.to_string(),
        oracle_id: format!("oid-{name}"),
        mana_cost: "{2}".into(),
        cmc: 2.0,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: r#"["G"]"#.into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: String::new(),
        rarity: "common".into(),
        edhrec_rank: Some(100),
        legalities: "{}".into(),
        set_code: "TST".into(),
        collector_number: "1".into(),
        scryfall_id: String::new(),
        released_at: String::new(),
        game_changer: None,
    }
}

fn suggestion(name: &str, type_line: &str, owned: i64, price: Option<f64>) -> Suggestion {
    Suggestion {
        card: card(name, type_line),
        owned,
        price_usd: price,
        tags: vec!["ramp".into()],
        score: 0.5,
    }
}

// deck_theme_words

#[test]
fn theme_words_rank_by_frequency_and_skip_generics() {
    let cards: std::collections::HashMap<String, CardRow> = [(
        "Frog Beast".to_string(),
        card("Frog Beast", "Creature — Frog Beast"),
    )]
    .into_iter()
    .collect();
    let deck = crate::deck::Deck::parse("// DECK\n2 Frog Beast\n1 Bolt\n").unwrap();
    // Bolt is unresolvable (not in the map) so contributes nothing; Frog
    // and Beast tie at 2 copies and sort alphabetically, and the suffix
    // reads "commander".
    let words = crate::deck::suggest::render::theme_words_for_test(&deck, &cards);
    assert_eq!(words, "Beast Frog commander");
}

#[test]
fn theme_words_empty_deck_falls_back_to_tribal() {
    let cards: std::collections::HashMap<String, CardRow> = std::collections::HashMap::new();
    let deck = crate::deck::Deck::parse("// DECK\n1 Bolt\n").unwrap();
    let words = crate::deck::suggest::render::theme_words_for_test(&deck, &cards);
    assert_eq!(words, "tribal commander");
}

// suggestions_json

#[test]
fn suggestion_json_covers_every_field() {
    let suggestions = vec![suggestion(
        "Frog Chief",
        "Legendary Creature — Frog",
        2,
        Some(1.25),
    )];
    let json = crate::deck::suggest::render::json_for_test(&suggestions).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let item = &parsed[0];
    assert_eq!(item["name"], "Frog Chief");
    assert_eq!(item["owned"], 2);
    assert_eq!(item["price"], 1.25);
    assert_eq!(item["edhrec_rank"], 100);
    assert_eq!(item["tags"], serde_json::json!(["ramp"]));
    assert_eq!(item["color_identity"], serde_json::json!(["G"]));
    assert_eq!(item["score"], 0.5);
}

#[test]
fn suggestion_json_empty_list_is_empty_array() {
    let json = crate::deck::suggest::render::json_for_test(&[]).unwrap();
    assert_eq!(json, "[]");
}
