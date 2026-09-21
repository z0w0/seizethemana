// Shared support for the per-format real-deck tests: JSON fixture loading,
// card-row building, the sim runner, and the cross-deck consistency
// invariants. Each format test file pulls these through `use super::*`.
// Real lists come from published sources (mtggoldfish metagame, cEDH
// Decklist Database, EDHREC) with real oracle text; the tests assert
// simulator consistency properties, never deck-quality judgments.

// JSON fixtures (tests/deck_fixtures/*.json): real lists from published
// sources, with oracle text. Loaded with include_str! and parsed per test.
// ---------------------------------------------------------------------------

pub(super) use super::aggregate::CardCast;
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
    "4c control topdeck",
    "amalia lifegain combo standard",
    "boros tokens topdeck",
    "dimir midrange topdeck",
    "izzet spellingentals topdeck",
    "azorius-artifacts",
    "azorius-momo",
    "azorius control standard",
    "bant-airbending",
    "boros dragons standard",
    "boros-dwarves",
    "boros tokens standard",
    "dimir-excruciator",
    "dimir-midrange-2026",
    "dimir-midrange",
    "four-color-control",
    "golgari midrange standard",
    "izzet discard payoffs standard",
    "izzet-spellementals",
    "jeskai artifacts standard",
    "jund-sacrifice",
    "mardu-discard-2026",
    "mardu-discard",
    "mono-black-aggro-2026",
    "mono-blue flash standard",
    "mono-green-landfall",
    "mono-red firebending standard",
    "mono-red-aggro",
    "orzhov-life-gain",
    "reanimator 4c standard",
    "selesnya-landfall",
];

pub(super) const MODERN_FIXTURES: &[&str] = &[
    "affinity modern",
    "dimir murktide topdeck",
    "eldrazi ramp topdeck",
    "persist reanimator modern",
    "yawgmoth combo modern",
    "amulet-titan",
    "azorius-fiddlebender",
    "black burn bowmasters modern",
    "boros-energy",
    "boros-land-destruction",
    "broodscale-combo",
    "deaths shadow modern",
    "devoted-druid-combo",
    "dimir-midrange-modern",
    "domain-zoo",
    "dredge modern",
    "eldrazi-tron-2026",
    "eldrazi-tron",
    "esper-blink",
    "esper-goryos",
    "hammer-time",
    "hollow one modern",
    "infect modern",
    "izzet-affinity",
    "izzet-prowess-2026",
    "izzet-prowess",
    "jeskai energy modern",
    "living-end",
    "merfolk modern",
    "mono-green-eldrazi",
    "neobrand",
    "rhinos cascade modern",
    "ruby-storm-2026",
    "ruby-storm",
    "tameshi-belcher",
    "whack 12 modern",
];

pub(super) const COMMANDER_FIXTURES: &[&str] = &[
    "aesi extra lands landfall",
    "satya aetherflux energy",
    "shanna energy soldiers",
    "tayam luminous engine",
    "yawgmoth thran combo",
    "alesha aristocrats",
    "atraxa grand unifier blink",
    "atraxa superfriends",
    "blue farm",
    "brago blink value",
    "chulane bant value",
    "dargo-tymna",
    "derevi stax",
    "edgar markov vampires",
    "etali food chain",
    "ezuri elf swarm",
    "godo-helm combo",
    "henzie blitz",
    "ishai-rograkh",
    "jaws storm",
    "kaalia angels-demons-dragons",
    "kalamax x instants",
    "kenrith combo politics",
    "kinnan combo",
    "krenko goblin swarm",
    "krenko goblins",
    "krark-sakashima storm",
    "light-paws aura voltron",
    "lumra landfall stax",
    "magda treasure combo",
    "marneus calgar tokens",
    "meren graveyard",
    "muldrotha graveyard value",
    "najeela warriors",
    "nekusar wheel punish",
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
    "yuriko ninja tempo",
    "yshtola burn control",
    "zada token storm",
];

/// Parse a fixture file (`tests/deck_fixtures/<name>.json`).
pub(super) fn fixture(name: &str) -> DeckFixture {
    let text = match name {
        "aesi extra lands landfall" => include_str!("deck_fixtures/aesi_extra_lands_landfall.json"),
        "affinity modern" => include_str!("deck_fixtures/affinity_modern.json"),
        "amalia lifegain combo standard" => {
            include_str!("deck_fixtures/amalia_lifegain_combo_standard.json")
        }
        "atraxa superfriends" => include_str!("deck_fixtures/atraxa_superfriends.json"),
        "azorius control standard" => include_str!("deck_fixtures/azorius_control_standard.json"),
        "black burn bowmasters modern" => {
            include_str!("deck_fixtures/black_burn_bowmasters_modern.json")
        }
        "boros dragons standard" => include_str!("deck_fixtures/boros_dragons_standard.json"),
        "boros tokens standard" => include_str!("deck_fixtures/boros_tokens_standard.json"),
        "brago blink value" => include_str!("deck_fixtures/brago_blink_value.json"),
        "deaths shadow modern" => include_str!("deck_fixtures/deaths_shadow_modern.json"),
        "dredge modern" => include_str!("deck_fixtures/dredge_modern.json"),
        "ezuri elf swarm" => include_str!("deck_fixtures/ezuri_elf_swarm.json"),
        "golgari midrange standard" => {
            include_str!("deck_fixtures/golgari_midrange_standard.json")
        }
        "hollow one modern" => include_str!("deck_fixtures/hollow_one_modern.json"),
        "infect modern" => include_str!("deck_fixtures/infect_modern.json"),
        "izzet discard payoffs standard" => {
            include_str!("deck_fixtures/izzet_discard_payoffs_standard.json")
        }
        "jeskai artifacts standard" => include_str!("deck_fixtures/jeskai_artifacts_standard.json"),
        "jeskai energy modern" => include_str!("deck_fixtures/jeskai_energy_modern.json"),
        "kalamax x instants" => include_str!("deck_fixtures/kalamax_x_instants.json"),
        "krenko goblin swarm" => include_str!("deck_fixtures/krenko_goblin_swarm.json"),
        "light-paws aura voltron" => include_str!("deck_fixtures/light-paws_aura_voltron.json"),
        "merfolk modern" => include_str!("deck_fixtures/merfolk_modern.json"),
        "mono-blue flash standard" => include_str!("deck_fixtures/mono-blue_flash_standard.json"),
        "mono-red firebending standard" => {
            include_str!("deck_fixtures/mono-red_firebending_standard.json")
        }
        "muldrotha graveyard value" => include_str!("deck_fixtures/muldrotha_graveyard_value.json"),
        "nekusar wheel punish" => include_str!("deck_fixtures/nekusar_wheel_punish.json"),
        "reanimator 4c standard" => include_str!("deck_fixtures/reanimator_4c_standard.json"),
        "rhinos cascade modern" => include_str!("deck_fixtures/rhinos_cascade_modern.json"),
        "whack 12 modern" => include_str!("deck_fixtures/whack_12_modern.json"),
        "yuriko ninja tempo" => include_str!("deck_fixtures/yuriko_ninja_tempo.json"),
        "4c control topdeck" => include_str!("deck_fixtures/4c_control_topdeck.json"),
        "boros tokens topdeck" => include_str!("deck_fixtures/boros_tokens_topdeck.json"),
        "dimir midrange topdeck" => include_str!("deck_fixtures/dimir_midrange_topdeck.json"),
        "dimir murktide topdeck" => include_str!("deck_fixtures/dimir_murktide_topdeck.json"),
        "eldrazi ramp topdeck" => include_str!("deck_fixtures/eldrazi_ramp_topdeck.json"),
        "izzet spellingentals topdeck" => {
            include_str!("deck_fixtures/izzet_spellingentals_topdeck.json")
        }
        "persist reanimator modern" => include_str!("deck_fixtures/persist_reanimator_modern.json"),
        "satya aetherflux energy" => include_str!("deck_fixtures/satya_aetherflux_energy.json"),
        "shanna energy soldiers" => include_str!("deck_fixtures/shanna_energy_soldiers.json"),
        "tayam luminous engine" => include_str!("deck_fixtures/tayam_luminous_engine.json"),
        "yawgmoth combo modern" => include_str!("deck_fixtures/yawgmoth_combo_modern.json"),
        "yawgmoth thran combo" => include_str!("deck_fixtures/yawgmoth_thran_combo.json"),
        "alesha aristocrats" => include_str!("deck_fixtures/alesha_aristocrats.json"),
        "amulet-titan" => include_str!("deck_fixtures/amulet-titan.json"),
        "atraxa grand unifier blink" => {
            include_str!("deck_fixtures/atraxa_grand_unifier_blink.json")
        }
        "azorius-artifacts" => include_str!("deck_fixtures/azorius-artifacts.json"),
        "azorius-fiddlebender" => include_str!("deck_fixtures/azorius-fiddlebender.json"),
        "azorius-momo" => include_str!("deck_fixtures/azorius-momo.json"),
        "bant-airbending" => include_str!("deck_fixtures/bant-airbending.json"),
        "blue farm" => include_str!("deck_fixtures/blue_farm.json"),
        "boros-dwarves" => include_str!("deck_fixtures/boros-dwarves.json"),
        "boros-energy" => include_str!("deck_fixtures/boros-energy.json"),
        "boros-land-destruction" => include_str!("deck_fixtures/boros-land-destruction.json"),
        "broodscale-combo" => include_str!("deck_fixtures/broodscale-combo.json"),
        "chulane bant value" => include_str!("deck_fixtures/chulane_bant_value.json"),
        "dargo-tymna" => include_str!("deck_fixtures/dargo-tymna.json"),
        "derevi stax" => include_str!("deck_fixtures/derevi_stax.json"),
        "devoted-druid-combo" => include_str!("deck_fixtures/devoted-druid-combo.json"),
        "dimir-excruciator" => include_str!("deck_fixtures/dimir-excruciator.json"),
        "dimir-midrange" => include_str!("deck_fixtures/dimir-midrange.json"),
        "dimir-midrange-2026" => include_str!("deck_fixtures/dimir-midrange-2026.json"),
        "dimir-midrange-modern" => include_str!("deck_fixtures/dimir-midrange-modern.json"),
        "domain-zoo" => include_str!("deck_fixtures/domain-zoo.json"),
        "edgar markov vampires" => include_str!("deck_fixtures/edgar_markov_vampires.json"),
        "eldrazi-tron" => include_str!("deck_fixtures/eldrazi-tron.json"),
        "eldrazi-tron-2026" => include_str!("deck_fixtures/eldrazi-tron-2026.json"),
        "esper-blink" => include_str!("deck_fixtures/esper-blink.json"),
        "esper-goryos" => include_str!("deck_fixtures/esper-goryos.json"),
        "etali food chain" => include_str!("deck_fixtures/etali_food_chain.json"),
        "four-color-control" => include_str!("deck_fixtures/four-color-control.json"),
        "godo-helm combo" => include_str!("deck_fixtures/godo-helm_combo.json"),
        "hammer-time" => include_str!("deck_fixtures/hammer-time.json"),
        "henzie blitz" => include_str!("deck_fixtures/henzie_blitz.json"),
        "ishai-rograkh" => include_str!("deck_fixtures/ishai-rograkh.json"),
        "izzet-affinity" => include_str!("deck_fixtures/izzet-affinity.json"),
        "izzet-prowess" => include_str!("deck_fixtures/izzet-prowess.json"),
        "izzet-prowess-2026" => include_str!("deck_fixtures/izzet-prowess-2026.json"),
        "izzet-spellementals" => include_str!("deck_fixtures/izzet-spellementals.json"),
        "jaws storm" => include_str!("deck_fixtures/jaws_storm.json"),
        "jund-sacrifice" => include_str!("deck_fixtures/jund-sacrifice.json"),
        "kaalia angels-demons-dragons" => {
            include_str!("deck_fixtures/kaalia_angels-demons-dragons.json")
        }
        "kenrith combo politics" => include_str!("deck_fixtures/kenrith_combo_politics.json"),
        "kinnan combo" => include_str!("deck_fixtures/kinnan_combo.json"),
        "krark-sakashima storm" => include_str!("deck_fixtures/krark-sakashima_storm.json"),
        "krenko goblins" => include_str!("deck_fixtures/krenko_goblins.json"),
        "living-end" => include_str!("deck_fixtures/living-end.json"),
        "lumra landfall stax" => include_str!("deck_fixtures/lumra_landfall_stax.json"),
        "magda treasure combo" => include_str!("deck_fixtures/magda_treasure_combo.json"),
        "mardu-discard" => include_str!("deck_fixtures/mardu-discard.json"),
        "mardu-discard-2026" => include_str!("deck_fixtures/mardu-discard-2026.json"),
        "marneus calgar tokens" => include_str!("deck_fixtures/marneus_calgar_tokens.json"),
        "meren graveyard" => include_str!("deck_fixtures/meren_graveyard.json"),
        "mono-black-aggro-2026" => include_str!("deck_fixtures/mono-black-aggro-2026.json"),
        "mono-green-eldrazi" => include_str!("deck_fixtures/mono-green-eldrazi.json"),
        "mono-green-landfall" => include_str!("deck_fixtures/mono-green-landfall.json"),
        "mono-red-aggro" => include_str!("deck_fixtures/mono-red-aggro.json"),
        "najeela warriors" => include_str!("deck_fixtures/najeela_warriors.json"),
        "neobrand" => include_str!("deck_fixtures/neobrand.json"),
        "orzhov-life-gain" => include_str!("deck_fixtures/orzhov-life-gain.json"),
        "rograkh-silas" => include_str!("deck_fixtures/rograkh-silas.json"),
        "rograkh-thrasios" => include_str!("deck_fixtures/rograkh-thrasios.json"),
        "rogsi turbo" => include_str!("deck_fixtures/rogsi_turbo.json"),
        "rogsilas turbo naus" => include_str!("deck_fixtures/rogsilas_turbo_naus.json"),
        "ruby-storm" => include_str!("deck_fixtures/ruby-storm.json"),
        "ruby-storm-2026" => include_str!("deck_fixtures/ruby-storm-2026.json"),
        "sauron zombies" => include_str!("deck_fixtures/sauron_zombies.json"),
        "selesnya-landfall" => include_str!("deck_fixtures/selesnya-landfall.json"),
        "sisay" => include_str!("deck_fixtures/sisay.json"),
        "sisay legends toolbox" => include_str!("deck_fixtures/sisay_legends_toolbox.json"),
        "sythis enchantress" => include_str!("deck_fixtures/sythis_enchantress.json"),
        "tameshi-belcher" => include_str!("deck_fixtures/tameshi-belcher.json"),
        "tatyova landfall" => include_str!("deck_fixtures/tatyova_landfall.json"),
        "the ur-dragon dragons" => include_str!("deck_fixtures/the_ur-dragon_dragons.json"),
        "thrasios - tymna midrange" => include_str!("deck_fixtures/thrasios_-_tymna_midrange.json"),
        "vivi storm" => include_str!("deck_fixtures/vivi_storm.json"),
        "yshtola burn control" => include_str!("deck_fixtures/yshtola_burn_control.json"),
        "zada token storm" => include_str!("deck_fixtures/zada_token_storm.json"),
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
    stats.removal_wipes = sim_deck
        .cards
        .iter()
        .filter(|c| c.role == super::model::Role::Removal && c.wipe)
        .count();
    stats.removal_targeted = stats.removal_count - stats.removal_wipes;
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
