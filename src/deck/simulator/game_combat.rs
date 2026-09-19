// The combat phase for the goldfish game loop, split from game_run to
// keep files small. Pure apart from the game state mutations (draws,
// counters, drains).

use super::game::{BODY_POWER, GameState, card_of};
use super::model::{Effect, SimDeck, Trigger};

/// One combat phase's census for the turn log.
pub(super) struct CombatOutcome {
    /// Total attacking power this turn (buffs, equipment, double strike
    /// included).
    pub(super) power: u32,
    /// Attacking bodies (the evasion-census denominator).
    pub(super) attackers: u32,
    /// Attacking bodies with evasion (trample/flying/menace).
    pub(super) evasive: u32,
    /// Token bodies created by attack triggers (capped at 4 by the
    /// caller's body count).
    pub(super) token_bodies: u32,
}

/// Run the combat phase: bodies attack, attack triggers fire, combat-
/// damage triggers resolve per connecting attacker (best case: every
/// attacker is unblocked). Buffs (static + equipment + landfall) and
/// double strike join the power sum.
pub(super) fn combat_phase(
    deck: &SimDeck,
    st: &mut GameState,
    turn: usize,
    turns: usize,
    land_drops: &[u8],
) -> CombatOutcome {
    let static_buff_power: i32 = st
        .battlefield
        .iter()
        .filter(|p| p.card < usize::MAX - 1)
        .map(|p| card_of(deck, p).buff.map(|(p_, _)| p_.max(0)).unwrap_or(0))
        .sum();
    let equipped_buff_power: i32 = st
        .battlefield
        .iter()
        .filter(|p| p.card < usize::MAX - 1)
        .filter_map(|p| card_of(deck, p).equipment)
        .map(|e| e.buff.0.max(0))
        .sum();
    let mut token_bodies = 0u32;
    let mut power_total: u32 = 0;
    let mut attackers: u32 = 0;
    let mut evasive: u32 = 0;
    // Spells cast this turn feed prowess (the cast path counts them).
    let prowess_bumps = st.prowess_casts;
    for perm in st.battlefield.clone() {
        let attacks = perm.animated
            || perm.crewed
            || (card_of(deck, &perm).is_creature && !perm.tapped && !perm.sick);
        if !attacks {
            continue;
        }
        let card = card_of(deck, &perm);
        attackers += 1;
        if card.evasion {
            evasive += 1;
        }
        let mut power = card.printed_power.unwrap_or(BODY_POWER) as i32;
        if card.is_station_card && !perm.animated && !perm.crewed {
            power = 0;
        }
        power += static_buff_power;
        power += equipped_buff_power;
        if card.landfall {
            // +1 per land drop made after the permanent entered.
            let start = perm.entered_turn.min(turns - 1);
            let drops_after: u32 = land_drops[start..turn].iter().map(|d| u32::from(*d)).sum();
            power += drops_after as i32;
        }
        if card.prowess {
            power += prowess_bumps as i32;
        }
        if card.double_strike {
            power *= 2;
        }
        power_total += power.max(0) as u32;
        for ability in card.abilities() {
            match ability.trigger {
                Trigger::OnAttack => match ability.effect {
                    Effect::Draw(n) => {
                        for _ in 0..n {
                            if let Some(i) = st.library.pop() {
                                st.hand.push(i);
                                st.seen += 1;
                                st.awareness_cards += 1;
                            }
                        }
                    }
                    Effect::Tokens(n) => token_bodies += n,
                    _ => {}
                },
                Trigger::OnCombatDamage if power > 0 => match ability.effect {
                    Effect::Draw(n) => {
                        for _ in 0..n {
                            if let Some(i) = st.library.pop() {
                                st.hand.push(i);
                                st.seen += 1;
                                st.awareness_cards += 1;
                            }
                        }
                    }
                    Effect::Counters(n) => {
                        if let Some(p) = st.battlefield.iter_mut().find(|q| q.card == perm.card) {
                            p.counters += n;
                        }
                    }
                    Effect::Drain(n) => {
                        st.drained += n;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    CombatOutcome {
        power: power_total,
        attackers,
        evasive,
        token_bodies,
    }
}
