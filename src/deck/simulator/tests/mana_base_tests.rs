// Mana-base calibration tests: the recalibrated flood detector, the
// mana_base verdict block, and the bracket target bands. A 44-land deck
// must report flood near its hypergeometric expectation (the old
// drops-made detector read it as 0.0%); a 35-land deck must not flood.

use super::deck::build_sim_deck;
use super::findings::{find_problems, mana_base};
use super::game::run_game;
use super::hypgeo::flood_expectation;
use super::model::Role;
use crate::db::CardRow;
use crate::deck::grammar::{Deck, DeckEntry};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

/// A filler spell row with no draw text (band fixtures must not trip the
/// cheap-cantrip credit).
fn dumb_spell_row(name: &str) -> (String, CardRow) {
    let (n, mut row) = spell_row(name);
    row.oracle_text = "Do nothing.".to_string();
    (n, row)
}

/// A land card row (the store join is bypassed: lands parse from the type
/// line even with empty oracle text).
fn land_row(name: &str) -> (String, CardRow) {
    (
        name.to_string(),
        CardRow {
            name: name.to_string(),
            oracle_id: "oid".into(),
            mana_cost: String::new(),
            cmc: 0.0,
            type_line: "Basic Land — Test".to_string(),
            colors: "[]".into(),
            color_identity: "[]".into(),
            keywords: "[]".into(),
            power: None,
            toughness: None,
            loyalty: None,
            oracle_text: String::new(),
            rarity: "basic".into(),
            edhrec_rank: None,
            legalities: "{}".into(),
            set_code: "tst".into(),
            collector_number: "1".into(),
            scryfall_id: format!("sid-{name}"),
            released_at: "2020-01-01".into(),
            game_changer: None,
        },
    )
}

/// A filler spell row (2-mana instant).
fn spell_row(name: &str) -> (String, CardRow) {
    (
        name.to_string(),
        CardRow {
            name: name.to_string(),
            oracle_id: "oid".into(),
            mana_cost: "{2}".into(),
            cmc: 2.0,
            type_line: "Instant".into(),
            colors: "[]".into(),
            color_identity: "[]".into(),
            keywords: "[]".into(),
            power: None,
            toughness: None,
            loyalty: None,
            oracle_text: "Draw a card.".into(),
            rarity: "common".into(),
            edhrec_rank: None,
            legalities: "{}".into(),
            set_code: "tst".into(),
            collector_number: "1".into(),
            scryfall_id: format!("sid-{name}"),
            released_at: "2020-01-01".into(),
            game_changer: None,
        },
    )
}

/// A commander deck with `n_lands` lands and filler spells.
fn deck_with_lands(n_lands: usize, name: &str) -> (Deck, HashMap<String, CardRow>) {
    deck_with_ramp(n_lands, 0, name)
}

/// A commander deck with `n_lands` lands, `n_ramp` two-mana rocks, and
/// filler spells. `lands_matter` gives the commander an extra-land-drop
/// engine (the widened-band signal).
fn deck_with_ramp(n_lands: usize, n_ramp: usize, name: &str) -> (Deck, HashMap<String, CardRow>) {
    let commander = if name == "landfall" {
        "Extra Land Commander"
    } else {
        "Test Commander"
    };
    let mut deck = Deck::default();
    deck.section_entries_mut("COMMANDER").push(DeckEntry {
        quantity: 1,
        name: commander.into(),
        set_code: None,
        collector_number: None,
        foil: false,
    });
    let mut cards: HashMap<String, CardRow> =
        [land_row("Island"), spell_row("Filler Spell"), rock_row()]
            .into_iter()
            .collect();
    if name == "landfall" {
        let mut row = spell_row(commander);
        row.1.type_line = "Legendary Creature — Merfolk Wizard".to_string();
        row.1.oracle_text = "You may play an additional land on each of your turns.".to_string();
        row.1.mana_cost = "{4}{G}{U}".into();
        row.1.cmc = 6.0;
        cards.insert(row.0.clone(), row.1);
    }
    let deck_section = deck.section_entries_mut("DECK");
    for _ in 0..n_lands {
        deck_section.push(DeckEntry {
            quantity: 1,
            name: "Island".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    let filler = 99 - n_lands - n_ramp;
    for _ in 0..n_ramp {
        deck_section.push(DeckEntry {
            quantity: 1,
            name: "Mana Rock".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    for _ in 0..filler {
        deck_section.push(DeckEntry {
            quantity: 1,
            name: "Filler Spell".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    let _ = name;
    (deck, cards)
}

fn flood_rate(n_lands: usize) -> f64 {
    let (deck, cards) = deck_with_lands(n_lands, "x");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let logs: Vec<_> = (0..4000)
        .map(|_| run_game(&sim_deck, &mut rng, 10))
        .collect();
    let stats = super::aggregate::aggregate(&logs, &sim_deck, 10);
    stats.flood_pct
}

#[test]
fn flood_rate_44_lands_near_expectation() {
    let lands44 = flood_rate(44);
    let expected = flood_expectation(44, 99, 11);
    assert!(
        lands44 > 0.25,
        "44-land deck must flood visibly; got {lands44:.3} (expectation {expected:.3})"
    );
    assert!(
        lands44 >= expected * 0.7 && lands44 <= expected + 0.15,
        "sim flood {lands44:.3} should track expectation {expected:.3}"
    );
}

#[test]
fn flood_rate_35_lands_low() {
    let lands35 = flood_rate(35);
    let expected = flood_expectation(35, 99, 11);
    assert!(
        lands35 < 0.20,
        "35-land deck should rarely flood; got {lands35:.3} (expectation {expected:.3})"
    );
    assert!(expected < 0.20, "the math itself must agree");
}

#[test]
fn flood_expectation_tracks_actual_draw_volume() {
    // A plain deck sees exactly 11 cards by t4 (opener + 4 draws), so
    // the recorded window must be 11 and the expectation must equal the
    // 11-card math.
    let (deck, cards) = deck_with_lands(37, "vel");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let logs: Vec<_> = (0..1000)
        .map(|_| run_game(&sim_deck, &mut rng, 10))
        .collect();
    let stats = super::aggregate::aggregate(&logs, &sim_deck, 10);
    let diff = (stats.flood_expectation - flood_expectation(37, 99, 11)).abs();
    assert!(diff < 1e-9, "plain deck's window is 11: diff {diff}");
    // Window sanity: logs record the per-game seen count at t4.
    assert!(
        logs.iter().all(|g| g.cards_seen_by_4 >= 11),
        "opener + 4 draws is at least 11 cards"
    );
}

#[test]
fn mana_base_verdicts_match_bands() {
    // A 35-land deck with 9 ramp: on target for bracket 3.
    let (deck, cards) = deck_with_ramp(35, 9, "ok");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, false);
    assert_eq!(verdict.lands, 35);
    assert_eq!(verdict.bracket_target_lands, [33, 38]);
    assert_eq!(verdict.verdict, "on target", "{}", verdict.verdict);
    assert_eq!(verdict.total_sources, 44, "35 lands + 9 rocks");
    assert_eq!(verdict.rocks, 9, "the rock rows classify as rocks");

    // A 44-land deck: trim lands.
    let (deck, cards) = deck_with_lands(44, "fat");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, false);
    assert!(
        verdict.verdict.contains("trim"),
        "44 lands should read as land-heavy: {}",
        verdict.verdict
    );
    // 30-land deck: add lands.
    let (deck, cards) = deck_with_lands(28, "thin");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, false);
    assert!(verdict.verdict.contains("add"), "{}", verdict.verdict);
}

/// A two-mana rock row ("Sol Ring class": artifact, taps for mana).
fn rock_row() -> (String, CardRow) {
    let mut row = spell_row("Mana Rock");
    row.1.type_line = "Artifact".to_string();
    row.1.oracle_text = "{T}: Add {C}.".to_string();
    row
}

#[test]
fn bracket_bands_shift_with_power() {
    // cEDH band is lower and ramp-heavier.
    let (deck, cards) = deck_with_ramp(29, 12, "cedh");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 5, false);
    assert_eq!(verdict.bracket_target_lands, [25, 31]);
    assert_eq!(verdict.bracket_target_ramp, [10, 16]);
    assert_eq!(verdict.verdict, "on target", "{}", verdict.verdict);
}

#[test]
fn lands_matter_widens_the_band() {
    let (deck, cards) = deck_with_ramp(42, 0, "landfall");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, false);
    assert_eq!(
        verdict.bracket_target_lands[1], 42,
        "a lands-matter deck's band widens by 4"
    );
    // The widened band is still a comparison: a 20-land landfall deck is
    // far under even the widened floor, and the verdict says so.
    let (deck, cards) = deck_with_ramp(20, 0, "landfall");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, false);
    assert!(
        verdict.verdict.contains("add") && verdict.verdict.contains("lands-matter"),
        "a starved lands-matter deck reads as land-short: {}",
        verdict.verdict
    );
}

#[test]
fn land_roles_still_parse() {
    // The helper's land rows must classify as lands in the sim deck.
    let (deck, cards) = deck_with_lands(40, "x");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    assert_eq!(
        sim_deck
            .cards
            .iter()
            .filter(|c| c.role == Role::Land)
            .count(),
        40
    );
}

#[test]
fn mana_base_reports_its_bracket() {
    let (deck, cards) = deck_with_ramp(35, 9, "b3");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, false);
    assert_eq!(verdict.bracket, Some(3));
}

/// A 60-card deck: no COMMANDER section, `n_lands` islands, and filler
/// spells of the given cost. Filler costs set the average MV.
fn sixty_card_deck(n_lands: usize, filler: &str) -> (Deck, HashMap<String, CardRow>) {
    let mut deck = Deck::default();
    let cards: HashMap<String, CardRow> = [land_row("Island"), dumb_spell_row(filler)]
        .into_iter()
        .collect();
    let deck_section = deck.section_entries_mut("DECK");
    for _ in 0..n_lands {
        deck_section.push(DeckEntry {
            quantity: 1,
            name: "Island".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    for _ in 0..(60 - n_lands) {
        deck_section.push(DeckEntry {
            quantity: 1,
            name: filler.to_string(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    (deck, cards)
}

fn sixty_card_deck_at_cost(n_lands: usize, cost: u32) -> (Deck, HashMap<String, CardRow>) {
    let filler = format!("Filler {cost}");
    let mut cards = sixty_card_deck(n_lands, &filler).1;
    let mut card = dumb_spell_row(&filler).1;
    card.mana_cost = format!("{{{cost}}}");
    card.cmc = f64::from(cost);
    cards.insert(filler.clone(), card);
    let (deck, _) = sixty_card_deck(n_lands, &filler);
    (deck, cards)
}

#[test]
fn sixty_card_midrange_24_lands_on_target() {
    // Plain 2-mana fillers: avg MV 2.0 sits at the midrange band's lower
    // edge (2.0 <= avg < 3.0 → 22-25). 24 lands: on target, no bracket.
    let (deck, cards) = sixty_card_deck(24, "Midrange Filler");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, true);
    assert_eq!(verdict.bracket, None, "60-card decks report no bracket");
    assert_eq!(
        verdict.bracket_target_lands,
        [22, 25],
        "{}",
        verdict.verdict
    );
    assert_eq!(verdict.verdict, "on target", "{}", verdict.verdict);
    assert_eq!(
        verdict.bracket_target_ramp,
        [0, 0],
        "no ramp band for 60-card decks"
    );
}

#[test]
fn sixty_card_band_boundaries_follow_avg_mv() {
    // avg MV 2.0 exactly (all 2-mana fillers) is the midrange band's lower
    // edge: [22, 25]. 21 lands: add lands.
    let (deck, cards) = sixty_card_deck_at_cost(21, 2);
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, true);
    assert_eq!(verdict.bracket_target_lands, [22, 25]);
    assert_eq!(verdict.verdict, "add 1 land (21 < band 22-25)");
}

#[test]
fn sixty_card_control_low_lands_adds_lands() {
    // avg MV 3.4 → control band [25, 28]; 20 lands: add 5.
    let (deck, cards) = sixty_card_deck_at_cost(20, 4);
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, true);
    assert_eq!(verdict.bracket_target_lands, [25, 28]);
    assert_eq!(verdict.lands, 20);
    assert_eq!(
        verdict.verdict, "add 5 lands (20 < band 25-28)",
        "{}",
        verdict.verdict
    );
}

#[test]
fn sixty_card_aggro_low_band() {
    // avg MV < 2 → aggro band [20, 22].
    let (deck, cards) = sixty_card_deck_at_cost(21, 1);
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, true);
    assert_eq!(verdict.bracket_target_lands, [20, 22]);
    assert_eq!(verdict.verdict, "on target", "{}", verdict.verdict);
}

#[test]
fn sixty_card_cheap_cantrips_lower_the_floor() {
    // A control curve (25-28) with 8 cheap cantrips credits 2 lands:
    // floor 23. 23 lands sit on target.
    let mut deck = Deck::default();
    let mut cards: HashMap<String, CardRow> = [
        land_row("Island"),
        spell_row("Big Filler"),
        spell_row("Cheap Cantrip"),
    ]
    .into_iter()
    .collect();
    let mut big = spell_row("Big Filler").1;
    big.mana_cost = "{4}".into();
    big.cmc = 4.0;
    cards.insert("Big Filler".to_string(), big);
    let section = deck.section_entries_mut("DECK");
    for _ in 0..23 {
        section.push(DeckEntry {
            quantity: 1,
            name: "Island".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    for _ in 0..8 {
        section.push(DeckEntry {
            quantity: 1,
            name: "Cheap Cantrip".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    for _ in 0..29 {
        section.push(DeckEntry {
            quantity: 1,
            name: "Big Filler".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, true);
    assert_eq!(
        verdict.bracket_target_lands,
        [23, 26],
        "8 cheap cantrips credit 2 lands"
    );
    assert_eq!(verdict.verdict, "on target", "{}", verdict.verdict);
}

#[test]
fn commander_bands_unchanged_regression() {
    // The commander branch must keep its bracket bands (regression guard
    // for the 60-card branch split).
    let (deck, cards) = deck_with_ramp(35, 9, "cmd");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let verdict = mana_base(&sim_deck, 3, false);
    assert_eq!(verdict.bracket, Some(3));
    assert_eq!(verdict.bracket_target_lands, [33, 38]);
    assert_eq!(verdict.bracket_target_ramp, [8, 11]);
}

#[test]
fn bracket_inference_follows_game_changers() {
    // No Game Changers → bracket 2; 1–3 → 3; more → 4.
    let make = |n_gc: usize| {
        let (deck, cards) = deck_with_ramp(35, 9, "infer");
        let mut cards = cards;
        for i in 0..n_gc {
            let mut row = spell_row(&format!("GC {i}"));
            row.1.game_changer = Some(true);
            cards.insert(row.0.clone(), row.1);
        }
        let mut deck = deck;
        deck.section_entries_mut("DECK").clear();
        let deck_section = deck.section_entries_mut("DECK");
        for i in 0..n_gc {
            deck_section.push(DeckEntry {
                quantity: 1,
                name: format!("GC {i}"),
                set_code: None,
                collector_number: None,
                foil: false,
            });
        }
        (deck, cards)
    };
    let (deck, cards) = make(0);
    assert_eq!(super::infer_bracket(&deck, &cards), 2);
    let (deck, cards) = make(2);
    assert_eq!(super::infer_bracket(&deck, &cards), 3);
    let (deck, cards) = make(5);
    assert_eq!(super::infer_bracket(&deck, &cards), 4);
}

#[test]
fn rock_heavy_screw_suggests_color_fix_not_lands() {
    // 30 lands + 14 rocks: screwing despite plenty of sources is a color
    // problem; the suggestion must not say "add land slots".
    let (deck, cards) = deck_with_ramp(30, 14, "rocky");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let mut rng = rand::SeedableRng::seed_from_u64(7);
    let logs: Vec<_> = (0..500)
        .map(|_| super::game::run_game(&sim_deck, &mut rng, 10))
        .collect();
    let stats = super::aggregate::aggregate(&logs, &sim_deck, 10);
    let problems = find_problems(&stats, &sim_deck);
    for p in &problems {
        if p.kind == "mana_screw" {
            assert!(
                p.suggestion.contains("color") || p.suggestion.contains("colors"),
                "a rock-heavy deck's screw advice names colors: {}",
                p.suggestion
            );
        }
    }
}

#[test]
fn bracket_inference_ignores_sideboard_game_changers() {
    // A sideboard GC is a wishlist entry: it must not push the inference.
    let (deck, cards) = deck_with_ramp(35, 9, "infer");
    let mut row = spell_row("GC side");
    row.1.game_changer = Some(true);
    let mut cards = cards;
    cards.insert(row.0.clone(), row.1);
    let mut deck = deck;
    deck.section_entries_mut("SIDEBOARD").push(DeckEntry {
        quantity: 1,
        name: "GC side".into(),
        set_code: None,
        collector_number: None,
        foil: false,
    });
    assert_eq!(super::infer_bracket(&deck, &cards), 2);
}

#[test]
fn flood_finding_respects_the_lands_matter_plan() {
    // A lands-matter deck inside its widened band floods by design: the
    // flood finding must not demand a land trim (that would contradict
    // the "on target" verdict on the same report).
    let (deck, cards) = deck_with_ramp(42, 0, "landfall");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let logs: Vec<_> = (0..4000)
        .map(|_| run_game(&sim_deck, &mut rng, 10))
        .collect();
    let problems = find_problems(
        &super::aggregate::aggregate(&logs, &sim_deck, 10),
        &sim_deck,
    );
    if let Some(flood) = problems.iter().find(|p| p.kind == "mana_flood") {
        assert!(
            flood.suggestion.contains("no land trim"),
            "lands-matter deck on target must not get a trim: {}",
            flood.suggestion
        );
    }
}
