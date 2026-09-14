// Tests for the simulator report module.

use super::aggregate::aggregate;
use super::model::*;
#[test]
fn json_report_adds_station_and_body_fields() {
    let deck = SimDeck {
        cards: vec![],
        commanders: vec![super::model::SimCard {
            name: "IGS".into(),
            cost: super::model::Cost {
                pips: [1, 1, 1, 1, 1],
                ..super::model::Cost::default()
            },
            role: Role::Wincon,
            is_station_card: true,
            station_tiers: vec![Tier {
                at: 12,
                animate: true,
                abilities: vec![],
            }],
            ..super::model::SimCard::default()
        }],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let logs: Vec<super::game::GameLog> = Vec::new();
    let stats = aggregate(&logs, &deck, 10);
    let report = super::report::json_report(&stats, &deck, "test", 42, &[], 0);
    let obj = report.as_object().unwrap();
    // New additive fields exist.
    assert!(obj.contains_key("station"));
    assert!(obj.contains_key("bodies_by_turn"));
    assert!(obj.contains_key("engines_online_by_turn"));
    assert!(obj.contains_key("assumptions"));
    // Station metrics present for a spacecraft commander.
    assert!(obj["station"].is_object());
    assert!(obj["station"]["online_by_t6"].is_number());
    // Stable legacy fields still present.
    for key in [
        "name",
        "format",
        "runs",
        "turns",
        "seed",
        "deck_shape",
        "opening_hand",
        "land_drops",
        "commander",
        "mana",
        "draw",
        "role_access",
        "velocity",
        "color_screw",
        "card_castability",
        "problems",
        "summary",
    ] {
        assert!(obj.contains_key(key), "missing key {key}");
    }
}

#[test]
fn assumptions_list_documents_limits() {
    let deck = SimDeck {
        cards: vec![],
        commanders: vec![super::model::SimCard {
            name: "IGS".into(),
            cost: super::model::Cost {
                pips: [1, 1, 1, 1, 1],
                ..super::model::Cost::default()
            },
            role: Role::Wincon,
            ..super::model::SimCard::default()
        }],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let list = super::report::assumptions(&deck);
    let joined = list.join("\n");
    assert!(joined.contains("no opponents"));
    assert!(joined.contains("hybrid pips"));
    assert!(joined.contains("commander"));
    assert!(joined.contains("recast tax"));
}

// Multi-ability tap merges: one permanent, one tap, one mana
