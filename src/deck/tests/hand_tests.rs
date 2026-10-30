use crate::deck::hand::{HandCard, HandReason, HandRow, advice_for, assess_hand};
use crate::deck::simulator::deal::deal_opener;
use crate::deck::simulator::format::rules_for;
use crate::deck::simulator::hand_payment::{KnownHandPayments, known_hand_payments};
use crate::deck::simulator::model::{Format, Role, SimCard, SimDeck};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// A commander deck: 40 basics + spells, one commander.
fn commander_deck(cheap: usize, expensive: usize) -> SimDeck {
    let mut cards = Vec::new();
    for _ in 0..40 {
        cards.push(SimCard {
            name: "Island".into(),
            role: Role::Land,
            ..SimCard::default()
        });
    }
    for n in 0..cheap {
        cards.push(SimCard {
            name: format!("Cheap {n}"),
            has_mana_cost: true,
            role: Role::Other,
            ..SimCard::default()
        });
    }
    for n in 0..expensive {
        cards.push(SimCard {
            name: format!("Expensive {n}"),
            role: Role::Wincon,
            cost: crate::deck::simulator::model::Cost {
                generic: 6,
                ..Default::default()
            },
            ..SimCard::default()
        });
    }
    SimDeck {
        companion: None,
        cards,
        commanders: vec![SimCard {
            name: "Commander".into(),
            role: Role::Wincon,
            ..SimCard::default()
        }],
        format: Format::Commander,
        rules: rules_for("commander"),
    }
}

/// Seed reproducibility: the same seed deals identical hands in order.
#[test]
fn same_seed_deals_same_hands() {
    let deck = commander_deck(20, 20);
    let mut rng_a = ChaCha8Rng::seed_from_u64(42);
    let mut rng_b = ChaCha8Rng::seed_from_u64(42);
    for _ in 0..5 {
        let a = deal_opener(&deck, &mut rng_a);
        let b = deal_opener(&deck, &mut rng_b);
        assert_eq!(a.hand, b.hand);
        assert_eq!(a.lands, b.lands);
        assert_eq!(a.mulliganed, b.mulliganed);
    }
}

/// Hands hold at most 7 cards, and the land counts match the mulligan
/// policy's keep band for kept hands.
#[test]
fn hands_respect_the_keep_band() {
    let deck = commander_deck(20, 20);
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    for _ in 0..50 {
        let opener = deal_opener(&deck, &mut rng);
        assert!(opener.hand.len() <= 7, "hand over 7 cards");
        if !opener.mulliganed {
            assert!(
                (2..=6).contains(&opener.lands),
                "kept opener holds {} lands",
                opener.lands
            );
        }
    }
}

/// Advice strings follow the policy: red-flag hands say mulligan, good
/// hands say keep, and redrawn hands still assess the kept cards. The
/// commander keep band is 2-6, so a 6-land hand is a keep.
#[test]
fn advice_matches_policy() {
    let band = (2u8, 6u8);
    assert!(advice_for(0, false, 2, band).contains("mulligan"));
    assert!(advice_for(7, false, 2, band).contains("mulligan"));
    // 6 lands is inside the commander keep band — never "mulligan".
    assert!(advice_for(6, false, 2, band).contains("Keep"));
    assert!(advice_for(4, false, 3, band).contains("Keep"));
    assert!(advice_for(4, false, 0, band).contains("Borderline"));
    assert!(advice_for(3, true, 2, band).contains("Redrawn"));
    // London formats keep only 2-5 lands: 6 lands advises a mulligan.
    let london = (2u8, 5u8);
    assert!(advice_for(6, false, 2, london).contains("mulligan"));
    assert!(advice_for(5, false, 2, london).contains("Keep"));
}

/// Early-play counting requires known mana costs and excludes expensive spells.
#[test]
fn early_plays_count_cheap_spells_only() {
    let deck = commander_deck(20, 20);
    let to_idx = |v: std::ops::Range<usize>| {
        v.map(|i| crate::deck::simulator::model::CardIdx(i as u32))
            .collect::<Vec<_>>()
    };
    let cheap = to_idx(40..45); // Cheap 0..5
    let cards = (0..20)
        .map(|n| {
            let row = row(&format!("Cheap {n}"), "{0}", "Artifact", "");
            (row.name.clone(), row)
        })
        .collect();
    assert_eq!(known_hand_payments(&deck, &cheap, &cards).payable_spells, 5);
    let expensive = to_idx(60..63); // Expensive 0..3
    assert_eq!(
        known_hand_payments(&deck, &expensive, &cards).payable_spells,
        0
    );
}

/// HandRow serializes with the documented keys.
#[test]
fn hand_row_shape() {
    let row = HandRow {
        cards: vec![HandCard {
            name: "Sol Ring".into(),
            mana_cost: "{1}".into(),
            mana_value: 1.0,
            type_line: "Artifact".into(),
        }],
        lands: 2,
        mulliganed: false,
        redraws: 0,
        bottomed_cards: Vec::new(),
        advice: "Keep this.".into(),
        assessment: assess_hand(2, 1, 7, (2, 6), false),
    };
    let json = serde_json::to_value(&row).expect("serialize");
    assert!(json.get("cards").is_some());
    assert!(json.get("lands").is_some());
    assert!(json.get("mulliganed").is_some());
    assert_eq!(json["redraws"], 0);
    assert_eq!(json["bottomed_cards"], serde_json::json!([]));
    assert!(json.get("advice").is_some());
    assert_eq!(json["assessment"]["evaluation_turn"], 2);
    assert_eq!(json["assessment"]["kept_size"], 7);
}

/// Stored data for hand tests; mechanics do not depend on fixture names.
fn row(name: &str, cost: &str, kind: &str, text: &str) -> crate::db::CardRow {
    crate::db::CardRow {
        name: name.into(),
        oracle_id: name.into(),
        mana_cost: cost.into(),
        cmc: 0.0,
        type_line: kind.into(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.into(),
        rarity: "rare".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: "tst".into(),
        collector_number: "1".into(),
        scryfall_id: name.into(),
        released_at: "2020-01-01".into(),
        game_changer: None,
        penny_rank: None,
        reserved: None,
    }
}

/// Run the bounded check against an exact set of stored card rows.
fn payable(rows: Vec<crate::db::CardRow>) -> usize {
    payments(rows).payable_spells
}

/// Keep unsupported-source evidence available for exact hand tests.
fn payments(rows: Vec<crate::db::CardRow>) -> KnownHandPayments {
    payments_with_commanders(rows, Vec::new())
}

/// Include the exact commander identity rows used by identity-limited lands.
fn payments_with_commanders(
    rows: Vec<crate::db::CardRow>,
    commanders: Vec<crate::db::CardRow>,
) -> KnownHandPayments {
    let cards = rows
        .iter()
        .chain(&commanders)
        .map(|row| (row.name.clone(), row.clone()))
        .collect();
    let deck = SimDeck {
        cards: rows
            .iter()
            .map(crate::deck::simulator::oracle_lower::parse_sim_card)
            .collect(),
        commanders: commanders
            .iter()
            .map(crate::deck::simulator::oracle_lower::parse_sim_card)
            .collect(),
        ..commander_deck(0, 0)
    };
    let hand: Vec<_> = (0..deck.cards.len())
        .map(|index| crate::deck::simulator::model::CardIdx(index as u32))
        .collect();
    known_hand_payments(&deck, &hand, &cards)
}

/// A valid land count cannot make wrong-color spells payable.
#[test]
fn wrong_color_opener_is_not_a_keep_claim() {
    let early = payable(vec![
        row("Green one", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Green two", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Black spell", "{B}{B}", "Sorcery", "Draw two cards."),
    ]);
    assert_eq!(early, 0);
    assert!(!advice_for(2, false, early, (2, 6)).contains("Keep"));
    assert!(
        assess_hand(2, early, 3, (2, 6), false)
            .reasons
            .contains(&HandReason::NoEarlyPaymentFound)
    );
}

/// One flexible land cannot pay two different colored pips at once.
#[test]
fn source_choices_are_not_independent_mana() {
    assert_eq!(
        payable(vec![
            row("Choice", "", "Land", "{T}: Add {W} or {U}."),
            row("Colorless", "", "Land", "{T}: Add {C}."),
            row("Two colors", "{W}{U}", "Sorcery", "Draw a card."),
        ]),
        0
    );
    assert_eq!(
        payable(vec![
            row("Choice", "", "Land", "{T}: Add {C} or {W}."),
            row("Two mana", "{1}{W}", "Sorcery", "Draw a card."),
        ]),
        0
    );
}

/// A tapped first land untaps on turn two; two tapped lands still give one mana.
#[test]
fn tapped_land_order_changes_two_turn_payment() {
    let tap = || row("Tapped", "", "Land", "Tapped enters tapped.\n{T}: Add {W}.");
    let spell = || row("Two mana", "{1}{W}", "Sorcery", "Draw a card.");
    assert_eq!(
        payable(vec![
            tap(),
            row("Untapped", "", "Land", "{T}: Add {C}."),
            spell()
        ]),
        1
    );
    assert_eq!(
        payable(vec![
            tap(),
            row(
                "Also tapped",
                "",
                "Land",
                "Also tapped enters tapped.\n{T}: Add {W}."
            ),
            spell()
        ]),
        0
    );
}

/// Restricted colored mana cannot pay an unrelated spell type.
#[test]
fn spending_restrictions_and_explicit_colorless_are_kept() {
    let restricted = || {
        row(
            "Restricted",
            "",
            "Land",
            "{T}: Add one mana of any color. Spend this mana only to cast creature spells.",
        )
    };
    assert_eq!(
        payable(vec![restricted(), row("Creature", "{G}", "Creature", "")]),
        1
    );
    assert_eq!(
        payable(vec![
            restricted(),
            row("Sorcery", "{G}", "Sorcery", "Draw a card.")
        ]),
        0
    );
    assert_eq!(
        payable(vec![
            row("Green", "", "Basic Land — Forest", ""),
            row("Colorless cost", "{C}", "Artifact", "")
        ]),
        0
    );
}

/// A physical MDFC cannot be both a mana source and the spell it pays for.
#[test]
fn mdfc_does_not_pay_for_itself() {
    assert_eq!(
        payable(vec![row(
            "Modal",
            "{G}",
            "Sorcery // Land",
            "Draw a card. // {T}: Add {G}."
        ),]),
        0
    );
    assert_eq!(
        payable(vec![
            row(
                "Modal",
                "{G}",
                "Sorcery // Land",
                "Draw a card. // {T}: Add {G}."
            ),
            row("Forest", "", "Basic Land — Forest", ""),
        ]),
        1
    );
}

/// Unknown or unsupported costs do not become free early plays.
#[test]
fn unsupported_costs_and_unknown_rows_do_not_establish_payment() {
    assert_eq!(
        payable(vec![
            row("Forest", "", "Basic Land — Forest", ""),
            row("No cost", "", "Sorcery", "Draw a card."),
            row("Snow", "{S}", "Artifact", ""),
            row("Hybrid", "{W/U}", "Sorcery", "Draw a card."),
        ]),
        0
    );
    let deck = commander_deck(1, 0);
    assert_eq!(
        known_hand_payments(
            &deck,
            &[crate::deck::simulator::model::CardIdx(40)],
            &Default::default()
        )
        .payable_spells,
        0
    );
}

/// A source's output does not erase its activation cost or board condition.
#[test]
fn conditional_and_paid_land_modes_do_not_get_free_credit() {
    assert_eq!(
        payable(vec![
            row("Paid mode", "", "Land", "{1}, {T}: Add {G}{G}."),
            row("Green spell", "{G}", "Creature", ""),
        ]),
        0
    );
    assert_eq!(
        payable(vec![
            row(
                "Gated mode",
                "",
                "Land",
                "{T}: Add {G}. Activate only if you control a legendary creature."
            ),
            row("Green spell", "{G}", "Creature", ""),
        ]),
        0
    );
}

/// Unsupported land-return and replacement costs cannot create surviving sources.
#[test]
fn unsupported_land_entry_does_not_fund_early_spells() {
    assert_eq!(
        payable(vec![
            row(
                "Azorius Chancery",
                "",
                "Land",
                "Azorius Chancery enters tapped.\nWhen Azorius Chancery enters, return a land you control to its owner's hand.\n{T}: Add {W}{U}."
            ),
            row("Two colors", "{W}{U}", "Sorcery", "Draw a card."),
        ]),
        0
    );
    assert_eq!(
        payable(vec![
            row(
                "Renamed Ruins",
                "",
                "Land",
                "If Renamed Ruins would enter the battlefield, sacrifice two untapped lands instead. If you do, put Renamed Ruins onto the battlefield. If you don't, put it into its owner's graveyard.\n{T}: Add {C}{C}{C}{C}."
            ),
            row("Two mana", "{2}", "Artifact", ""),
        ]),
        0
    );
}

/// The creature land cannot tap as the second drop, in either listed hand order.
#[test]
fn creature_land_needs_time_before_it_can_tap() {
    let arbor = || row("Dryad Arbor", "", "Land Creature — Forest Dryad", "");
    let thicket = || {
        row(
            "Tranquil Thicket",
            "",
            "Land",
            "Tranquil Thicket enters tapped.\n{T}: Add {G}.",
        )
    };
    let spell = || row("Double green", "{G}{G}", "Creature", "");
    assert_eq!(payable(vec![arbor(), thicket(), spell()]), 0);
    assert_eq!(payable(vec![thicket(), arbor(), spell()]), 0);
    assert_eq!(
        payable(vec![arbor(), row("One green", "{G}", "Creature", "")]),
        1
    );
    let hasty = row("Hasty Grove", "", "Land Creature — Forest Dryad", "Haste");
    assert_eq!(payable(vec![thicket(), hasty, spell()]), 1);
}

/// A colorless choice can pay C, but cannot also pay a colored pip on the same tap.
#[test]
fn colorless_choice_uses_one_source_once() {
    let source = || row("Choice", "", "Land", "{T}: Add {C} or {W}.");
    assert_eq!(
        payable(vec![source(), row("Colorless", "{C}", "Artifact", "")]),
        1
    );
    assert_eq!(
        payable(vec![source(), row("White", "{W}", "Creature", "")]),
        1
    );
    assert_eq!(
        payable(vec![source(), row("Both", "{C}{W}", "Artifact", "")]),
        0
    );
}

/// Graveyard-dependent colors do not become an unconditional rainbow source.
#[test]
fn unsupported_graveyard_colors_do_not_fund_a_keep() {
    let havens = || {
        row(
            "The Grey Havens",
            "",
            "Legendary Land",
            "{T}: Add {C}.\n{T}: Add one mana of any color among legendary creature cards in your graveyard.",
        )
    };
    assert_eq!(
        payable(vec![havens(), row("Green", "{G}", "Creature", ""),]),
        0
    );
    assert_eq!(
        payable(vec![havens(), row("Colorless", "{C}", "Artifact", "")]),
        1
    );
}

/// Hybrid and twobrid symbols use the same exact payments as the simulator.
#[test]
fn hybrid_and_twobrid_costs_use_legal_mana_choices() {
    for (cost, sources, expected) in [
        ("{W/U}", vec!["{W}"], 1),
        ("{W/U}", vec!["{U}"], 1),
        ("{W/U}", vec!["{G}"], 0),
        ("{2/W}", vec!["{W}"], 1),
        ("{2/W}", vec!["{C}"], 0),
        ("{2/W}", vec!["{C}", "{C}"], 1),
        ("{2/W}{2/W}", vec!["{W}", "{C}"], 0),
        ("{2/W}{2/W}", vec!["{W}", "{W}"], 1),
        ("{C}{W/U}", vec!["{C}", "{U}"], 1),
        ("{C}{W/U}", vec!["{W}", "{U}"], 0),
    ] {
        let mut rows: Vec<_> = sources
            .iter()
            .enumerate()
            .map(|(index, color)| {
                row(
                    &format!("Source {index}"),
                    "",
                    "Land",
                    &format!("{{T}}: Add {color}."),
                )
            })
            .collect();
        rows.push(row("Spell", cost, "Artifact", ""));
        assert_eq!(payable(rows), expected, "{cost} from {sources:?}");
    }
}

/// Phyrexian life payments do not replace the other symbols in a cost.
#[test]
fn phyrexian_costs_allow_life_payment_without_free_generic_mana() {
    assert_eq!(payable(vec![row("Phyrexian", "{W/P}", "Artifact", "")]), 1);
    assert_eq!(
        payable(vec![row("Phyrexian", "{1}{W/P}", "Artifact", "")]),
        0
    );
    assert_eq!(
        payable(vec![
            row("Colorless", "", "Land", "{T}: Add {C}."),
            row("Phyrexian", "{1}{W/P}", "Artifact", ""),
        ]),
        1
    );
    assert_eq!(
        payable(vec![
            row("Green", "", "Basic Land — Forest", ""),
            row("Phyrexian", "{W}{U/P}", "Creature", ""),
        ]),
        0
    );
}

/// Pip-only mana can pay a colored symbol but cannot pay generic demand.
#[test]
fn pip_only_mana_preserves_its_spend_rule() {
    let source = || {
        row(
            "Pip source",
            "",
            "Land",
            "{T}: Add {G}. This mana can't be spent to pay generic mana costs.",
        )
    };
    assert_eq!(
        payable(vec![source(), row("Green", "{G}", "Creature", "")]),
        1
    );
    assert_eq!(
        payable(vec![source(), row("Generic", "{1}", "Artifact", "")]),
        0
    );
    assert_eq!(
        payable(vec![
            source(),
            row("Colorless", "", "Land", "{T}: Add {C}."),
            row("Mixed", "{1}{G}", "Creature", ""),
        ]),
        1
    );
}

/// One tap of a pain land cannot produce both of its separate mana modes.
#[test]
fn pain_land_modes_do_not_reuse_the_same_land() {
    let source = || {
        row(
            "Pain land",
            "",
            "Land",
            "{T}: Add {C}.\n{T}: Add {R} or {G}. Pain land deals 1 damage to you.",
        )
    };
    assert_eq!(
        payable(vec![source(), row("Red", "{R}", "Creature", "")]),
        1
    );
    assert_eq!(
        payable(vec![source(), row("Colorless", "{C}", "Artifact", "")]),
        1
    );
    assert_eq!(
        payable(vec![source(), row("Both", "{C}{R}", "Artifact", "")]),
        0
    );
    assert_eq!(
        payable(vec![source(), row("Both colors", "{R}{G}", "Creature", "")]),
        0
    );
}

/// A supported land-type gate stays visible and needs a matching physical land.
#[test]
fn gated_land_keeps_its_typed_requirement_visible() {
    let source = row(
        "Verge",
        "",
        "Land",
        "{T}: Add {B}.\n{T}: Add {R}. Activate only if you control a Swamp or a Mountain.",
    );
    let card = crate::deck::simulator::oracle_lower::parse_sim_card(&source);
    assert!(card.unlocked_abilities(0).any(|ability| ability.condition
        == Some(
            crate::deck::simulator::model::SimAbilityCondition::ControlsLandTypes([
                false, false, true, true, false
            ],)
        )));
    for (type_line, payable) in [
        ("Land", 0),
        ("Land — Swamp", 1),
        ("Land — Mountain", 1),
        ("Land — Forest", 0),
    ] {
        let result = payments(vec![
            source.clone(),
            row("Gate control", "", type_line, "{T}: Add {C}."),
            row("Red", "{R}", "Creature", ""),
        ]);
        assert_eq!(result.payable_spells, payable, "{type_line}");
        assert!(!result.skipped_requirements, "{type_line}");
        assert!(
            !assess_hand(2, 0, 3, (2, 6), result.skipped_requirements)
                .reasons
                .contains(&HandReason::UnsupportedPaymentRequirements)
        );
    }
}

/// Hand advice still names an activation gate that has no supported predicate.
#[test]
fn unsupported_land_gate_stays_visible_in_hand_advice() {
    let result = payments(vec![
        row(
            "Unsupported gate",
            "",
            "Land",
            "{T}: Add {B}.\n{T}: Add {R}. Activate only if you control an Elephant.",
        ),
        row("Colorless", "", "Land", "{T}: Add {C}."),
        row("Red", "{R}", "Creature", ""),
    ]);
    assert_eq!(result.payable_spells, 0);
    assert!(result.skipped_requirements);
    assert!(
        assess_hand(2, 0, 3, (2, 6), result.skipped_requirements)
            .reasons
            .contains(&HandReason::UnsupportedPaymentRequirements)
    );
}

/// Two mana of one chosen color cannot split into two different colors.
#[test]
fn one_color_mana_lots_keep_their_shared_color() {
    let source = || {
        row(
            "Double source",
            "",
            "Land",
            "{T}: Add two mana of any one color.",
        )
    };
    assert_eq!(
        payable(vec![source(), row("Two green", "{G}{G}", "Creature", "")]),
        1
    );
    assert_eq!(
        payable(vec![source(), row("Two colors", "{W}{U}", "Creature", "")]),
        0
    );
}

/// Life spent on different sources and a Phyrexian spell shares one total.
#[test]
fn life_payments_cannot_reuse_the_same_life() {
    assert_eq!(
        payable(vec![
            row("First", "", "Land", "{T}, Pay 20 life: Add {G}."),
            row("Second", "", "Land", "{T}, Pay 20 life: Add {G}."),
            row("Green", "{G}{G}", "Creature", ""),
        ]),
        0
    );
    assert_eq!(
        payable(vec![
            row("Expensive", "", "Land", "{T}, Pay 39 life: Add {C}."),
            row("Phyrexian", "{1}{W/P}", "Artifact", ""),
        ]),
        0
    );
}

/// Snow and missing source data remain unavailable rather than free.
#[test]
fn unsupported_payment_evidence_is_explicit() {
    let result = payments(vec![
        row(
            "Snow source",
            "",
            "Snow Basic Land — Plains",
            "{T}: Add {W}.",
        ),
        row("Snow cost", "{S}", "Artifact", ""),
    ]);
    assert_eq!(result.payable_spells, 0);
    assert!(result.skipped_requirements);
    let mut deck = commander_deck(1, 0);
    deck.cards[40].cost.generic = 1;
    let result = known_hand_payments(
        &deck,
        &[crate::deck::simulator::model::CardIdx(40)],
        &Default::default(),
    );
    assert_eq!(result.payable_spells, 0);
    assert!(result.skipped_requirements);
}

/// Identity-limited mana uses declared identity rather than unrestricted colors.
#[test]
fn commander_identity_mana_cannot_pay_off_identity_colors() {
    for apostrophe in ["'", "’"] {
        for (cost, expected) in [("{B}", 1), ("{U}", 1), ("{W}", 0)] {
            let source = row(
                "Identity land",
                "",
                "Land",
                &format!(
                    "{{T}}: Add one mana of any color in your commander{apostrophe}s color identity."
                ),
            );
            let mut commander = row("Leader", "{B}", "Legendary Creature", "{U}: Draw a card.");
            commander.color_identity = "[\"B\",\"U\"]".to_string();
            let result = payments_with_commanders(
                vec![source, row("Spell", cost, "Artifact", "")],
                vec![commander],
            );
            assert_eq!(
                result.payable_spells, expected,
                "{cost}, apostrophe {apostrophe}"
            );
            assert!(!result.skipped_requirements);
        }
    }
}

/// Each partner contributes its declared colors to identity-limited mana.
#[test]
fn partner_identities_share_the_identity_limited_source() {
    let mut black = row("Black leader", "{B}", "Legendary Creature", "Partner");
    black.color_identity = "[\"B\"]".to_string();
    let mut blue = row("Blue leader", "{U}", "Legendary Creature", "Partner");
    blue.color_identity = "[\"U\"]".to_string();
    let result = payments_with_commanders(
        vec![
            row(
                "Identity land",
                "",
                "Land",
                "{T}: Add one mana of any color in your commander's color identity.",
            ),
            row("Blue spell", "{U}", "Creature", ""),
            row("White spell", "{W}", "Creature", ""),
        ],
        vec![black, blue],
    );
    assert_eq!(result.payable_spells, 1);
    assert!(!result.skipped_requirements);
}

/// A known colorless identity provides no colored mana, not a rainbow choice.
#[test]
fn colorless_commander_identity_produces_no_colored_mana() {
    let commander = row("Colorless leader", "{3}", "Legendary Creature", "");
    let result = payments_with_commanders(
        vec![
            row(
                "Identity land",
                "",
                "Land",
                "{T}: Add one mana of any color in your commander's color identity.",
            ),
            row("Green spell", "{G}", "Creature", ""),
            row("Generic spell", "{1}", "Artifact", ""),
        ],
        vec![commander],
    );
    assert_eq!(result.payable_spells, 0);
    assert!(!result.skipped_requirements);
}

/// Unknown identities stay unplanned while a separate colorless mode remains legal.
#[test]
fn unknown_identity_does_not_erase_an_independent_colorless_mode() {
    for identity in ["invalid JSON", "[\"Z\"]"] {
        let mut commander = row("Leader", "{B}", "Legendary Creature", "");
        commander.color_identity = identity.to_string();
        let result = payments_with_commanders(
            vec![
                row(
                    "Mixed land",
                    "",
                    "Land",
                    "{T}: Add {C}.\n{T}: Add one mana of any color in your commander's color identity.",
                ),
                row("Black spell", "{B}", "Creature", ""),
                row("Colorless spell", "{C}", "Artifact", ""),
            ],
            vec![commander],
        );
        assert_eq!(result.payable_spells, 1);
        assert!(result.skipped_requirements);
    }
    let result = payments(vec![
        row(
            "Identity land",
            "",
            "Land",
            "{T}: Add one mana of any color in your commander's color identity.",
        ),
        row("Black spell", "{B}", "Creature", ""),
    ]);
    assert_eq!(result.payable_spells, 0);
    assert!(result.skipped_requirements);
}
