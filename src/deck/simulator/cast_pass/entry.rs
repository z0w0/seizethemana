//! Battlefield entries produced by supported spell resolutions.

use super::super::game::{
    GameState, controlled_land_types, fire_on_enter, new_perm_with, new_token_perm,
    register_loyalty_token_source, take_uid,
};
use super::super::model::{CardIdx, SacrificeSearch, SimDeck};
use super::super::resolution::{begin_resolution, end_resolution};

/// Read the actual board before entry, without counting hidden or unchosen land faces.
fn land_entry_counts(st: &GameState) -> (u32, u32) {
    let lands = st
        .battlefield
        .iter()
        .filter(|permanent| super::super::game::is_land_permanent(permanent))
        .count() as u32;
    let opponents = st.opponents.seats.iter().filter(|seat| seat.active).count() as u32;
    (lands, opponents)
}

/// Preview tapped entry without spending life or changing the board.
pub(in crate::deck::simulator) fn entry_is_tapped(
    deck: &SimDeck,
    st: &GameState,
    index: CardIdx,
    land_face: bool,
) -> bool {
    let card = &deck[index];
    if card.is_mdfc_spell && !land_face {
        return false;
    }
    let (lands, opponents) = land_entry_counts(st);
    let tapped = if land_face && let Some(spec) = &card.behold_untapped_entry {
        !super::super::game_effects::behold::available(deck, st, spec, Some(index))
    } else {
        card.entry_is_tapped_with_land_types(
            lands,
            opponents,
            controlled_land_types(deck, &st.battlefield),
        )
    };
    tapped
        || land_face && card.life_to_untap > 0 && i64::from(st.life) < i64::from(card.life_to_untap)
}

/// Build a front-face entry with the shared tapped-state and optional life rule.
pub(in crate::deck::simulator) fn new_entry_permanent(
    deck: &SimDeck,
    st: &mut GameState,
    index: CardIdx,
    turn: u32,
    forced_tapped: bool,
) -> super::super::game::Permanent {
    let uid = take_uid(st);
    let mut permanent = new_perm_with(uid, deck, index, turn, false);
    permanent.tapped = entry_tapped(deck, st, index, forced_tapped, permanent.land_face);
    permanent
}

/// Apply one entry choice. A forced tapped destination never pays optional untap life.
pub(in crate::deck::simulator) fn entry_tapped(
    deck: &SimDeck,
    st: &mut GameState,
    index: CardIdx,
    forced_tapped: bool,
    land_face: bool,
) -> bool {
    if forced_tapped || entry_is_tapped(deck, st, index, land_face) {
        return true;
    }
    if land_face {
        let life = deck[index].life_to_untap;
        st.life -= life as i32;
        st.life_paid += life;
    }
    false
}

/// Dispatch entry events from the physical face after its placement is complete.
pub(in crate::deck::simulator) fn fire_entry_triggers(
    deck: &SimDeck,
    st: &mut GameState,
    position: usize,
    turn: u32,
) {
    let land = super::super::game::is_land_permanent(&st.battlefield[position]);
    fire_on_enter(deck, st, position, turn, false);
    if land {
        super::super::game::fire_triggers(
            deck,
            st,
            super::super::model::SimTrigger::LandEnters,
            turn,
        );
    }
}

/// Put a resolved permanent onto the battlefield with its entry counters.
#[allow(clippy::too_many_arguments)]
pub(super) fn enter_cast_card(
    deck: &SimDeck,
    st: &mut GameState,
    index: CardIdx,
    turn: u32,
    x: u32,
    offspring: bool,
    entries: &mut Vec<(u32, CardIdx)>,
    repeatable_sources: &mut Vec<(u32, u32)>,
) {
    let card = &deck[index];
    if card.is_instant_or_sorcery {
        return;
    }
    let mut entry = new_entry_permanent(deck, st, index, turn, false);
    let uid = entry.uid;
    if card.enter_counters.is_x() {
        entry.counters = super::super::model::Counters {
            loyalty: entry.counters.loyalty,
            ..card.enter_counters.with_paid_x(x)
        };
    }
    st.battlefield_seen.entry(index).or_insert(turn);
    st.battlefield.push(entry);
    register_loyalty_token_source(deck, st, st.battlefield.len() - 1, repeatable_sources);
    entries.push((uid, index));
    if offspring {
        let uid = take_uid(st);
        st.battlefield.push(new_token_perm(uid, turn));
    }
    let first = st.battlefield.len() - 1 - usize::from(offspring);
    for position in first..st.battlefield.len() {
        fire_entry_triggers(deck, st, position, turn);
    }
}

/// Resolve a search whose mana value comes from a creature sacrificed as a cost.
pub(super) fn resolve_sacrifice_search(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    sacrificed_mana_value: Option<u32>,
    search: Option<SacrificeSearch>,
    entries: &mut Vec<(u32, CardIdx)>,
) {
    let (Some(mana_value), Some(search)) = (sacrificed_mana_value, search) else {
        return;
    };
    let maximum = mana_value.saturating_add(search.mana_value_offset);
    let position = st.library.iter().rposition(|index| {
        let card = &deck[*index];
        card.is_creature
            && if search.allow_lower_mana_value {
                card.mana_value <= maximum
            } else {
                card.mana_value == maximum
            }
    });
    let Some(position) = position else {
        if search.shuffle {
            super::super::rng::shuffle(&mut st.library, &mut st.library_rng);
        }
        return;
    };
    let index = st.library.remove(position);
    let mut entry = new_entry_permanent(deck, st, index, turn, false);
    let uid = entry.uid;
    entry.counters.plus1 += u32::from(search.enters_with_plus1_counter);
    st.battlefield_seen.entry(index).or_insert(turn);
    st.battlefield.push(entry);
    entries.push((uid, index));
    if search.shuffle {
        super::super::rng::shuffle(&mut st.library, &mut st.library_rng);
    }
    fire_entry_triggers(deck, st, st.battlefield.len() - 1, turn);
}

/// Exchange graveyard creatures for the original creature battlefield batch.
pub(super) fn exchange_graveyard_creatures(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    entries: &mut Vec<(u32, CardIdx)>,
) {
    let mut returned = Vec::new();
    st.graveyard.retain(|index| {
        if deck[*index].is_creature {
            returned.push(*index);
            false
        } else {
            true
        }
    });
    st.exile.extend(&returned);
    let victims: Vec<_> = st
        .battlefield
        .iter()
        .filter(|permanent| super::super::game::is_creature_permanent(deck, permanent))
        .map(|permanent| permanent.uid)
        .collect();
    super::super::game_effects::resolve_sacrifice_batch(deck, st, turn, &victims);
    let first = st.battlefield.len();
    let mut batch = Vec::new();
    for index in returned {
        let position = st
            .exile
            .iter()
            .position(|exiled| *exiled == index)
            .expect("exchange card remains in the linked exile batch");
        st.exile.remove(position);
        st.battlefield_seen.entry(index).or_insert(turn);
        let entry = new_entry_permanent(deck, st, index, turn, false);
        let uid = entry.uid;
        batch.push(entry);
        entries.push((uid, index));
    }
    st.battlefield.extend(batch);
    for position in first..st.battlefield.len() {
        fire_entry_triggers(deck, st, position, turn);
    }
}

/// Reveal X cards and capture enters triggers after the full eligible batch enters.
pub(super) fn reveal_permanents(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    x: u32,
    entries: &mut Vec<(u32, CardIdx)>,
) {
    begin_resolution(st);
    let mut eligible = Vec::new();
    for _ in 0..x {
        let Some(index) = st.library.pop() else { break };
        st.seen += 1;
        st.awareness_cards += 1;
        let card = &deck[index];
        if !card.is_permanent || card.mana_value > x {
            super::super::game_effects::move_to_graveyard(
                deck,
                st,
                index,
                turn,
                super::super::game_effects::CardZone::Library,
            );
            continue;
        }
        eligible.push(index);
    }
    let first = st.battlefield.len();
    let mut batch = Vec::new();
    for index in eligible {
        st.battlefield_seen.entry(index).or_insert(turn);
        let entry = new_entry_permanent(deck, st, index, turn, false);
        let uid = entry.uid;
        batch.push(entry);
        entries.push((uid, index));
    }
    st.battlefield.extend(batch);
    for position in first..st.battlefield.len() {
        fire_entry_triggers(deck, st, position, turn);
    }
    end_resolution(deck, st, None);
}

/// MDFC front-type eligibility and physical-face cleanup boundaries.
#[cfg(test)]
#[path = "tests/entry_mdfc_tests.rs"]
mod entry_mdfc_tests;
