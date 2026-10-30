//! Production cast attempts used to prove that a work limit left a legal action.

use super::super::game_effects::PreparedSpellTargets;
use super::super::game_mana::cast_cost_increase;
use super::super::model::Cost;
use super::*;

/// Settle each hand cast before selecting another and report real remaining work at the cap.
pub(super) fn cast_hand_pass(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    repeatable_sources: &mut Vec<(u32, u32)>,
    pip_blocks: &mut Vec<(CardIdx, usize)>,
    blocked_colors: &mut [bool; 5],
) -> u32 {
    let mut queue = hand_queue(deck, st);
    let mut cast_ids = Vec::new();
    let mut cast_entries = Vec::new();
    let mut spent = 0;
    let mut casts = 0;
    for _ in 0..super::super::model::MAX_LOOP_PASSES {
        if st.completion.is_some() {
            break;
        }
        if !cast_sweep::cast_pass(
            deck,
            st,
            pool,
            turn,
            &mut queue,
            &mut cast_ids,
            &mut cast_entries,
            &mut spent,
            repeatable_sources,
            pip_blocks,
            blocked_colors,
        ) {
            break;
        }
        casts += 1;
        queue = hand_queue(deck, st);
        if queue.is_empty() {
            break;
        }
    }
    if casts == super::super::model::MAX_LOOP_PASSES
        && st.completion.is_none()
        && has_hand_cast(deck, st, pool, turn, &queue)
    {
        super::super::resolution::mark_incomplete(
            st,
            super::super::game::IncompleteReason::CastPass {
                configured_limit: super::super::model::MAX_LOOP_PASSES,
                remaining_work: 1,
            },
        );
    }
    spent
}

/// Choose the same cheapest-first queue after each settled hand cast.
fn hand_queue(deck: &SimDeck, st: &GameState) -> Vec<CardIdx> {
    let mut queue: Vec<_> = st
        .hand
        .iter()
        .copied()
        .filter(|index| goldfish_castable(&deck[*index]))
        .collect();
    queue.sort_by_key(|index| deck[*index].min_cost.total());
    queue
}

/// Probe the real hand-cast gate on copied state without spending real resources.
pub(super) fn has_hand_cast(
    deck: &SimDeck,
    st: &GameState,
    pool: &ManaPool,
    turn: usize,
    queue: &[CardIdx],
) -> bool {
    if st.completion.is_some() {
        return false;
    }
    let mut trial = st.clone();
    let mut trial_pool = pool.clone();
    cast_sweep::cast_pass(
        deck,
        &mut trial,
        &mut trial_pool,
        turn,
        &mut queue.to_vec(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut 0,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut [false; 5],
    )
}

/// Check the exact preparation gate shared with the production graveyard pass.
pub(super) fn has_graveyard_cast(deck: &SimDeck, st: &GameState, pool: &ManaPool) -> bool {
    prepare_replay(deck, st, pool).is_some()
}

/// A complete graveyard candidate with targets selected before its costs change zones.
struct Replay {
    index: CardIdx,
    escape: bool,
    raw_cost: Cost,
    targets: PreparedSpellTargets,
    fodder: Vec<CardIdx>,
}

/// Find one payable replay, without changing physical cards or the random stream.
fn prepare_replay(deck: &SimDeck, st: &GameState, pool: &ManaPool) -> Option<Replay> {
    if st.completion.is_some() {
        return None;
    }
    let escape_active = st
        .battlefield
        .iter()
        .any(|permanent| card_of(deck, permanent).spell_data.grants_escape);
    let mut candidates: Vec<_> = st
        .graveyard
        .iter()
        .copied()
        .filter_map(|index| {
            let card = &deck[index];
            let generation = super::super::game::graveyard_generation(st, index);
            let flashback = (st.flashback_permissions.contains(&(index, generation))
                || card.spell_data.own_flashback.is_some())
                && card.is_instant_or_sorcery;
            let escape =
                (escape_active || card.spell_data.own_escape.is_some()) && card.role != Role::Land;
            (flashback || escape).then_some((index, escape && !flashback))
        })
        .collect();
    candidates.sort_by_key(|(index, _)| deck[*index].min_cost.total());
    candidates
        .into_iter()
        .find_map(|(index, escape)| prepare_candidate(deck, st, pool, index, escape))
}

/// Validate a replay's targets and all mana and nonmana costs before changing zones.
fn prepare_candidate(
    deck: &SimDeck,
    st: &GameState,
    pool: &ManaPool,
    index: CardIdx,
    escape: bool,
) -> Option<Replay> {
    if !additional_costs_payable(deck, st, index) {
        return None;
    }
    let targets = prepare_spell_targets(deck, st, &deck[index])?;
    let fodder: Vec<_> = st
        .graveyard
        .iter()
        .copied()
        .filter(|other| *other != index)
        .take(if escape { 3 } else { 0 })
        .collect();
    if escape && fodder.len() < 3 {
        return None;
    }
    let card = &deck[index];
    let raw_cost = if escape {
        card.spell_data.own_escape
    } else {
        card.spell_data.own_flashback
    }
    .unwrap_or(card.min_cost);
    let cost = adjust_cast_cost(
        deck,
        card,
        &st.battlefield,
        raw_cost.with_x(0)?,
        cast_cost_increase(deck, st, card, Some(index)),
    );
    let life_cost = card.spell_data.additional_cost_life + phyrexian_life_charge(&cost);
    if i64::from(st.life) < i64::from(life_cost) {
        return None;
    }
    payment::prepare_cast_payment(deck, st, pool, index, &cost)?;
    if !fodder.is_empty() {
        let mut after_exile = st.clone();
        after_exile
            .graveyard
            .retain(|held| *held != index && !fodder.contains(held));
        payment::prepare_cast_payment(deck, &after_exile, pool, index, &cost)?;
    }
    Some(Replay {
        index,
        escape,
        raw_cost,
        targets,
        fodder,
    })
}

/// Commit one fully prepared replay and settle its effects before choosing another.
pub(super) fn cast_one_from_graveyard(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    mana_spent: &mut [f64],
    repeatable_sources: &mut Vec<(u32, u32)>,
) -> bool {
    let Some(replay) = prepare_replay(deck, st, pool) else {
        return false;
    };
    for other in replay.fodder {
        let position = st
            .graveyard
            .iter()
            .position(|held| *held == other)
            .expect("escape fodder remains in its prepared graveyard");
        st.exile.push(st.graveyard.remove(position));
    }
    let position = st
        .graveyard
        .iter()
        .position(|held| *held == replay.index)
        .expect("the prepared replay remains in its graveyard");
    st.graveyard.remove(position);
    let generation = super::super::game::graveyard_generation(st, replay.index);
    st.flashback_permissions.remove(&(replay.index, generation));
    let mut spent = 0;
    resolve_cast(
        deck,
        st,
        pool,
        turn,
        replay.index,
        &replay.raw_cost,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut spent,
        repeatable_sources,
        true,
        if replay.escape {
            CastPayment::Mana
        } else {
            CastPayment::Flashback
        },
        replay.targets,
    );
    st.replay_casts += 1;
    super::super::game::milestone_for_turn(st, turn as u32).graveyard_casts += 1;
    mana_spent[turn - 1] += f64::from(spent);
    true
}
