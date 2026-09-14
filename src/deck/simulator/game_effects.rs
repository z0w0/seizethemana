// Effect execution and the tap-budget pass for the goldfish game loop,
// split from game.rs to keep files small.

use super::game::{BODY_POWER, GameState, InPlay, Pool, card_of};
use super::game_mana::{add_yield, effective_min_cost};
use super::model::{Effect, Role, SimDeck};

/// Execute one ability effect against the game state. Pure helper for the
/// trigger paths; the upkeep/combat paths call it too.
pub(super) fn apply_effect(deck: &SimDeck, effect: &Effect, st: &mut GameState, turn: u32) {
    match effect {
        Effect::Draw(n) => {
            for _ in 0..*n {
                if let Some(i) = st.library.pop() {
                    st.hand.push(i);
                    st.seen += 1;
                }
            }
        }
        Effect::Tutor | Effect::ExtraLand => {
            if let Some(i) = st.library.pop() {
                st.hand.push(i);
                st.seen += 1;
            }
        }
        Effect::Mill(n) => {
            for _ in 0..*n {
                if let Some(i) = st.library.pop() {
                    st.graveyard_seen.entry(i).or_insert(turn);
                    st.graveyard.push(i);
                    st.seen += 1;
                }
            }
        }
        Effect::ReturnFromGraveyard { to_hand, count } => {
            for _ in 0..*count {
                let Some(i) = st.graveyard.pop() else {
                    break;
                };
                if *to_hand {
                    st.hand.push(i);
                    st.seen += 1;
                } else if let Some(card) = deck.cards.get(i)
                    && card.is_creature
                {
                    // Returns as a body once; the card leaves the log.
                    st.battlefield_seen.entry(i).or_insert(turn);
                    st.battlefield.push(InPlay {
                        card: i,
                        tapped: false,
                        sick: true,
                        counters: 0,
                        animated: false,
                        crewed: false,
                        entered_turn: turn as usize,
                        saga_step: 0,
                        fired: false,
                        blink_pending: false,
                        loyalty: 0,
                        is_commander: false,
                        commander_slot: 0,
                    });
                } else {
                    st.hand.push(i);
                    st.seen += 1;
                }
            }
        }
        Effect::Wheel => {
            for i in st.hand.drain(..) {
                st.graveyard_seen.entry(i).or_insert(turn);
                st.graveyard.push(i);
            }
            for _ in 0..7 {
                if let Some(i) = st.library.pop() {
                    st.hand.push(i);
                    st.seen += 1;
                }
            }
        }
        Effect::Loot(n) => {
            for _ in 0..*n {
                if let Some(i) = st.library.pop() {
                    st.hand.push(i);
                    st.seen += 1;
                }
                // Discard the oldest hand card into the graveyard.
                if !st.hand.is_empty() {
                    let discarded = st.hand.remove(0);
                    st.graveyard_seen.entry(discarded).or_insert(turn);
                    st.graveyard.push(discarded);
                }
            }
        }
        Effect::Tokens(_) => {
            // Tokens join as small station/crew fuel bodies. Their
            // static data mirrors a 2/2 body with no abilities.
            st.battlefield.push(InPlay {
                card: usize::MAX - 1,
                tapped: false,
                sick: true,
                counters: 0,
                animated: false,
                crewed: false,
                entered_turn: turn as usize,
                saga_step: 0,
                fired: false,
                blink_pending: false,
                loyalty: 0,
                is_commander: false,
                commander_slot: 0,
            });
        }
        _ => {}
    }
}

/// The tap-budget pass: mana only while casting still needs mana, then
/// station, then crew.
pub(super) fn tap_budget(
    deck: &SimDeck,
    battlefield: &mut [InPlay],
    pool: &mut Pool,
    hand: &[usize],
) {
    // Remaining demand: the cheapest uncast spell still in hand.
    let cheapest: Option<u32> = hand
        .iter()
        .filter(|i| deck.cards[**i].role != Role::Land)
        .map(|i| effective_min_cost(deck, &deck.cards[*i], battlefield).total())
        .min();

    // Creatures and crewed vehicles able to tap this turn.
    let tappable: Vec<usize> = battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            !p.tapped
                && !p.sick
                && !p.is_commander
                && !card_of(deck, p).is_station_card
                && (card_of(deck, p).is_creature || p.animated)
        })
        .map(|(i, _)| i)
        .collect();

    // 7a Mana taps: only while the cheapest spell still needs mana.
    if let Some(need) = cheapest {
        for &bi in &tappable {
            if pool.total() >= need {
                break;
            }
            let card = card_of(deck, &battlefield[bi]);
            if let Some(y) = &card.tap {
                add_yield(y, pool);
                battlefield[bi].tapped = true;
            }
        }
    }

    // 7b Station: tap remaining bodies into the highest-threshold unfilled
    // spacecraft/planet, one at a time, counters = body power.
    for &bi in &tappable {
        if battlefield[bi].tapped {
            continue;
        }
        let target = battlefield
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                *i != bi
                    && card_of(deck, p).is_station_card
                    && card_of(deck, p)
                        .animate_at()
                        .is_some_and(|at| p.counters < at)
            })
            .max_by_key(|(_, p)| card_of(deck, p).animate_at().unwrap_or(0))
            .map(|(i, _)| i);
        if let Some(ti) = target {
            battlefield[ti].counters += body_power(&battlefield[bi], deck);
            battlefield[bi].tapped = true;
        }
    }

    // 7c Crew: tap bodies into untapped Vehicles (total power ≥ crew N).
    for vi in 0..battlefield.len() {
        if battlefield[vi].tapped
            || battlefield[vi].sick
            || card_of(deck, &battlefield[vi]).crew.is_none()
        {
            continue;
        }
        let crew_n = card_of(deck, &battlefield[vi]).crew.unwrap_or(1);
        let bodies_needed = crew_n.div_ceil(BODY_POWER);
        let mut tapped_bodies = 0;
        let mut power = 0u32;
        for (i, perm) in battlefield.iter_mut().enumerate() {
            if tapped_bodies >= bodies_needed {
                break;
            }
            if i != vi
                && !perm.tapped
                && !perm.sick
                && !perm.is_commander
                && (card_of(deck, perm).is_creature || perm.animated)
            {
                perm.tapped = true;
                tapped_bodies += 1;
                power += body_power(perm, deck);
            }
        }
        if power >= crew_n {
            battlefield[vi].crewed = true;
            battlefield[vi].sick = false;
            // OnCrewed triggers fire now (tokens join next turn's bodies).
        }
    }
}

/// Body power for a permanent: the printed power when the card row has
/// one, else the flat token value. Crew and station math use it.
pub(super) fn body_power(perm: &InPlay, deck: &SimDeck) -> u32 {
    if perm.card < usize::MAX - 1 {
        deck.cards[perm.card].printed_power.unwrap_or(BODY_POWER)
    } else {
        BODY_POWER
    }
}
