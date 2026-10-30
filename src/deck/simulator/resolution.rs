//! Settle supported triggers after the action that caused them finishes.

use std::collections::VecDeque;

use super::game::{
    CardRef, GameState, ManaPool, Permanent, card_of, condition_met, graveyard_generation,
};
use super::game_effects::{
    PreparedSpellTargets, apply_prepared_spell_effect_at, prepare_effect_targets,
};
use super::model::{
    CardIdx, Counters, LibraryGraveyardTrigger, SimAbility, SimDeck, SimEffect,
    SimResolutionPayment,
};

/// Nested action scopes and triggers waiting for the next settlement point.
#[derive(Default, Clone)]
pub(super) struct ResolutionState {
    /// True if a supported trigger chain reached the settlement safety limit.
    pub(super) trigger_limit_reached: bool,
    depth: usize,
    settling: bool,
    pending: VecDeque<PendingAction>,
}

/// Immutable definition location and the source facts a trigger can still need.
#[derive(Clone)]
pub(super) struct PendingTrigger {
    source: SourceSnapshot,
    striation: usize,
    ability: usize,
    turn: u32,
    limited: bool,
    ability_id: usize,
    rearm_blink: bool,
    targets: PendingTargets,
}

/// Target choice is deferred until a pending trigger batch reaches the stack.
#[derive(Clone)]
enum PendingTargets {
    Unprepared,
    Prepared(PreparedSpellTargets),
    Unavailable,
}

/// Source identity and last-known values, without copying card definitions.
#[derive(Clone)]
struct SourceSnapshot {
    uid: u32,
    card: CardRef,
    counters: Counters,
}

/// The small set of delayed actions supported by the existing card model.
#[derive(Clone)]
enum PendingAction {
    Trigger(PendingTrigger),
    Undying {
        card: CardIdx,
        generation: u32,
        turn: u32,
    },
    LibraryGraveyard {
        card: CardIdx,
        generation: u32,
        turn: u32,
    },
    SagaChapter {
        source: SourceSnapshot,
        chapter: usize,
        turn: u32,
        targets: PendingTargets,
    },
}

/// Keep triggers pending until the matching outer action scope ends.
pub(super) fn begin_resolution(st: &mut GameState) {
    st.resolution.depth += 1;
}

/// Finish one action scope and settle its captured triggers in stable order.
pub(super) fn end_resolution(deck: &SimDeck, st: &mut GameState, pool: Option<&mut ManaPool>) {
    st.resolution.depth = st
        .resolution
        .depth
        .checked_sub(1)
        .expect("each resolution end has a matching begin");
    if st.resolution.depth == 0 && st.completion.is_none() {
        let mut reasons = Vec::new();
        if st.life <= 0 {
            reasons.push(super::game::PlayerLossReason::ZeroLife);
        }
        if st.failed_draw {
            reasons.push(super::game::PlayerLossReason::FailedDraw);
        }
        if !reasons.is_empty() {
            st.completion = Some(super::game::GameCompletion::PlayerLost {
                reasons,
                turn: st.current_turn,
                phase: st.current_phase,
            });
            st.resolution.pending.clear();
            return;
        }
    }
    settle_pending(deck, st, pool);
}

/// Settle captured triggers when no spell, ability, or cost is in progress.
pub(super) fn settle_pending(deck: &SimDeck, st: &mut GameState, mut pool: Option<&mut ManaPool>) {
    if st.resolution.depth != 0 || st.resolution.settling || st.completion.is_some() {
        return;
    }
    st.resolution.settling = true;
    prepare_pending_targets(deck, st);
    let mut resolved = 0;
    while let Some(action) = st.resolution.pending.pop_front() {
        if st.completion.is_some() {
            st.resolution.pending.clear();
            break;
        }
        if resolved == MAX_TRIGGER_SETTLEMENTS {
            st.resolution.trigger_limit_reached = true;
            st.completion = Some(super::game::GameCompletion::Incomplete {
                reason: super::game::IncompleteReason::TriggerSettlement {
                    configured_limit: MAX_TRIGGER_SETTLEMENTS as u32,
                    remaining_work: st.resolution.pending.len() as u32 + 1,
                },
                turn: st.current_turn,
                phase: st.current_phase,
            });
            st.resolution.pending.clear();
            break;
        }
        resolved += 1;
        let waiting = st.resolution.pending.len();
        begin_resolution(st);
        let completed_saga = match action {
            PendingAction::Trigger(trigger) => {
                resolve_trigger(deck, st, pool.as_deref_mut(), trigger);
                None
            }
            PendingAction::Undying {
                card,
                generation,
                turn,
            } => {
                return_undying(deck, st, card, generation, turn);
                None
            }
            PendingAction::LibraryGraveyard {
                card,
                generation,
                turn,
            } => {
                resolve_library_graveyard(deck, st, card, generation, turn);
                None
            }
            PendingAction::SagaChapter {
                source,
                chapter,
                turn,
                targets,
            } => {
                let uid = source.uid;
                resolve_saga_chapter(
                    deck,
                    st,
                    pool.as_deref_mut(),
                    source,
                    chapter,
                    turn,
                    targets,
                );
                Some((uid, turn))
            }
        };
        end_resolution(deck, st, pool.as_deref_mut());
        if st.completion.is_some() {
            st.resolution.pending.clear();
            break;
        }
        if let Some((uid, turn)) = completed_saga {
            sacrifice_completed_saga(deck, st, uid, turn);
        }
        prepare_pending_targets(deck, st);
        let new_triggers = st.resolution.pending.len() - waiting;
        st.resolution.pending.rotate_right(new_triggers);
        st.opponents.settle_losses();
    }
    st.resolution.settling = false;
}

/// Bound recursive supported triggers without limiting normal turn actions.
const MAX_TRIGGER_SETTLEMENTS: usize = 1024;

/// Stop a game after a safety limit leaves simulator work unresolved.
pub(super) fn mark_incomplete(st: &mut GameState, reason: super::game::IncompleteReason) {
    if st.completion.is_none() {
        st.completion = Some(super::game::GameCompletion::Incomplete {
            reason,
            turn: st.current_turn,
            phase: st.current_phase,
        });
        st.resolution.pending.clear();
    }
}

/// Prepare each new batch against one boundary state, before any member resolves.
fn prepare_pending_targets(deck: &SimDeck, st: &mut GameState) {
    let mut pending = std::mem::take(&mut st.resolution.pending);
    for action in &mut pending {
        let (effects, targets) = match action {
            PendingAction::Trigger(trigger) => {
                let definition = definition_of(deck, trigger.source.card);
                let ability = &definition.striations[trigger.striation].abilities[trigger.ability];
                (ability.effect_sequence(), &mut trigger.targets)
            }
            PendingAction::SagaChapter {
                source,
                chapter,
                targets,
                ..
            } => (
                definition_of(deck, source.card).saga.chapters[*chapter].as_slice(),
                targets,
            ),
            PendingAction::Undying { .. } | PendingAction::LibraryGraveyard { .. } => continue,
        };
        if matches!(targets, PendingTargets::Unprepared) {
            *targets = match prepare_effect_targets(deck, st, effects) {
                Some(prepared) => PendingTargets::Prepared(prepared),
                None => PendingTargets::Unavailable,
            };
        }
    }
    st.resolution.pending = pending;
}

/// Capture a supported trigger while its source and event conditions are valid.
pub(super) fn prepare_trigger(
    deck: &SimDeck,
    st: &GameState,
    source: &Permanent,
    ability: &SimAbility,
    turn: u32,
) -> Option<PendingTrigger> {
    let limited = ability.once_per_turn || ability.event_filter.first_each_turn;
    if !ability.kind.is_triggered()
        || source.face_down
        || limited && st.triggered_this_turn.contains(&(source.uid, ability.id))
        || ability
            .condition
            .is_some_and(|condition| !condition_met(deck, st, &condition))
    {
        return None;
    }
    let (striation, position) = card_of(deck, source)
        .striations
        .iter()
        .enumerate()
        .filter(|(_, tier)| tier.at == 0 || source.counters.charge >= tier.at)
        .find_map(|(tier, striation)| {
            striation
                .abilities
                .iter()
                .position(|candidate| std::ptr::eq(candidate, ability))
                .map(|position| (tier, position))
        })
        .expect("queued ability belongs to its immutable source definition");
    Some(PendingTrigger {
        source: SourceSnapshot::from(source),
        striation,
        ability: position,
        turn,
        limited,
        ability_id: ability.id,
        rearm_blink: false,
        targets: PendingTargets::Unprepared,
    })
}

/// Reserve trigger limits when the event occurs, before any trigger resolves.
pub(super) fn enqueue_trigger(st: &mut GameState, trigger: PendingTrigger) {
    if trigger.limited
        && !st
            .triggered_this_turn
            .insert((trigger.source.uid, trigger.ability_id))
    {
        return;
    }
    st.resolution
        .pending
        .push_back(PendingAction::Trigger(trigger));
}

/// Preserve the supported one-shot blink re-arm on a captured enters trigger.
pub(super) fn rearm_enter_blink(trigger: &mut PendingTrigger) {
    trigger.rearm_blink = true;
}

/// Queue undying from the dying object's counters, not its later incarnation.
pub(super) fn queue_undying(deck: &SimDeck, st: &mut GameState, victim: &Permanent, turn: u32) {
    if let Some(card) = victim.card.deck_idx()
        && !victim.face_down
        && deck[card].has_undying
        && victim.counters.plus1 == 0
    {
        let generation = graveyard_generation(st, card);
        st.resolution.pending.push_back(PendingAction::Undying {
            card,
            generation,
            turn,
        });
    }
}

/// Capture a library-only graveyard trigger before another zone change occurs.
pub(super) fn queue_library_graveyard(
    deck: &SimDeck,
    st: &mut GameState,
    card: CardIdx,
    turn: u32,
) {
    if deck[card].library_graveyard_trigger.is_some() {
        let generation = graveyard_generation(st, card);
        st.resolution
            .pending
            .push_back(PendingAction::LibraryGraveyard {
                card,
                generation,
                turn,
            });
    }
}

/// Queue a Saga chapter by its physical source and immutable chapter position.
pub(super) fn queue_saga_chapter(
    st: &mut GameState,
    source: &Permanent,
    chapter: usize,
    turn: u32,
) {
    st.resolution.pending.push_back(PendingAction::SagaChapter {
        source: SourceSnapshot::from(source),
        chapter,
        turn,
        targets: PendingTargets::Unprepared,
    });
}

impl From<&Permanent> for SourceSnapshot {
    fn from(source: &Permanent) -> Self {
        Self {
            uid: source.uid,
            card: source.card,
            counters: source.counters,
        }
    }
}

/// Find a card definition without requiring its source to remain in play.
fn definition_of(deck: &SimDeck, card: CardRef) -> &super::model::SimCard {
    match card {
        CardRef::Deck(card) => &deck[card],
        CardRef::Commander { slot } => &deck.commanders[slot],
        CardRef::Token(super::game::TokenKind::Creature) => super::game::token_creature_card(),
        CardRef::Token(super::game::TokenKind::Spirit) => super::game::token_spirit_card(),
        CardRef::Token(super::game::TokenKind::Army) => super::game::token_army_card(),
        CardRef::Token(super::game::TokenKind::Jace) => super::game::jace_token_card(),
    }
}

/// Resolve a captured definition even if its source left the battlefield.
fn resolve_trigger(
    deck: &SimDeck,
    st: &mut GameState,
    mut pool: Option<&mut ManaPool>,
    trigger: PendingTrigger,
) {
    let mut targets = match trigger.targets {
        PendingTargets::Prepared(targets) => targets,
        PendingTargets::Unavailable => return,
        PendingTargets::Unprepared => {
            unreachable!("pending trigger targets are prepared before settlement")
        }
    };
    let definition = definition_of(deck, trigger.source.card);
    let ability = &definition.striations[trigger.striation].abilities[trigger.ability];
    if !targets.any_legal_target(st)
        || ability
            .condition
            .is_some_and(|condition| !condition_met(deck, st, &condition))
        || ability.opponent_tax.is_some()
        || !pay_resolution_cost(st, pool.as_deref_mut(), ability.resolution_payment)
    {
        return;
    }
    for effect in ability.effect_sequence() {
        resolve_source_effect(
            deck,
            st,
            pool.as_deref_mut(),
            &trigger.source,
            effect,
            trigger.turn,
            &mut targets,
        );
    }
    if trigger.rearm_blink
        && let Some(source) = st
            .battlefield
            .iter_mut()
            .find(|source| source.uid == trigger.source.uid)
    {
        source.returned_trigger_pending = true;
    }
}

/// Resolve a Saga chapter with the targets chosen for its pending trigger batch.
fn resolve_saga_chapter(
    deck: &SimDeck,
    st: &mut GameState,
    mut pool: Option<&mut ManaPool>,
    source: SourceSnapshot,
    chapter: usize,
    turn: u32,
    targets: PendingTargets,
) {
    let mut targets = match targets {
        PendingTargets::Prepared(targets) => targets,
        PendingTargets::Unavailable => return,
        PendingTargets::Unprepared => {
            unreachable!("pending Saga targets are prepared before settlement")
        }
    };
    if !targets.any_legal_target(st) {
        return;
    }
    for effect in &definition_of(deck, source.card).saga.chapters[chapter] {
        if st.completion.is_some() {
            break;
        }
        resolve_source_effect(
            deck,
            st,
            pool.as_deref_mut(),
            &source,
            effect,
            turn,
            &mut targets,
        );
    }
}

/// Sacrifice the completed physical Saga after its full chapter and loss checks finish.
fn sacrifice_completed_saga(deck: &SimDeck, st: &mut GameState, uid: u32, turn: u32) {
    let completed = st
        .battlefield
        .iter()
        .find(|permanent| permanent.uid == uid)
        .is_some_and(|permanent| {
            let chapters = card_of(deck, permanent).chapter_count();
            chapters > 0 && permanent.entered_turn > 0 && permanent.saga_step as usize >= chapters
        });
    if completed {
        super::game_effects::resolve_sacrifice_uid(deck, st, turn, uid);
    }
}

/// Pay only from a supplied live pool; a missing pool grants no paid reward.
fn pay_resolution_cost(
    st: &mut GameState,
    pool: Option<&mut ManaPool>,
    payment: Option<SimResolutionPayment>,
) -> bool {
    let Some(SimResolutionPayment::PayMana(cost)) = payment else {
        return true;
    };
    let Some(pool) = pool else { return false };
    let life = super::game_mana::phyrexian_life_charge(&cost);
    if st.life < life as i32 {
        return false;
    }
    let Some(payment) = super::game_mana::payment_plan(&cost, pool, &[]) else {
        return false;
    };
    assert!(
        payment.commit(pool),
        "prepared trigger payment remains payable"
    );
    st.life -= life as i32;
    st.life_paid += life;
    true
}

/// Apply source-bound effects by uid, using last-known counters after removal.
fn resolve_source_effect(
    deck: &SimDeck,
    st: &mut GameState,
    pool: Option<&mut ManaPool>,
    source: &SourceSnapshot,
    effect: &SimEffect,
    turn: u32,
    targets: &mut PreparedSpellTargets,
) {
    let counters = st
        .battlefield
        .iter()
        .find(|permanent| permanent.uid == source.uid)
        .map_or(source.counters, |permanent| permanent.counters);
    match effect {
        SimEffect::Mana(yield_) => {
            if let Some(pool) = pool {
                super::game_mana::add_yield_turns(deck, yield_, pool, turn, &st.battlefield);
            }
        }
        SimEffect::ManaPerCounter(yield_) => {
            if let Some(pool) = pool {
                for _ in 0..counters.charge {
                    super::game_mana::add_yield_turns(deck, yield_, pool, turn, &st.battlefield);
                }
            }
        }
        SimEffect::Counters(amount) => {
            if let Some(permanent) = st.battlefield.iter_mut().find(|p| p.uid == source.uid) {
                permanent.counters.charge += amount;
            }
        }
        SimEffect::AddBurdenCounter => {
            if let Some(permanent) = st.battlefield.iter_mut().find(|p| p.uid == source.uid) {
                permanent.counters.burden += 1;
                let draws = permanent.counters.burden;
                for _ in 0..draws {
                    super::game_effects::draw_one(deck, st, turn);
                    if st.failed_draw {
                        break;
                    }
                }
            }
        }
        SimEffect::BurdenLifeLoss => {
            st.life -= counters.burden as i32;
            st.life_paid += counters.burden;
        }
        _ => apply_prepared_spell_effect_at(
            deck,
            effect,
            st,
            turn,
            definition_of(deck, source.card).mills_opponent,
            source.card.deck_idx(),
            targets,
        ),
    }
}

/// Return the same graveyard card as a fresh permanent after undying resolves.
fn return_undying(deck: &SimDeck, st: &mut GameState, card: CardIdx, generation: u32, turn: u32) {
    if graveyard_generation(st, card) != generation {
        return;
    }
    let Some(position) = st.graveyard.iter().position(|held| *held == card) else {
        return;
    };
    st.graveyard.remove(position);
    st.battlefield_seen.entry(card).or_insert(turn);
    let mut permanent = super::cast_pass::new_entry_permanent(deck, st, card, turn, false);
    permanent.counters.plus1 = 1;
    st.battlefield.push(permanent);
    super::cast_pass::fire_entry_triggers(deck, st, st.battlefield.len() - 1, turn);
}

/// Resolve a queued library-to-graveyard trigger only while its card remains.
fn resolve_library_graveyard(
    deck: &SimDeck,
    st: &mut GameState,
    card: CardIdx,
    generation: u32,
    turn: u32,
) {
    if graveyard_generation(st, card) != generation {
        return;
    }
    let Some(position) = st.graveyard.iter().position(|held| *held == card) else {
        return;
    };
    match deck[card].library_graveyard_trigger {
        Some(LibraryGraveyardTrigger::ReturnToBattlefield) => {
            st.graveyard.remove(position);
            st.battlefield_seen.entry(card).or_insert(turn);
            let entry = super::cast_pass::new_entry_permanent(deck, st, card, turn, false);
            st.battlefield.push(entry);
            super::cast_pass::fire_entry_triggers(deck, st, st.battlefield.len() - 1, turn);
        }
        Some(LibraryGraveyardTrigger::DrainAndGain(amount)) => {
            st.graveyard.remove(position);
            st.exile.push(card);
            st.opponent_life_lost += st
                .opponents
                .lose_life(amount, super::model::LifeLossScope::EachOpponent);
            st.life_gained += amount;
            st.life += amount as i32;
        }
        None => {}
    }
}

/// Tests for event snapshots and atomic supported effect resolution.
#[cfg(test)]
#[path = "tests/resolution_tests.rs"]
mod resolution_tests;

/// Target selection boundaries and graveyard-entry identity regressions.
#[cfg(test)]
#[path = "tests/resolution_target_tests.rs"]
mod resolution_target_tests;
