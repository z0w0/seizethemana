//! Commander source bookkeeping for the goldfish game loop.

use super::model::{SimDeck, SimEffect, SimTrigger};

/// Per-deck upkeep trigger data used to register commander sources.
pub(super) struct CommanderProfile {
    /// Draw amount per commander slot, when present.
    pub(super) upkeep_draw_amounts: Vec<Option<u32>>,
    /// True when the commander has another supported upkeep effect.
    pub(super) has_other_upkeep_effect: Vec<bool>,
    /// The commander spacecraft's animation threshold.
    pub(super) station_at: Option<u32>,
}

impl CommanderProfile {
    /// Commander upkeep trigger data comes from card construction.
    /// Attack-gated draws live in parsed abilities
    /// and fire through the combat path once the spacecraft animates.
    /// Per-commander: each partner registers its own repeatable source, so a
    /// partner pair never shares (or doubles) a firing.
    pub(super) fn new(deck: &SimDeck) -> Self {
        let upkeep_draw_amounts = deck
            .commanders
            .iter()
            .map(|cmd| {
                Some(
                    cmd.striations
                        .iter()
                        .filter(|t| t.at == 0)
                        .flat_map(|t| t.abilities.iter())
                        .filter(|a| a.trigger == SimTrigger::Upkeep)
                        .flat_map(|ability| ability.effect_sequence())
                        .filter_map(|effect| match effect {
                            SimEffect::Draw(n) => Some(*n),
                            _ => None,
                        })
                        .sum::<u32>(),
                )
            })
            .collect();
        // Commanders with another upkeep effect register a source so the
        // upkeep loop runs the
        // real parsed abilities for a pushed slot.
        let has_other_upkeep_effect = deck
            .commanders
            .iter()
            .map(|cmd| {
                cmd.striations
                    .iter()
                    .filter(|t| t.at == 0)
                    .flat_map(|t| t.abilities.iter())
                    .any(|ability| {
                        ability.trigger == SimTrigger::Upkeep
                            && ability.effect_sequence().iter().any(|effect| {
                                matches!(
                                    effect,
                                    SimEffect::Mill(_)
                                        | SimEffect::ReturnFromGraveyard { .. }
                                        | SimEffect::LoseLife { .. }
                                        | SimEffect::Damage { .. }
                                        | SimEffect::Tokens(_)
                                        | SimEffect::Counters(_)
                                        | SimEffect::DiscardHandThenDrawSeven
                                        | SimEffect::DrawThenDiscard(_)
                                )
                            })
                    })
            })
            .collect();
        let station_at = deck.commanders.first().and_then(|cmd| cmd.animate_at());
        Self {
            upkeep_draw_amounts,
            has_other_upkeep_effect,
            station_at,
        }
    }
}
