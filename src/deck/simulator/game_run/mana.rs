//! Mana-pool building for the goldfish turn loop: taps, gates, banked
//! mana, counter-scaled taps, grants, and the Treasure bank.

use super::super::game::{
    GameState, ManaPool, Permanent, card_of, is_creature_permanent, is_land_permanent,
    permanent_abilities,
};
use super::super::game_mana::{
    add_yield, add_yield_turns, effective_min_cost, pay_cost, payable, pips_ok,
};
use super::super::model::{Role, Scale, SimAbilityKind, SimDeck, SimEffect, SimTrigger};
use super::add_nonland_mana;

/// Bounded choices among complete one-mana land abilities.
#[path = "mana_modes.rs"]
mod mana_modes;
pub(super) use mana_modes::add_chosen_source_yield;

/// Build the turn's spendable pool from untapped permanents: taps,
/// verge gates, banked-mana releases, counter-scaled taps, static
/// grants, and the Treasure bank.
pub(in crate::deck::simulator) fn build_pool(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
) -> ManaPool {
    let mut pool = ManaPool::default();
    release_banked_mana(deck, st, &mut pool, turn);
    let uids: Vec<u32> = st
        .battlefield
        .iter()
        .map(|permanent| permanent.uid)
        .collect();
    for uid in uids {
        let Some(pos) = st
            .battlefield
            .iter()
            .position(|permanent| permanent.uid == uid)
        else {
            continue;
        };
        let perm = st.battlefield[pos].clone();
        if perm.tapped {
            continue;
        }
        // Land and noncreature mana sources are reserved when their mana
        // joins the pool. This prevents the same permanent from stationing
        // or crewing after it paid for a cast.
        if is_creature_permanent(deck, &perm) && perm.card.deck_idx().is_some() {
            continue;
        }
        if try_granted_mana(deck, st, &perm, &mut pool, turn).is_some() {
            continue;
        }
        if perm.face_down {
            continue;
        }
        let card = card_of(deck, &perm);
        if !mana_condition_met(deck, st, card) {
            continue;
        }
        if mana_modes::simple_source(deck, st, &perm).is_some() {
            continue;
        }
        let has_mana_mode = permanent_abilities(deck, &perm)
            .any(|ability| ability.kind == SimAbilityKind::ManaActivated);
        if has_mana_mode || card.is_mdfc_spell {
            activate_mana_mode(deck, st, &perm, &mut pool, turn);
            continue;
        }
        let Some(y) = &card.tap else {
            continue;
        };
        if let Some(Scale::PerChargeCounter) = y.scaling {
            // Counter-scaled taps resolve at the tap: one any-color pip
            // per charge counter on the source (Astral Cornucopia
            // class).
            let yield_ = super::super::model::ManaYield {
                any_pips: perm.counters.charge,
                scaling: None,
                ..y.clone()
            };
            add_nonland_mana(deck, st, &perm, &yield_, &mut pool, turn);
            st.battlefield[pos].tapped = true;
            if !is_land_permanent(&perm) {
                fire_tapped_for_mana_triggers(deck, st, &mut pool, turn);
            }
            continue;
        }
        if card.gate_types.is_empty() {
            if !add_nonland_mana(deck, st, &perm, y, &mut pool, turn) {
                continue;
            }
            st.battlefield[pos].tapped = true;
            if !is_land_permanent(&perm) {
                fire_tapped_for_mana_triggers(deck, st, &mut pool, turn);
            }
            continue;
        }
        add_gated_yield(deck, st, &perm, y, &mut pool, turn);
        st.battlefield[pos].tapped = true;
    }
    pool.treasures = std::mem::take(&mut st.treasure_bank);
    mana_modes::build_simple_sources(deck, st, &mut pool, turn);
    pool
}

/// Use one complete mana mode from the permanent, preserving that mode's
/// own costs and output instead of merging costs across the card.
pub(in crate::deck::simulator) fn activate_mana_mode(
    deck: &SimDeck,
    st: &mut GameState,
    source: &Permanent,
    pool: &mut ManaPool,
    turn: u32,
) -> bool {
    activate_mana_mode_for_use(deck, st, source, pool, turn, false, None)
}

/// Choose the separate granted tap ability before checking any native mode requirements.
pub(in crate::deck::simulator) fn try_granted_mana(
    deck: &SimDeck,
    st: &mut GameState,
    source: &Permanent,
    pool: &mut ManaPool,
    turn: u32,
) -> Option<bool> {
    let position = st
        .battlefield
        .iter()
        .position(|item| item.uid == source.uid)?;
    let source = &st.battlefield[position];
    let grants = super::grants_active(deck, st);
    let creature = is_creature_permanent(deck, source);
    if !(is_land_permanent(source) && grants.lands || creature && grants.creatures) {
        return None;
    }
    if st.completion.is_some()
        || source.tapped
        || creature
            && (source.summoning_sick || source.entered_turn >= turn as usize)
            && !super::super::game::effective_keywords(deck, st, source)
                .contains(super::super::model::Keyword::Haste)
    {
        return Some(false);
    }
    let source = source.clone();
    let output = super::resolved_source_yield(
        deck,
        st,
        &source,
        &super::super::model::ManaYield::default(),
        true,
    );
    super::super::resolution::begin_resolution(st);
    st.battlefield[position].tapped = true;
    add_nonland_mana(deck, st, &source, &output, pool, turn);
    if !is_land_permanent(&source) {
        fire_tapped_for_mana_triggers(deck, st, pool, turn);
    }
    super::super::resolution::end_resolution(deck, st, Some(pool));
    Some(true)
}

/// Select a printed mode without treating spell-only mana as an upkeep resource.
fn activate_mana_mode_for_use(
    deck: &SimDeck,
    st: &mut GameState,
    source: &Permanent,
    pool: &mut ManaPool,
    turn: u32,
    nonspell: bool,
    selected: Option<&mana_modes::SelectedMode>,
) -> bool {
    if st.completion.is_some() {
        return false;
    }
    let Some(current_source) = st
        .battlefield
        .iter()
        .find(|permanent| permanent.uid == source.uid)
        .cloned()
    else {
        return false;
    };
    let source = &current_source;
    if let Some(activated) = try_granted_mana(deck, st, source, pool, turn) {
        return activated;
    }
    if source.face_down {
        return false;
    }
    let card = card_of(deck, source);
    if card.flags.requires_metalcraft
        && st
            .battlefield
            .iter()
            .filter(|permanent| !permanent.face_down && card_of(deck, permanent).is_artifact)
            .count()
            < 3
    {
        return false;
    }
    if selected.is_none()
        && let Some(simple) = mana_modes::simple_source(deck, st, source)
    {
        return mana_modes::activate_simple_sources(deck, st, pool, &[simple], turn, nonspell);
    }
    let candidate = permanent_abilities(deck, source)
        .filter(|ability| ability.kind == SimAbilityKind::ManaActivated)
        .filter(|ability| selected.is_none_or(|chosen| chosen.ability_id == Some(ability.id)))
        .filter(|ability| {
            ability
                .condition
                .is_none_or(|condition| super::super::game::condition_met(deck, st, &condition))
        })
        .filter_map(|ability| {
            let activation = ability.activation.as_ref()?;
            let ability_key = (source.uid, ability.id);
            if activation
                .has_restriction(super::super::model::SimActivationRestriction::OncePerTurn)
                && st.activated_this_turn.contains(&ability_key)
                || activation
                    .has_restriction(super::super::model::SimActivationRestriction::OncePerGame)
                    && st.activated_once.contains(&ability_key)
            {
                return None;
            }
            let yield_ = ability
                .effect_sequence()
                .iter()
                .find_map(|effect| match effect {
                    SimEffect::Mana(yield_) | SimEffect::ManaPerCounter(yield_) => Some(yield_),
                    _ => None,
                })?;
            if yield_.opponent_any && !st.opponents.seats.iter().any(|seat| seat.active) {
                return None;
            }
            let mana_cost = activation.mana_cost();
            if !payable(&mana_cost, pool) || !pips_ok(&mana_cost, pool) {
                return None;
            }
            if activation.taps_source()
                && (source.tapped
                    || (super::super::game::is_creature_permanent(deck, source)
                        && (source.summoning_sick || source.entered_turn >= turn as usize)
                        && !super::super::game::effective_keywords(deck, st, source)
                            .contains(super::super::model::Keyword::Haste)))
            {
                return None;
            }
            if activation.untaps_source()
                && (!source.tapped
                    || super::super::game::is_creature_permanent(deck, source)
                        && source.summoning_sick
                        && !super::super::game::effective_keywords(deck, st, source)
                            .contains(super::super::model::Keyword::Haste))
            {
                return None;
            }
            if activation.sacrifices_source() && source.card.deck_idx().is_none() {
                return None;
            }
            let sacrifice_uids = prepare_mana_sacrifices(deck, st, source, activation)?;
            let life_cost = activation.life_payment()
                + super::super::game_mana::phyrexian_life_charge(&mana_cost);
            if i64::from(st.life) < i64::from(life_cost)
                || activation.energy_payment() > st.player_counters.energy
                || activation.charge_counter_payment() > source.counters.charge
            {
                return None;
            }
            if nonspell && (yield_.restriction.is_some() || yield_.cannot_pay_generic)
                || !nonspell && !mana_mode_has_target(deck, st, yield_.restriction)
            {
                return None;
            }
            let mut trial_pool = pool.clone();
            if !pay_cost(&mana_cost, &mut trial_pool) {
                return None;
            }
            let paid_pool = trial_pool.clone();
            let before_production = trial_pool.total();
            if !mana_modes::produce_mode_yield(
                deck,
                st,
                source,
                yield_,
                &mut trial_pool,
                turn,
                selected,
            ) {
                return None;
            }
            let output = trial_pool.total() - before_production;
            let score = output.saturating_sub(mana_cost.total());
            Some((score, ability, activation, sacrifice_uids, paid_pool))
        })
        .max_by_key(|candidate| candidate.0);
    let Some((_, ability, activation, sacrifice_uids, paid_pool)) = candidate else {
        return false;
    };
    super::super::resolution::begin_resolution(st);
    let mana_cost = activation.mana_cost();
    let life_cost =
        activation.life_payment() + super::super::game_mana::phyrexian_life_charge(&mana_cost);
    *pool = paid_pool;
    st.life -= i32::try_from(life_cost).expect("payable life cost fits the current life total");
    st.life_paid += life_cost;
    st.player_counters.energy -= activation.energy_payment();
    if activation.charge_counter_payment() > 0
        && let Some(permanent) = st
            .battlefield
            .iter_mut()
            .find(|item| item.uid == source.uid)
    {
        permanent.counters.charge -= activation.charge_counter_payment();
    }
    if activation.taps_source()
        && let Some(permanent) = st
            .battlefield
            .iter_mut()
            .find(|item| item.uid == source.uid)
    {
        permanent.tapped = true;
    }
    if activation.untaps_source()
        && let Some(permanent) = st
            .battlefield
            .iter_mut()
            .find(|item| item.uid == source.uid)
    {
        permanent.tapped = false;
    }
    super::super::game_effects::activation::resolve_sacrifice_batch(
        deck,
        st,
        turn,
        &sacrifice_uids,
    );
    for effect in ability.effect_sequence() {
        if st.completion.is_some() {
            break;
        }
        match effect {
            SimEffect::Mana(yield_) => {
                mana_modes::produce_mode_yield(deck, st, source, yield_, pool, turn, selected);
            }
            SimEffect::ManaPerCounter(yield_) => {
                for _ in 0..activation.charge_counter_payment() {
                    add_mode_yield(deck, st, source, yield_, pool, turn);
                }
            }
            other => super::super::game_effects::apply_effect_at(
                deck,
                other,
                st,
                turn,
                false,
                source.card.deck_idx(),
            ),
        }
    }
    if activation.taps_source() && !is_land_permanent(source) {
        fire_tapped_for_mana_triggers(deck, st, pool, turn);
    }
    let ability_key = (source.uid, ability.id);
    if activation.has_restriction(super::super::model::SimActivationRestriction::OncePerTurn) {
        st.activated_this_turn.insert(ability_key);
    }
    if activation.has_restriction(super::super::model::SimActivationRestriction::OncePerGame) {
        st.activated_once.insert(ability_key);
    }
    super::super::resolution::end_resolution(deck, st, Some(pool));
    true
}

/// Tap a legal unrestricted source for a generic nonspell payment.
/// Basic land identity and metalcraft tap data share the normal production path.
pub(in crate::deck::simulator) fn tap_for_nonspell_payment(
    deck: &SimDeck,
    st: &mut GameState,
    source: &Permanent,
    pool: &mut ManaPool,
    turn: u32,
) -> bool {
    let Some(current_source) = st
        .battlefield
        .iter()
        .find(|item| item.uid == source.uid)
        .cloned()
    else {
        return false;
    };
    let source = &current_source;
    if let Some(activated) = try_granted_mana(deck, st, source, pool, turn) {
        return activated;
    }
    if source.face_down || source.tapped {
        return false;
    }
    if super::super::game::is_creature_permanent(deck, source)
        && source.summoning_sick
        && !super::super::game::effective_keywords(deck, st, source)
            .contains(super::super::model::Keyword::Haste)
    {
        return false;
    }
    let card = card_of(deck, source);
    if card.flags.requires_metalcraft
        && st
            .battlefield
            .iter()
            .filter(|permanent| !permanent.face_down && card_of(deck, permanent).is_artifact)
            .count()
            < 3
    {
        return false;
    }
    if card.is_mdfc_spell
        || permanent_abilities(deck, source)
            .any(|ability| ability.kind == SimAbilityKind::ManaActivated)
    {
        return activate_mana_mode_for_use(deck, st, source, pool, turn, true, None);
    }
    if !(card.is_basic_land || card.flags.requires_metalcraft) {
        return false;
    }
    let Some(yield_) = card
        .tap
        .as_ref()
        .filter(|yield_| yield_.restriction.is_none() && !yield_.cannot_pay_generic)
    else {
        return false;
    };
    if !add_mode_yield(deck, st, source, yield_, pool, turn) {
        return false;
    }
    st.battlefield
        .iter_mut()
        .find(|permanent| permanent.uid == source.uid)
        .expect("prepared mana source remains on the battlefield")
        .tapped = true;
    if !is_land_permanent(source) {
        fire_tapped_for_mana_triggers(deck, st, pool, turn);
    }
    true
}

/// True when at least one held spell can use the mode's restricted mana.
fn mana_mode_has_target(
    deck: &SimDeck,
    st: &GameState,
    restriction: Option<super::super::model::SpendRestriction>,
) -> bool {
    let Some(restriction) = restriction else {
        return true;
    };
    st.hand.iter().any(|index| {
        let card = &deck[*index];
        match restriction {
            super::super::model::SpendRestriction::Creature => card.is_creature,
            super::super::model::SpendRestriction::Legendary => card.is_legendary,
            super::super::model::SpendRestriction::Artifact => card.is_artifact,
            super::super::model::SpendRestriction::InstantSorcery => card.is_instant_or_sorcery,
        }
    })
}

/// Resolve one mana mode through its land gate or ordinary mana source.
pub(in crate::deck::simulator) fn add_mode_yield(
    deck: &SimDeck,
    st: &GameState,
    source: &Permanent,
    yield_: &super::super::model::ManaYield,
    pool: &mut ManaPool,
    turn: u32,
) -> bool {
    let Some(yield_) = opponent_mana_yield(st, yield_) else {
        return false;
    };
    let before = pool.total();
    if card_of(deck, source).gate_types.is_empty() {
        super::super::game_run::add_nonland_mana(deck, st, source, &yield_, pool, turn);
    } else {
        add_gated_yield(deck, st, source, &yield_, pool, turn);
    }
    pool.total() > before
}

/// Add an activation's mana only when its opponent-dependent source is eligible.
pub(in crate::deck::simulator) fn add_activation_yield(
    deck: &SimDeck,
    st: &GameState,
    yield_: &super::super::model::ManaYield,
    pool: &mut ManaPool,
    turn: u32,
) -> bool {
    let Some(yield_) = opponent_mana_yield(st, yield_) else {
        return false;
    };
    let before = pool.total();
    super::super::game_mana::add_yield_turns(deck, &yield_, pool, turn, &st.battlefield);
    pool.total() > before
}

/// Resolve opponent-dependent mana colors and reject ineligible sources.
fn opponent_mana_yield(
    st: &GameState,
    yield_: &super::super::model::ManaYield,
) -> Option<super::super::model::ManaYield> {
    if !yield_.opponent_any {
        return Some(yield_.clone());
    }
    if !st.opponents.seats.iter().any(|seat| seat.active) {
        return None;
    }
    let Some(colors) = st.opponents.known_land_colors() else {
        return Some(yield_.clone());
    };
    if !colors.iter().any(|color| *color) {
        return None;
    }
    Some(super::super::model::ManaYield {
        opponent_any: false,
        any_pips: 0,
        choice: colors,
        ..yield_.clone()
    })
}

/// Keep simple land taps on the payment-aware path and check other mana conditions.
pub(in crate::deck::simulator) fn mana_yield_eligible(
    deck: &SimDeck,
    st: &GameState,
    source: &Permanent,
    yield_: &super::super::model::ManaYield,
    taps_source: bool,
) -> bool {
    // The general activation pass has no complete spell payment or chosen mode.
    if taps_source && mana_modes::simple_source(deck, st, source).is_some() {
        return false;
    }
    opponent_mana_yield(st, yield_).is_some()
        && (yield_.commander_pips == 0
            || deck.commander_color_identity().iter().any(|color| *color)
            || yield_.any_pips > 0
            || yield_.colorless > 0
            || yield_.choice_colorless
            || yield_.choice.iter().any(|color| *color)
            || yield_.fixed.iter().any(|pips| *pips > 0))
}

/// Select distinct creatures for every sacrifice in a native mana payment.
fn prepare_mana_sacrifices(
    deck: &SimDeck,
    st: &GameState,
    source: &Permanent,
    activation: &super::super::model::SimActivation,
) -> Option<Vec<u32>> {
    let sacrifice_source = activation.sacrifices_source();
    let exclude_source = sacrifice_source || activation.creature_sacrifice_excludes_source();
    if sacrifice_source
        && !st
            .battlefield
            .iter()
            .any(|permanent| permanent.uid == source.uid)
    {
        return None;
    }
    let mut victims: Vec<u32> = st
        .battlefield
        .iter()
        .filter(|permanent| {
            super::super::game::is_creature_permanent(deck, permanent)
                && (!exclude_source || permanent.uid != source.uid)
        })
        .map(|permanent| permanent.uid)
        .take(activation.creature_sacrifices() as usize)
        .collect();
    if victims.len() != activation.creature_sacrifices() as usize {
        return None;
    }
    if sacrifice_source {
        victims.push(source.uid);
    }
    Some(victims)
}

/// Resolve supported nonland mana-tap triggers without repeating same-type bonus mana.
pub(in crate::deck::simulator) fn fire_tapped_for_mana_triggers(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) {
    let triggers: Vec<_> = st
        .battlefield
        .iter()
        .flat_map(|source| {
            super::super::game::permanent_abilities(deck, source)
                .filter(|ability| {
                    ability.kind.is_triggered()
                        && ability.trigger == SimTrigger::TappedForMana
                        && ability.condition.is_none_or(|condition| {
                            super::super::game::condition_met(deck, st, &condition)
                        })
                })
                .map(|ability| {
                    (
                        source.uid,
                        source.card.deck_idx(),
                        card_of(deck, source).mills_opponent,
                        ability,
                    )
                })
        })
        .collect();
    for (uid, source_card, mills_opponent, ability) in triggers {
        if st.completion.is_some() {
            return;
        }
        let key = (uid, ability.id);
        if ability.once_per_turn && st.triggered_this_turn.contains(&key)
            || ability
                .condition
                .is_some_and(|condition| !super::super::game::condition_met(deck, st, &condition))
        {
            continue;
        }
        super::super::resolution::begin_resolution(st);
        for effect in ability.effect_sequence() {
            if st.completion.is_some() {
                break;
            }
            match effect {
                // The source-tap path already copies the tapped source's
                // mana type for this parsed trigger.
                SimEffect::Mana(_)
                    if source_card
                        .is_some_and(|index| deck[index].flags.bonus_mana_on_nonland_tap) => {}
                SimEffect::Mana(yield_) => {
                    add_yield_turns(deck, yield_, pool, turn, &st.battlefield);
                }
                SimEffect::ManaPerCounter(yield_) => {
                    let count = st
                        .battlefield
                        .iter()
                        .find(|permanent| permanent.uid == uid)
                        .map_or(0, |permanent| permanent.counters.charge);
                    for _ in 0..count {
                        add_yield_turns(deck, yield_, pool, turn, &st.battlefield);
                    }
                }
                other => super::super::game_effects::apply_effect_at(
                    deck,
                    other,
                    st,
                    turn,
                    mills_opponent,
                    source_card,
                ),
            }
        }
        super::super::resolution::end_resolution(deck, st, Some(pool));
        if ability.once_per_turn {
            st.triggered_this_turn.insert(key);
        }
    }
}

/// Tap legal creature mana sources only when the current pool cannot pay
/// the cheapest spell still in hand. Tapped dorks cannot also station or crew.
pub(in crate::deck::simulator) fn tap_dorks_for_mana(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) {
    let need = st
        .hand
        .iter()
        .filter(|idx| deck[**idx].role != Role::Land)
        .map(|idx| effective_min_cost(deck, &deck[*idx], &st.battlefield).total())
        .min();
    let Some(need) = need else { return };
    let uids: Vec<_> = st
        .battlefield
        .iter()
        .map(|permanent| permanent.uid)
        .collect();
    for uid in uids {
        if super::super::game_mana::usable_for_noncreature(pool) >= need {
            break;
        }
        let Some(pos) = st
            .battlefield
            .iter()
            .position(|permanent| permanent.uid == uid)
        else {
            continue;
        };
        let perm = st.battlefield[pos].clone();
        if is_creature_permanent(deck, &perm)
            && try_granted_mana(deck, st, &perm, pool, turn).is_some()
        {
            continue;
        }
        let card = card_of(deck, &perm);
        if perm.tapped
            || perm.face_down
            || (perm.summoning_sick || perm.entered_turn >= turn as usize)
                && super::super::game::is_creature_permanent(deck, &perm)
                && !super::super::game::effective_keywords(deck, st, &perm)
                    .contains(super::super::model::Keyword::Haste)
            || !super::super::game::is_creature_permanent(deck, &perm)
            || !mana_condition_met(deck, st, card)
        {
            continue;
        }
        let Some(yield_) = card.tap.clone() else {
            continue;
        };
        let has_mana_mode = permanent_abilities(deck, &perm)
            .any(|ability| ability.kind == SimAbilityKind::ManaActivated);
        if has_mana_mode || card.is_mdfc_spell {
            activate_mana_mode(deck, st, &perm, pool, turn);
            continue;
        }
        if !add_nonland_mana(deck, st, &perm, &yield_, pool, turn) {
            continue;
        }
        st.battlefield[pos].tapped = true;
        fire_tapped_for_mana_triggers(deck, st, pool, turn);
    }
}

/// Activate a noncreature rock after it enters, then revisit the cast
/// choices. Its mana source is tapped exactly once for the turn.
pub(in crate::deck::simulator) fn tap_new_rocks(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) {
    let uids: Vec<u32> = st
        .battlefield
        .iter()
        .filter(|permanent| permanent.entered_turn == turn as usize)
        .map(|permanent| permanent.uid)
        .collect();
    for uid in uids {
        let Some(pos) = st
            .battlefield
            .iter()
            .position(|permanent| permanent.uid == uid)
        else {
            continue;
        };
        let perm = st.battlefield[pos].clone();
        if !is_creature_permanent(deck, &perm)
            && try_granted_mana(deck, st, &perm, pool, turn).is_some()
        {
            continue;
        }
        let card = card_of(deck, &perm);
        if perm.tapped
            || perm.face_down
            || perm.entered_turn != turn as usize
            || is_creature_permanent(deck, &perm)
            || !mana_condition_met(deck, st, card)
        {
            continue;
        }
        if mana_modes::simple_source(deck, st, &perm).is_some() {
            continue;
        }
        let Some(yield_) = card.tap.clone() else {
            continue;
        };
        let has_mana_mode = permanent_abilities(deck, &perm)
            .any(|ability| ability.kind == SimAbilityKind::ManaActivated);
        if has_mana_mode || card.is_mdfc_spell {
            activate_mana_mode(deck, st, &perm, pool, turn);
            continue;
        }
        if !add_nonland_mana(deck, st, &perm, &yield_, pool, turn) {
            continue;
        }
        st.battlefield[pos].tapped = true;
        fire_tapped_for_mana_triggers(deck, st, pool, turn);
    }
    mana_modes::build_simple_sources(deck, st, pool, turn);
}

/// Check player-controlled conditions that gate a card's mana ability.
fn mana_condition_met(deck: &SimDeck, st: &GameState, card: &super::super::model::SimCard) -> bool {
    !card.flags.requires_metalcraft
        || st
            .battlefield
            .iter()
            .filter(|permanent| !permanent.face_down && card_of(deck, permanent).is_artifact)
            .count()
            >= 3
}

/// Banked-mana engines release in the first main phase (Coalition Relic):
/// counters × N mana joins the pool, counters clear.
fn release_banked_mana(deck: &SimDeck, st: &mut GameState, pool: &mut ManaPool, turn: u32) {
    let releases: Vec<(u32, super::super::model::ManaYield, u32)> = st
        .battlefield
        .iter()
        .filter_map(|perm| {
            let card = card_of(deck, perm);
            let release = card.unlocked_abilities(perm.counters.charge).find(|a| {
                a.trigger == SimTrigger::PrecombatMain
                    && matches!(
                        a.effect_sequence().first(),
                        Some(SimEffect::ManaPerCounter(_))
                    )
            })?;
            let y = match release.effect_sequence().first()? {
                SimEffect::ManaPerCounter(y) => y.clone(),
                _ => return None,
            };
            Some((perm.uid, y, perm.counters.charge))
        })
        .collect();
    for (uid, y, counters) in releases {
        for _ in 0..counters {
            add_yield_turns(deck, &y, pool, turn, &st.battlefield);
        }
        // The contributing permanent clears its own bank; a second
        // copy of the card keeps its counters for its own upkeep.
        if let Some(perm) = st.battlefield.iter_mut().find(|p| p.uid == uid) {
            perm.counters.charge = 0;
        }
    }
}

/// Verge-gate lands: the gated tap mode unlocks when another land of a
/// gated type is in play; locked lands yield only the ungated first
/// mode (the first fixed pip, or the first choice color).
fn add_gated_yield(
    deck: &SimDeck,
    st: &GameState,
    perm: &Permanent,
    y: &super::super::model::ManaYield,
    pool: &mut ManaPool,
    turn: u32,
) {
    let gates_open = card_of(deck, perm).gate_types.iter().any(|want| {
        st.battlefield.iter().any(|p| {
            p.uid != perm.uid && is_land_permanent(p) && card_of(deck, p).land_types[want.index()]
        })
    });
    if gates_open {
        add_yield_turns(deck, y, pool, turn, &st.battlefield);
        return;
    }
    let ungated_color = y
        .fixed
        .iter()
        .position(|p| *p > 0)
        .or_else(|| y.choice.iter().position(|c| *c));
    let Some(ci) = ungated_color else {
        if y.colorless > 0 {
            pool.colorless += 1;
        }
        return;
    };
    let mut ungated = super::super::model::ManaYield {
        ..super::super::model::ManaYield::default()
    };
    ungated.fixed[ci] = 1;
    add_yield(&ungated, pool);
}

/// Read card-level tap grants only while the provider's printed front is exposed.
pub(in crate::deck::simulator) fn grants_active(
    deck: &SimDeck,
    st: &GameState,
) -> super::super::model::Grants {
    let mut grants = super::super::model::Grants::default();
    for perm in &st.battlefield {
        let card = card_of(deck, perm);
        if perm.face_down || perm.land_face && card.is_mdfc_spell {
            continue;
        }
        match card.flags.grant {
            Some(super::super::model::Grant::Creatures) => grants.creatures = true,
            Some(super::super::model::Grant::Lands) => grants.lands = true,
            None => {}
        }
    }
    grants
}
