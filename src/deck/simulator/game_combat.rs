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
/// double strike join the power sum. Player-targeted combat drains
/// resolve at the format's opponent multiplier (three opponents in the
/// commander family).
pub(super) fn combat_phase(
    deck: &SimDeck,
    st: &mut GameState,
    turn: usize,
    turns: usize,
    land_drops: &[u8],
) -> CombatOutcome {
    let drain_mult = deck.format.drain_mult();
    let static_buff_power: i32 = st
        .battlefield
        .iter()
        .filter(|p| p.card < usize::MAX - 1)
        .map(|p| card_of(deck, p).buff.map(|(p_, _)| p_.max(0)).unwrap_or(0))
        .sum();
    // Equipped gear buffs its own host only: (host uid, buff power).
    // Hosts are matched by uid so board shifts between turns cannot
    // move the buff to a different permanent.
    let equip_buffs: Vec<(u32, i32)> = st
        .battlefield
        .iter()
        .filter(|p| p.equipped && p.card < usize::MAX - 1)
        .filter_map(|p| {
            let host = p.equip_host?;
            Some((host, card_of(deck, p).equipment?))
        })
        .map(|(host, e)| (host, e.buff.0.max(0)))
        .collect();
    let mut token_bodies = 0u32;
    let mut power_total: u32 = 0;
    let mut attackers: u32 = 0;
    let mut evasive: u32 = 0;
    // Spells cast this turn feed prowess (the cast path counts them).
    let prowess_bumps = st.prowess_casts;
    // One-shot board buffs that entered this turn ("+X/+X where X is the
    // number of creatures you control"): each attacker gets +X for this
    // turn only, X = the body count, capped at 20.
    let board_buff_x = if st.battlefield.iter().any(|p| {
        p.entered_turn == turn && p.card < usize::MAX - 1 && card_of(deck, p).buffs_board_on_enter
    }) {
        st.battlefield
            .iter()
            .filter(|p| card_of(deck, p).is_creature || p.animated)
            .count()
            .min(20) as i32
    } else {
        0
    };
    // Snapshot the board: OnAttack/OnCombatDamage triggers push tokens
    // and would shift indices mid-loop. Only the light per-permanent
    // fields the loop reads are copied.
    let snapshot: Vec<(usize, bool, bool, bool)> = st
        .battlefield
        .iter()
        .enumerate()
        .map(|(pi, perm)| {
            (
                pi,
                perm.animated,
                perm.crewed,
                card_of(deck, perm).is_creature && !perm.tapped && !perm.sick,
            )
        })
        .collect();
    for (pi, animated, crewed, untapped_creature) in snapshot {
        let attacks = animated || crewed || untapped_creature;
        if !attacks {
            continue;
        }
        let perm = st.battlefield[pi].clone();
        let card = card_of(deck, &perm);
        attackers += 1;
        if card.evasion {
            evasive += 1;
        }
        let mut power = card.printed_power.unwrap_or(BODY_POWER) as i32;
        // Counter-powered bodies: the entered +1/+1 counters join the
        // attack power.
        if card.counters_are_power {
            power += perm.counters as i32;
        }
        if card.is_station_card && !perm.animated && !perm.crewed {
            power = 0;
        }
        power += static_buff_power;
        // Equipment buffs only their equipped host: the gear's power
        // joins this attacker when THIS permanent paid the equip cost.
        power += equip_buffs
            .iter()
            .filter(|(host, _)| perm.uid != 0 && *host == perm.uid)
            .map(|(_, buff)| *buff)
            .sum::<i32>();
        if card.landfall {
            // +1 per land drop made after the permanent entered.
            let start = perm.entered_turn.min(turns - 1);
            let drops_after: u32 = land_drops[start..turn].iter().map(|d| u32::from(*d)).sum();
            power += drops_after as i32;
        }
        if board_buff_x > 0 {
            power += board_buff_x;
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
                        // The attacking copy's own uid keys the counters:
                        // a second copy of the same card must not receive
                        // the combat trigger.
                        if let Some(p) = st.battlefield.iter_mut().find(|q| q.uid == perm.uid) {
                            p.counters += n;
                        }
                    }
                    Effect::Drain(n) => {
                        st.drained += n * drain_mult;
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
