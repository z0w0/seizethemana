//! Resolve ordered spell effects and their supported variable amounts.

use super::super::game::{GameState, ManaPool, card_of};
use super::super::game_effects::{PreparedSpellTargets, apply_prepared_spell_effect_at};
use super::super::game_mana::add_yield_turns;
use super::super::model::{CardIdx, SimDeck, SimEffect, XClass};

/// Resolve every lowered spell effect in its Oracle order.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_spell_effects(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    index: CardIdx,
    turn: u32,
    x: u32,
    kicked: bool,
    mut targets: PreparedSpellTargets,
) {
    let card = &deck[index];
    for effect in &card.spell_data.effects {
        let effect = scaled_effect(effect, card.spell_data.x_class, x, kicked);
        match &effect {
            SimEffect::Mana(yield_) => add_yield_turns(deck, yield_, pool, turn, &st.battlefield),
            SimEffect::Counters(amount) => {
                if let Some(permanent) = st
                    .battlefield
                    .iter_mut()
                    .filter(|p| card_of(deck, p).is_station_card)
                    .max_by_key(|p| card_of(deck, p).animate_at().unwrap_or(0))
                {
                    permanent.counters.charge += amount;
                }
            }
            _ => apply_prepared_spell_effect_at(
                deck,
                &effect,
                st,
                turn,
                card.mills_opponent,
                Some(index),
                &mut targets,
            ),
        }
    }
    if let Some(rule) = card.spell_data.reveal_rule {
        super::resolve_reveal_rule(deck, st, rule, turn);
    }
}

/// Substitute a chosen X and the supported kicker rider without changing order.
fn scaled_effect(effect: &SimEffect, class: Option<XClass>, x: u32, kicked: bool) -> SimEffect {
    match effect {
        SimEffect::Draw(0) if class == Some(XClass::Draw) => SimEffect::Draw(x),
        SimEffect::Mill(0) if class == Some(XClass::Mill) => SimEffect::Mill(x),
        SimEffect::Tokens(0) if class == Some(XClass::Tokens) => SimEffect::Tokens(x),
        SimEffect::Treasures(0) if class == Some(XClass::Treasures) => SimEffect::Treasures(x),
        SimEffect::LoseLife { amount, scope } => SimEffect::LoseLife {
            amount: if *amount == 0 && class == Some(XClass::Drain) {
                x
            } else {
                *amount * (1 + u32::from(kicked))
            },
            scope: *scope,
        },
        SimEffect::Damage { amount, scope } => SimEffect::Damage {
            amount: if *amount == 0 && class == Some(XClass::Drain) {
                x
            } else {
                *amount * (1 + u32::from(kicked))
            },
            scope: *scope,
        },
        _ => effect.clone(),
    }
}
