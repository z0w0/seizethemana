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
