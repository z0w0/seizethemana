// Problem findings from the aggregated simulation stats: the deck
// problems (`find_problems`), the mana-base verdict, and combo-pair
// assembly timing. Split from `aggregate` to keep files small.

use super::aggregate::{CardCast, SimStats};
use super::game::GameLog;
use super::model::{Role, SimDeck};

/// One combo pair's assembly timing: share of games where both pieces
/// were seen in hand by the target turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ComboAccess {
    /// Both piece names, "A + B".
    pub pair: String,
    /// Target turn: the later piece's cast-on-curve turn.
    pub target_turn: u32,
    /// Share of games with both pieces seen in hand by the target turn,
    /// serialized 0-100 (the JSON percent scale).
    #[serde(serialize_with = "crate::deck::simulator::report::serialize_pct")]
    pub pct_games: f64,
}

/// Compute combo pair access from game logs. Pieces the deck does not
/// hold report 0% so the gap stays visible.
pub fn piece_pair_access(
    logs: &[GameLog],
    deck: &SimDeck,
    pairs: &[(String, String)],
    turns: u32,
) -> Vec<ComboAccess> {
    let n = logs.len() as f64;
    if n == 0.0 {
        return Vec::new();
    }
    let find_idx = |name: &str| deck.cards.iter().position(|c| c.name == name);
    let mut out = Vec::new();
    for (a, b) in pairs {
        let (Some(ia), Some(ib)) = (find_idx(a), find_idx(b)) else {
            out.push(ComboAccess {
                pair: format!("{a} + {b}"),
                target_turn: 0,
                pct_games: 0.0,
            });
            continue;
        };
        let target = deck.cards[ia]
            .cost
            .total()
            .max(deck.cards[ib].cost.total())
            .max(1)
            .min(turns);
        let both = logs
            .iter()
            .filter(|log| {
                let seen_a = log.card_first_seen.get(&ia).is_some_and(|t| *t <= target);
                let seen_b = log.card_first_seen.get(&ib).is_some_and(|t| *t <= target);
                seen_a && seen_b
            })
            .count() as f64
            / n;
        out.push(ComboAccess {
            pair: format!("{a} + {b}"),
            target_turn: target,
            pct_games: both,
        });
    }
    out
}

/// Per-color cast-block share for one card: the deck had enough total
/// mana but missed this color's pips when the card was in hand.
#[derive(Debug, Clone)]
pub struct PipBlock {
    /// Card name.
    pub name: String,
    /// WUBRG color letter that was missing.
    pub color: char,
    /// Share of games with at least one pip-blocked cast of this card
    /// for this color.
    pub pct_games: f64,
}

/// Severity-scaled magnitude word for a share (the suggestion sizes to the
/// problem, not a fixed "2-3").
fn magnitude(pct: f64) -> &'static str {
    if pct >= 30.0 {
        "3-4"
    } else if pct >= 20.0 {
        "2-3"
    } else {
        "1-2"
    }
}

/// Nonland mana sources that join per turn: rocks, dorks, and ramp
/// spells. A rock-heavy deck's sources do not read as land-screwed.
fn ramp_source_count(deck: &SimDeck) -> usize {
    use super::model::Role;
    deck.cards
        .iter()
        .filter(|c| matches!(c.role, Role::Rock | Role::Dork | Role::RampSpell))
        .count()
}

/// The deck's mana base against the target bands (research-derived:
/// EDHREC average decks n=46 across 11 commanders, 2026-09; Sam Black's
/// cEDH land-count guidance; Frank Karsten's 60-card land-count method).
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ManaBase {
    /// Land count (basics + nonbasic lands).
    pub lands: usize,
    /// Rocks.
    pub rocks: usize,
    /// Creature mana sources.
    pub dorks: usize,
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
    pub bracket: Option<u8>,
    /// True when `bracket` was inferred from the Game Changer census
    /// rather than passed explicitly.
    pub bracket_inferred: bool,
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
    card.draws_on_cast > 0 || card.scry_on_cast > 0 || card.surveils
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
pub fn mana_base(deck: &SimDeck, bracket: u8, bracket_inferred: bool) -> ManaBase {
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
    let lands = deck.cards.iter().filter(|c| c.role == Role::Land).count();
    let mdfc_lands = deck
        .cards
        .iter()
        .filter(|c| c.is_mdfc_spell)
        .map(mdfc_weight)
        .sum::<f64>();
    let lands = (lands as f64 + mdfc_lands).round() as usize;
    let rocks = deck.cards.iter().filter(|c| c.role == Role::Rock).count();
    let dorks = deck.cards.iter().filter(|c| c.role == Role::Dork).count();
    let ramp_spells = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::RampSpell)
        .count();
    let total_sources = lands + rocks + dorks + ramp_spells;
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
            rocks,
            dorks,
            ramp_spells,
            total_sources,
            bracket_target_lands: land_band,
            bracket_target_ramp: [0, 0],
            verdict,
            bracket: None,
            bracket_inferred: false,
        };
    }
    // Lands-matter: the commander or any card name carries a landfall /
    // lands-matter engine signal. The parse marks extra-land-drop boards
    // (`extra_land_drops`); their presence widens the band.
    let lands_matter = deck.cards.iter().any(|c| c.extra_land_drops)
        || deck.commanders.iter().any(|c| c.extra_land_drops);
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
    } else if ramp_spells + rocks + dorks < ramp_band[0] {
        format!(
            "add {} ramp ({} of band {}-{}){}",
            (ramp_band[0] - (rocks + dorks + ramp_spells)).max(1),
            rocks + dorks + ramp_spells,
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
        rocks,
        dorks,
        ramp_spells,
        total_sources,
        bracket_target_lands: land_band,
        bracket_inferred,
        bracket_target_ramp: ramp_band,
        verdict,
        bracket: Some(bracket),
    }
}

/// One deck problem found by the simulation.
#[derive(Debug, Clone)]
pub struct Problem {
    /// Problem kind ("mana_flood", "color_screw", "draw_starvation", ...).
    pub kind: &'static str,
    /// Severity bucket from the affected-game share.
    pub severity: &'static str,
    /// Share of games affected, when game-count based.
    pub pct_games: Option<f64>,
    /// Color letter for `color_screw` problems.
    pub color: Option<char>,
    /// Human explanation with the numbers.
    pub detail: String,
    /// Category + magnitude suggestion (never specific cards).
    pub suggestion: String,
    /// Cards or counts behind the finding, filled by
    /// [`super::findings_detail::explain`] (empty until then).
    pub offenders: Vec<ProblemOffender>,
}

/// Re-export so callers of `Problem` can name the offender type without
/// importing the detail module.
pub use super::findings_detail::ProblemOffender;

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
    for card in deck.cards.iter().filter(|c| c.role == Role::Land) {
        let Some(y) = &card.tap else { continue };
        let serves = y.any_pips > 0
            || y.opponent_any
            || y.choice.iter().any(|c| *c)
            || y.fixed[color_index] > 0;
        if !serves {
            continue;
        }
        let multi = y.any_pips > 0
            || y.opponent_any
            || y.choice.iter().filter(|c| **c).count() > 1
            || y.fixed.iter().filter(|p| **p > 0).count() > 1;
        if multi {
            choice += 1;
        } else {
            dedicated += 1;
        }
        if card.enters_tapped {
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
pub fn find_problems(stats: &SimStats, deck: &SimDeck) -> Vec<Problem> {
    let commander = !deck.commanders.is_empty();
    let turns = stats.turns as usize;
    let mut problems = Vec::new();
    screw_flood_problems(stats, deck, turns, &mut problems);
    commander_late_problem(stats, deck, commander, turns, &mut problems);
    color_screw_problems(stats, deck, &mut problems);
    draw_unused_problems(stats, commander, turns, &mut problems);
    dead_cards_problem(stats, deck, commander, &mut problems);
    access_problems(stats, commander, turns, &mut problems);
    // Cause-level detail: every problem gets its offender list (and, where
    // the data supports it, a cause-specific suggestion) from the same
    // aggregated stats — with the same dead-card threshold the finding
    // itself used.
    let dead_threshold = if commander { 0.60 } else { 0.55 };
    for problem in &mut problems {
        super::findings_detail::explain_with_threshold(problem, stats, deck, dead_threshold);
    }
    problems
}

/// Mana screw and flood findings, against the draw-adjusted expectation.
fn screw_flood_problems(
    stats: &SimStats,
    deck: &SimDeck,
    turns: usize,
    problems: &mut Vec<Problem>,
) {
    if turns >= 4 && stats.screw_pct >= 0.20 {
        let constructed = deck.format == super::model::Format::Constructed;
        let sources = ramp_source_count(deck);
        // 60-card decks fix screw with land slots (no Sol Ring class); the
        // commander suggestion sizes rocks when the ramp census is thin.
        let suggestion = if constructed {
            format!(
                "add {} lands ({} nonland ramp sources already)",
                magnitude(stats.screw_pct * 100.0),
                sources
            )
        } else if sources < 6 {
            format!(
                "add {} two-mana rocks or land slots (only {sources} nonland ramp sources)",
                magnitude(stats.screw_pct * 100.0)
            )
        } else if sources >= 10 {
            // A rock-heavy deck screwing on color, not volume: more lands
            // will not help. The color_screw findings name the missing pips.
            "the deck has enough mana sources overall — fix the missing colors instead (the color findings above name the cards and the fix)".to_string()
        } else {
            format!(
                "add {} land slots ({} nonland ramp sources already)",
                magnitude(stats.screw_pct * 100.0),
                sources
            )
        };
        problems.push(Problem {
            kind: "mana_screw",
            severity: severity(stats.screw_pct * 100.0),
            pct_games: Some(stats.screw_pct * 100.0),
            color: None,
            detail: format!(
                "you run out of lands often: {:.1}% of games had 2 or fewer lands by turn 4",
                stats.screw_pct * 100.0
            ),
            suggestion,
            offenders: Vec::new(),
        });
    }
    // Flood fires when the rate sits well above the velocity-adjusted
    // expectation (the 11-card baseline is meaningless for cantrip
    // decks, and a rate at or under the expectation is no finding at
    // all). Lands-matter decks flood by design: their finding reads as
    // an observation, never a trim instruction.
    if turns >= 4 && stats.flood_pct > 0.0 {
        let lands_matter = deck.cards.iter().any(|c| c.extra_land_drops)
            || deck.commanders.iter().any(|c| c.extra_land_drops);
        let detail = format!(
            "too many lands: {:.1}% of games saw 6 or more lands by turn 4 (about {:.1}% is normal at this deck's draw rate)",
            stats.flood_pct * 100.0,
            stats.flood_expectation * 100.0
        );
        // At or under the expectation is not a finding: the deck draws
        // its share of lands, no trim implied.
        if stats.flood_pct > stats.flood_expectation + 0.10 {
            let lands_matter_note = if lands_matter {
                " (this deck wants lots of lands — check your deck's game plan before trimming)"
            } else {
                ""
            };
            problems.push(Problem {
                kind: "mana_flood",
                severity: severity(stats.flood_pct * 100.0),
                pct_games: Some(stats.flood_pct * 100.0),
                color: None,
                detail,
                suggestion: format!(
                    "trim {} land slots toward the curve{lands_matter_note}",
                    magnitude(stats.flood_pct * 100.0)
                ),
                offenders: Vec::new(),
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
    problems: &mut Vec<Problem>,
) {
    if commander && turns >= 4 {
        let cmc_turn =
            (deck.commanders.first().map(|c| c.cost.total()).unwrap_or(0) as usize).clamp(1, turns);
        let by_curve = stats.commander_castable_by[cmc_turn.min(12)];
        if by_curve < 0.60 {
            problems.push(Problem {
                kind: "commander_late",
                severity: severity((1.0 - by_curve) * 100.0),
                pct_games: Some((1.0 - by_curve) * 100.0),
                color: None,
                detail: format!(
                    "your commander comes down late: castable by turn {cmc_turn} in only {:.1}% of games",
                    by_curve * 100.0
                ),
                suggestion: "add 2-3 ramp sources or lower the early curve".to_string(),
                offenders: Vec::new(),
            });
        }
    }
}

/// Color screw findings: any color pip missed in 10%+ of games.
fn color_screw_problems(stats: &SimStats, deck: &SimDeck, problems: &mut Vec<Problem>) {
    use super::model::COLORS;
    /// True for the unlimited basic land names (the same exemption
    /// `deck update`'s singleton guard uses). Wastes and snow basics are
    /// limited-supply cards, so they stay tracked.
    fn is_basic_name(name: &str) -> bool {
        matches!(name, "Plains" | "Island" | "Swamp" | "Mountain" | "Forest")
    }
    let basic_count = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::Land && is_basic_name(&c.name))
        .count();
    for (i, pct) in stats.color_screw.iter().enumerate() {
        if *pct >= 0.10 {
            // Rank the fix by what the deck lacks. At few basics the
            // standard "swap basics" advice is wrong: the deck's fix is
            // any-color sources (rainbow lands, Fellwar-class rocks,
            // Prism-class banks).
            let (few_sources, choice_sources, shape) = color_source_shape(deck, i);
            let suggestion = if basic_count <= 8 {
                "add any-color sources (rainbow lands, Fellwar-class rocks, or banked-pip artifacts)".to_string()
            } else if choice_sources > 0 {
                format!(
                    "swap basics for lands that also tap for {} (choice sources exist but basics still dominate)",
                    COLORS[i],
                )
            } else {
                format!(
                    "add ~2-3 {} sources (dual lands that tap for {} beat more basics)",
                    COLORS[i], COLORS[i],
                )
            };
            problems.push(Problem {
                kind: "color_screw",
                severity: severity(pct * 100.0),
                pct_games: Some(pct * 100.0),
                color: Some(COLORS[i]),
                detail: format!(
                    "you have enough lands, but not the right colors: {} mana is missing in {:.1}% of games ({} dedicated {} source{}, {} choice land{})",
                    COLORS[i],
                    pct * 100.0,
                    few_sources,
                    shape,
                    if few_sources == 1 { "" } else { "s" },
                    choice_sources,
                    if choice_sources == 1 { "" } else { "s" }
                ),
                suggestion,
                offenders: Vec::new(),
            });
        }
    }
}

/// Draw starvation and unused-mana findings.
fn draw_unused_problems(
    stats: &SimStats,
    commander: bool,
    turns: usize,
    problems: &mut Vec<Problem>,
) {
    let draw_turn = if commander { 6 } else { 5 };
    if turns >= draw_turn && stats.starved_pct >= 0.25 {
        problems.push(Problem {
            kind: "draw_starvation",
            severity: severity(stats.starved_pct * 100.0),
            pct_games: Some(stats.starved_pct * 100.0),
            color: None,
            detail: format!(
                "you run out of cards to play: {:.1}% of games saw no draw source by turn {draw_turn}",
                stats.starved_pct * 100.0
            ),
            suggestion: "add 2-3 draw engines".to_string(),
            offenders: Vec::new(),
        });
    }
    if turns >= 6 && stats.unused_mana[5] >= 2.5 {
        problems.push(Problem {
            kind: "mana_unused",
            severity: severity((stats.unused_mana[5] / 2.5) * 100.0),
            pct_games: None,
            color: None,
            detail: format!(
                "you end turns with unused mana: {:.1} left over on average by turn 6",
                stats.unused_mana[5]
            ),
            suggestion: "add cheaper spells or more card draw to spend the mana".to_string(),
            offenders: Vec::new(),
        });
    }
}

/// Dead-cards finding: names cast on time under the format threshold.
fn dead_cards_problem(
    stats: &SimStats,
    deck: &SimDeck,
    commander: bool,
    problems: &mut Vec<Problem>,
) {
    let threshold = if commander { 0.60 } else { 0.55 };
    // Board-discount cards (improvise, affinity) cast far earlier in real
    // games than the parse-time floor implies; exempt them from the
    // dead-cards finding. Their castability rows still show the curve.
    let discount_names: std::collections::HashSet<&str> = deck
        .cards
        .iter()
        .filter(|c| c.board_discount)
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
        worst.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        worst.truncate(3);
        let names: Vec<String> = worst.iter().map(|(n, _)| (*n).clone()).collect();
        problems.push(Problem {
            kind: "dead_cards",
            severity: severity(dead_names.len() as f64 * 5.0),
            pct_games: None,
            color: None,
            detail: format!(
                "these cards sit in your hand too long: {} cards cast on time under {:.0}% of the time. Worst: {}",
                dead_names.len(),
                threshold * 100.0,
                names.join(", ")
            ),
            suggestion: "cut or discount late cards, or add ramp".to_string(),
            offenders: Vec::new(),
        });
    }
}

/// Starved-category and interaction-readiness findings.
fn access_problems(stats: &SimStats, commander: bool, turns: usize, problems: &mut Vec<Problem>) {
    if turns >= 5 && stats.removal_count > 0 && stats.removal_access_5 < 0.40 {
        problems.push(Problem {
            kind: "category_starved",
            severity: severity((1.0 - stats.removal_access_5) * 100.0),
            pct_games: Some((1.0 - stats.removal_access_5) * 100.0),
            color: None,
            detail: format!(
                "you rarely see a removal spell: only {:.1}% of games had one by turn 5 ({} copies)",
                stats.removal_access_5 * 100.0,
                stats.removal_count
            ),
            suggestion: "add 2-3 interaction pieces".to_string(),
            offenders: Vec::new(),
        });
    }
    if commander && turns >= 8 && stats.wincon_count > 0 && stats.wincon_access_8 < 0.40 {
        problems.push(Problem {
            kind: "category_starved",
            severity: severity((1.0 - stats.wincon_access_8) * 100.0),
            pct_games: Some((1.0 - stats.wincon_access_8) * 100.0),
            color: None,
            detail: format!(
                "you rarely see a way to win: only {:.1}% of games had a win condition in hand by turn 8 ({} copies)",
                stats.wincon_access_8 * 100.0,
                stats.wincon_count
            ),
            suggestion: "add 1-2 win conditions or more draw".to_string(),
            offenders: Vec::new(),
        });
    }
    // Interaction readiness: access is fine but the answer is rarely
    // affordable with spare mana. Capacity, not events.
    if turns >= 5
        && stats.interaction_instant_count > 0
        && stats.interaction_ready_by_turn[4] < 0.40
        && stats.removal_access_5 >= 0.40
    {
        problems.push(Problem {
            kind: "interaction_unready",
            severity: severity((1.0 - stats.interaction_ready_by_turn[4]) * 100.0),
            pct_games: Some((1.0 - stats.interaction_ready_by_turn[4]) * 100.0),
            color: None,
            detail: format!(
                "you hold answers but cannot afford to cast them when it matters: instant-speed answers were in hand with enough spare mana by turn 5 in only {:.1}% of games ({} copies)",
                stats.interaction_ready_by_turn[4] * 100.0,
                stats.interaction_instant_count
            ),
            suggestion: "add cheaper instant-speed answers".to_string(),
            offenders: Vec::new(),
        });
    }
}
