// Commander engine bookkeeping for the goldfish game loop: the
// per-deck commander profile (synthetic upkeep tiers) and the sentinel
// uids that key each cast commander's own engine.

use super::model::{Effect, SimDeck, Trigger};

/// Per-deck commander engine profile: the synthetic upkeep tiers.
pub(super) struct CommanderProfile {
    /// Upkeep draw amount per commander slot.
    pub(super) engine_draws: Vec<Option<u32>>,
    /// True when the commander carries any other OnUpkeep engine.
    pub(super) engine_other: Vec<bool>,
    /// The commander spacecraft's animation threshold.
    pub(super) station_at: Option<u32>,
}

/// Sentinel uid for a cast commander's upkeep engine. Real card uids
/// count up from 0; sentinels count down from `u32::MAX`, so they never
/// collide with battlefield uids. The low bits carry the commander slot,
/// so each cast commander fires only its own abilities.
pub(super) fn commander_sentinel_uid(slot: usize) -> u32 {
    u32::MAX - (slot as u32).min(15)
}

/// True when the uid is a commander sentinel (not a battlefield uid).
pub(super) fn is_commander_sentinel(uid: u32) -> bool {
    uid > u32::MAX - 16
}

/// The commander slot encoded in a sentinel uid.
pub(super) fn commander_sentinel_slot(uid: u32) -> usize {
    (u32::MAX - uid) as usize
}

/// Upkeep abilities of one commander (by deck slot), in effect form.
pub(super) fn commander_upkeep_effects(deck: &SimDeck, slot: usize) -> Vec<Effect> {
    deck.commanders
        .get(slot)
        .map(|cmd| {
            cmd.abilities()
                .filter(|a| a.trigger == Trigger::OnUpkeep)
                .map(|a| a.effect.clone())
                .collect()
        })
        .unwrap_or_default()
}

impl CommanderProfile {
    /// The commander's synthetic engine tier (upkeep/end-step draws only)
    /// comes from deck construction. Attack-gated draws live in real tiers
    /// and fire through the combat path once the spacecraft animates.
    /// Per-commander: each partner registers its own upkeep engine, so a
    /// partner pair never shares (or doubles) a firing.
    pub(super) fn new(deck: &SimDeck) -> Self {
        let engine_draws = deck
            .commanders
            .iter()
            .map(|cmd| {
                Some(
                    cmd.station_tiers
                        .iter()
                        .filter(|t| t.at == 0)
                        .flat_map(|t| t.abilities.iter())
                        .filter(|a| a.trigger == Trigger::OnUpkeep)
                        .filter_map(|a| match a.effect {
                            Effect::Draw(n) => Some(n),
                            _ => None,
                        })
                        .sum::<u32>(),
                )
            })
            .collect();
        // Commanders with any other OnUpkeep engine (drain, mill, tokens,
        // recursion) register a zero-draw engine: the upkeep loop runs the
        // real parsed abilities for a pushed slot.
        let engine_other = deck
            .commanders
            .iter()
            .map(|cmd| {
                cmd.station_tiers
                    .iter()
                    .filter(|t| t.at == 0)
                    .flat_map(|t| t.abilities.iter())
                    .any(|a| {
                        a.trigger == Trigger::OnUpkeep
                            && matches!(
                                a.effect,
                                Effect::Mill(_)
                                    | Effect::ReturnFromGraveyard { .. }
                                    | Effect::Drain(_)
                                    | Effect::Tokens(_)
                                    | Effect::Wheel
                                    | Effect::Loot(_)
                            )
                    })
            })
            .collect();
        let station_at = deck.commanders.first().and_then(|cmd| cmd.animate_at());
        Self {
            engine_draws,
            engine_other,
            station_at,
        }
    }
}
