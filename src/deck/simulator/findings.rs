//! Findings from the aggregated simulation stats: deck diagnoses
//! (`analyze_findings`), the mana-base verdict, and combo-pair
//! assembly timing.

use super::aggregate::{CardCast, SimStats};
use super::game::GameLog;
use super::model::{ManaColor, Role, SimDeck};

/// One combo pair's assembly timing: share of games where both pieces
/// were seen in hand by the target turn.
#[derive(Debug, Clone)]
pub struct ComboAccess {
    /// Both piece names, "A + B".
    pub pair: String,
    /// Target turn: the later piece's cast-on-curve turn.
    pub target_turn: u32,
    /// Share of games with both pieces seen by the target turn.
    pub game_share: f64,
    /// Finished games where both pieces reached hand by the target turn.
    pub event_games: u32,
    /// Finished games, including supported player losses.
    pub observed_games: u32,
}

/// Measure first-hand access with the same predicate used by streamed runs.
pub fn piece_pair_access(
    logs: &[GameLog],
    deck: &SimDeck,
    pairs: &[(String, String)],
    turns: u32,
) -> Vec<ComboAccess> {
    let mut accumulator = super::pair_access::PairAccumulator::new(deck, pairs, turns);
    for log in logs {
        accumulator.push(log);
    }
    accumulator.finish()
}

/// Per-color cast-block share for one card: the deck had enough total
/// mana but missed this color's pips when the card was in hand.
#[derive(Debug, Clone)]
pub struct PipBlock {
    /// Card name.
    pub name: String,
    /// WUBRG color letter that was missing.
    pub color: ManaColor,
    /// Share of games with at least one pip-blocked cast of this card
    /// for this color.
    pub game_share: f64,
}

/// Nonland mana-source census for the findings' deck context.
fn ramp_source_count(deck: &SimDeck) -> usize {
    use super::model::Role;
    deck.library_cards()
        .filter(|c| matches!(c.role, Role::Rock | Role::Dork | Role::RampSpell))
        .count()
}

/// The deck's mana base against the target bands (research-derived:
/// EDHREC average decks n=46 across 11 commanders, 2026-09; Sam Black's
/// cEDH land-count guidance; Frank Karsten's 60-card land-count method).
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ManaBase {
    /// Land count (basics + nonbasic lands).
    pub lands: usize,
    /// Artifact mana sources.
    pub artifact_mana_sources: usize,
    /// Creature mana sources.
    pub creature_mana_sources: usize,
    /// Ramp spells (search lands, ritual-class accelerants).
    pub ramp_spells: usize,
    /// Total mana sources (lands + ramp).
    pub total_sources: usize,
    /// `[min, max]` land band for the deck's bracket (commander) or
    /// average-curve class (60-card formats).
    pub bracket_target_lands: [usize; 2],
    /// `[min, max]` ramp band for the deck's bracket; `[0, 0]` for
    /// 60-card formats (no separate ramp verdict).
    pub bracket_target_ramp: [usize; 2],
    /// The verdict sentence; "on target" when both counts sit in band.
    pub verdict: String,
    /// The bracket the bands came from (inferred from the Game Changer
    /// census when the command got no explicit `--bracket`); `null` for
    /// 60-card formats.
    #[serde(deserialize_with = "super::report_schema::deserialize_present_option")]
    pub bracket: Option<u8>,
    /// True when `bracket` was inferred from the Game Changer census
    /// rather than passed explicitly.
    pub bracket_inferred_from_game_changers: bool,
}

/// Land and ramp target bands by bracket (1-5). Lands-matter decks widen
/// the land band by 4.
fn bracket_bands(bracket: u8, lands_matter: bool) -> ([usize; 2], [usize; 2]) {
    let widen = if lands_matter { 4 } else { 0 };
    let (lands, ramp) = match bracket {
        5 => ([25usize, 31], [10usize, 16]),
        4 => ([32, 36], [9, 12]),
        3 => ([33, 38], [8, 11]),
        // Brackets 1-2 share the casual band.
        _ => ([34, 40], [7, 12]),
    };
    ([lands[0], lands[1] + widen], ramp)
}

/// True when the card draws or cantrips (velocity, not raw card count).
fn is_draw_or_cantrip(card: &super::model::SimCard) -> bool {
    card.spell_data.draws_on_cast > 0
        || card.spell_data.scry_on_cast > 0
        || card.spell_data.surveils_on_cast > 0
}

/// Land band for a 60-card deck from its average nonland mana value and
/// cheap-draw census (Karsten): count every four cheap draw or cantrip
/// spells as one land, capped at 2.
fn constructed_land_band(deck: &SimDeck) -> [usize; 2] {
    let nonland: Vec<f64> = deck
        .cards
        .iter()
        .filter(|c| c.role != Role::Land)
        .map(|c| c.cost.total() as f64)
        .collect();
    let avg_mv = if nonland.is_empty() {
        2.0
    } else {
        nonland.iter().sum::<f64>() / nonland.len() as f64
    };
    let cheap_draws = deck
        .cards
        .iter()
        .filter(|c| c.role != Role::Land && c.cost.total() <= 2 && is_draw_or_cantrip(c))
        .count();
    let credit = (cheap_draws / 4).min(2);
    let mut band = if avg_mv < 2.0 {
        [20usize, 22]
    } else if avg_mv < 3.0 {
        [22, 25]
    } else {
        [25, 28]
    };
    band[0] = band[0].saturating_sub(credit).max(16);
    band[1] = band[1].saturating_sub(credit).max(17);
    band
}

/// Build the mana-base verdict for a deck.
///
/// Commander-family decks compare against bracket bands; 60-card decks
/// compare against Karsten-style bands derived from the deck's average
/// mana value, with `bracket: null` and no ramp verdict. A bracket
/// outside 1-5 falls back to the casual band.
pub fn mana_base(
    deck: &SimDeck,
    bracket: u8,
    bracket_inferred_from_game_changers: bool,
) -> ManaBase {
    use super::model::Role;
    // Land/spell MDFCs count as partial land sources (Karsten's
    // weights, shared with the mana audit) — they are one card, not two.
    let mdfc_weight = |c: &super::model::SimCard| -> f64 {
        if c.is_mdfc_spell {
            super::model::mdfc_land_weight(c.mythic)
        } else {
            0.0
        }
    };
    let lands = deck
        .library_cards()
        .filter(|c| c.role == Role::Land)
        .count();
    let mdfc_lands = deck
        .library_cards()
        .filter(|c| c.is_mdfc_spell)
        .map(mdfc_weight)
        .sum::<f64>();
    let lands = (lands as f64 + mdfc_lands).round() as usize;
    let artifact_mana_sources = deck
        .library_cards()
        .filter(|c| c.role == Role::Rock)
        .count();
    let creature_mana_sources = deck
        .library_cards()
        .filter(|c| c.role == Role::Dork)
        .count();
    let ramp_spells = deck
        .library_cards()
        .filter(|c| c.role == Role::RampSpell)
        .count();
    let total_sources = lands + artifact_mana_sources + creature_mana_sources + ramp_spells;
    if deck.format == super::model::Format::Constructed {
        let land_band = constructed_land_band(deck);
        let verdict = if lands < land_band[0] {
            format!(
                "add {} land{} ({} < band {}-{})",
                (land_band[0] - lands).max(1),
                if land_band[0] - lands == 1 { "" } else { "s" },
                lands,
                land_band[0],
                land_band[1]
            )
        } else if lands > land_band[1] {
            format!(
                "trim {} land{} ({} > band {}-{})",
                (lands - land_band[1]).max(1),
                if lands - land_band[1] == 1 { "" } else { "s" },
                lands,
                land_band[0],
                land_band[1]
            )
        } else {
            "on target".to_string()
        };
        return ManaBase {
            lands,
            artifact_mana_sources,
            creature_mana_sources,
            ramp_spells,
            total_sources,
            bracket_target_lands: land_band,
            bracket_target_ramp: [0, 0],
            verdict,
            bracket: None,
            bracket_inferred_from_game_changers: false,
        };
    }
    // Lands-matter: the commander or any card name carries a landfall /
    // lands-matter engine signal. The parse marks extra-land-drop boards
    // (`extra_land_drops`); their presence widens the band.
    let lands_matter = deck.library_cards().any(|c| c.flags.extra_land_drops > 0)
        || deck.commanders.iter().any(|c| c.flags.extra_land_drops > 0);
    let bracket = bracket.clamp(1, 5);
    let (land_band, ramp_band) = bracket_bands(bracket, lands_matter);
    // The lands-matter label rides on any verdict, so the widened band is
    // still a real comparison (a 20-land landfall deck reads "add lands").
    let suffix = if lands_matter {
        " (lands-matter band)"
    } else {
        ""
    };
    let verdict = if lands < land_band[0] {
        format!(
            "add {} lands ({} < band {}-{}){}",
            (land_band[0] - lands).max(2),
            lands,
            land_band[0],
            land_band[1],
            suffix
        )
    } else if lands > land_band[1] {
        format!(
            "trim {} lands ({} > band {}-{}); add rocks if sources are low{}",
            (lands - land_band[1]).max(1),
            lands,
            land_band[0],
            land_band[1],
            suffix
        )
    } else if ramp_spells + artifact_mana_sources + creature_mana_sources < ramp_band[0] {
        format!(
            "add {} ramp ({} of band {}-{}){}",
            (ramp_band[0] - (artifact_mana_sources + creature_mana_sources + ramp_spells)).max(1),
            artifact_mana_sources + creature_mana_sources + ramp_spells,
            ramp_band[0],
            ramp_band[1],
            suffix
        )
    } else if lands_matter {
        format!("on target{suffix}")
    } else {
        "on target".to_string()
    };
    ManaBase {
        lands,
        artifact_mana_sources,
        creature_mana_sources,
        ramp_spells,
        total_sources,
        bracket_target_lands: land_band,
        bracket_inferred_from_game_changers,
        bracket_target_ramp: ramp_band,
        verdict,
        bracket: Some(bracket),
    }
}

/// One deck problem found by the simulation.
#[derive(Debug, Clone)]
pub struct Finding {
    /// Stable finding kind that names the measured deficit.
    pub kind: &'static str,
    /// Severity bucket from the affected-game share.
    pub severity: &'static str,
    /// Share of games affected, when game-count based.
    pub game_share: Option<f64>,
    /// Color letter for `color_screw` problems.
    pub color: Option<ManaColor>,
    /// Human explanation with the numbers.
    pub explanation: String,
    /// Category + magnitude suggestion (never specific cards).
    pub suggestion: String,
    /// Cards or counts behind the finding, filled by the detail pass.
    pub evidence: Vec<FindingEvidence>,
}

/// Re-export so callers of `Finding` can name the evidence type without
/// importing the detail module.
pub use super::findings_detail::FindingEvidence;

/// Severity bucket for an affected-game share.
fn severity(pct: f64) -> &'static str {
    if pct >= 20.0 {
        "high"
    } else if pct >= 10.0 {
        "medium"
    } else {
        "low"
    }
}

/// Dedicated and choice source counts for one WUBRG color.
///
/// Dedicated = lands whose tap yield serves exactly this color (fixed pips
/// or single-color choice); choice = multi-color pickers that can serve it.
/// Shape names how the majority of dedicated sources enter.
fn color_source_shape(deck: &SimDeck, color_index: usize) -> (usize, usize, &'static str) {
    use super::model::Role;
    let mut dedicated = 0usize;
    let mut choice = 0usize;
    let mut enters_tapped = 0usize;
    let commander_colors = deck.commander_color_identity();
    for card in deck.library_cards().filter(|c| c.role == Role::Land) {
        let Some(y) = &card.tap else { continue };
        let eligible: [bool; 5] = std::array::from_fn(|index| {
            y.choice[index] || y.commander_pips > 0 && commander_colors[index]
        });
        let serves = !y.opponent_any
            && (y.any_pips > 0 || eligible[color_index] || y.fixed[color_index] > 0);
        if !serves {
            continue;
        }
        let multi = y.any_pips > 0
            || eligible.iter().filter(|c| **c).count() > 1
            || y.fixed.iter().filter(|p| **p > 0).count() > 1;
        if multi {
            choice += 1;
        } else {
            dedicated += 1;
        }
        if !multi && card.enters_tapped {
            enters_tapped += 1;
        }
    }
    let shape = if dedicated > 0 && enters_tapped * 2 >= dedicated {
        "tapped"
    } else {
        "untapped"
    };
    (dedicated, choice, shape)
}

/// Find deck problems from the aggregated stats.
pub fn analyze_findings(stats: &SimStats, deck: &SimDeck) -> Vec<Finding> {
    if stats.finished_games == 0 {
        return Vec::new();
    }
    let commander = !deck.commanders.is_empty();
    let turns = stats.turns as usize;
    let mut findings = Vec::new();
    screw_flood_problems(stats, deck, turns, &mut findings);
    commander_late_problem(stats, deck, commander, turns, &mut findings);
    color_screw_problems(stats, deck, &mut findings);
    draw_unused_problems(stats, turns, &mut findings);
    dead_cards_problem(stats, deck, commander, &mut findings);
    access_problems(stats, commander, turns, &mut findings);
    // Cause-level detail: every problem gets its offender list (and, where
    // the data supports it, a cause-specific suggestion) from the same
    // aggregated stats — with the same dead-card threshold the finding
    // itself used.
    let dead_threshold = if commander { 0.60 } else { 0.55 };
    for finding in &mut findings {
        super::findings_detail::explain_with_threshold(finding, stats, deck, dead_threshold);
    }
    findings
}

/// Land-drop and land-access observations against a uniform-draw baseline.
fn screw_flood_problems(
    stats: &SimStats,
    deck: &SimDeck,
    turns: usize,
    findings: &mut Vec<Finding>,
) {
    if turns >= 4 && stats.screw_pct >= 0.20 {
        let sources = ramp_source_count(deck);
        let suggestion = format!(
            "review land access, colors, and ramp timing before changing land slots. The deck has {sources} classified nonland ramp sources; that count alone does not establish payment readiness"
        );
        findings.push(Finding {
            kind: "insufficient_land_drops",
            severity: severity(stats.screw_pct * 100.0),
            game_share: Some(stats.screw_pct),
            color: None,
            explanation: format!(
                "{:.1}% of games made at most 2 land drops by turn 4",
                stats.screw_pct * 100.0
            ),
            suggestion,
            evidence: Vec::new(),
        });
    }
    if turns >= 4 && stats.flood_pct > 0.0 {
        let lands_matter = deck.library_cards().any(|c| c.flags.extra_land_drops > 0)
            || deck.commanders.iter().any(|c| c.flags.extra_land_drops > 0);
        let detail = format!(
            "{:.1}% of games saw 6 or more lands by turn 4. The mean uniform-draw baseline over recorded sample sizes is {:.1}%; selection and search can change this comparison",
            stats.flood_pct * 100.0,
            stats.flood_expectation * 100.0
        );
        if stats.flood_pct > stats.flood_expectation + 0.10 {
            let suggestion = if lands_matter {
                "this deck uses extra land drops; this observation alone supports no land trim. Review land use and draw or search effects"
            } else {
                "review land use and draw or search effects before changing land slots; the uniform-draw baseline does not prove an excess land count"
            };
            findings.push(Finding {
                kind: "excess_lands_seen",
                severity: severity(stats.flood_pct * 100.0),
                game_share: Some(stats.flood_pct),
                color: None,
                explanation: detail,
                suggestion: suggestion.to_string(),
                evidence: Vec::new(),
            });
        }
    }
}

/// Commander-cast-too-late finding against the on-curve turn.
fn commander_late_problem(
    stats: &SimStats,
    deck: &SimDeck,
    commander: bool,
    turns: usize,
    findings: &mut Vec<Finding>,
) {
    if commander && turns >= 4 {
        let cmc_turn =
            (deck.commanders.first().map(|c| c.cost.total()).unwrap_or(0) as usize).max(1);
        if cmc_turn > turns || cmc_turn >= stats.commander_castable_by.len() {
            return;
        }
        let by_curve = stats.commander_castable_by[cmc_turn];
        if by_curve < 0.60 {
            findings.push(Finding {
                kind: "late_commander_cast",
                severity: severity((1.0 - by_curve) * 100.0),
                game_share: Some(1.0 - by_curve),
                color: None,
                explanation: format!(
                    "commander mana was ready by turn {cmc_turn} in only {:.1}% of games. This measures payment readiness, not actual casts",
                    by_curve * 100.0
                ),
                suggestion: "check the commander's color requirements and supported payment paths before changing ramp or the curve".to_string(),
                evidence: Vec::new(),
            });
        }
    }
}

/// Color screw findings: any color pip missed in 10%+ of games.
fn color_screw_problems(stats: &SimStats, deck: &SimDeck, findings: &mut Vec<Finding>) {
    use super::model::COLORS;
    /// True for the unlimited basic land names (the same exemption
    /// `deck update`'s singleton guard uses). Wastes and snow basics are
    /// limited-supply cards, so they stay tracked.
    fn is_basic_name(name: &str) -> bool {
        crate::collection::is_basic_name(name)
    }
    let basic_count = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::Land && is_basic_name(&c.name))
        .count();
    for (i, pct) in stats.color_screw.iter().enumerate() {
        if *pct >= 0.10 {
            let (few_sources, choice_sources, shape) = color_source_shape(deck, i);
            let suggestion = if basic_count <= 8 {
                "review sources that make the missing color; opponent-dependent generic credit cannot pay colored costs".to_string()
            } else if choice_sources > 0 {
                format!(
                    "review the balance of basic lands and choice sources that make {}",
                    COLORS[i].symbol(),
                )
            } else {
                format!(
                    "review sources that make {} and their entry timing",
                    COLORS[i].symbol(),
                )
            };
            findings.push(Finding {
                kind: "insufficient_color_mana",
                severity: severity(pct * 100.0),
                game_share: Some(*pct),
                color: Some(COLORS[i]),
                explanation: format!(
                    "{} pips blocked payment in {:.1}% of games ({} dedicated {} source{}, {} choice land{})",
                    COLORS[i].symbol(),
                    pct * 100.0,
                    few_sources,
                    shape,
                    if few_sources == 1 { "" } else { "s" },
                    choice_sources,
                    if choice_sources == 1 { "" } else { "s" }
                ),
                suggestion,
                evidence: Vec::new(),
            });
        }
    }
}

/// Draw-role access and unused-mana observations without an assumed cause.
fn draw_unused_problems(stats: &SimStats, turns: usize, findings: &mut Vec<Finding>) {
    let draw_turn = 6;
    if turns >= draw_turn && stats.starved_pct >= 0.25 {
        findings.push(Finding {
            kind: "limited_draw_access",
            severity: severity(stats.starved_pct * 100.0),
            game_share: Some(stats.starved_pct),
            color: None,
            explanation: format!(
                "{:.1}% of games recorded no draw-role card access by turn {draw_turn}. This does not measure an empty hand or cards gained",
                stats.starved_pct * 100.0
            ),
            suggestion: "check draw-card coverage and actual hand use before adding draw engines; this access measure alone does not establish a draw shortage".to_string(),
            evidence: Vec::new(),
        });
    }
    if turns >= 6 && stats.unused_mana.get(5).is_some_and(|mana| *mana >= 2.5) {
        findings.push(Finding {
            kind: "unused_mana",
            severity: severity((stats.unused_mana[5] / 2.5) * 100.0),
            game_share: None,
            color: None,
            explanation: format!(
                "after turn 6 main actions, {:.1} pool mana remained unspent on average among finished games that completed that phase. This does not count untapped sources or unspent Treasures",
                stats.unused_mana[5]
            ),
            suggestion: "check which spells were available, their payment requirements, and unsupported mechanics before changing the curve or draw count".to_string(),
            evidence: Vec::new(),
        });
    }
}

/// Dead-cards finding: names cast on time under the format threshold.
fn dead_cards_problem(
    stats: &SimStats,
    deck: &SimDeck,
    commander: bool,
    findings: &mut Vec<Finding>,
) {
    let threshold = if commander { 0.60 } else { 0.55 };
    // Board-discount cards (improvise, affinity) cast far earlier in real
    // games than the parse-time floor implies; exempt them from the
    // dead-cards finding. Their castability rows still show the curve.
    let discount_names: std::collections::HashSet<&str> = deck
        .cards
        .iter()
        .filter(|c| c.battlefield_discount)
        .map(|c| c.name.as_str())
        .collect();
    // Reactive spells (removal, fogs, protection) never fire in a
    // goldfish: their castability row measures the mana base, not the
    // card. Exempt them from the dead-cards finding; role_access judges
    // them instead.
    let reactive_names: std::collections::HashSet<&str> = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::Removal)
        .map(|c| c.name.as_str())
        .collect();
    let dead: Vec<&CardCast> = stats
        .card_castability
        .iter()
        .filter(|c| {
            c.pct_by_target < threshold
                && c.target_turn <= stats.turns
                && !discount_names.contains(c.name.as_str())
                && !reactive_names.contains(c.name.as_str())
        })
        .collect();
    // Distinct names only: a 4-of reports once, not four times.
    let dead_names: Vec<String> = dead
        .iter()
        .map(|c| c.name.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if dead_names.len() >= 3 {
        // Worst by average castability across each card's copies.
        let mut worst: Vec<(&String, f64)> = dead_names
            .iter()
            .map(|name| {
                let rows: Vec<f64> = dead
                    .iter()
                    .filter(|c| &c.name == name)
                    .map(|c| c.pct_by_target)
                    .collect();
                (name, rows.iter().sum::<f64>() / rows.len().max(1) as f64)
            })
            .collect();
        worst.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(b.0)));
        worst.truncate(3);
        let names: Vec<String> = worst.iter().map(|(n, _)| (*n).clone()).collect();
        findings.push(Finding {
            kind: "low_castability",
            severity: severity(dead_names.len() as f64 * 5.0),
            game_share: None,
            color: None,
            explanation: format!(
                "{} cards had mana ready by their target turn in fewer than {:.0}% of games. This does not measure hand access or actual casts. Lowest readiness: {}",
                dead_names.len(),
                threshold * 100.0,
                names.join(", ")
            ),
            suggestion: "review the listed costs, color blocks, and mechanic coverage before changing these cards or adding ramp".to_string(),
            evidence: Vec::new(),
        });
    }
}

/// Starved-category and interaction-readiness findings.
fn access_problems(stats: &SimStats, commander: bool, turns: usize, findings: &mut Vec<Finding>) {
    if turns >= 5 && stats.removal_count > 0 && stats.removal_access_5 < 0.40 {
        findings.push(Finding {
            kind: "low_removal_access",
            severity: severity((1.0 - stats.removal_access_5) * 100.0),
            game_share: Some(1.0 - stats.removal_access_5),
            color: None,
            explanation: format!(
                "you rarely see a removal spell: only {:.1}% of games had one by turn 5 ({} copies)",
                stats.removal_access_5 * 100.0,
                stats.removal_count
            ),
            suggestion: "review removal classification and card-access paths before changing the interaction count".to_string(),
            evidence: Vec::new(),
        });
    }
    if commander && turns >= 8 && stats.wincon_count > 0 && stats.wincon_access_8 < 0.40 {
        findings.push(Finding {
            kind: "low_win_condition_access",
            severity: severity((1.0 - stats.wincon_access_8) * 100.0),
            game_share: Some(1.0 - stats.wincon_access_8),
            color: None,
            explanation: format!(
                "{:.2}% of finished games had a card classified as a win condition reach hand by turn 8 ({} classified copies). This measures card access, not an available win",
                stats.wincon_access_8 * 100.0,
                stats.wincon_count
            ),
            suggestion: "review win-condition classification and card-access paths before changing win conditions or draw".to_string(),
            evidence: Vec::new(),
        });
    }
    // Interaction readiness: access is fine but the answer is rarely
    // affordable with spare mana. Capacity, not events.
    if turns >= 5
        && stats.interaction_instant_count > 0
        && stats
            .interaction_ready_by_turn
            .get(4)
            .is_some_and(|share| *share < 0.40)
        && stats.removal_access_5 >= 0.40
    {
        findings.push(Finding {
            kind: "limited_interaction_readiness",
            severity: severity((1.0 - stats.interaction_ready_by_turn[4]) * 100.0),
            game_share: Some(1.0 - stats.interaction_ready_by_turn[4]),
            color: None,
            explanation: format!(
                "instant-speed interaction was both in hand and payable by turn 5 in {:.1}% of games ({} copies). This share includes games without interaction access",
                stats.interaction_ready_by_turn[4] * 100.0,
                stats.interaction_instant_count
            ),
            suggestion: "compare card access with payment requirements; this readiness share alone does not separate missing cards from unavailable mana".to_string(),
            evidence: Vec::new(),
        });
    }
}

/// Checks that finding explanations name their actual measurement and scope.
#[cfg(test)]
#[path = "tests/findings_copy_tests.rs"]
mod findings_copy_tests;
