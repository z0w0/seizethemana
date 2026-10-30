//! Tests for the offender assembly in `findings_detail`: each problem kind
//! gets its card-level causes from data the aggregation stage produced.

use crate::deck::simulator::aggregate::{SimStats, aggregate};
use crate::deck::simulator::findings::{Finding, analyze_findings};
use crate::deck::simulator::findings_detail::explain_with_threshold;
use crate::deck::simulator::format::rules_for;
use crate::deck::simulator::game::run_game;
use crate::deck::simulator::model::{Cost, Format, ManaColor, Role, SimCard, SimDeck};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// A commander deck with a known shape: `lands` basics, `cheap` 2-mana
/// spells, `expensive` 7-mana spells, one 6-mana commander.
fn deck(lands: usize, cheap: usize, expensive: usize) -> SimDeck {
    let mut cards = Vec::new();
    for _ in 0..lands {
        cards.push(SimCard {
            name: "Island".into(),
            role: Role::Land,
            ..SimCard::default()
        });
    }
    for n in 0..cheap {
        cards.push(SimCard {
            name: format!("Cheap {n}"),
            role: Role::Other,
            ..SimCard::default()
        });
    }
    for n in 0..expensive {
        cards.push(SimCard {
            name: format!("Expensive {n}"),
            role: Role::Wincon,
            cost: Cost {
                generic: 7,
                ..Cost::default()
            },
            ..SimCard::default()
        });
    }
    SimDeck {
        companion: None,
        cards,
        commanders: vec![SimCard {
            name: "Big Commander".into(),
            role: Role::Wincon,
            cost: Cost {
                generic: 6,
                ..Cost::default()
            },
            ..SimCard::default()
        }],
        format: Format::Commander,
        rules: rules_for("commander"),
    }
}

/// Simulate and aggregate a deck (few runs; problem detection only).
fn stats_for(deck: &SimDeck, runs: u32) -> SimStats {
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let logs: Vec<_> = (0..runs).map(|_| run_game(deck, &mut rng, 10)).collect();
    aggregate(&logs, deck, 10)
}

/// Color screw findings carry the worst pip blockers as offenders.
#[test]
fn color_screw_names_blockers() {
    let stats = SimStats {
        turns: 10,
        pip_blocks: vec![
            crate::deck::simulator::findings::PipBlock {
                name: "Double Blue".into(),
                color: ManaColor::Blue,
                game_share: 0.31,
            },
            crate::deck::simulator::findings::PipBlock {
                name: "Triple Red".into(),
                color: ManaColor::Blue,
                game_share: 0.12,
            },
            crate::deck::simulator::findings::PipBlock {
                name: "Other Color".into(),
                color: ManaColor::Red,
                game_share: 0.50,
            },
        ],
        ..SimStats::default()
    };
    let deck = deck(35, 20, 5);
    let mut problem = Finding {
        kind: "insufficient_color_mana",
        severity: "high",
        game_share: Some(0.31),
        color: Some(ManaColor::Blue),
        explanation: "test".to_string(),
        suggestion: "add sources".to_string(),
        evidence: Vec::new(),
    };
    explain_with_threshold(&mut problem, &stats, &deck, 0.60);
    assert_eq!(problem.evidence.len(), 2, "only this color's blockers");
    assert_eq!(problem.evidence[0].name, "Double Blue");
    assert!(problem.suggestion.contains("blue"));
    assert!(problem.suggestion.contains("Double Blue"));
}

/// Mana screw findings carry the deck's land/ramp census as offenders.
#[test]
fn mana_screw_shows_the_census() {
    let deck = deck(38, 20, 5);
    let stats = stats_for(&deck, 50);
    let mut problem = Finding {
        kind: "insufficient_land_drops",
        severity: "high",
        game_share: Some(0.25),
        color: None,
        explanation: "test".to_string(),
        suggestion: "add lands".to_string(),
        evidence: Vec::new(),
    };
    explain_with_threshold(&mut problem, &stats, &deck, 0.60);
    assert!(!problem.evidence.is_empty());
    assert!(problem.evidence[0].name.contains("land"));
}

/// Commander-late findings name the commander and its cost.
#[test]
fn commander_late_names_the_commander() {
    let deck = deck(35, 20, 5);
    let stats = stats_for(&deck, 50);
    let mut problem = Finding {
        kind: "late_commander_cast",
        severity: "medium",
        game_share: Some(0.15),
        color: None,
        explanation: "test".to_string(),
        suggestion: "add ramp".to_string(),
        evidence: Vec::new(),
    };
    explain_with_threshold(&mut problem, &stats, &deck, 0.60);
    assert_eq!(problem.evidence.len(), 1);
    assert_eq!(problem.evidence[0].name, "Big Commander");
    // The detail carries the cost and an average turn count — never a
    // percent-scaled number (a turn of 6 must not render as 600).
    let detail = &problem.evidence[0].explanation;
    assert!(detail.contains("costs 6 mana"), "{detail}");
    assert!(!detail.contains("600"), "{detail}");
}

/// The full finding analysis fills evidence automatically, so
/// any problem reaching a report carries its cause.
#[test]
fn find_problems_attaches_offenders() {
    let deck = deck(38, 20, 10);
    let stats = stats_for(&deck, 200);
    let findings = analyze_findings(&stats, &deck);
    for problem in &findings {
        // Every problem kind that supports offenders has them; others
        // stay empty (never a panic either way).
        assert_eq!(
            problem.evidence.is_empty(),
            !matches!(
                problem.kind,
                "insufficient_color_mana"
                    | "low_castability"
                    | "insufficient_land_drops"
                    | "excess_lands_seen"
                    | "late_commander_cast"
            ),
            "kind {} offenders empty = {}",
            problem.kind,
            problem.evidence.is_empty()
        );
    }
}

/// A card between the 60-card bar (0.55) and the commander bar (0.60)
/// counts as a dead-card offender in a 60-card deck but not in a
/// commander deck: the threshold follows the deck shape.
#[test]
fn dead_card_threshold_follows_deck_shape() {
    let commander_deck = deck(35, 20, 5);
    let commander_stats = stats_for(&commander_deck, 50);
    let mut commander_problem = Finding {
        kind: "low_castability",
        severity: "medium",
        game_share: None,
        color: None,
        explanation: "test".to_string(),
        suggestion: "test".to_string(),
        evidence: Vec::new(),
    };
    explain_with_threshold(
        &mut commander_problem,
        &commander_stats,
        &commander_deck,
        0.60,
    );
    let commander_offenders: Vec<String> = commander_problem
        .evidence
        .iter()
        .map(|o| o.name.clone())
        .collect();
    let flat_deck = SimDeck {
        companion: None,
        cards: commander_deck.cards.clone(),
        commanders: Vec::new(),
        format: Format::Constructed,
        rules: rules_for("modern"),
    };
    let flat_stats = stats_for(&flat_deck, 50);
    let mut flat_problem = Finding {
        kind: "low_castability",
        severity: "medium",
        game_share: None,
        color: None,
        explanation: "test".to_string(),
        suggestion: "test".to_string(),
        evidence: Vec::new(),
    };
    explain_with_threshold(&mut flat_problem, &flat_stats, &flat_deck, 0.55);
    // The 60-card deck's offender list is a superset: the lower bar
    // admits everything the commander bar admits, plus borderline cards.
    for name in commander_offenders {
        assert!(
            flat_problem.evidence.iter().any(|o| o.name == name),
            "60-card offenders miss {name}"
        );
    }
}

/// A short run does not establish failure at a later target turn.
#[test]
fn findings_do_not_clamp_late_targets_to_the_horizon() {
    let deck = deck(35, 0, 3);
    let stats = SimStats {
        turns: 4,
        card_castability: (0..3)
            .map(|index| crate::deck::simulator::aggregate::CardCast {
                name: format!("Expensive {index}"),
                cmc: 7.0,
                target_turn: 7,
                pct_by_target: 0.0,
                avg_first_castable_turn: 5.0,
                payable_games: 0,
            })
            .collect(),
        runs: 100,
        finished_games: 100,
        ..SimStats::default()
    };
    let findings = analyze_findings(&stats, &deck);
    assert!(
        !findings
            .iter()
            .any(|finding| matches!(finding.kind, "late_commander_cast" | "low_castability"))
    );
    let mut finding = Finding {
        kind: "low_castability",
        severity: "high",
        game_share: None,
        color: None,
        explanation: String::new(),
        suggestion: String::new(),
        evidence: vec![],
    };
    explain_with_threshold(&mut finding, &stats, &deck, 0.60);
    assert!(finding.evidence.is_empty());
}

/// Draw-role access does not prove hand depletion or a need for more engines.
#[test]
fn draw_findings_only_state_the_observed_access() {
    let mut deck = deck(35, 0, 0);
    deck.commanders.clear();
    let stats = SimStats {
        turns: 6,
        starved_pct: 0.75,
        runs: 100,
        finished_games: 100,
        ..SimStats::default()
    };
    let findings = analyze_findings(&stats, &deck);
    let draw = findings
        .iter()
        .find(|finding| finding.kind == "limited_draw_access")
        .expect("access finding");
    assert!(
        draw.explanation
            .contains("no draw-role card access by turn 6")
    );
    assert!(!draw.explanation.contains("run out of cards"));
    assert!(!draw.suggestion.contains("add 2-3"));
    let short = SimStats { turns: 5, ..stats };
    assert!(
        !analyze_findings(&short, &deck)
            .iter()
            .any(|finding| finding.kind == "limited_draw_access")
    );
}

/// No commander timing observation must not render a fake average turn zero.
#[test]
fn unseen_commander_timing_is_unavailable() {
    let deck = deck(35, 0, 0);
    let stats = SimStats {
        runs: 1,
        finished_games: 1,
        turns: 10,
        commander_castable_by: vec![0.0; 11],
        ..SimStats::default()
    };
    let findings = analyze_findings(&stats, &deck);
    let commander = findings
        .iter()
        .find(|finding| finding.kind == "late_commander_cast")
        .expect("commander finding");
    assert!(commander.evidence[0].explanation.contains("unavailable"));
    assert!(!commander.evidence[0].explanation.contains("turn 0"));
}

/// Source context must check the requested color, not any available choice.
#[test]
fn source_context_does_not_credit_unrelated_or_opponent_colors() {
    use crate::deck::simulator::model::ManaYield;
    let mut deck = deck(0, 0, 0);
    deck.cards = vec![
        SimCard {
            role: Role::Land,
            tap: Some(ManaYield {
                choice: [true, true, false, false, false],
                ..ManaYield::default()
            }),
            ..SimCard::default()
        },
        SimCard {
            role: Role::Land,
            tap: Some(ManaYield {
                any_pips: 1,
                opponent_any: true,
                ..ManaYield::default()
            }),
            ..SimCard::default()
        },
    ];
    let stats = SimStats {
        turns: 10,
        color_screw: [0.0, 0.0, 0.5, 0.0, 0.0],
        runs: 100,
        finished_games: 100,
        ..SimStats::default()
    };
    let findings = analyze_findings(&stats, &deck);
    let color = findings
        .iter()
        .find(|finding| finding.kind == "insufficient_color_mana")
        .expect("color finding");
    assert!(color.explanation.contains("0 dedicated"));
    assert!(color.explanation.contains("0 choice"));
    assert!(!color.suggestion.contains("Fellwar"));
}

/// The accumulator averages per-game baselines, not one baseline at a mean sample size.
#[test]
fn flood_finding_names_the_recorded_sample_baseline() {
    let deck = deck(35, 0, 0);
    let stats = SimStats {
        turns: 4,
        flood_pct: 0.8,
        flood_expectation: 0.15,
        runs: 100,
        finished_games: 100,
        ..SimStats::default()
    };
    let findings = analyze_findings(&stats, &deck);
    let flood = findings
        .iter()
        .find(|finding| finding.kind == "excess_lands_seen")
        .expect("land access finding");
    assert!(
        flood
            .explanation
            .contains("mean uniform-draw baseline over recorded sample sizes is 15.0%")
    );
    assert!(!flood.explanation.contains("mean sample size"));
}
