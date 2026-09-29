//! The combat phase for the goldfish game loop, split from game_run to
//! keep files small. Pure apart from the game state mutations (draws,
//! counters, drains).

use super::game::{GameState, TOKEN_CREATURE_POWER, card_of};
use super::game_effects::apply_effect_at;
use super::model::{SimAbility, SimDeck, SimEffect, SimTrigger};

/// Triggered combat ability snapshot for one source permanent.
struct CombatTrigger {
    source_uid: u32,
    source_card: Option<super::model::CardIdx>,
    mills_opponent: bool,
    ability: SimAbility,
}

/// One combat phase's census for the turn log.
pub(super) struct CombatOutcome {
    /// Total attacking power this turn (buffs, equipment, double strike
    /// included).
    pub(super) power: u32,
    /// Attacking bodies (the evasion-census denominator).
    pub(super) attackers: u32,
    /// Attacking bodies with evasion (trample/flying/menace).
    pub(super) evasive: u32,
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
    land_drops: &[u8],
) -> CombatOutcome {
    let life_loss_mult = deck.format.life_loss_mult();
    let static_buff_power: i32 = st
        .battlefield
        .iter()
        .filter(|p| p.card.deck_idx().is_some())
        .map(|p| {
            card_of(deck, p)
                .flags
                .buff
                .map(|(p_, _)| p_.max(0))
                .unwrap_or(0)
        })
        .sum();
    // Equipped gear buffs its own host only: (host uid, buff power).
    // Hosts are matched by uid so board shifts between turns cannot
    // move the buff to a different permanent.
    let equip_buffs: Vec<(u32, i32)> = st
        .battlefield
        .iter()
        .filter(|p| p.equipped && p.card.deck_idx().is_some())
        .filter_map(|p| {
            let host = p.equip_host?;
            Some((host, card_of(deck, p).flags.equipment?))
        })
        .map(|(host, e)| (host, e.buff.0.max(0)))
        .collect();
    let mut power_total: u32 = 0;
    let mut attackers: u32 = 0;
    let mut evasive: u32 = 0;
    // Spells cast this turn feed prowess (the cast path counts them).
    let prowess_bumps = st.prowess_casts;
    let mut listeners = combat_triggers(deck, st);
    let mut listener_uids: Vec<u32> = st.battlefield.iter().map(|perm| perm.uid).collect();
    // One-shot board buffs that entered this turn ("+X/+X where X is the
    // number of creatures you control"): each attacker gets +X for this
    // turn only, X = the body count, capped at 20.
    let battlefield_buff_x = if st.battlefield.iter().any(|p| {
        p.entered_turn == turn
            && p.card.deck_idx().is_some()
            && card_of(deck, p).buffs_battlefield_on_entry
    }) {
        st.battlefield
            .iter()
            .filter(|p| super::game::is_creature_permanent(deck, p))
            .count()
            .min(20) as i32
    } else {
        0
    };
    // Snapshot the board: Attacks/CombatDamage triggers push tokens
    // and would shift indices mid-loop. Only the light per-permanent
    // fields the loop reads are copied.
    let snapshot: Vec<(usize, bool, bool, bool, bool)> = st
        .battlefield
        .iter()
        .enumerate()
        .map(|(pi, perm)| {
            let card = card_of(deck, perm);
            (
                pi,
                perm.animated,
                perm.crewed,
                perm.attacking_this_turn,
                // Printed creatures and living-metal Vehicles are
                // creatures independent of any animation this turn.
                (card.is_creature || card.keyword_abilities.living_metal) && !perm.tapped,
            )
        })
        .collect();
    for (pi, animated, crewed, attacking_entered, untapped_creature) in snapshot {
        // Summoning sickness (CR 302.6): a creature that has not been
        // under its controller's control since the turn began cannot
        // attack. A crewed Vehicle or animated spacecraft that entered
        // the battlefield this turn becomes a creature only now, so it
        // waits one turn unless it has haste.
        let entered = st.battlefield[pi].entered_turn;
        let entry_turn_is_past = entered < turn;
        let perm = st.battlefield[pi].clone();
        let keywords = super::game::effective_keywords(deck, st, &perm);
        // Living metal (CR 702.161): during your turn the Vehicle is an
        // artifact creature. The goldfish's combat is always the
        // player's turn, so the flag counts as animated.
        let living_metal = card_of(deck, &perm).keyword_abilities.living_metal;
        let attacks = attacking_entered
            || ((animated || crewed || living_metal)
                && (entry_turn_is_past || keywords.contains(super::model::Keyword::Haste)))
            || (untapped_creature
                && (!perm.summoning_sick || keywords.contains(super::model::Keyword::Haste)));
        if !attacks {
            continue;
        }
        let card = card_of(deck, &perm);
        attackers += 1;
        st.attacked_this_turn = true;
        if keywords.contains(super::model::Keyword::Trample)
            || keywords.contains(super::model::Keyword::Flying)
            || keywords.contains(super::model::Keyword::Menace)
        {
            evasive += 1;
        }
        // Face-down bodies are 2/2 creatures with no text (CR 708.2).
        let mut power = if perm.face_down {
            TOKEN_CREATURE_POWER as i32
        } else if perm.army {
            // Army bodies are 0/0; their +1/+1 counters are the power.
            perm.counters.plus1 as i32
        } else {
            card.printed_power.unwrap_or(TOKEN_CREATURE_POWER) as i32
        };
        // Counter-powered bodies: the entered +1/+1 counters join the
        // attack power.
        if card.counters_are_power {
            power += perm.counters.plus1 as i32;
        }
        if card.is_station_card && !perm.animated && !perm.crewed {
            power = 0;
        }
        // Saddle (CR 702.171b): "while saddled" buffs join the attack
        // only while the saddle cost is paid this turn.
        if perm.saddled
            && let Some((buff_power, _)) = card.flags.saddled_buff
        {
            power += buff_power;
        }
        power += static_buff_power;
        // Equipment buffs only their equipped host: the gear's power
        // joins this attacker when THIS permanent paid the equip cost.
        power += equip_buffs
            .iter()
            .filter(|(host, _)| perm.uid != 0 && *host == perm.uid)
            .map(|(_, buff)| *buff)
            .sum::<i32>();
        if card.flags.landfall {
            // +1 per land drop made after the permanent entered. Same-turn
            // entrants exclude this turn's own drops (their landfall
            // resolved at entry), matching every other turn's slice.
            let drops_after: u32 = land_drops[perm.entered_turn..turn]
                .iter()
                .map(|d| u32::from(*d))
                .sum();
            power += drops_after as i32;
        }
        if battlefield_buff_x > 0 {
            power += battlefield_buff_x;
        }
        if keywords.contains(super::model::Keyword::Prowess) {
            power += prowess_bumps as i32;
        }
        if keywords.contains(super::model::Keyword::DoubleStrike) {
            power *= 2;
        }
        // Lifelink (CR 702.15): each connecting attacker gains life
        // equal to the damage it deals. The goldfish is unblocked, so
        // the attacker's power is the life gained.
        if keywords.contains(super::model::Keyword::Lifelink) {
            st.life += power.max(0);
            st.life_gained += power.max(0) as u32;
        }
        power_total += power.max(0) as u32;
        // The Ring (CR 701.54c): at level 2+ the Ring-bearer's attack
        // draws and discards; at level 4+ its combat damage drains each
        // opponent by 3. The goldfish is unblocked, so every connecting
        // Ring-bearer damage event counts.
        if st.ring_bearer == Some(perm.uid) && power > 0 {
            if st.ring_tempts >= 2 {
                apply_effect_at(
                    deck,
                    &SimEffect::DrawThenDiscard(1),
                    st,
                    turn as u32,
                    false,
                    None,
                );
            }
            if st.ring_tempts >= 4 {
                st.opponent_life_lost += 3 * life_loss_mult;
            }
        }
        refresh_combat_triggers(deck, st, &mut listeners, &mut listener_uids);
        power_total +=
            fire_combat_triggers(deck, st, &listeners, SimTrigger::Attacks, perm.uid, turn);
        if power > 0 {
            refresh_combat_triggers(deck, st, &mut listeners, &mut listener_uids);
            fire_combat_triggers(
                deck,
                st,
                &listeners,
                SimTrigger::CombatDamage,
                perm.uid,
                turn,
            );
        }
    }
    if attackers > 0 {
        refresh_combat_triggers(deck, st, &mut listeners, &mut listener_uids);
        fire_combat_triggers(deck, st, &listeners, SimTrigger::PlayerAttacks, 0, turn);
    }
    CombatOutcome {
        power: power_total,
        attackers,
        evasive,
    }
}

/// Resolve a supported trigger for one attacking or damaging permanent.
fn fire_combat_triggers(
    deck: &SimDeck,
    st: &mut GameState,
    listeners: &[CombatTrigger],
    trigger: SimTrigger,
    event_uid: u32,
    turn: usize,
) -> u32 {
    let triggers: Vec<&CombatTrigger> = listeners
        .iter()
        .filter(|listener| {
            listener.ability.trigger == trigger
                && super::game::trigger_subject_matches(
                    listener.ability.event_subject,
                    listener.source_uid,
                    event_uid,
                )
                && listener
                    .ability
                    .condition
                    .is_none_or(|condition| super::game::condition_met(deck, st, &condition))
        })
        .collect();
    let mut mobilized_power = 0;
    for listener in triggers {
        let source_uid = listener.source_uid;
        let source_card = listener.source_card;
        let mills_opponent = listener.mills_opponent;
        let ability = &listener.ability;
        if !st.battlefield.iter().any(|perm| perm.uid == source_uid) {
            continue;
        }
        let ability_key = (source_uid, ability.id);
        if ability.once_per_turn && st.triggered_this_turn.contains(&ability_key) {
            continue;
        }
        if ability
            .condition
            .is_some_and(|condition| !super::game::condition_met(deck, st, &condition))
        {
            continue;
        }
        for effect in ability.effect_sequence() {
            match effect {
                SimEffect::Counters(amount) => {
                    if let Some(source) = st.battlefield.iter_mut().find(|p| p.uid == source_uid) {
                        source.counters.charge += amount;
                    }
                }
                SimEffect::Mobilize(amount) if trigger == SimTrigger::Attacks => {
                    super::game_effects::mobilize(deck, st, turn as u32, *amount);
                    mobilized_power += (*amount).min(8) * TOKEN_CREATURE_POWER;
                }
                SimEffect::Tokens(amount) if trigger == SimTrigger::Attacks => {
                    apply_effect_at(
                        deck,
                        &SimEffect::Tokens((*amount).min(4)),
                        st,
                        turn as u32,
                        mills_opponent,
                        source_card,
                    );
                }
                _ => apply_effect_at(deck, effect, st, turn as u32, mills_opponent, source_card),
            }
        }
        if ability.once_per_turn {
            st.triggered_this_turn.insert(ability_key);
        }
    }
    mobilized_power
}

/// Snapshot combat triggers from every permanent, including listeners
/// whose subject is another permanent's event.
fn combat_triggers(deck: &SimDeck, st: &GameState) -> Vec<CombatTrigger> {
    st.battlefield
        .iter()
        .flat_map(|source| {
            super::game::permanent_abilities(deck, source)
                .filter(|ability| {
                    ability.kind.is_triggered()
                        && matches!(
                            ability.trigger,
                            SimTrigger::Attacks
                                | SimTrigger::PlayerAttacks
                                | SimTrigger::CombatDamage
                        )
                })
                .map(|ability| CombatTrigger {
                    source_uid: source.uid,
                    source_card: source.card.deck_idx(),
                    mills_opponent: card_of(deck, source).mills_opponent,
                    ability: ability.clone(),
                })
        })
        .collect()
}

/// Refresh the trigger snapshot after a combat effect changes battlefield
/// identities.
fn refresh_combat_triggers(
    deck: &SimDeck,
    st: &GameState,
    listeners: &mut Vec<CombatTrigger>,
    listener_uids: &mut Vec<u32>,
) {
    let current_uids: Vec<u32> = st.battlefield.iter().map(|perm| perm.uid).collect();
    if *listener_uids != current_uids {
        *listeners = combat_triggers(deck, st);
        *listener_uids = current_uids;
    }
}
