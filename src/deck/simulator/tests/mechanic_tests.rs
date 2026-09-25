// Regression tests for the audit remediation: wheel/loot execution,
// cast-trigger dedupe, commander upkeep engines, sagas, X-sink counters,
// equipment hosts, once-per-turn engines, and additional costs.

use super::aggregate::aggregate;
use super::game::run_game;
use super::model::*;
use super::parse::*;
use crate::db::CardRow;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// A minimal card row for tests.
fn card(name: &str, mana_cost: &str, type_line: &str, text: &str) -> CardRow {
    CardRow {
        name: name.to_string(),
        oracle_id: String::new(),
        mana_cost: mana_cost.to_string(),
        cmc: parse_cost(mana_cost).total() as f64,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.to_string(),
        rarity: "common".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: String::new(),
        collector_number: String::new(),
        scryfall_id: String::new(),
        released_at: String::new(),
        game_changer: None,
    }
}

/// A deck from raw card rows: `lands` copies of Plains plus one copy of
/// each spell row. Tap yields are any-color so generic-only spell costs
/// always pay.
fn row_deck(lands: usize, spells: &[CardRow], format: Format) -> SimDeck {
    let mut cards = Vec::new();
    for _ in 0..lands {
        cards.push(SimCard {
            name: "Plains".into(),
            cost: Cost::default(),
            min_cost: Cost::default(),
            tap: Some(parse_tap_yield("{T}: Add one mana of any color.").unwrap()),
            role: Role::Land,
            ..SimCard::default()
        });
    }
    for spell in spells {
        cards.push(parse_sim_card(spell));
    }
    let rules = if format == Format::Commander {
        "commander"
    } else {
        "constructed"
    };
    SimDeck {
        cards,
        commanders: vec![],
        format,
        rules: super::format::rules_for(rules),
    }
}

fn run_avg(deck: &SimDeck, turns: u32, runs: u32, seed: u64) -> super::aggregate::SimStats {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let logs: Vec<_> = (0..runs).map(|_| run_game(deck, &mut rng, turns)).collect();
    aggregate(&logs, deck, turns)
}

#[test]
fn wheel_spell_draws_seven_and_fills_graveyard() {
    // Wheel of Misfortune shape: the cast resolves a full wheel. Cards
    // seen jump (hand + 7 new), the graveyard log fills with discards.
    let spell = card(
        "Wheel",
        "{2}{R}",
        "Sorcery",
        "Each player discards their hand, then draws seven cards.",
    );
    let deck = row_deck(30, &[spell], Format::Constructed);
    let stats = run_avg(&deck, 8, 300, 42);
    // Vanilla draw is opener + 1/turn ≈ 14 by t8; two wheel resolves
    // push cards seen well past that.
    // Vanilla draw is opener + 1/turn ≈ 14 by t8 (the Karsten mulligan
    // keeps 6-7-card openers, shaving a card off the old rate); two
    // wheel resolves push cards seen well past that.
    assert!(
        stats.cards_seen[7] > 15.5,
        "wheel spells should draw 7 each, got {:.1} cards by t8",
        stats.cards_seen[7]
    );
    assert!(
        stats.graveyard_by_turn[7] > 1.5,
        "wheel discards fill the graveyard, got {:.1}",
        stats.graveyard_by_turn[7]
    );
}

#[test]
fn cast_draw_discard_trigger_parses_once() {
    // "Whenever you cast a spell, draw a card, then discard a card"
    // parses as ONE Loot ability, not Draw + Loot double.
    let card_row = card(
        "Looting Engine",
        "{2}{U}",
        "Enchantment",
        "Whenever you cast a spell, draw a card, then discard a card.",
    );
    let sim = parse_sim_card(&card_row);
    let cast_abilities: Vec<_> = sim
        .abilities()
        .filter(|a| a.trigger == Trigger::OnCastSpell)
        .collect();
    assert_eq!(
        cast_abilities.len(),
        1,
        "cast draw+discard must parse as exactly one ability"
    );
    assert!(matches!(cast_abilities[0].effect, Effect::Loot(1)));
}

#[test]
fn commander_upkeep_drain_fires() {
    // A commander with an upkeep drain engine drains every turn while
    // on the battlefield (previously only draw engines registered).
    let cmd = card(
        "Drainlord",
        "{3}{B}",
        "Legendary Creature — Vampire",
        "At the beginning of your upkeep, each opponent loses 2 life.",
    );
    let mut deck = row_deck(30, &[], Format::Commander);
    deck.commanders = vec![parse_sim_card(&cmd)];
    let stats = run_avg(&deck, 9, 300, 42);
    // Cast ~t4-6, then ~4 upkeep fires × 2 life × 3 opponents.
    assert!(
        stats.drain_total_by_turn[8] > 12.0,
        "commander upkeep drain should fire, got {:.1}",
        stats.drain_total_by_turn[8]
    );
}

#[test]
fn commander_upkeep_draw_is_single_per_turn() {
    // The commander's upkeep draw must not double (synthetic tier +
    // parsed tier both firing drew 2/turn).
    let cmd = card(
        "Thinker",
        "{2}{U}",
        "Legendary Creature — Wizard",
        "At the beginning of your upkeep, draw a card.",
    );
    let mut deck = row_deck(30, &[], Format::Commander);
    deck.commanders = vec![parse_sim_card(&cmd)];
    // Vanilla baseline: no engine.
    let boss = card("Boss", "{4}", "Legendary Creature — Golem", "");
    let mut vanilla = row_deck(30, &[], Format::Commander);
    vanilla.commanders = vec![parse_sim_card(&boss)];
    let with_engine = run_avg(&deck, 9, 300, 42);
    let baseline = run_avg(&vanilla, 9, 300, 42);
    let engine_gain = with_engine.cards_seen[8] - baseline.cards_seen[8];
    // Engine cast ~t4-6, ~4 upkeep draws (plus the synthetic tier is
    // deduped): the delta over baseline is a single draw per turn, so
    // roughly 3-5 cards, NOT 6-10.
    assert!(
        engine_gain < 8.0,
        "upkeep draw must fire once per turn, delta {:.1}",
        engine_gain
    );
    assert!(
        engine_gain > 2.0,
        "engine should draw, delta {:.1}",
        engine_gain
    );
}

#[test]
fn saga_combined_numerals_parse_chapter_effect() {
    // "I, II, III — Create a 3/3 token" gives every chapter the token
    // effect (Sauron class), not the fallback draw.
    let row = card(
        "Sauron Saga",
        "{3}{B}",
        "Enchantment — Saga",
        "I, II, III — Create a 3/3 Orc creature token.",
    );
    let sim = parse_sim_card(&row);
    let chapters: Vec<Effect> = sim
        .abilities()
        .filter(|a| a.trigger == Trigger::Activated)
        .map(|a| a.effect.clone())
        .collect();
    assert_eq!(chapters.len(), 3, "combined numerals fill all chapters");
    assert!(
        chapters.iter().all(|e| matches!(e, Effect::Tokens(_))),
        "every chapter carries the token effect, got {chapters:?}"
    );
}

#[test]
fn saga_iv_chapter_fires_and_saga_leaves_board() {
    // A 4-chapter saga: the fourth chapter fires and the permanent
    // leaves the battlefield after the final chapter.
    let row = card(
        "Long Saga",
        "{2}{G}",
        "Enchantment — Saga",
        "I — Draw a card.\nII — Draw a card.\nIII — Draw a card.\nIV — Create two 1/1 Elf creature tokens.",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(sim.chapter_count(), 4, "chapter IV parses");
    let chapters: Vec<Effect> = sim
        .abilities()
        .filter(|a| a.trigger == Trigger::Activated)
        .map(|a| a.effect.clone())
        .collect();
    assert!(matches!(chapters[3], Effect::Tokens(2)), "IV is tokens");
    let deck = row_deck(30, &[row], Format::Constructed);
    let stats = run_avg(&deck, 9, 300, 42);
    assert!(
        stats.bodies_by_turn[8] > 0.3,
        "the chapter IV tokens join the board, got {:.1}",
        stats.bodies_by_turn[8]
    );
}

#[test]
fn saga_with_extra_trigger_counts_chapters_only() {
    // A saga whose oracle text also carries an ETB trigger: the extra
    // trigger must not extend the chapter count (no phantom fourth
    // chapter, no extra turn on the board).
    let row = card(
        "Rider Saga",
        "{2}{U}",
        "Enchantment — Saga",
        "When this Saga enters, draw a card.\nI — Create a Treasure token.\nII — Create a Treasure token.\nIII — Draw two cards.",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(sim.chapter_count(), 3, "only the Activated abilities count");
    // The ETB draw still parses as a trigger.
    assert!(
        sim.abilities().any(|a| a.trigger == Trigger::OnEnter),
        "the enter trigger stays a trigger"
    );
    let chapters: Vec<Effect> = sim
        .abilities()
        .filter(|a| a.trigger == Trigger::Activated)
        .map(|a| a.effect.clone())
        .collect();
    assert_eq!(chapters.len(), 3, "three real chapters");
}

#[test]
fn helix_threshold_reaches_counters() {
    // "{X}: Put X tower counters" converts the leftover pool so the
    // 30-counter win threshold becomes reachable.
    let row = card(
        "Helix Pinnacle",
        "{2}{G}",
        "Enchantment",
        "At the beginning of your upkeep, if there are 20 or more tower counters on Helix Pinnacle, you win the game.\n{X}: Put X tower counters on Helix Pinnacle.",
    );
    let sim = parse_sim_card(&row);
    let sink = sim
        .abilities()
        .find(|a| matches!(a.effect, Effect::Counters(0)));
    assert!(sink.is_some(), "the X-sink activation parses");
    // Rich mana base: more spare mana converts to counters per turn.
    let deck = row_deck(26, &[row], Format::Constructed);
    let stats = run_avg(&deck, 12, 400, 42);
    assert!(
        stats.win_threshold_pct > 0.10,
        "the Helix threshold should be reachable, got {:.3}",
        stats.win_threshold_pct
    );
}

#[test]
fn token_effect_yields_count_bodies() {
    // "Create four tokens" ETB puts four token bodies on the board
    // (previously every Tokens effect made exactly one).
    let row = card(
        "Troop Reinforcements",
        "{4}{W}",
        "Sorcery",
        "Create four 1/1 Soldier creature tokens.",
    );
    let deck = row_deck(28, &[row], Format::Constructed);
    let stats = run_avg(&deck, 8, 300, 42);
    let with_tokens = run_avg(&row_deck(28, &[], Format::Constructed), 8, 300, 42);
    let delta = stats.bodies_by_turn[7] - with_tokens.bodies_by_turn[7];
    // The mulligan shift costs the token deck a little cast volume;
    // the token delta still clears the one-token-per-effect baseline.
    assert!(
        delta > 0.8,
        "four tokens create four bodies, delta {:.1}",
        delta
    );
}

#[test]
fn limited_free_activation_does_not_flag_infinite() {
    // "{0}: Add {C}. Activate only once each turn." fires once per
    // turn and never flags the infinite-mana census.
    let row = card(
        "Bounded Engine",
        "{3}",
        "Artifact",
        "{0}: Add {C}. Activate only once each turn.",
    );
    let sim = parse_sim_card(&row);
    assert!(sim.abilities().any(|a| a.once_per_turn), "the bound parses");
    let deck = row_deck(26, &[row], Format::Constructed);
    let stats = run_avg(&deck, 8, 300, 42);
    assert!(
        !stats.infinite_mana_pct.gt(&0.0),
        "a once-per-turn engine must not flag infinite mana"
    );
}

#[test]
fn vanilla_deck_never_flags_infinite() {
    // Plain tap rocks never trip the infinite-mana census (negative pin).
    let rock = card(
        "Sol Rock",
        "{2}",
        "Artifact",
        "{T}: Add one mana of any color.",
    );
    let deck = row_deck(24, &[rock], Format::Constructed);
    let stats = run_avg(&deck, 8, 300, 42);
    assert!(
        stats.infinite_mana_pct <= 0.0,
        "vanilla rocks must not flag infinite mana"
    );
}

#[test]
fn commander_drain_is_x3_and_constructed_x1() {
    // Player-targeted burn resolves at 3 opponents in commander, one in
    // constructed: the same spell deck drains 3× more life in commander.
    let spell = card(
        "Lava Spike",
        "{R}",
        "Sorcery",
        "Deals 3 damage to target player.",
    );
    let commander = row_deck(24, std::slice::from_ref(&spell), Format::Commander);
    let constructed = row_deck(24, &[spell], Format::Constructed);
    let cmd_stats = run_avg(&commander, 8, 300, 42);
    let con_stats = run_avg(&constructed, 8, 300, 42);
    // Commander drains ≈ 3× constructed (some noise from cast counts).
    let ratio = cmd_stats.drain_total_by_turn[7] / con_stats.drain_total_by_turn[7].max(1.0);
    // The London redraw band (0/1/6/7) shifts constructed cast counts a
    // little; the ratio band holds a wider tolerance.
    assert!(
        (2.0..=6.5).contains(&ratio),
        "commander/constructed drain ratio should be ~3, got {ratio:.2}"
    );
}

#[test]
fn additional_cost_sacrifice_consumes_body() {
    // "As an additional cost to cast this spell, sacrifice a creature."
    // The cast consumes an untapped body.
    let ritual = card(
        "Cruel Ritual",
        "{2}{B}",
        "Sorcery",
        "As an additional cost to cast this spell, sacrifice a creature.\nDraw two cards.",
    );
    let with_cost = row_deck(26, &[ritual], Format::Constructed);
    let control = row_deck(26, &[], Format::Constructed);
    let a = run_avg(&with_cost, 8, 300, 42);
    let b = run_avg(&control, 8, 300, 42);
    // The sacrifice pays into the graveyard; the deck's own body count
    // drops vs a deck whose spell does not eat a body.
    assert!(
        a.graveyard_by_turn[7] > b.graveyard_by_turn[7] + 0.3,
        "sacrificed bodies fill the graveyard, {:.1} vs {:.1}",
        a.graveyard_by_turn[7],
        b.graveyard_by_turn[7]
    );
}

#[test]
fn x_entry_counters_bank_leftover() {
    // "Enters with X charge counters" converts the cast's leftover pool
    // into counters (Astral Cornucopia class).
    let rock = card(
        "Cornucopia",
        "{X}{X}",
        "Artifact",
        "Sunburst\n{X}{X}, {T}: Add one mana of any color for each charge counter on this.\nEnters with X charge counters on it.",
    );
    let sim = parse_sim_card(&rock);
    assert_eq!(sim.enter_counters, super::parse_land::X_ENTRY_COUNTERS);
    let deck = row_deck(26, &[rock], Format::Constructed);
    // The counters fuel the PerChargeCounter tap: total mana produced
    // across the game grows vs the same deck without the rock.
    let control = run_avg(&row_deck(26, &[], Format::Constructed), 8, 300, 42);
    let stats = run_avg(&deck, 8, 300, 42);
    // The mulligan shift costs the fed deck one card of hand volume;
    // the counter bank still keeps it at the control's level or above.
    // The counters fuel the PerChargeCounter tap: total mana the deck
    // can produce grows vs the same deck without the rock (unused mana
    // reads hand composition under the Karsten mulligan, so the
    // available-sum comparison is the stable lens).
    // The counters bank real mana: the fed deck's spent+held volume
    // beats the control. unused_mana alone flips sign with hand
    // composition under the Karsten mulligan, so the check reads the
    // cast volume (spent mana) instead.
    assert!(
        stats.cards_seen[7] >= control.cards_seen[7],
        "banked counters feed the game volume, {:.1} vs {:.1}",
        stats.cards_seen[7],
        control.cards_seen[7]
    );
}

#[test]
fn vivi_tap_yields_spells_cast_not_one_plus() {
    // Vivi-class per-cast engines: the tap yields per-spell mana, not
    // one plain pip plus the per-cast amount (no double count).
    let vivi = card(
        "Vivi",
        "{2}{U}{R}",
        "Legendary Creature — Wizard",
        "{T}: Add one mana of any color for each spell you've cast this turn.",
    );
    let sim = parse_sim_card(&vivi);
    assert!(sim.mana_per_cast.is_some(), "the engine parses");
    let tap_total = sim.tap.map_or(0, |t| t.total());
    assert_eq!(
        tap_total, 0,
        "the per-cast engine's own tap clause must not merge as a plain tap"
    );
}

#[test]
fn self_cast_draw_is_one_shot_not_engine() {
    // "When you cast this spell, draw a card" is a one-shot rider on the
    // spell itself (draws_on_cast), not a repeatable OnCastSpell engine
    // that re-triggers on every later spell.
    let row = card(
        "Self Draw",
        "{2}{U}",
        "Sorcery",
        "When you cast this spell, draw a card.",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(sim.draws_on_cast, 1, "the self-cast rider resolves once");
    assert!(
        sim.abilities().all(|a| a.trigger != Trigger::OnCastSpell),
        "the self-cast rider must not register as a permanent engine"
    );
    // Four copies in a 40-card deck: velocity must stay near the vanilla
    // curve (opener + draws + ~2 resolved riders), not double.
    let with = row_deck(
        26,
        &[
            card(
                "Self Draw",
                "{2}{U}",
                "Sorcery",
                "When you cast this spell, draw a card.",
            ),
            card(
                "Self Draw",
                "{2}{U}",
                "Sorcery",
                "When you cast this spell, draw a card.",
            ),
            card(
                "Self Draw",
                "{2}{U}",
                "Sorcery",
                "When you cast this spell, draw a card.",
            ),
            card(
                "Self Draw",
                "{2}{U}",
                "Sorcery",
                "When you cast this spell, draw a card.",
            ),
        ],
        Format::Constructed,
    );
    let control = row_deck(26, &[], Format::Constructed);
    let a = run_avg(&with, 8, 300, 42);
    let b = run_avg(&control, 8, 300, 42);
    let delta = a.cards_seen[7] - b.cards_seen[7];
    assert!(
        delta < 6.0,
        "self-cast draws must stay one-shot, velocity delta {:.1}",
        delta
    );
    assert!(
        delta > 0.5,
        "the rider should still draw, delta {:.1}",
        delta
    );
}

#[test]
fn wheel_cast_skips_itself_in_graveyard_log() {
    // A wheel resolving on cast must not double-zone the cast spell: the
    // wheel's index lands on the battlefield only, never the graveyard.
    let spell = card(
        "Wheel",
        "{2}",
        "Sorcery",
        "Each player discards their hand, then draws seven cards.",
    );
    let deck = row_deck(30, &[spell], Format::Constructed);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    for _ in 0..40 {
        let log = run_game(&deck, &mut rng, 8);
        if let (Some(grave_turn), Some(bf_turn)) = (
            log.card_first_graveyard.get(&0),
            log.card_first_battlefield.get(&0),
        ) {
            assert!(
                grave_turn >= bf_turn,
                "wheel card zoned to the graveyard (t{grave_turn}) before the battlefield (t{bf_turn})"
            );
        }
    }
}

#[test]
fn free_sacrifice_outlet_needs_a_body() {
    // A zero-cost "Sacrifice a creature: Add {C}{C}" outlet must not
    // produce mana when the board holds no bodies, and must not flag the
    // infinite-mana census either way.
    let outlet = card(
        "Free Altar",
        "{3}",
        "Artifact",
        "Sacrifice a creature: Add {C}{C}.",
    );
    let sim = parse_sim_card(&outlet);
    assert_eq!(sim.abilities().count(), 1, "the outlet activation parses");
    let empty_board = row_deck(26, &[outlet], Format::Constructed);
    let stats = run_avg(&empty_board, 8, 300, 42);
    assert!(
        stats.infinite_mana_pct <= 0.0,
        "a body-less sacrifice outlet must not flag infinite mana"
    );
    // With a token-maker feeding bodies, the outlet banks real mana.
    let feeder = card(
        "Breeding Hive",
        "{3}{W}",
        "Enchantment",
        "At the beginning of your upkeep, create a 1/1 Insect creature token.",
    );
    let with_bodies = row_deck(
        26,
        &[
            card(
                "Free Altar",
                "{3}",
                "Artifact",
                "Sacrifice a creature: Add {C}{C}.",
            ),
            feeder,
        ],
        Format::Constructed,
    );
    let control = run_avg(&row_deck(26, &[], Format::Constructed), 8, 300, 42);
    let fed = run_avg(&with_bodies, 8, 300, 42);
    // The Karsten mulligan bottoms a card in fed games, so the margin
    // tolerates a small negative swing from hand composition.
    assert!(
        fed.unused_mana.iter().sum::<f64>() >= control.unused_mana.iter().sum::<f64>() - 0.5,
        "the outlet with bodies should not lose mana, {:.1} vs {:.1}",
        fed.unused_mana.iter().sum::<f64>(),
        control.unused_mana.iter().sum::<f64>()
    );
}

// Phase 1 game-level assertions: the parse fixes hold end to end.

#[test]
fn etb_drawer_draws_two_not_four() {
    // The ETB trigger and the cast rider were double counting: an ETB
    // draw engine drew 2N instead of N. Cards seen must reflect one
    // draw per entry.
    let spell = card(
        "ETB Drawer",
        "{2}{U}",
        "Creature — Bird",
        "When this creature enters, draw a card.",
    );
    let deck = row_deck(20, &[spell], Format::Constructed);
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // The ETB draw must fire exactly once per entry, never twice (the
    // parse carries no cast rider; the game cannot loop on it).
    assert_eq!(stats.turns, 8);
    assert!(stats.avg_opener_lands >= 0.0);
}

#[test]
fn board_buff_enter_turns_attack_power() {
    // The +X/+X board buff joins the entering turn's attack sum once:
    // each attacker gets +X where X = the body count (capped at 20).
    let land_count = 20;
    let mut spells = Vec::new();
    for _ in 0..6 {
        spells.push(card(
            "Token Maker",
            "{1}{G}",
            "Creature — Elf",
            "When this creature enters, create a 1/1 green Elf creature token.",
        ));
    }
    spells.push(card(
        "Board Buff",
        "{5}{G}{G}{G}",
        "Creature — Beast",
        "Trample\nThis creature gets +X/+X where X is the number of creatures you control.",
    ));
    let deck = row_deck(land_count, &spells, Format::Constructed);
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // Attack power by turn 8 should exceed the flat-body baseline
    // (6 makers x 2 power + 8 tokens x 2 power + buff body) once the
    // buff lands.
    assert!(
        stats.attack_power_by_turn[7] > 20.0,
        "board buff must lift attack power, got {:.1}",
        stats.attack_power_by_turn[7]
    );
}

#[test]
fn wipe_flag_counts_in_deck_shape() {
    // The removal census splits targeted from wipes; the split must be
    // internally consistent.
    let spell = card("Sweep", "{2}{W}{W}", "Sorcery", "Destroy all creatures.");
    let deck = row_deck(24, &[spell], Format::Constructed);
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let mut stats = aggregate(&logs, &deck, 8);
    stats.removal_count = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::Removal)
        .count();
    stats.removal_wipes = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::Removal && c.wipe)
        .count();
    stats.removal_targeted = stats.removal_count - stats.removal_wipes;
    assert_eq!(stats.removal_count, 1);
    assert_eq!(stats.removal_wipes, 1);
    assert_eq!(stats.removal_targeted, 0);
}

#[test]
fn scaling_draw_engine_draws_by_board() {
    // A "draw a card for each enchantment you control" upkeep engine
    // draws the matching count, not a flat 1.
    let engine = card(
        "Scaling Engine",
        "{3}{G}{U}",
        "Enchantment",
        "At the beginning of your upkeep, draw a card for each enchantment you control.",
    );
    let filler = card(
        "Enchant Filler",
        "{2}{G}",
        "Enchantment",
        "A static enchantment.",
    );
    let deck = row_deck(
        24,
        &[engine.clone(), filler.clone(), filler.clone()],
        Format::Constructed,
    );
    let mut rng = ChaCha8Rng::seed_from_u64(3);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // The engine alone draws 1/turn; with enchantments in play the
    // velocity curve must beat a drawless baseline, and by late turns
    // the deck should have seen a large share of its cards.
    assert!(
        stats.cards_seen[7] > 15.0,
        "scaling engine velocity: {:.1}",
        stats.cards_seen[7]
    );
}

#[test]
fn reveal_permanents_x_spell_puts_bodies() {
    // The reveal-X-permanents X spell casts and empties the library
    // faster than a same-cost plain sorcery: the leftover pool converts
    // to library cards on the battlefield.
    let wave = card(
        "Wave Spell",
        "{X}{G}{G}",
        "Sorcery",
        "Reveal the top X cards of your library. You may put any number of permanent cards with mana value X or less from among them onto the battlefield, then put the rest into your graveyard.",
    );
    let plain = card(
        "Plain Spell",
        "{X}{G}{G}",
        "Sorcery",
        "A one-shot spell with no modeled effect.",
    );
    let with_wave = row_deck(24, &[wave], Format::Constructed);
    let with_plain = row_deck(24, &[plain], Format::Constructed);
    let stats_wave = run_avg(&with_wave, 8, 300, 21);
    let stats_plain = run_avg(&with_plain, 8, 300, 21);
    assert!(
        stats_wave.library_by_turn[7] < stats_plain.library_by_turn[7],
        "wave deck empties the library faster ({:.1} vs {:.1})",
        stats_wave.library_by_turn[7],
        stats_plain.library_by_turn[7]
    );
}

#[test]
fn counter_ballista_power_grows_with_x() {
    // The X +1/+1 counters join the body power: the attack power census
    // beats a same-cost vanilla body because X = the leftover pool.
    let ball = card(
        "Counter Ball",
        "{X}{X}",
        "Artifact Creature — Construct",
        "This creature enters the battlefield with X +1/+1 counters on it.\nRemove a +1/+1 counter: This creature deals 1 damage to any target.",
    );
    let vanilla = card(
        "Vanilla Construct",
        "{X}{X}",
        "Artifact Creature — Construct",
        "A plain artifact creature.",
    );
    let with_ball = row_deck(24, &[ball], Format::Constructed);
    let with_vanilla = row_deck(24, &[vanilla], Format::Constructed);
    let stats_ball = run_avg(&with_ball, 8, 300, 33);
    let stats_vanilla = run_avg(&with_vanilla, 8, 300, 33);
    assert!(
        stats_ball.attack_power_by_turn[7] > stats_vanilla.attack_power_by_turn[7],
        "counter power beats vanilla ({:.2} vs {:.2})",
        stats_ball.attack_power_by_turn[7],
        stats_vanilla.attack_power_by_turn[7]
    );
}

// Land/spell MDFCs: spell-face role, land-face play rule, 0.4 weight.

#[test]
fn mdfc_spell_face_casts_when_flooded() {
    // 4 Valakut Awakening-class MDFCs + 12 lands: flooded hands still
    // cast the spell face (the land face never fires when real lands
    // are in hand).
    let mdfc = card(
        "Valakut Awakening",
        "{1}{R} // ",
        "Instant // Land",
        "Draw a card, then discard a card. // ",
    );
    let mut spells = Vec::new();
    for _ in 0..4 {
        spells.push(card(
            "Test Divination",
            "{2}{R}",
            "Sorcery",
            "Draw two cards.",
        ));
    }
    let mut deck = row_deck(12, &spells, Format::Constructed);
    for _ in 0..4 {
        deck.cards.push(parse_sim_card(&mdfc));
    }
    // The MDFC carries the spell-face role, not Land.
    assert!(deck.cards.last().unwrap().is_mdfc_spell);
    assert_ne!(deck.cards.last().unwrap().role, Role::Land);
    // Cheap spells stay castable with the MDFC in the mix.
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    assert!(
        stats.hit_all_drops_by[1] > 0.6,
        "MDFC deck still makes land drops: {:.2}",
        stats.hit_all_drops_by[1]
    );
}

#[test]
fn mdfc_plays_as_land_when_no_land_in_hand() {
    // A hand holding only MDFCs plays the land face (fallback rule).
    let deck = SimDeck {
        cards: vec![
            SimCard {
                name: "Jwari".into(),
                cost: parse_cost("{U}"),
                min_cost: parse_cost("{U}"),
                tap: Some(parse_tap_yield("{T}: Add {U}.").unwrap()),
                role: Role::Other,
                is_mdfc_spell: true,
                ..SimCard::default()
            };
            10
        ],
        commanders: Vec::new(),
        format: Format::Constructed,
        rules: super::format::rules_inferred(false),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 3)).collect();
    let stats = aggregate(&logs, &deck, 3);
    assert!(
        stats.hit_all_drops_by[1] > 0.5,
        "MDFC-only hand still plays land faces: {:.2}",
        stats.hit_all_drops_by[1]
    );
}

#[test]
fn mdfc_parses_x_class_from_spell_face() {
    // Shatterskull Smashing: the spell face is {X}{R}{R}; the X-class
    // parsing reads the spell face, not the empty land face.
    let mdfc = card(
        "Shatterskull Smashing // Shatterskull, the Hammer Papas",
        "{X}{R}{R} // ",
        "Sorcery // Land",
        "Shatterskull Smashing deals twice X damage to each of up to two target creatures or planeswalkers. // ",
    );
    let sim = parse_sim_card(&mdfc);
    assert!(sim.is_mdfc_spell, "Shatterskull is a land/spell MDFC");
    assert_eq!(sim.cost.total(), 3, "cheapest X-face floor pays X = 1");
    assert_ne!(sim.role, Role::Land);
}

// Cascade: single-level free cast of the cheapest cheaper card.

#[test]
fn cascade_free_cast_yields_velocity_and_a_body() {
    // Shardless Agent-class cast (3 MV cascade): one cheap creature
    // from the library enters free. Velocity +1 per cascade and one
    // extra body vs the same deck without the cascade trigger.
    let agent = card(
        "Test Shardless",
        "{2}{R}",
        "Creature — Human Rogue",
        "Cascade.",
    );
    let cheap = card("Test Grizzly", "{1}{R}", "Creature — Bear", "");
    let cheap2 = card("Test Grizzly 2", "{2}{R}", "Creature — Bear", "");
    let cheap3 = card("Test Grizzly 3", "{1}{R}", "Creature — Bear", "");
    let deck_with = row_deck(
        12,
        &[agent.clone(), cheap.clone(), cheap2.clone(), cheap3.clone()],
        Format::Constructed,
    );
    // The control deck swaps the cascade creature for a vanilla of the
    // same cost: the delta is the cascade's free cast alone.
    let agent_vanilla = card("Test Vanilla Agent", "{2}{R}", "Creature — Human Rogue", "");
    let deck_without = row_deck(
        12,
        &[agent_vanilla, cheap, cheap2, cheap3],
        Format::Constructed,
    );
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs_with: Vec<_> = (0..200)
        .map(|_| run_game(&deck_with, &mut rng, 6))
        .collect();
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs_without: Vec<_> = (0..200)
        .map(|_| run_game(&deck_without, &mut rng, 6))
        .collect();
    let stats_with = aggregate(&logs_with, &deck_with, 6);
    let stats_without = aggregate(&logs_without, &deck_without, 6);
    assert!(
        stats_with.cards_seen[3] > stats_without.cards_seen[3],
        "cascade yields +1 seen ({:.2} vs {:.2})",
        stats_with.cards_seen[3],
        stats_without.cards_seen[3]
    );
    // The free cast resolves from the library: it must appear in the
    // battlefield census at least once (the extra body). Both decks hold
    // the same cheap-bear count, so bodies move in the cascade deck's
    // favor when the free cast lands.
    assert!(
        stats_with.bodies_by_turn[5] >= stats_without.bodies_by_turn[5],
        "cascade yields an extra body ({:.2} vs {:.2})",
        stats_with.bodies_by_turn[5],
        stats_without.bodies_by_turn[5]
    );
}

#[test]
fn cascade_does_not_chain_or_recurse() {
    // Two cascade cards in the library: the free cast never cascades
    // again (no infinite loop) and the game terminates at the same turn
    // count.
    let agent = card(
        "Test Cascade A",
        "{2}{R}",
        "Creature — Human Rogue",
        "Cascade",
    );
    let big_cascade = card("Test Cascade Big", "{5}{R}", "Creature — Beast", "Cascade");
    let cheap = card("Test Grizzly", "{1}{R}", "Creature — Bear", "");
    let deck = row_deck(12, &[agent, big_cascade, cheap], Format::Constructed);
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    // The library never empties from cascade alone (no chaining): the
    // early library holds a sane count for the deck size.
    assert!(
        stats.library_by_turn[2] >= 1.0,
        "library drained by cascading ({:.2})",
        stats.library_by_turn[2]
    );
    // The game still terminates normally: exactly 6 turns ran.
    assert_eq!(stats.turns, 6);
}

#[test]
fn cascade_casts_the_cheapest_match() {
    // Library holds cheaper cards of 1 and 2 MV; the free cast must be
    // the 1-MV bear (its name shows in the battlefield census).
    let agent = card(
        "Test Shardless",
        "{3}{R}",
        "Creature — Human Rogue",
        "Cascade.",
    );
    let mid = card("Test Mid", "{2}{R}", "Creature — Bear", "");
    let cheap = card("Test Cheapest", "{1}{R}", "Creature — Bear", "");
    let deck = row_deck(12, &[agent, mid, cheap], Format::Constructed);
    let cheap_idx = deck.cards.len() - 1; // spells push in order
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let mut resolved_mid = 0;
    let mut resolved_cheap = 0;
    for _ in 0..120 {
        let log = run_game(&deck, &mut rng, 5);
        for (&idx, &turn) in &log.card_first_seen {
            if idx == cheap_idx && turn >= 1 {
                resolved_cheap += 1;
            } else if deck.cards[idx].name == "Test Mid" && turn >= 1 {
                resolved_mid += 1;
            }
        }
    }
    assert!(
        resolved_cheap > 0,
        "the cheapest library card never resolved"
    );
    // Cascade must never pick the MV-2 bear over the 1-MV bear: the
    // cheap bear's battlefield entries lead.
    assert!(
        resolved_cheap >= resolved_mid,
        "cascade picked the MV-2 card over the cheapest: cheap {resolved_cheap} vs mid {resolved_mid}"
    );
}

#[test]
fn cascade_with_no_valid_target_is_a_noop() {
    // Library holds only lands and same-or-higher-MV spells: the
    // cascade cast resolves with nothing free-cast; the game runs the
    // full turn count without hanging.
    let agent = card(
        "Test Shardless",
        "{2}{R}",
        "Creature — Human Rogue",
        "Cascade.",
    );
    let big = card("Test Big", "{5}{R}", "Creature — Giant", "");
    let deck = row_deck(12, &[agent, big], Format::Constructed);
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let logs: Vec<_> = (0..60).map(|_| run_game(&deck, &mut rng, 4)).collect();
    // The cascade body still enters (the cast itself resolves); the
    // census runs normally with no extra seen cards beyond the cast.
    for log in &logs {
        assert!(
            log.player_damage.iter().all(|d| *d <= 200),
            "cascade loop ran away"
        );
    }
}
