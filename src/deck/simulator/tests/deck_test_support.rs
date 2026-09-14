// Shared support for the per-format real-deck tests: JSON fixture loading,
// card-row building, the sim runner, and the cross-deck consistency
// invariants. Each format test file pulls these through `use super::*`.
// Real lists come from published sources (mtggoldfish metagame, cEDH
// Decklist Database, EDHREC) with real oracle text; the tests assert
// simulator consistency properties, never deck-quality judgments.

// JSON fixtures (tests/deck_fixtures/*.json): real lists from published
// sources, with oracle text. Loaded with include_str! and parsed per test.
// ---------------------------------------------------------------------------

/// One card in a fixture file.
use super::aggregate::aggregate;
pub(super) use super::deck::build_sim_deck;
pub(super) use super::game::run_game;

pub(super) use super::parse::{parse_cost, parse_sim_card};
use crate::db::CardRow;
use crate::deck::grammar::{Deck, DeckEntry};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

#[derive(serde::Deserialize)]
pub(super) struct FixtureCard {
    name: String,
    cost: String,
    #[serde(rename = "type_line")]
    type_line: String,
    keywords: String,
    text: String,
}

/// One deck fixture: commanders, maindeck cards, and the played list.
#[derive(serde::Deserialize)]
pub(super) struct DeckFixture {
    commanders: Vec<FixtureCard>,
    cards: Vec<FixtureCard>,
    list: Vec<(String, i64)>,
}

/// Every fixture id, by format. The sweep tests iterate these.
pub(super) const STANDARD_FIXTURES: &[&str] = &[
    "azorius-artifacts",
    "azorius-momo",
    "bant-airbending",
    "boros-dwarves",
    "dimir-excruciator",
    "dimir-midrange-2026",
    "dimir-midrange",
    "four-color-control",
    "izzet-spellementals",
    "jund-sacrifice",
    "mardu-discard-2026",
    "mardu-discard",
    "mono-black-aggro-2026",
    "mono-green-landfall",
    "mono-red-aggro",
    "orzhov-life-gain",
    "selesnya-landfall",
];

pub(super) const MODERN_FIXTURES: &[&str] = &[
    "amulet-titan",
    "azorius-fiddlebender",
    "boros-energy",
    "boros-land-destruction",
    "broodscale-combo",
    "devoted-druid-combo",
    "dimir-midrange-modern",
    "domain-zoo",
    "eldrazi-tron-2026",
    "eldrazi-tron",
    "esper-blink",
    "esper-goryos",
    "hammer-time",
    "izzet-affinity",
    "izzet-prowess-2026",
    "izzet-prowess",
    "living-end",
    "mono-green-eldrazi",
    "neobrand",
    "ruby-storm-2026",
    "ruby-storm",
    "tameshi-belcher",
];

pub(super) const COMMANDER_FIXTURES: &[&str] = &[
    "alesha aristocrats",
    "atraxa grand unifier blink",
    "blue farm",
    "chulane bant value",
    "dargo-tymna",
    "derevi stax",
    "edgar markov vampires",
    "etali food chain",
    "godo-helm combo",
    "henzie blitz",
    "ishai-rograkh",
    "jaws storm",
    "kaalia angels-demons-dragons",
    "kenrith combo politics",
    "kinnan combo",
    "krark-sakashima storm",
    "krenko goblins",
    "lumra landfall stax",
    "magda treasure combo",
    "marneus calgar tokens",
    "meren graveyard",
    "najeela warriors",
    "rograkh-silas",
    "rograkh-thrasios",
    "rogsi turbo",
    "rogsilas turbo naus",
    "sauron zombies",
    "sisay legends toolbox",
    "sisay",
    "sythis enchantress",
    "tatyova landfall",
    "the ur-dragon dragons",
    "thrasios - tymna midrange",
    "vivi storm",
    "yshtola burn control",
    "zada token storm",
];

/// Parse a fixture file (`tests/deck_fixtures/<name>.json`).
pub(super) fn fixture(name: &str) -> DeckFixture {
    let text = match name {
        "alesha aristocrats" => include_str!("deck_fixtures/alesha aristocrats.json"),
        "amulet-titan" => include_str!("deck_fixtures/amulet-titan.json"),
        "atraxa grand unifier blink" => {
            include_str!("deck_fixtures/atraxa grand unifier blink.json")
        }
        "azorius-artifacts" => include_str!("deck_fixtures/azorius-artifacts.json"),
        "azorius-fiddlebender" => include_str!("deck_fixtures/azorius-fiddlebender.json"),
        "azorius-momo" => include_str!("deck_fixtures/azorius-momo.json"),
        "bant-airbending" => include_str!("deck_fixtures/bant-airbending.json"),
        "blue farm" => include_str!("deck_fixtures/blue farm.json"),
        "boros-dwarves" => include_str!("deck_fixtures/boros-dwarves.json"),
        "boros-energy" => include_str!("deck_fixtures/boros-energy.json"),
        "boros-land-destruction" => include_str!("deck_fixtures/boros-land-destruction.json"),
        "broodscale-combo" => include_str!("deck_fixtures/broodscale-combo.json"),
        "chulane bant value" => include_str!("deck_fixtures/chulane bant value.json"),
        "dargo-tymna" => include_str!("deck_fixtures/dargo-tymna.json"),
        "derevi stax" => include_str!("deck_fixtures/derevi stax.json"),
        "devoted-druid-combo" => include_str!("deck_fixtures/devoted-druid-combo.json"),
        "dimir-excruciator" => include_str!("deck_fixtures/dimir-excruciator.json"),
        "dimir-midrange" => include_str!("deck_fixtures/dimir-midrange.json"),
        "dimir-midrange-2026" => include_str!("deck_fixtures/dimir-midrange-2026.json"),
        "dimir-midrange-modern" => include_str!("deck_fixtures/dimir-midrange-modern.json"),
        "domain-zoo" => include_str!("deck_fixtures/domain-zoo.json"),
        "edgar markov vampires" => include_str!("deck_fixtures/edgar markov vampires.json"),
        "eldrazi-tron" => include_str!("deck_fixtures/eldrazi-tron.json"),
        "eldrazi-tron-2026" => include_str!("deck_fixtures/eldrazi-tron-2026.json"),
        "esper-blink" => include_str!("deck_fixtures/esper-blink.json"),
        "esper-goryos" => include_str!("deck_fixtures/esper-goryos.json"),
        "etali food chain" => include_str!("deck_fixtures/etali food chain.json"),
        "four-color-control" => include_str!("deck_fixtures/four-color-control.json"),
        "godo-helm combo" => include_str!("deck_fixtures/godo-helm combo.json"),
        "hammer-time" => include_str!("deck_fixtures/hammer-time.json"),
        "henzie blitz" => include_str!("deck_fixtures/henzie blitz.json"),
        "ishai-rograkh" => include_str!("deck_fixtures/ishai-rograkh.json"),
        "izzet-affinity" => include_str!("deck_fixtures/izzet-affinity.json"),
        "izzet-prowess" => include_str!("deck_fixtures/izzet-prowess.json"),
        "izzet-prowess-2026" => include_str!("deck_fixtures/izzet-prowess-2026.json"),
        "izzet-spellementals" => include_str!("deck_fixtures/izzet-spellementals.json"),
        "jaws storm" => include_str!("deck_fixtures/jaws storm.json"),
        "jund-sacrifice" => include_str!("deck_fixtures/jund-sacrifice.json"),
        "kaalia angels-demons-dragons" => {
            include_str!("deck_fixtures/kaalia angels-demons-dragons.json")
        }
        "kenrith combo politics" => include_str!("deck_fixtures/kenrith combo politics.json"),
        "kinnan combo" => include_str!("deck_fixtures/kinnan combo.json"),
        "krark-sakashima storm" => include_str!("deck_fixtures/krark-sakashima storm.json"),
        "krenko goblins" => include_str!("deck_fixtures/krenko goblins.json"),
        "living-end" => include_str!("deck_fixtures/living-end.json"),
        "lumra landfall stax" => include_str!("deck_fixtures/lumra landfall stax.json"),
        "magda treasure combo" => include_str!("deck_fixtures/magda treasure combo.json"),
        "mardu-discard" => include_str!("deck_fixtures/mardu-discard.json"),
        "mardu-discard-2026" => include_str!("deck_fixtures/mardu-discard-2026.json"),
        "marneus calgar tokens" => include_str!("deck_fixtures/marneus calgar tokens.json"),
        "meren graveyard" => include_str!("deck_fixtures/meren graveyard.json"),
        "mono-black-aggro-2026" => include_str!("deck_fixtures/mono-black-aggro-2026.json"),
        "mono-green-eldrazi" => include_str!("deck_fixtures/mono-green-eldrazi.json"),
        "mono-green-landfall" => include_str!("deck_fixtures/mono-green-landfall.json"),
        "mono-red-aggro" => include_str!("deck_fixtures/mono-red-aggro.json"),
        "najeela warriors" => include_str!("deck_fixtures/najeela warriors.json"),
        "neobrand" => include_str!("deck_fixtures/neobrand.json"),
        "orzhov-life-gain" => include_str!("deck_fixtures/orzhov-life-gain.json"),
        "rograkh-silas" => include_str!("deck_fixtures/rograkh-silas.json"),
        "rograkh-thrasios" => include_str!("deck_fixtures/rograkh-thrasios.json"),
        "rogsi turbo" => include_str!("deck_fixtures/rogsi turbo.json"),
        "rogsilas turbo naus" => include_str!("deck_fixtures/rogsilas turbo naus.json"),
        "ruby-storm" => include_str!("deck_fixtures/ruby-storm.json"),
        "ruby-storm-2026" => include_str!("deck_fixtures/ruby-storm-2026.json"),
        "sauron zombies" => include_str!("deck_fixtures/sauron zombies.json"),
        "selesnya-landfall" => include_str!("deck_fixtures/selesnya-landfall.json"),
        "sisay" => include_str!("deck_fixtures/sisay.json"),
        "sisay legends toolbox" => include_str!("deck_fixtures/sisay legends toolbox.json"),
        "sythis enchantress" => include_str!("deck_fixtures/sythis enchantress.json"),
        "tameshi-belcher" => include_str!("deck_fixtures/tameshi-belcher.json"),
        "tatyova landfall" => include_str!("deck_fixtures/tatyova landfall.json"),
        "the ur-dragon dragons" => include_str!("deck_fixtures/the ur-dragon dragons.json"),
        "thrasios - tymna midrange" => include_str!("deck_fixtures/thrasios - tymna midrange.json"),
        "vivi storm" => include_str!("deck_fixtures/vivi storm.json"),
        "yshtola burn control" => include_str!("deck_fixtures/yshtola burn control.json"),
        "zada token storm" => include_str!("deck_fixtures/zada token storm.json"),
        _ => unreachable!("unknown fixture {name}"),
    };
    serde_json::from_str(text).expect("fixture JSON parses")
}

/// Fixture cards as a name → row map (maindeck only).
pub(super) fn fixture_cards(name: &str) -> HashMap<String, CardRow> {
    let f = fixture(name);
    f.cards
        .into_iter()
        .map(|c| {
            let row = real_card(&c.name, &c.cost, &c.type_line, &c.keywords, &c.text);
            (c.name, row)
        })
        .collect()
}

/// Fixture commanders + maindeck in one name → row map.
pub(super) fn fixture_map(name: &str) -> HashMap<String, CardRow> {
    let f = fixture(name);
    let mut m = HashMap::new();
    for c in f.commanders.into_iter().chain(f.cards) {
        let row = real_card(&c.name, &c.cost, &c.type_line, &c.keywords, &c.text);
        m.insert(c.name, row);
    }
    m
}

/// The fixture's played list as a `Deck` (commanders in COMMANDER section).
pub(super) fn fixture_deck(name: &str) -> Deck {
    let f = fixture(name);
    let mut deck = Deck::default();
    if !f.commanders.is_empty() {
        let cmd = deck.section_entries_mut("COMMANDER");
        for c in &f.commanders {
            cmd.push(DeckEntry {
                quantity: 1,
                name: c.name.clone(),
                set_code: None,
                collector_number: None,
                foil: false,
            });
        }
    }
    let list = deck.section_entries_mut("DECK");
    for (cname, qty) in &f.list {
        list.push(DeckEntry {
            quantity: *qty,
            name: cname.clone(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    deck
}

/// Run `runs` games and aggregate.
pub(super) fn sim(
    deck: &Deck,
    cards: &HashMap<String, CardRow>,
    runs: u32,
    turns: u32,
) -> super::aggregate::SimStats {
    let sim_deck = build_sim_deck(deck, cards, None);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let logs: Vec<_> = (0..runs)
        .map(|_| run_game(&sim_deck, &mut rng, turns))
        .collect();
    let mut stats = aggregate(&logs, &sim_deck, turns);
    stats.land_count = sim_deck
        .cards
        .iter()
        .filter(|c| c.role == super::model::Role::Land)
        .count();
    stats.draw_count = sim_deck
        .cards
        .iter()
        .filter(|c| c.role == super::model::Role::Draw)
        .count();
    stats.wincon_count = sim_deck
        .cards
        .iter()
        .filter(|c| c.role == super::model::Role::Wincon)
        .count();
    stats.removal_count = sim_deck
        .cards
        .iter()
        .filter(|c| c.role == super::model::Role::Removal)
        .count();
    stats
}

// ---------------------------------------------------------------------------
// Shared consistency invariants. Every real deck must hold these; failure
// points at a simulator bug, not at deck quality.
// ---------------------------------------------------------------------------

/// Velocity is cumulative: cards_seen never regresses turn over turn.
pub(super) fn assert_velocity_monotone(stats: &super::aggregate::SimStats) {
    for w in stats.cards_seen.windows(2) {
        assert!(
            w[1] >= w[0] - 0.01,
            "velocity regressed: {:.2} → {:.2}",
            w[0],
            w[1]
        );
    }
}

/// Land drops never regress and never exceed the physical maximum.
pub(super) fn assert_land_drops_sane(stats: &super::aggregate::SimStats, turns: u32) {
    assert!(
        stats.p50_drops_by_4 <= 4 + (turns > 4) as u32,
        "median drops by t4 impossibly high: {}",
        stats.p50_drops_by_4
    );
    assert!(
        stats.hit_all_drops_by[1] >= 0.5,
        "no deck misses the t1 land drop this often: {:.2}",
        stats.hit_all_drops_by[1]
    );
    assert!(
        stats.avg_opener_lands >= 0.5,
        "openers have no lands at all"
    );
    assert!(stats.avg_opener_lands <= 7.0);
}

/// minimum cost allows (cmc rounds down + 1 for the cast turn itself).
pub(super) fn assert_castability_not_before_cost(
    stats: &super::aggregate::SimStats,
    cards: &HashMap<String, CardRow>,
) {
    for c in &stats.card_castability {
        let Some(row) = cards.get(&c.name) else {
            continue;
        };
        // Floor is the discount-aware min cost (affinity-lite, warp),
        // matching what the sim actually pays; the printed cost is the
        // full-price ceiling.
        let sim = parse_sim_card(row);
        let floor = sim.min_cost.total() as f64;
        let face_min = row
            .mana_cost
            .split(" // ")
            .filter(|face| !face.trim().is_empty())
            .map(parse_cost)
            .map(|cost| cost.total())
            .min()
            .unwrap_or(0) as f64;
        // Expensive cards cannot be board-ready absurdly early: the pool
        // grows by at most a few mana per turn. Ramp shells reach 11 by
        // t7 (temples, labyrinths), so the bound is logarithmic-ish:
        // min cost 11+ can be ready before 11 but not before the midgame.
        let grace = (floor * 0.5).max(1.5);
        assert!(
            c.avg_first_castable_turn + grace >= floor,
            "{} castable avg t{:.2} but min cost {} (printed {})",
            c.name,
            c.avg_first_castable_turn,
            sim.min_cost.total(),
            row.mana_cost
        );
        // Reductions (warp, affinity-lite) only go down for one-face cards.
        // Split cards ("Dusk // Dawn") concatenate both faces in one cost
        // string, which over-counts; over-costing is the safe direction
        // (casts later than reality). MDFC lands have one empty face and
        // cost nothing to play as a land.
        let faces = row
            .mana_cost
            .split(" // ")
            .filter(|face| !face.trim().is_empty())
            .count();
        if faces > 1 {
            // Split/Adventure cards pay their cheaper face (parse_cost_faces);
            // reductions may discount it further. Only the mechanical
            // castability bound above applies here.
            continue;
        }
        // Reductions only go down. Exact printed alternative costs (warp)
        // can be much cheaper; approximated reductions ("{x} less",
        // affinity-lite) respect the documented 2-generic floor.
        assert!(
            floor <= face_min,
            "{} min cost {} exceeds printed face {}",
            c.name,
            floor,
            face_min
        );
        let warp_text = row.oracle_text.to_ascii_lowercase().contains("warp ");
        if !warp_text {
            assert!(
                floor + 2.0 >= face_min,
                "{} min cost {} below printed face {} by more than the discount floor",
                c.name,
                floor,
                face_min
            );
        }
    }
}

/// A real card's metadata, with oracle text from the fixture file.
pub(super) fn real_card(
    name: &str,
    mana_cost: &str,
    type_line: &str,
    keywords: &str,
    text: &str,
) -> CardRow {
    CardRow {
        name: name.to_string(),
        oracle_id: String::new(),
        mana_cost: mana_cost.to_string(),
        cmc: parse_cost(mana_cost).total() as f64,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: keywords.to_string(),
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
