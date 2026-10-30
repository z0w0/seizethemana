//! Nonconsuming Behold choices from the hand and controlled battlefield.

use crate::deck::simulator::game::{GameState, Permanent, card_of, is_creature_permanent};
use crate::deck::simulator::model::behold::{BeholdAlternative, BeholdSpec};
use crate::deck::simulator::model::{CardIdx, SimDeck};

/// Find one legal reveal or controlled permanent without spending any resource.
///
/// Exclude a pending cast from the hand. For land entry, call this before adding
/// the entering land to the battlefield and exclude that land's hand index.
pub(in crate::deck::simulator) fn available(
    deck: &SimDeck,
    st: &GameState,
    spec: &BeholdSpec,
    excluded_hand: Option<CardIdx>,
) -> bool {
    if spec.count != 1 || spec.card_types.is_empty() && spec.subtypes.is_empty() {
        return false;
    }
    st.hand.iter().any(|index| {
        Some(*index) != excluded_hand
            && spec.matches_characteristics(&deck[*index].card_types, &deck[*index].subtypes)
    }) || st
        .battlefield
        .iter()
        .any(|permanent| permanent_matches(deck, permanent, spec))
}

/// Return only the additional generic mana owed for a Behold-or-pay cost.
pub(in crate::deck::simulator) fn generic_surcharge(
    deck: &SimDeck,
    st: &GameState,
    alternative: &BeholdAlternative,
    excluded_hand: Option<CardIdx>,
) -> u32 {
    if available(deck, st, &alternative.spec, excluded_hand) {
        0
    } else {
        alternative.generic
    }
}

/// Untap only this search's result if a legal Behold choice still exists.
///
/// Pass the result's stable battlefield uid after entry and shuffle. A missing
/// result or an object that left the battlefield never selects a different land.
pub(in crate::deck::simulator) fn untap_search_result(
    deck: &SimDeck,
    st: &mut GameState,
    spec: &BeholdSpec,
    result_uid: Option<u32>,
) {
    if let Some(uid) = result_uid
        && available(deck, st, spec, None)
        && let Some(permanent) = st
            .battlefield
            .iter_mut()
            .find(|permanent| permanent.uid == uid)
    {
        permanent.tapped = false;
    }
}

/// Use the current face and creature state instead of hidden printed qualities.
fn permanent_matches(deck: &SimDeck, permanent: &Permanent, spec: &BeholdSpec) -> bool {
    if permanent.face_down {
        return spec.subtypes.is_empty() && spec.card_types.iter().all(|kind| kind == "creature");
    }
    let card = card_of(deck, permanent);
    let front_is_land = card.card_types.iter().any(|kind| kind == "land");
    let hidden_printed_face = card.is_mdfc_spell && permanent.land_face != front_is_land;
    spec.card_types.iter().all(|kind| match kind.as_str() {
        "creature" => is_creature_permanent(deck, permanent),
        "land" => permanent.land_face,
        _ => !hidden_printed_face && card.card_types.contains(kind),
    }) && (spec.subtypes.is_empty()
        || !hidden_printed_face
            && spec
                .subtypes
                .iter()
                .all(|kind| card.subtypes.contains(kind)))
}

/// Predicate checks cover both zones, current faces, tokens, and unspent resources.
#[cfg(test)]
#[path = "tests/behold_tests.rs"]
mod behold_tests;
