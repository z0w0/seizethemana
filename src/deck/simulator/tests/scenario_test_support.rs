//! Controlled setup shared by simulator action, card, and turn scenarios.

use super::game::GameState;
use super::model::{Format, SimDeck};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::cost::parse_cost;
use crate::db::CardRow;
use std::collections::HashMap;

/// Build a card row for controlled simulator tests.
pub(super) fn row(name: &str, cost: &str, type_line: &str, text: &str) -> CardRow {
    CardRow {
        name: name.into(),
        oracle_id: String::new(),
        mana_cost: cost.into(),
        cmc: parse_cost(cost).total() as f64,
        type_line: type_line.into(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.into(),
        rarity: "common".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: String::new(),
        collector_number: String::new(),
        scryfall_id: String::new(),
        released_at: String::new(),
        game_changer: None,
        penny_rank: None,
        reserved: None,
    }
}

/// Parse test rows into a constructed simulator deck.
pub(super) fn deck(rows: &[CardRow]) -> SimDeck {
    SimDeck {
        companion: None,
        cards: rows.iter().map(parse_sim_card).collect(),
        commanders: Vec::new(),
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    }
}

/// Create a game state with exact hand and library card indexes.
pub(super) fn state(hand: Vec<usize>, library: Vec<usize>) -> GameState {
    let to_idx = |v: Vec<usize>| {
        v.into_iter()
            .map(|i| super::model::CardIdx(i as u32))
            .collect()
    };
    GameState {
        completion: None,
        failed_draw: false,
        current_turn: 0,
        current_phase: super::game::GamePhase::Upkeep,
        graveyard_generations: HashMap::new(),
        resolution: Default::default(),
        library_rng: super::rng::game_rng(0, 0, super::rng::RngDomain::Library),
        battlefield: Vec::new(),
        library: to_idx(library),
        hand: to_idx(hand),
        seen: 0,
        graveyard: Vec::new(),
        exile: Vec::new(),
        battlefield_seen: HashMap::new(),
        graveyard_seen: HashMap::new(),
        card_first_seen: HashMap::new(),
        treasure_bank: 0,
        milled_self: 0,
        milled_opp: 0,
        opponent_life_lost: 0,
        damage_dealt_this_turn: 0,
        life_gained: 0,
        flashback_permissions: std::collections::HashSet::new(),
        replay_casts: 0,
        milestones_by_turn: HashMap::new(),
        life_paid: 0,
        life_funded_draws: 0,
        life: 20,
        is_monarch: false,
        awareness_cards: 0,
        extra_turns_queued: 0,
        prowess_casts: 0,
        infinite_mana_suspected: false,
        next_uid: 0,
        player_counters: Default::default(),
        activated_this_turn: Default::default(),
        activated_once: Default::default(),
        triggered_this_turn: Default::default(),
        companion_fetched: false,
        ring_tempts: 0,
        extra_land_drops_this_turn: 0,
        spells_cast_this_turn: 0,
        attacked_this_turn: false,
        plotted: Vec::new(),
        ring_bearer: None,
        commander_casts: Vec::new(),
        opponents: super::opponents::OpponentTable::for_format(Format::Constructed),
    }
}
