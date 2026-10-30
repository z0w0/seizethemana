//! Bounded external activity for solitaire games, without opponent deck AI.
//!
//! Counts and scheduled actions share one resource ledger per seat. The scenario
//! supplies events; player abilities resolve through the normal effect engine.

use serde::{Deserialize, Serialize};

use super::model::{Format, LifeLossScope};

/// Resolve external events against player abilities without an opponent AI.
mod runtime;
pub(super) use runtime::{pay_cumulative_upkeep, run_opponent_round};

/// Version of the illustrative schedule, not an estimate of real table activity.
pub const MODEL_VERSION: u32 = 1;

/// External events and shared payments observed during one game.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpponentActivity {
    /// Individual successful opponent draws.
    pub draws: u32,
    /// Opponent spells whose card and mana costs were paid.
    pub casts: u32,
    /// Successful optional tax payments, not mana spent.
    pub payments: u32,
    /// Scheduled spells skipped because their required resources were absent.
    pub skipped_casts: u32,
}

/// Optional tax choices remain limited by actual remaining sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxPolicy {
    /// Keep resources for the next scheduled action, then pay from spare sources.
    Spare,
    /// Limit discretionary payments between untaps as well as source spending.
    CappedSpare(u32),
    /// Decline all optional tax payments.
    Decline,
}

/// Reproducible scenario parameters. Values are illustrative, not calibrated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpponentConfig {
    /// Whether scheduled opponent turns generate external events.
    pub active: bool,
    /// Maximum scheduled lands per seat; later draws remain available for spells.
    pub land_target: u32,
    /// Spell costs indexed by that seat's turn, repeating the final cost later.
    pub spell_costs: Vec<u32>,
    /// Shared optional-payment policy used by every engine.
    pub tax_policy: TaxPolicy,
    /// Optional declared land colors per seat, using WUBRG letters. An empty
    /// string means no eligible color; omission keeps the generic-credit model.
    #[serde(default)]
    pub land_colors: Option<Vec<String>>,
}

impl OpponentConfig {
    /// Reject an undefined schedule before starting any games.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.spell_costs.is_empty(),
            "opponent spell_costs must not be empty"
        );
        if let Some(colors) = &self.land_colors {
            anyhow::ensure!(
                colors
                    .iter()
                    .all(|colors| colors.chars().all(|color| "WUBRG".contains(color))),
                "opponent land_colors must use only WUBRG letters"
            );
        }
        Ok(())
    }
}

impl Default for OpponentConfig {
    fn default() -> Self {
        Self {
            active: true,
            land_target: 6,
            spell_costs: vec![2, 2, 3, 3, 4, 4, 5],
            tax_policy: TaxPolicy::CappedSpare(4),
            land_colors: None,
        }
    }
}

/// Facts about a synthetic seat. Unknown subtypes and colors are not invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpponentSeat {
    /// Current life total, preserved across scheduled turns.
    pub life: i32,
    /// Whether the seat still contributes events and eligible targets.
    pub active: bool,
    /// Abstract hand cards; each scheduled land and spell consumes one.
    pub hand: u32,
    /// Remaining abstract library cards.
    pub library: u32,
    /// Lands currently controlled, also the available source count at untap.
    pub lands: u32,
    /// Generic creature permanents. No creature subtype is implied.
    pub creatures: u32,
    /// Noncreature artifacts, separate from the creature count.
    pub artifacts: u32,
    /// Unspent sources since the seat's untap, not floated mana.
    pub unspent_sources: u32,
    /// Resources reserved for its next scheduled spell.
    pub reserved: u32,
    /// Cost of this turn's planned spell, separate from source reservations.
    pub planned_spell_cost: Option<u32>,
    /// Optional tax spending since its untap.
    pub taxes_paid: u32,
    /// Number of actual scheduled turns taken by this seat.
    pub turns: u32,
}

/// External event produced only after the relevant card and mana costs are paid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpponentEvent {
    /// One individual draw, including one draw within a multi-card effect.
    Draw,
    /// A cast event. A copy that was not cast does not produce this event.
    Cast {
        /// Creature and noncreature spells trigger different engines.
        noncreature: bool,
        /// First noncreature cast by this seat in this table turn.
        first_noncreature: bool,
    },
}

/// Shared table state and activity counters for one game.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpponentTable {
    /// Exact synthetic settings used for this game.
    pub config: OpponentConfig,
    /// Persistent state in fixed seat order.
    pub seats: Vec<OpponentSeat>,
    /// Scheduled individual opponent draws that actually happened.
    pub draws: u32,
    /// Scheduled casts that could pay both card and mana costs.
    pub casts: u32,
    /// Optional trigger payments made from the shared ledger.
    pub payments: u32,
    /// Planned spells skipped for lack of a card or enough sources.
    pub skipped_casts: u32,
    /// Age counters for supported cumulative upkeep, keyed by source object.
    #[serde(skip)]
    pub(crate) upkeep_ages: std::collections::BTreeMap<u32, u32>,
}

impl OpponentTable {
    /// Use the same starting life as the simulated player's format rules.
    pub fn for_rules(rules: super::format::FormatRules) -> Self {
        let mut table = Self::for_format(rules.shape);
        for seat in &mut table.seats {
            seat.life = rules.starting_life;
        }
        table
    }

    /// Export observations without retaining the table's mutable state.
    pub fn activity(&self) -> OpponentActivity {
        OpponentActivity {
            draws: self.draws,
            casts: self.casts,
            payments: self.payments,
            skipped_casts: self.skipped_casts,
        }
    }
    /// Resolve declared production capability without treating tapped sources as
    /// colorless. Capability describes what lands could produce, not spare mana.
    pub fn known_land_colors(&self) -> Option<[bool; 5]> {
        let declarations = self.config.land_colors.as_ref()?;
        let mut colors = [false; 5];
        for (index, state) in self.seats.iter().enumerate() {
            if !state.active || state.lands == 0 {
                continue;
            }
            let declared = declarations.get(index)?;
            for (index, color) in "WUBRG".chars().enumerate() {
                colors[index] |= declared.contains(color);
            }
        }
        Some(colors)
    }

    /// Create the default external table for the selected library format.
    pub fn for_format(format: Format) -> Self {
        let count = format.life_loss_mult();
        let life = if format == Format::Commander { 40 } else { 20 };
        let library = if format == Format::Commander { 92 } else { 53 };
        Self {
            config: OpponentConfig {
                active: format == Format::Commander,
                ..OpponentConfig::default()
            },
            seats: (0..count)
                .map(|_| OpponentSeat {
                    life,
                    active: true,
                    hand: 7,
                    library,
                    lands: 0,
                    creatures: 0,
                    artifacts: 0,
                    unspent_sources: 0,
                    reserved: 0,
                    planned_spell_cost: None,
                    taxes_paid: 0,
                    turns: 0,
                })
                .collect(),
            draws: 0,
            casts: 0,
            payments: 0,
            skipped_casts: 0,
            upkeep_ages: std::collections::BTreeMap::new(),
        }
    }

    /// Start a seat's turn. Taxes during its draw step affect later main actions.
    pub fn begin_turn(&mut self, seat: usize) -> Option<OpponentEvent> {
        if !self.config.active {
            return None;
        }
        let state = &mut self.seats[seat];
        if !state.active {
            return None;
        }
        state.turns += 1;
        state.unspent_sources = state.lands;
        state.taxes_paid = 0;
        state.reserved = 0;
        state.planned_spell_cost = None;
        if state.library == 0 {
            state.active = false;
            return None;
        }
        state.library -= 1;
        state.hand += 1;
        let land_planned = u32::from(state.lands < self.config.land_target && state.hand > 0);
        if state.turns >= 2 && state.hand > land_planned {
            let cost = self
                .config
                .spell_costs
                .get(state.turns.saturating_sub(1) as usize)
                .or_else(|| self.config.spell_costs.last())
                .copied()
                .expect("validated opponent schedule has at least one spell cost");
            state.planned_spell_cost = Some(cost);
            if cost <= state.unspent_sources + land_planned {
                state.reserved = cost.saturating_sub(land_planned);
            }
        }
        self.draws += 1;
        Some(OpponentEvent::Draw)
    }

    /// Spend hand cards and sources on a land and one staggered scheduled spell.
    pub fn main_action(&mut self, seat: usize) -> Option<OpponentEvent> {
        if !self.config.active {
            return None;
        }
        let state = &mut self.seats[seat];
        if !state.active {
            return None;
        }
        if state.hand > 0 && state.lands < self.config.land_target {
            state.hand -= 1;
            state.lands += 1;
            state.unspent_sources += 1;
        }
        state.reserved = 0;
        if state.turns < 2 {
            return None;
        }
        let Some(cost) = state.planned_spell_cost.take() else {
            self.skipped_casts += 1;
            return None;
        };
        if state.hand == 0 || state.unspent_sources < cost {
            self.skipped_casts += 1;
            return None;
        }
        state.hand -= 1;
        state.unspent_sources -= cost;
        let kind = (state.turns + seat as u32) % 3;
        self.casts += 1;
        Some(OpponentEvent::Cast {
            noncreature: kind != 0,
            first_noncreature: kind != 0,
        })
    }

    /// Resolve a scheduled permanent only after its cast triggers have resolved.
    pub fn resolve_main_spell(&mut self, seat: usize) {
        let state = &mut self.seats[seat];
        if !state.active {
            return;
        }
        match (state.turns + seat as u32) % 3 {
            0 => state.creatures += 1,
            1 => state.artifacts += 1,
            _ => {}
        }
    }

    /// Pay one optional tax without reusing sources or reserved action resources.
    pub fn pay_tax(&mut self, seat: usize, amount: u32) -> bool {
        let state = &mut self.seats[seat];
        if !state.active || !self.config.active {
            return false;
        }
        let budget = match self.config.tax_policy {
            TaxPolicy::Decline => return false,
            TaxPolicy::Spare => u32::MAX,
            TaxPolicy::CappedSpare(budget) => budget.saturating_sub(state.taxes_paid),
        };
        if amount > budget || amount > state.unspent_sources.saturating_sub(state.reserved) {
            return false;
        }
        state.unspent_sources -= amount;
        state.taxes_paid += amount;
        self.payments += 1;
        true
    }

    /// Apply player-directed life loss once per affected active seat.
    /// Single-target effects choose lowest life, then fixed seat order.
    pub fn lose_life(&mut self, amount: u32, scope: LifeLossScope) -> u32 {
        if scope == LifeLossScope::SelfOnly {
            return 0;
        }
        let target = self
            .seats
            .iter()
            .enumerate()
            .filter(|(_, state)| state.active)
            .min_by_key(|(index, state)| (state.life, *index))
            .map(|(index, _)| index);
        let mut total: u32 = 0;
        for (index, state) in self.seats.iter_mut().enumerate() {
            if !state.active || scope == LifeLossScope::TargetPlayer && target != Some(index) {
                continue;
            }
            state.life = state
                .life
                .saturating_sub(i32::try_from(amount).unwrap_or(i32::MAX));
            total = total.saturating_add(amount);
        }
        total
    }

    /// Remove seats with lethal life before the next normal action or event.
    pub fn settle_losses(&mut self) {
        for state in &mut self.seats {
            if state.life <= 0 {
                state.active = false;
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/opponents_tests.rs"]
mod opponents_tests;
