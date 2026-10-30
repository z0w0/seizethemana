//! Captured land entry rules and controlled basic-land-type conditions.

use super::cast_pass::{entry_is_tapped, new_entry_permanent, play_land};
use super::commander_test_support::named_row;
use super::game::{GameState, condition_met, controlled_land_types, new_perm_with};
use super::hand_payment::{KnownHandPayments, known_hand_payments};
use super::model::BasicLandType::{Forest, Island, Mountain, Plains, Swamp};
use super::model::{BasicLandType, CardIdx, LandEntryCondition, SimAbilityCondition, SimDeck};
use super::oracle_ast::{AbilityRestriction, OracleAbility, OracleLandEntryCondition};
use super::oracle_parser::parse_oracle_card;
use super::scenario_test_support::{deck, row, state};
use crate::db::CardRow;

/// Checklands and the two printed basic land types that allow untapped entry.
const CHECKLANDS: [(&str, BasicLandType, BasicLandType); 10] = [
    ("Glacial Fortress", Plains, Island),
    ("Drowned Catacomb", Island, Swamp),
    ("Dragonskull Summit", Swamp, Mountain),
    ("Rootbound Crag", Mountain, Forest),
    ("Sunpetal Grove", Forest, Plains),
    ("Isolated Chapel", Plains, Swamp),
    ("Sulfur Falls", Island, Mountain),
    ("Woodland Cemetery", Swamp, Forest),
    ("Clifftop Retreat", Mountain, Plains),
    ("Hinterland Harbor", Forest, Island),
];

/// Shock lands whose optional entry payment costs two life.
const SHOCKLANDS: [&str; 10] = [
    "Hallowed Fountain",
    "Watery Grave",
    "Blood Crypt",
    "Stomping Ground",
    "Temple Garden",
    "Godless Shrine",
    "Steam Vents",
    "Overgrown Tomb",
    "Sacred Foundry",
    "Breeding Pool",
];

/// Slow lands that need two other lands before entry.
const SLOWLANDS: [&str; 10] = [
    "Deserted Beach",
    "Shipwreck Marsh",
    "Haunted Ridge",
    "Rockfall Vale",
    "Overgrown Farmland",
    "Shattered Sanctum",
    "Stormcarved Coast",
    "Deathcap Glade",
    "Sundown Pass",
    "Dreamroot Cascade",
];

/// Verge lands, their unconditional mana type, and their gated mana type.
const VERGES: [(&str, BasicLandType, BasicLandType); 10] = [
    ("Floodfarm Verge", Plains, Island),
    ("Gloomlake Verge", Island, Swamp),
    ("Blazemire Verge", Swamp, Mountain),
    ("Thornspire Verge", Mountain, Forest),
    ("Hushwood Verge", Forest, Plains),
    ("Bleachbone Verge", Swamp, Plains),
    ("Riverpyre Verge", Mountain, Island),
    ("Wastewood Verge", Forest, Swamp),
    ("Sunbillow Verge", Plains, Mountain),
    ("Willowrush Verge", Island, Forest),
];

/// Build the WUBRG mask used by the parsed land conditions.
fn type_mask(first: BasicLandType, second: BasicLandType) -> [bool; 5] {
    std::array::from_fn(|index| index == first.index() || index == second.index())
}

/// Make a controlled land without using its name to supply the subtype.
fn typed_land(kind: BasicLandType, tapped: bool) -> CardRow {
    row(
        "Controlled typed land",
        "",
        &format!("Land — {}", kind.name()),
        if tapped {
            "This land enters tapped."
        } else {
            ""
        },
    )
}

/// Put known permanents on the board before the tested land enters.
fn entry_state(cards: &SimDeck, other_lands: usize) -> GameState {
    let mut game = state(vec![0], vec![]);
    for index in 1..=other_lands {
        game.battlefield.push(new_perm_with(
            index as u32,
            cards,
            CardIdx(index as u32),
            0,
            true,
        ));
    }
    game.next_uid = 20;
    game
}

/// Run the bounded payment check with only the listed physical cards in hand.
fn hand_payments(mut rows: Vec<CardRow>, cost: &str) -> KnownHandPayments {
    rows.push(row("Land-funded spell", cost, "Sorcery", ""));
    let cards = deck(&rows);
    let hand: Vec<_> = (0..rows.len()).map(|index| CardIdx(index as u32)).collect();
    let metadata = rows
        .into_iter()
        .map(|row| (row.name.clone(), row))
        .collect();
    known_hand_payments(&cards, &hand, &metadata)
}

/// All ten captured checklands retain a separate AST entry condition before lowering.
#[test]
fn checkland_conditions_pass_through_the_ast() {
    for (name, first, second) in CHECKLANDS {
        for renamed in [false, true] {
            let mut source = named_row(name);
            if renamed {
                source.name = "Renamed checkland".into();
            }
            let oracle = parse_oracle_card(&source);
            let types = type_mask(first, second);
            assert_eq!(
                oracle.land.untapped_entry_condition,
                Some(OracleLandEntryCondition::ControlsLandTypes(types)),
                "{name}"
            );
            let cards = deck(&[source]);
            assert_eq!(
                cards.cards[0].untapped_entry_condition,
                Some(LandEntryCondition::ControlsLandTypes(types))
            );
            assert!(cards.cards[0].enters_tapped);
        }
    }
}

/// Either named subtype qualifies, even when that controlled land is tapped.
#[test]
fn checklands_use_either_controlled_type_before_entry() {
    for (name, first, second) in CHECKLANDS {
        let unrelated = BasicLandType::ALL
            .into_iter()
            .find(|kind| *kind != first && *kind != second)
            .expect("each checkland omits three basic land types");
        for (support, tapped) in [
            (typed_land(first, true), false),
            (typed_land(second, true), false),
            (typed_land(unrelated, false), true),
            (row("Typeless land", "", "Land", "{T}: Add {C}."), true),
        ] {
            let cards = deck(&[named_row(name), support]);
            let mut game = entry_state(&cards, 1);
            assert_eq!(
                entry_is_tapped(&cards, &game, CardIdx(0), true),
                tapped,
                "{name}"
            );
            assert!(play_land(&cards, &mut game, 1));
            assert_eq!(
                game.battlefield.last().expect("land entered").tapped,
                tapped
            );
            assert_eq!(game.life_paid, 0);
        }
    }
}

/// Cards in hand and the entering land itself are not controlled lands before entry.
#[test]
fn checklands_do_not_count_hand_cards_or_the_entering_source() {
    for (name, first, _) in CHECKLANDS {
        let mut source = named_row(name);
        source.type_line = format!("Land — {}", first.name());
        let cards = deck(&[source, typed_land(first, false)]);
        let mut game = state(vec![0, 1], vec![]);
        assert_eq!(controlled_land_types(&cards, &game.battlefield), [false; 5]);
        assert!(new_entry_permanent(&cards, &mut game, CardIdx(0), 1, false).tapped);
        assert_eq!(game.hand, [CardIdx(0), CardIdx(1)]);
    }
}

/// Shock and surveil lands supply printed subtypes without needing the Basic supertype.
#[test]
fn checklands_accept_shock_and_surveil_land_types() {
    for name in ["Hallowed Fountain", "Meticulous Archive"] {
        let cards = deck(&[named_row("Glacial Fortress"), named_row(name)]);
        let mut game = entry_state(&cards, 1);
        assert!(!cards.cards[1].is_basic_land);
        assert!(play_land(&cards, &mut game, 1));
        assert!(!game.battlefield.last().expect("checkland entered").tapped);
        assert_eq!(game.life_paid, 0);
    }
}

/// Hidden land backs, face-down cards, and nonland types cannot satisfy a type gate.
#[test]
fn controlled_land_types_follow_the_physical_face() {
    let cards = deck(&[
        row(
            "Spell // Typed land",
            "{1}",
            "Creature // Land — Forest",
            "// {T}: Add {G}.",
        ),
        row("Nonland subtype control", "{1}", "Artifact — Forest", ""),
        named_row("Breeding Pool"),
    ]);
    for (index, land_face, face_down, expected) in [
        (0, false, false, [false; 5]),
        (0, true, false, type_mask(Forest, Forest)),
        (0, true, true, [false; 5]),
        (1, true, false, [false; 5]),
        (2, true, true, [false; 5]),
        (2, true, false, type_mask(Forest, Island)),
    ] {
        let mut game = state(vec![], vec![]);
        let mut permanent = new_perm_with(1, &cards, CardIdx(index), 0, true);
        permanent.land_face = land_face;
        permanent.face_down = face_down;
        game.battlefield.push(permanent);
        assert_eq!(controlled_land_types(&cards, &game.battlefield), expected);
    }
}

/// Every captured shock uses its life choice, except when the destination forces tapped entry.
#[test]
fn shocklands_preserve_life_and_forced_tapped_entry() {
    for name in SHOCKLANDS {
        for (life, forced, tapped, paid) in [
            (20, false, false, 2),
            (1, false, true, 0),
            (20, true, true, 0),
        ] {
            let cards = deck(&[named_row(name)]);
            let mut game = state(vec![0], vec![]);
            game.life = life;
            let permanent = new_entry_permanent(&cards, &mut game, CardIdx(0), 1, forced);
            assert_eq!(permanent.tapped, tapped, "{name}");
            assert_eq!(game.life_paid, paid, "{name}");
            assert_eq!(game.life, life - paid as i32);
        }
    }
}

/// Every captured slow land checks the exact count before entry.
#[test]
fn slowlands_keep_the_two_other_lands_boundary() {
    for name in SLOWLANDS {
        for (count, forced, tapped) in [
            (0, false, true),
            (1, false, true),
            (2, false, false),
            (2, true, true),
        ] {
            let mut rows = vec![named_row(name)];
            rows.extend((0..count).map(|_| row("Other land", "", "Land", "")));
            let cards = deck(&rows);
            let mut game = entry_state(&cards, count);
            assert_eq!(
                new_entry_permanent(&cards, &mut game, CardIdx(0), 1, forced).tapped,
                tapped,
                "{name}, other lands={count}"
            );
            assert_eq!(game.life_paid, 0);
        }
    }
}

/// A forced tapped destination overrides a met checkland entry condition.
#[test]
fn forced_tapped_checklands_do_not_lose_their_destination_rule() {
    for (name, first, _) in CHECKLANDS {
        let cards = deck(&[named_row(name), typed_land(first, false)]);
        let mut game = entry_state(&cards, 1);
        assert!(new_entry_permanent(&cards, &mut game, CardIdx(0), 1, true).tapped);
        assert_eq!(game.life_paid, 0);
    }
}

/// Extra clauses and unsupported selectors cannot leave a usable entry fragment.
#[test]
fn checkland_near_matches_remain_tapped() {
    for condition in [
        "you control a Plains or an Island and an Elf",
        "you control another Plains or an Island",
        "you control a Plains or another Island",
        "you control a Plains and an Island",
        "you control an Elf or an Island",
        "you control a Plains or an Island and have 10 or more life",
    ] {
        let mut source = named_row("Glacial Fortress");
        source.oracle_text =
            format!("This land enters tapped unless {condition}.\n{{T}}: Add {{W}} or {{U}}.");
        let oracle = parse_oracle_card(&source);
        assert_eq!(oracle.land.untapped_entry_condition, None, "{condition}");
        let cards = deck(&[source, typed_land(Plains, false)]);
        let game = entry_state(&cards, 1);
        assert!(
            entry_is_tapped(&cards, &game, CardIdx(0), true),
            "{condition}"
        );
    }
}

/// Each Verge keeps one unconditional mode and one typed mode through lowering.
#[test]
fn verge_gates_belong_to_their_own_mana_mode() {
    for (name, first, second) in VERGES {
        let source = named_row(name);
        let oracle = parse_oracle_card(&source);
        let types = type_mask(first, second);
        let restrictions: Vec<_> = oracle
            .abilities
            .iter()
            .filter_map(|ability| {
                let OracleAbility::Activated(ability) = ability else {
                    return None;
                };
                ability.restrictions.iter().find_map(|restriction| {
                    if let AbilityRestriction::ControlsLandTypes(types) = restriction {
                        Some(*types)
                    } else {
                        None
                    }
                })
            })
            .collect();
        assert_eq!(restrictions, [types], "{name}");
        let cards = deck(&[source]);
        let abilities: Vec<_> = cards.cards[0].unlocked_abilities(0).collect();
        assert_eq!(abilities.len(), 2, "{name}");
        assert_eq!(
            abilities
                .iter()
                .filter(|ability| ability.condition.is_none())
                .count(),
            1
        );
        assert_eq!(
            abilities
                .iter()
                .filter(|ability| {
                    ability.condition == Some(SimAbilityCondition::ControlsLandTypes(types))
                })
                .count(),
            1
        );
        assert_eq!(cards.cards[0].gate_types.len(), 2);
    }
}

/// Tapped lands can open a Verge gate, but a typeless land and a hand card cannot.
#[test]
fn verge_gates_match_either_controlled_land_type() {
    for (name, first, second) in VERGES {
        let condition = SimAbilityCondition::ControlsLandTypes(type_mask(first, second));
        let unrelated = BasicLandType::ALL
            .into_iter()
            .find(|kind| *kind != first && *kind != second)
            .expect("each Verge omits three basic land types");
        for (support, expected) in [
            (typed_land(first, true), true),
            (typed_land(second, true), true),
            (typed_land(unrelated, false), false),
            (row("Typeless land", "", "Land", ""), false),
        ] {
            let cards = deck(&[named_row(name), support]);
            let game = entry_state(&cards, 1);
            assert_eq!(condition_met(&cards, &game, &condition), expected, "{name}");
            let mut game = state(vec![1], vec![]);
            game.battlefield
                .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
            assert!(!condition_met(&cards, &game, &condition), "{name}");
        }
    }
}

/// A typed Verge can satisfy its own activation gate after it enters, regardless of its name.
#[test]
fn verge_gates_include_the_controlled_source() {
    for (name, first, second) in VERGES {
        let mut source = named_row(name);
        source.name = "Renamed typed Verge".into();
        source.type_line = format!("Land — {}", first.name());
        let cards = deck(&[source]);
        let mut game = state(vec![], vec![]);
        game.battlefield
            .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
        assert!(condition_met(
            &cards,
            &game,
            &SimAbilityCondition::ControlsLandTypes(type_mask(first, second)),
        ));
    }
}

/// Unsupported Verge requirements cannot enable the gated color under a familiar name.
#[test]
fn verge_near_matches_do_not_grant_a_partial_mode() {
    for condition in [
        "you control a Plains or an Island and an Elf",
        "you control another Plains or an Island",
        "you control a Plains or another Island",
        "you control a Plains and an Island",
        "you control an Elf or an Island",
    ] {
        let mut source = named_row("Floodfarm Verge");
        source.oracle_text =
            format!("{{T}}: Add {{W}}.\n{{T}}: Add {{U}}. Activate only if {condition}.");
        let cards = deck(&[source]);
        assert_eq!(
            cards.cards[0].unlocked_abilities(0).count(),
            1,
            "{condition}"
        );
        assert!(cards.cards[0].gate_types.is_empty(), "{condition}");
    }
}

/// Checklands fund two-source costs only when one legal land order has an untapped second drop.
#[test]
fn known_hand_checklands_use_actual_land_orders_and_types() {
    for (name, first, second) in CHECKLANDS {
        for kind in [first, second] {
            let result = hand_payments(vec![named_row(name), typed_land(kind, true)], "{2}");
            assert_eq!(result.payable_spells, 1, "{name}, {}", kind.name());
            assert!(!result.skipped_requirements);
        }
        let mut second_checkland = named_row(name);
        second_checkland.name = "Second physical checkland".into();
        assert_eq!(
            hand_payments(vec![named_row(name), second_checkland], "{2}").payable_spells,
            0
        );
        assert_eq!(
            hand_payments(vec![named_row(name)], "{1}").payable_spells,
            1
        );
        let tapland = row(
            "Typeless tapland",
            "",
            "Land",
            "This land enters tapped.\n{T}: Add {C}.",
        );
        assert_eq!(
            hand_payments(vec![named_row(name), tapland], "{2}").payable_spells,
            0
        );
    }
}

/// A tapped second land can enable the first Verge without itself producing mana that turn.
#[test]
fn known_hand_verges_keep_their_type_gate_and_one_source_limit() {
    for (name, base, gated) in VERGES {
        let gated_symbol = ["W", "U", "B", "R", "G"][gated.index()];
        let result = hand_payments(vec![named_row(name)], &format!("{{{gated_symbol}}}"));
        assert_eq!(result.payable_spells, 0, "{name}");
        let result = hand_payments(
            vec![named_row(name), typed_land(base, true)],
            &format!("{{{gated_symbol}}}"),
        );
        assert_eq!(result.payable_spells, 1, "{name}");
        assert!(!result.skipped_requirements);
        let result = hand_payments(
            vec![named_row(name), typed_land(gated, true)],
            &format!("{{{gated_symbol}}}{{{gated_symbol}}}"),
        );
        assert_eq!(result.payable_spells, 1, "{name}");
        let mut typed_source = named_row(name);
        typed_source.type_line = format!("Land — {}", base.name());
        assert_eq!(
            hand_payments(vec![typed_source], "{2}").payable_spells,
            0,
            "{name}"
        );
        let typeless = row("Typeless support", "", "Land", "{T}: Add {C}.");
        assert_eq!(
            hand_payments(
                vec![named_row(name), typeless],
                &format!("{{{gated_symbol}}}")
            )
            .payable_spells,
            0,
            "{name}"
        );
    }
}
