//! Check the source inventory, known rule facts, and narrow execution controls.

use super::cast_pass::fire_cast_triggers;
use super::game::{ManaPool, new_perm_with};
use super::model::CardIdx;
use super::popular_corpus_tests::{corpus, pinned_row};
use super::turn_loop_tests::{cast, deck, row, state};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// Read the audit without treating source inventory as execution evidence.
fn audit_fixture() -> Value {
    serde_json::from_str(include_str!("popular_card_fixtures/audit.json"))
        .expect("the detailed source audit parses")
}

/// Find a selected card's audit record for rule-fact assertions.
fn audited_card<'a>(audit: &'a Value, name: &str) -> &'a Value {
    audit["cards"]
        .as_array()
        .expect("cards are a list")
        .iter()
        .find(|card| card["name"] == name)
        .expect("the selected card has an audit record")
}

/// Find the exact rules paragraph that a factual assertion covers.
fn audited_ability<'a>(audit: &'a Value, name: &str, prefix: &str) -> &'a Value {
    audited_card(audit, name)["abilities"]
        .as_array()
        .expect("abilities are a list")
        .iter()
        .find(|ability| {
            ability["oracle_text"]
                .as_str()
                .expect("Oracle text is text")
                .starts_with(prefix)
        })
        .expect("the selected Oracle paragraph has an audit record")
}

/// Require one inventory record for every selected card and exact rules paragraph.
#[test]
fn popular_audit_covers_every_card_and_exact_oracle_paragraph() {
    let audit: Value = serde_json::from_str(include_str!("popular_card_fixtures/audit.json"))
        .expect("the detailed source audit parses");
    let source = corpus();
    let source_cards = source["cards"].as_array().expect("corpus cards are a list");
    let audited = audit["cards"].as_array().expect("audited cards are a list");
    assert_eq!(audited.len(), source_cards.len());
    assert_eq!(audit["summary"]["inventory_cards"], 351);
    assert_eq!(audit["summary"]["inventory_oracle_paragraphs"], 609);
    assert_eq!(audit["summary"]["lathril_cards"], 83);
    assert_eq!(audit["summary"]["edhrec_cards"], 250);
    let mut names = HashSet::new();
    for (index, card) in audited.iter().enumerate() {
        let name = card["name"].as_str().expect("reviewed card has a name");
        assert!(names.insert(name));
        let pinned = source_cards
            .iter()
            .find(|card| card["name"] == name)
            .expect("reviewed card belongs to the corpus");
        assert_eq!(card["oracle_text_sha256"], pinned["oracle_text_sha256"]);
        assert_eq!(card["name"], source_cards[index]["name"]);
        assert_eq!(card["oracle_id"], pinned["metadata"]["oracle_id"]);
        assert_eq!(card["selection_tags"], pinned["selection_tags"]);
        assert_eq!(card["whole_card_execution"], "not_established");
        assert_eq!(card["whole_card_reporting"], "not_established");
        assert!(
            !card["review_note"]
                .as_str()
                .expect("card has a specific review")
                .is_empty()
        );
        let metadata = &pinned["metadata"];
        let texts: Vec<(&str, &str)> = if let Some(faces) = metadata["card_faces"].as_array() {
            faces
                .iter()
                .map(|face| {
                    (
                        face["name"].as_str().expect("face has a name"),
                        face["oracle_text"].as_str().unwrap_or_default(),
                    )
                })
                .collect()
        } else {
            vec![(
                metadata["name"].as_str().expect("card has a name"),
                metadata["oracle_text"].as_str().unwrap_or_default(),
            )]
        };
        let abilities = card["abilities"]
            .as_array()
            .expect("reviewed abilities are a list");
        let expected: Vec<(&str, usize, &str)> = texts
            .iter()
            .flat_map(|(face, text)| {
                text.lines()
                    .enumerate()
                    .map(move |(line, text)| (*face, line + 1, text))
            })
            .collect();
        assert_eq!(
            abilities.len(),
            expected.len(),
            "{name}: source paragraph coverage"
        );
        for (ability, (face, line, text)) in abilities.iter().zip(expected) {
            assert_eq!(ability["face"], face);
            assert_eq!(ability["line"], line);
            assert_eq!(ability["oracle_text"], text);
            assert!(
                !ability["findings"]
                    .as_array()
                    .expect("ability findings are a list")
                    .is_empty()
            );
            assert!(
                !ability["affected_metrics"]
                    .as_array()
                    .expect("metrics are a list")
                    .is_empty()
            );
            for requirement in ["cost", "effect", "target", "timing", "replacement"] {
                assert!(
                    ability["obligations"][requirement].is_string(),
                    "{name}: {requirement}"
                );
            }
        }
    }
    assert_eq!(names.len(), 351);
}

/// Check finding references without treating file existence as semantic proof.
#[test]
fn popular_audit_findings_have_specific_rules_and_real_source_paths() {
    let audit: Value = serde_json::from_str(include_str!("popular_card_fixtures/audit.json"))
        .expect("the detailed source audit parses");
    let mut paths = HashSet::new();
    for card in audit["cards"].as_array().expect("cards are a list") {
        for ability in card["abilities"].as_array().expect("abilities are a list") {
            for finding in ability["findings"].as_array().expect("findings are a list") {
                assert_ne!(finding["profile"], "fallback");
                assert!(
                    finding["reason"]
                        .as_str()
                        .is_some_and(|reason| !reason.is_empty())
                );
                for path in finding["evidence_paths"]
                    .as_array()
                    .expect("paths are a list")
                {
                    paths.insert(path.as_str().expect("evidence path is text"));
                }
            }
        }
    }
    for path in paths {
        let file = path
            .split("::")
            .next()
            .expect("evidence includes a file path");
        assert!(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(file).is_file(),
            "missing evidence: {path}"
        );
    }
}

/// Parser assertions do not establish runtime execution or compiler coverage.
#[test]
fn popular_audit_keeps_parser_compiler_and_runtime_evidence_separate() {
    let audit = audit_fixture();
    for card in audit["cards"].as_array().expect("cards are a list") {
        for ability in card["abilities"].as_array().expect("abilities are a list") {
            let runtime = ability["runtime_coverage"]["tests"]
                .as_array()
                .expect("runtime tests are a list");
            assert!(
                runtime
                    .iter()
                    .all(|test| !test.as_str().expect("test name is text").contains("parser"))
            );
            assert_eq!(
                ability["execution_status"] == "tested_subset",
                !runtime.is_empty()
            );
            assert_eq!(
                ability["compiler_coverage"]["status"],
                "not_established_by_this_audit"
            );
        }
    }
    for name in ["Nature's Lore", "Three Visits", "Farseek"] {
        let ability = &audited_card(&audit, name)["abilities"][0];
        assert_eq!(ability["execution_status"], "not_established");
        assert_eq!(
            ability["parser_coverage"]["status"],
            "linked_parser_assertions_only"
        );
        assert_eq!(ability["runtime_coverage"]["status"], "not_established");
        assert_eq!(
            ability["parser_coverage"]["tests"],
            serde_json::json!([
                "popular_runtime_tests::popular_land_search_parser_keeps_matching_and_nonmatching_types"
            ])
        );
        assert!(
            ability["runtime_coverage"]["tests"]
                .as_array()
                .expect("tests are a list")
                .is_empty()
        );
    }
}

/// Equip keeps its printed payment, own-creature target, and sorcery timing.
#[test]
fn popular_audit_equip_costs_targets_and_timing_match_printed_rules() {
    let audit = audit_fixture();
    for (name, clause, cost, target) in [
        (
            "Skullclamp",
            "Equip {1}",
            "{1}",
            "Target creature you control.",
        ),
        (
            "Lightning Greaves",
            "Equip {0}",
            "{0}",
            "Target creature you control.",
        ),
        (
            "Swiftfoot Boots",
            "Equip {1}",
            "{1}",
            "Target creature you control.",
        ),
        (
            "Brotherhood Regalia",
            "Equip legendary creature {1}",
            "{1}",
            "Target legendary creature you control.",
        ),
        (
            "Brotherhood Regalia",
            "Equip {3}",
            "{3}",
            "Target creature you control.",
        ),
    ] {
        let ability = audited_ability(&audit, name, clause);
        let keyword = &ability["keyword_abilities"][0];
        assert_eq!(keyword["keyword"], "Equip");
        assert_eq!(keyword["ability_kind"], "activated");
        for facts in [&ability["obligations"], &keyword["obligations"]] {
            assert_eq!(facts["cost"], cost, "{name}: payment");
            assert_eq!(facts["target"], target, "{name}: target");
            assert_eq!(facts["timing"], "Activate only as a sorcery.");
        }
        assert_eq!(ability["execution_status"], "not_established");
    }
}

/// Cumulative upkeep adds age before offering its full per-age payment.
#[test]
fn popular_audit_remora_upkeep_is_a_trigger_with_age_scaled_payment() {
    let audit = audit_fixture();
    let ability = audited_ability(&audit, "Mystic Remora", "Cumulative upkeep");
    assert_eq!(ability["keyword_abilities"][0]["ability_kind"], "triggered");
    assert_eq!(
        ability["obligations"]["timing"],
        "At the beginning of your upkeep."
    );
    assert_eq!(
        ability["obligations"]["cost"],
        "Pay {1} for each age counter after adding the new age counter, or sacrifice this permanent."
    );
    assert_eq!(
        ability["obligations"]["effect"],
        "Put an age counter on this permanent, then offer its complete upkeep payment; sacrifice it if that payment is not made."
    );
    assert!(
        ability["affected_metrics"]
            .as_array()
            .expect("metrics are a list")
            .contains(&serde_json::json!("age_counters"))
    );
}

/// Storm uses all players' prior casts; Partner governs commander selection.
#[test]
fn popular_audit_storm_and_partner_do_not_use_combat_keyword_defaults() {
    let audit = audit_fixture();
    let storm = audited_ability(&audit, "Flusterstorm", "Storm");
    assert_eq!(
        storm["obligations"]["effect"],
        "When you cast this spell, copy it once for each spell cast before it this turn by any player. Put copies on the stack; the copies are not cast."
    );
    assert_eq!(
        storm["affected_metrics"],
        serde_json::json!([
            "spell_copies",
            "spell_history",
            "stack_order",
            "target_legality"
        ])
    );
    assert!(
        storm["obligations"]["target"]
            .as_str()
            .expect("target rules are text")
            .contains("new legal targets")
    );
    let partner = audited_ability(&audit, "Miara, Thorn of the Glade", "Partner");
    assert_eq!(
        partner["keyword_abilities"][0]["ability_kind"],
        "static_deck_construction"
    );
    assert_eq!(
        partner["affected_metrics"],
        serde_json::json!(["color_identity", "commander_selection", "deck_construction"])
    );
    assert_eq!(
        partner["obligations"]["timing"],
        "Applies during deck construction and commander selection before the game."
    );
}

/// A choice without modal bullets is not a modal ability.
#[test]
fn popular_audit_nonmodal_choices_keep_their_actual_rule_profiles() {
    let audit = audit_fixture();
    for (name, profile) in [
        ("Victimize", "targeted_recursion"),
        ("Muldrotha, the Gravetide", "permanent_type_play_permission"),
        ("Sylvan Library", "drawn_card_history_and_life_choices"),
    ] {
        for ability in audited_card(&audit, name)["abilities"]
            .as_array()
            .expect("abilities are a list")
        {
            assert_eq!(ability["is_modal_ability"], false);
            assert_eq!(ability["findings"][0]["profile"], profile);
            assert!(
                ability["findings"]
                    .as_array()
                    .expect("findings are a list")
                    .iter()
                    .all(|finding| finding["profile"] != "modal")
            );
        }
    }
    let library = audited_ability(&audit, "Sylvan Library", "At the beginning");
    assert_eq!(
        library["obligations"]["timing"],
        "At the beginning of your draw step, not your upkeep."
    );
    assert_eq!(
        audited_ability(&audit, "Abrade", "Choose one")["is_modal_ability"],
        true
    );
    assert_eq!(
        audited_ability(&audit, "Austere Command", "Choose two")["is_modal_ability"],
        true
    );
    for card in audit["cards"].as_array().expect("cards are a list") {
        let abilities = card["abilities"].as_array().expect("abilities are a list");
        for ability in abilities
            .iter()
            .filter(|ability| ability["is_modal_ability"] == true)
        {
            let group: Vec<_> = abilities
                .iter()
                .filter(|candidate| candidate["ability_group"] == ability["ability_group"])
                .collect();
            assert!(group.iter().any(|part| {
                part["oracle_text"]
                    .as_str()
                    .expect("Oracle text is text")
                    .starts_with('•')
            }));
            assert!(
                group.iter().any(|part| {
                    let text = part["oracle_text"]
                        .as_str()
                        .expect("Oracle text is text")
                        .to_ascii_lowercase();
                    !text.starts_with('•')
                        && [
                            "choose one",
                            "choose two",
                            "choose up to one",
                            "choose up to two",
                        ]
                        .iter()
                        .any(|header| text.contains(header))
                }),
                "{}: modal group needs a choice header",
                card["name"]
            );
        }
    }
}

/// Ability words, keyword actions, and reminder words are not keyword abilities.
#[test]
fn popular_audit_keyword_labels_do_not_match_actions_or_reminder_substrings() {
    let audit = audit_fixture();
    let excluded = [
        "Constellation",
        "Ferocious",
        "Landfall",
        "Magecraft",
        "Vivid",
        "Scry",
        "Mill",
        "Food",
        "Treasure",
    ];
    for card in audit["cards"].as_array().expect("cards are a list") {
        for ability in card["abilities"].as_array().expect("abilities are a list") {
            for keyword in ability["keyword_abilities"]
                .as_array()
                .expect("keywords are a list")
            {
                assert!(!excluded.contains(&keyword["keyword"].as_str().expect("keyword is text")));
                if keyword["rules_review_status"] == "unreviewed" {
                    for key in ["cost", "effect", "target", "timing", "replacement"] {
                        assert!(
                            keyword["obligations"][key]
                                .as_str()
                                .expect("obligation is text")
                                .starts_with("Unreviewed:")
                        );
                    }
                }
            }
            assert!(
                ability["findings"]
                    .as_array()
                    .expect("findings are a list")
                    .iter()
                    .all(|finding| finding["profile"] != "keyword")
            );
        }
    }
    let merchant = &audited_card(&audit, "Gray Merchant of Asphodel")["abilities"][0];
    assert!(
        merchant["keyword_abilities"]
            .as_array()
            .expect("keywords are a list")
            .is_empty()
    );
    assert!(
        !merchant["affected_metrics"]
            .as_array()
            .expect("metrics are a list")
            .contains(&serde_json::json!("evasion"))
    );
    assert_eq!(
        audited_ability(&audit, "Opt", "Scry")["keyword_action"]["name"],
        "Scry"
    );
    assert_eq!(
        audited_ability(&audit, "Setessan Champion", "Constellation")["ability_word"]["name"],
        "Constellation"
    );
}

/// Inventory coverage cannot turn open semantic rules or old hashes into support.
#[test]
fn popular_audit_scope_exposes_unfinished_rules_and_historical_engine_evidence() {
    let audit = audit_fixture();
    let review: Value = serde_json::from_str(include_str!("popular_card_fixtures/review.json"))
        .expect("the review summary parses");
    for document in [&audit, &review] {
        assert_eq!(document["status"], "source_audit_with_runtime_gaps");
        assert_eq!(
            document["engine_evidence_status"],
            "post_phase_review_required_hashes_not_refreshed"
        );
        assert!(
            !document["semantic_review_unfinished_categories"]
                .as_array()
                .expect("unfinished categories are a list")
                .is_empty()
        );
    }
    assert!(
        audit["summary"]["unreviewed_obligation_fields"]
            .as_u64()
            .expect("count is an integer")
            > 0
    );
    assert_eq!(audit["summary"]["whole_card_exact_execution_claims"], 0);
    assert_eq!(audit["summary"]["whole_card_exact_reporting_claims"], 0);
    for card in audit["cards"].as_array().expect("cards are a list") {
        assert_eq!(card["semantic_review_status"], "partial_not_complete");
        for ability in card["abilities"].as_array().expect("abilities are a list") {
            for key in ["cost", "effect", "target", "timing", "replacement"] {
                if ability["obligation_review"][key] == "unreviewed" {
                    assert!(
                        ability["obligations"][key]
                            .as_str()
                            .expect("obligation is text")
                            .starts_with("Unreviewed:")
                    );
                }
            }
        }
    }
    for snapshot in audit["inspected_source_files"]
        .as_array()
        .expect("snapshots are a list")
    {
        assert_eq!(
            snapshot["status"],
            "historical_snapshot_not_refreshed_during_active_writes"
        );
    }
    for gap in review["priority_gaps"].as_array().expect("gaps are a list") {
        assert!(gap.get("compiler_evidence").is_none());
        assert_eq!(
            gap["engine_evidence_status"],
            "pending_post_phase_recheck_not_current_verified_failure"
        );
    }
}

/// Pay for Leaf-Crowned Visionary only on an Elf cast and only with available green mana.
#[test]
fn popular_paid_elf_draw_keeps_type_filter_and_resolution_payment() {
    for renamed in [false, true] {
        for (subtype, green, expected_draws) in [("Elf", 1, 1), ("Elf", 0, 0), ("Human", 1, 0)] {
            let mut source = pinned_row("Leaf-Crowned Visionary");
            if renamed {
                source.name = "Renamed paid tribal draw".into();
            }
            let cards = deck(&[
                source,
                row("Cast object", "{0}", &format!("Creature — {subtype}"), ""),
                row("Drawn object", "{99}", "Sorcery", ""),
            ]);
            let mut game = state(vec![], vec![2]);
            game.battlefield
                .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
            let mut pool = ManaPool {
                fixed: [0, 0, 0, 0, green],
                ..ManaPool::default()
            };
            fire_cast_triggers(&cards, &mut game, &mut pool, 1, &cards[CardIdx(1)]);
            assert_eq!(game.seen, expected_draws);
            assert_eq!(game.hand.len(), expected_draws as usize);
            assert_eq!(game.library.len(), 1 - expected_draws as usize);
            assert_eq!(pool.fixed[4], green - expected_draws);
            assert_eq!(game.life, 20);
        }
    }
}

/// Creature-only cast listeners do not draw for a noncreature spell.
#[test]
fn popular_beast_whisperer_matches_creatures_not_noncreatures() {
    for (kind, expected_draws) in [("Creature — Elf", 1), ("Instant", 0)] {
        let cards = deck(&[
            pinned_row("Beast Whisperer"),
            row("Cast object", "{0}", kind, ""),
            row("Drawn object", "{99}", "Sorcery", ""),
        ]);
        let mut game = state(vec![], vec![2]);
        game.battlefield
            .push(new_perm_with(8, &cards, CardIdx(0), 0, false));
        fire_cast_triggers(
            &cards,
            &mut game,
            &mut ManaPool::default(),
            1,
            &cards[CardIdx(1)],
        );
        assert_eq!(game.seen, expected_draws);
        assert_eq!(game.hand.len(), expected_draws as usize);
        assert_eq!(game.library.len(), 1 - expected_draws as usize);
        assert_eq!(game.life, 20);
    }
}

/// An enchantment cast resolves Sythis's life and draw once; other spells do neither.
#[test]
fn popular_sythis_preserves_enchantment_filter_and_both_rewards() {
    for (kind, matches) in [("Enchantment", true), ("Creature — Elf", false)] {
        let cards = deck(&[
            pinned_row("Sythis, Harvest's Hand"),
            row("Cast object", "{0}", kind, ""),
            row("Drawn object", "{99}", "Sorcery", ""),
        ]);
        let mut game = state(vec![], vec![2]);
        game.battlefield
            .push(new_perm_with(9, &cards, CardIdx(0), 0, false));
        fire_cast_triggers(
            &cards,
            &mut game,
            &mut ManaPool::default(),
            1,
            &cards[CardIdx(1)],
        );
        assert_eq!(game.life, 20 + i32::from(matches));
        assert_eq!(game.life_gained, u32::from(matches));
        assert_eq!(game.seen, u32::from(matches));
        assert_eq!(game.hand.len(), usize::from(matches));
    }
}

/// A fixed-count draw spell pays its cost and moves exactly three library copies.
#[test]
fn popular_harmonize_draws_three_and_conserves_physical_copies() {
    let mut rows = vec![pinned_row("Harmonize")];
    rows.extend((0..4).map(|_| row("Expensive object", "{99}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut game = state(vec![0], vec![1, 2, 3, 4]);
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 0, 2],
        colorless: 2,
        ..ManaPool::default()
    };
    assert_eq!(cast(&cards, &mut game, &mut pool), [4.0]);
    assert_eq!(game.hand, [CardIdx(4), CardIdx(3), CardIdx(2)]);
    assert_eq!(game.library, [CardIdx(1)]);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.seen, 3);
    assert_eq!(pool.total(), 0);
    assert_eq!(game.life, 20);
}
