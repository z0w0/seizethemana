//! Karsten colored-source audit: static weighted-source census against
//! published heuristic floors. These are not cast-on-curve guarantees.
//! No simulation: pure math on the deck's
//! card census. Powers `stm deck mana` and the `colored_sources` block in
//! the simulate report. Source: Frank Karsten, "How Many Sources Do You
//! Need to Consistently Cast Your Spells? A 2022 Update".

/// WUBRG color letters.
pub const WUBRG: [char; 5] = ['W', 'U', 'B', 'R', 'G'];

/// Partial source credit per card class (Karsten 2022).
const DORK_CREDIT: f64 = 0.5;
const ROCK_CREDIT: f64 = 0.75;
const CANTRIP_CREDIT: f64 = 0.25;
/// Cap on total cantrip credit (effects, per Karsten's worked examples).
const CANTRIP_CAP: usize = 10;
/// MDFC land faces count as a partial source (Karsten).
const MDFC_SOURCE_CREDIT: f64 = 0.8;
/// Requirement floors per number of pips, commander (99-card deck).
/// Stored table, not extrapolated (Karsten 2022).
const COMMANDER_REQUIREMENTS: [f64; 4] = [0.0, 12.0, 17.0, 21.0];

/// Convert an integral, nonnegative card quantity to a platform count.
fn quantity_count(quantity: f64) -> usize {
    if quantity.is_finite() && quantity > 0.0 {
        quantity.trunc() as usize
    } else {
        0
    }
}

/// Requirement floors for a 60-card deck at 24–25 lands, indexed by
/// `(total pips, same-color pips)`: Karsten's main table. Entry
/// `(total, same)` = sources needed for a cost with `total` colored pips
/// of which `same` are of the required color.
const SIXTY_REQUIREMENTS: &[(u8, u8, u8, f64)] = &[
    // (generic pips, total colored pips, max same-color pips) → sources
    // needed. Karsten 2022, 24-25 land baseline. The generic count keys
    // the shape: {1}{C}{C} and {C}{C} carry the same colored pips but
    // very different requirements.
    (0, 5, 1, 9.0),
    (0, 4, 1, 9.0),
    (0, 3, 1, 10.0),
    (0, 2, 1, 12.0),
    (0, 1, 1, 13.0), // 1 pip of one color
    (0, 3, 3, 23.0), // {C}{C}{C}
    (0, 2, 2, 21.0), // {C}{C}
    (1, 2, 2, 18.0), // {1}{C}{C}
    (1, 1, 1, 13.0), // {1}{C}
    (2, 3, 3, 22.0), // {2}{C}{C}{C}
    (0, 4, 4, 24.0), // {C}{C}{C}{C}
    (2, 2, 2, 22.0), // {2}{C}{C}
    (1, 4, 4, 24.0), // {1}{C}{C}{C}{C}
];

/// Land-count correction: when the deck runs more/fewer lands than the
/// 24–25 baseline, shift the requirement per Karsten's 20/30-land
/// columns (approximated linearly: ±1 source per ±3 lands, clamped to
/// ±3). Commander decks get no correction (the baseline is wide).
fn land_count_correction(lands: usize, is_commander: bool) -> f64 {
    if is_commander {
        return 0.0;
    }
    if lands <= 22 {
        -1.0
    } else if lands >= 28 {
        2.0
    } else {
        0.0
    }
}

/// The deck's effective land count: full lands plus MDFC partials
/// (Karsten weights). Uses the same is-mythic rule as the sim.
#[must_use]
pub fn effective_lands(rows: &[(crate::db::CardRow, f64)]) -> f64 {
    rows.iter()
        .map(|(card, qty)| {
            if is_mdfc(card) {
                let weight = crate::deck::simulator::model::mdfc_land_weight(is_mythic(card));
                return weight * *qty;
            }
            if crate::deck::stats::is_land(card) {
                return *qty;
            }
            0.0
        })
        .sum()
}

/// True when the card is a modal double-faced card: one land face and
/// one nonland face.
fn is_mdfc(card: &crate::db::CardRow) -> bool {
    let mut has_land = false;
    let mut has_spell = false;
    for face in card.type_line.split(" // ") {
        if face.split('—').next().unwrap_or("").contains("Land") {
            has_land = true;
        } else {
            has_spell = true;
        }
    }
    has_land && has_spell
}

/// True when the card is mythic rare (MDFC mythic land weight rule).
fn is_mythic(card: &crate::db::CardRow) -> bool {
    card.rarity == "mythic"
}

/// Per-card class tags used to weight credits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CardClass {
    Land,
    Dork,
    Rock,
    /// Cheap land-search ramp spell (Farseek class). Karsten gives ramp
    /// spells no colored-source credit (the searched land enters the
    /// battlefield from the library, not a mana ability); the class
    /// exists to document that exclusion, so the census arm is empty.
    RampSpell,
    Cantrip,
    Other,
}

/// Choose the source-credit class used by the static census.
fn classify(card: &crate::db::CardRow) -> CardClass {
    if crate::deck::stats::is_land(card) {
        return CardClass::Land;
    }
    if taps_for_mana(card) {
        return mana_tap_class(card);
    }
    let text = card.oracle_text.to_lowercase();
    if card.cmc <= 2.0
        && (text.contains("draw a card") || text.contains("scry") || text.contains("surveil"))
    {
        return CardClass::Cantrip;
    }
    CardClass::Other
}

/// Oracle-text markers of a mana-tap ability.
const MANA_TAP_MARKERS: &[&str] = &["add {", "add one mana of any color", "mana of any color"];

/// True when the card's oracle text taps for mana.
fn taps_for_mana(card: &crate::db::CardRow) -> bool {
    let text = card.oracle_text.to_lowercase();
    MANA_TAP_MARKERS.iter().any(|m| text.contains(m))
}

/// Split the mana-tap class by card type: creature = dork, artifact or
/// enchantment = rock, cheap land-search spell = ramp spell.
fn mana_tap_class(card: &crate::db::CardRow) -> CardClass {
    let text = card.oracle_text.to_lowercase();
    if text.contains("search your library for a") && text.contains("land") && card.cmc <= 2.0 {
        return CardClass::RampSpell;
    }
    if card.type_line.contains("Creature") {
        return CardClass::Dork;
    }
    if card.type_line.contains("Artifact") || card.type_line.contains("Enchantment") {
        return CardClass::Rock;
    }
    CardClass::Other
}

/// Colors a nonland card's activated mana ability can add.
fn mana_ability_colors(
    card: &crate::db::CardRow,
    deck_colors: &str,
    commander_colors: &str,
) -> Vec<char> {
    let oracle = crate::deck::simulator::oracle_parser::parse_oracle_card(card);
    let mut colors = [false; 5];
    for ability in &oracle.abilities {
        let crate::deck::simulator::oracle_ast::OracleAbility::Activated(ability) = ability else {
            continue;
        };
        if !ability.is_mana_ability {
            continue;
        }
        for effect in &ability.effects {
            if let crate::deck::simulator::oracle_ast::OracleEffect::Mana(yield_) = effect {
                for (index, letter) in WUBRG.iter().enumerate() {
                    colors[index] |= !yield_.opponent_any
                        && (yield_.fixed[index] > 0
                            || yield_.choice[index]
                            || yield_.any_pips > 0 && deck_colors.contains(*letter)
                            || yield_.commander_pips > 0 && commander_colors.contains(*letter));
                }
            }
        }
    }
    WUBRG
        .into_iter()
        .zip(colors)
        .filter_map(|(letter, present)| present.then_some(letter))
        .collect()
}

/// Weighted source census per color for the deck.
#[derive(Debug, Clone, Default)]
pub struct SourceCensus {
    /// Total weighted sources per WUBRG color.
    pub sources: Vec<f64>,
    /// Land-face credit per color.
    pub lands: Vec<f64>,
    /// Dork credit per color.
    pub dorks: Vec<f64>,
    /// Rock credit per color.
    pub rocks: Vec<f64>,
    /// Cantrip credit per color.
    pub cantrips: Vec<f64>,
    /// Lands not assumed to enter untapped on turn one.
    pub tapland_count: usize,
    /// Turn-one sources with no assumed reveal, board, or fetch-mana payment.
    pub untapped_t1: Vec<f64>,
}

/// Add weighted source credit once for each listed color.
fn add(map: &mut [f64], colors: &[char], weight: f64) {
    for c in colors {
        if let Some(i) = WUBRG.iter().position(|w| w == c) {
            map[i] += weight;
        }
    }
}

/// Compute the census. `rows` pairs maindeck card rows with their copy
/// counts; `deck_colors` is the deck's WUBRG letter set. Without commander
/// context, this function does not credit commander-identity production.
#[must_use]
pub fn census(rows: &[(crate::db::CardRow, f64)], deck_colors: &str) -> SourceCensus {
    census_with_commander_colors(rows, deck_colors, "")
}

/// Keep ordinary deck colors separate from the known commander identity.
fn census_with_commander_colors(
    rows: &[(crate::db::CardRow, f64)],
    deck_colors: &str,
    commander_colors: &str,
) -> SourceCensus {
    let mut out = SourceCensus {
        sources: vec![0.0; 5],
        lands: vec![0.0; 5],
        dorks: vec![0.0; 5],
        rocks: vec![0.0; 5],
        cantrips: vec![0.0; 5],
        tapland_count: 0,
        untapped_t1: vec![0.0; 5],
    };
    let mut cantrip_effects = 0usize;
    for (card, qty) in rows {
        let class = classify(card);
        match class {
            CardClass::Land => {
                let colors = weighted_land_colors(card, deck_colors, commander_colors);
                if !land_untapped_on_turn_one(card) {
                    out.tapland_count += quantity_count(*qty);
                }
                add(
                    &mut out.untapped_t1,
                    &turn_one_colors(card, rows, deck_colors, commander_colors),
                    *qty,
                );
                // Land/spell MDFC: the land face counts as 0.8 source of
                // its color (Karsten's MDFC credit).
                let credit = if is_mdfc(card) {
                    MDFC_SOURCE_CREDIT
                } else {
                    1.0
                };
                for c in &colors {
                    add(&mut out.lands, &[*c], credit * qty);
                }
            }
            CardClass::Dork => {
                for c in mana_ability_colors(card, deck_colors, commander_colors) {
                    add(&mut out.dorks, &[c], DORK_CREDIT * qty);
                }
            }
            CardClass::Rock => {
                for c in mana_ability_colors(card, deck_colors, commander_colors) {
                    add(&mut out.rocks, &[c], ROCK_CREDIT * qty);
                }
            }
            CardClass::Cantrip => {
                if cantrip_effects >= CANTRIP_CAP {
                    continue;
                }
                let credit_qty = quantity_count(*qty).min(CANTRIP_CAP - cantrip_effects);
                cantrip_effects += credit_qty;
                // Flat 0.25 per cantrip effect to each deck color
                // (Karsten's approximation of the producing fraction).
                for letter in deck_colors.chars() {
                    add(
                        &mut out.cantrips,
                        &[letter],
                        CANTRIP_CREDIT
                            * f64::from(
                                u32::try_from(credit_qty).expect("cantrip credit is capped"),
                            ),
                    );
                }
            }
            _ => {}
        }
    }
    for i in 0..5 {
        out.sources[i] = out.lands[i] + out.dorks[i] + out.rocks[i] + out.cantrips[i];
    }
    out
}

/// Potential land colors from typed mana and fetch data, never unknown opponent colors.
fn weighted_land_colors(
    card: &crate::db::CardRow,
    deck_colors: &str,
    commander_colors: &str,
) -> Vec<char> {
    use crate::deck::simulator::model::BasicLandType;
    let mut colors = mana_ability_colors(card, deck_colors, commander_colors);
    let oracle = crate::deck::simulator::oracle_parser::parse_oracle_card(card);
    for kind in BasicLandType::ALL {
        let typed = card.type_line.split(" // ").any(|face| {
            face.split_once('—')
                .is_some_and(|(_, types)| types.split_whitespace().any(|word| word == kind.name()))
        });
        let fetched = oracle.land.fetch.as_ref().is_some_and(|fetch| {
            fetch.target_types.is_empty() || fetch.target_types.contains(&kind)
        });
        let letter = WUBRG[kind.index()];
        if (typed || fetched || oracle.land.produces_any_color && deck_colors.contains(letter))
            && !colors.contains(&letter)
        {
            colors.push(letter);
        }
    }
    colors
}

/// Entry costs and land-return effects that the bounded advice check cannot execute.
pub(crate) fn unsupported_land_entry(card: &crate::db::CardRow) -> bool {
    card.oracle_text
        .to_ascii_lowercase()
        .split(['.', '\n'])
        .any(|clause| {
            clause.contains("enter")
                && ["sacrifice", "discard", "exile", "return"]
                    .iter()
                    .any(|word| clause.contains(word))
        })
}

/// Entry that needs no other card on turn one; printed life payments are allowed.
pub(crate) fn land_untapped_on_turn_one(card: &crate::db::CardRow) -> bool {
    let entry = card
        .oracle_text
        .to_ascii_lowercase()
        .split(['.', '\n'])
        .filter(|clause| clause.contains("enters"))
        .collect::<Vec<_>>()
        .join(".");
    if unsupported_land_entry(card) {
        return false;
    }
    if !entry.contains("tapped") {
        return true;
    }
    let oracle = crate::deck::simulator::oracle_parser::parse_oracle_card(card);
    oracle.land.life_to_untap > 0
        || entry.contains("two or fewer other lands")
        || entry.contains("it's your first, second, or third turn")
}

/// A land creature needs printed haste to tap on the turn it enters (CR 302.6).
pub(crate) fn land_can_tap_on_entry(card: &crate::db::CardRow) -> bool {
    use crate::deck::simulator::oracle_ast::OracleKeywordName;
    let creature_land = card.type_line.split(" // ").any(|face| {
        let head = face.split('—').next().unwrap_or("");
        head.split_whitespace().any(|word| word == "Land")
            && head.split_whitespace().any(|word| word == "Creature")
    });
    !creature_land
        || crate::deck::simulator::oracle_parser::parse_oracle_card(card)
            .has_keyword(&OracleKeywordName::Haste)
}

/// Unconditional source-tap mana modes with no extra resource costs.
/// This conservative subset is shared by known-hand and turn-one checks.
pub(crate) fn simple_land_mana(
    card: &crate::db::CardRow,
) -> Vec<crate::deck::simulator::model::ManaYield> {
    use crate::deck::simulator::oracle_ast::{
        ActivationCost, ActivationTarget, OracleAbility, OracleEffect,
    };
    if unsupported_land_entry(card) {
        return Vec::new();
    }
    let oracle = crate::deck::simulator::oracle_parser::parse_oracle_card(card);
    let mut modes = Vec::new();
    for ability in &oracle.abilities {
        let OracleAbility::Activated(ability) = ability else {
            continue;
        };
        if !ability.is_mana_ability
            || !ability.restrictions.is_empty()
            || ability.effects.iter().any(|effect| {
                matches!(effect, OracleEffect::Unsupported(text)
                    if !text.to_ascii_lowercase().starts_with("spend this mana only to cast ")
                        || !ability.effects.iter().any(|effect| matches!(effect,
                            OracleEffect::Mana(yield_) if yield_.restriction.is_some())))
            })
            || !ability.costs.iter().all(|cost| {
                matches!(
                    cost,
                    ActivationCost::Tap(ActivationTarget::Source) | ActivationCost::PayLife(_)
                )
            })
        {
            continue;
        }
        for effect in &ability.effects {
            if let OracleEffect::Mana(yield_) = effect
                && !yield_.opponent_any
                && yield_.scaling.is_none()
                && !yield_.legendary_colors
            {
                modes.push(yield_.clone());
            }
        }
    }
    for (index, kind) in crate::deck::simulator::model::BasicLandType::ALL
        .into_iter()
        .enumerate()
    {
        if card
            .type_line
            .split('—')
            .nth(1)
            .is_some_and(|types| types.split_whitespace().any(|word| word == kind.name()))
        {
            let mut yield_ = crate::deck::simulator::model::ManaYield::default();
            yield_.fixed[index] = 1;
            modes.push(yield_);
        }
    }
    modes
}

/// Colors available from direct mana or an affordable fetch with a known target.
fn turn_one_colors(
    card: &crate::db::CardRow,
    rows: &[(crate::db::CardRow, f64)],
    deck_colors: &str,
    commander_colors: &str,
) -> Vec<char> {
    use crate::deck::simulator::oracle_ast::OracleAbility;
    if !land_untapped_on_turn_one(card) || !land_can_tap_on_entry(card) {
        return Vec::new();
    }
    let mut modes = simple_land_mana(card);
    let oracle = crate::deck::simulator::oracle_parser::parse_oracle_card(card);
    for ability in &oracle.abilities {
        let OracleAbility::Activated(ability) = ability else {
            continue;
        };
        let Some(spec) = free_fetch_search(ability) else {
            continue;
        };
        for (target, quantity) in rows {
            if *quantity > 0.0 && target.name != card.name && ready_fetch_target(target, spec) {
                modes.extend(simple_land_mana(target));
            }
        }
    }
    WUBRG
        .into_iter()
        .enumerate()
        .filter_map(|(index, letter)| {
            modes
                .iter()
                .any(|yield_| {
                    yield_.restriction.is_none()
                        && (yield_.fixed[index] > 0
                            || yield_.choice[index]
                            || yield_.any_pips > 0 && deck_colors.contains(letter)
                            || yield_.commander_pips > 0 && commander_colors.contains(letter))
                })
                .then_some(letter)
        })
        .collect()
}

/// Land search that can pay its own turn-one activation without another card.
fn free_fetch_search(
    ability: &crate::deck::simulator::oracle_ast::OracleActivatedAbility,
) -> Option<&crate::deck::simulator::model::SearchSpec> {
    use crate::deck::simulator::model::{SearchCardType, SearchDestination};
    use crate::deck::simulator::oracle_ast::{
        ActivationCost, ActivationTarget, CostObject, OracleEffect,
    };
    let sacrifices_source = ability.costs.iter().any(|cost| {
        matches!(
            cost,
            ActivationCost::Sacrifice {
                count: 1,
                object: CostObject::Source,
                exclude_source: false,
            }
        )
    });
    if !sacrifices_source
        || !ability.restrictions.is_empty()
        || !ability.costs.iter().all(|cost| {
            matches!(
                cost,
                ActivationCost::Tap(ActivationTarget::Source)
                    | ActivationCost::PayLife(_)
                    | ActivationCost::Sacrifice {
                        count: 1,
                        object: CostObject::Source,
                        exclude_source: false,
                    }
            )
        })
    {
        return None;
    }
    ability.effects.iter().find_map(|effect| match effect {
        OracleEffect::Search(spec)
            if spec.destination == SearchDestination::Battlefield
                && matches!(
                    spec.card_type,
                    Some(SearchCardType::Land | SearchCardType::BasicLand)
                )
                && spec.color.is_none()
                && !spec.colorless
                && spec.mana_value.is_none()
                && spec.max_mana_value.is_none()
                && spec.min_mana_value.is_none()
                && spec.top_count.is_none() =>
        {
            Some(spec)
        }
        _ => None,
    })
}

/// A land front face that satisfies the search and needs no assumed entry card.
fn ready_fetch_target(
    card: &crate::db::CardRow,
    spec: &crate::deck::simulator::model::SearchSpec,
) -> bool {
    let front_type = card.type_line.split(" // ").next().unwrap_or("");
    let (head, types) = front_type.split_once('—').unwrap_or((front_type, ""));
    head.split_whitespace().any(|word| word == "Land")
        && (!spec.basic_land_only || head.split_whitespace().any(|word| word == "Basic"))
        && land_untapped_on_turn_one(card)
        && land_can_tap_on_entry(card)
        && (spec.land_types.iter().all(|allowed| *allowed)
            || crate::deck::simulator::model::BasicLandType::ALL
                .into_iter()
                .enumerate()
                .any(|(index, kind)| {
                    spec.land_types[index]
                        && types.split_whitespace().any(|word| word == kind.name())
                }))
}

/// Pip counts per color for a mana cost string ("{2}{W}{W}").
fn cost_pips(cost: &str) -> Vec<(char, usize)> {
    let mut out = Vec::new();
    for letter in WUBRG {
        let mut n = 0;
        for pip in cost.split('{') {
            let pip = pip.trim_end_matches('}');
            if pip.len() == 1 && pip.starts_with(letter) {
                n += 1;
            }
            // Hybrid pips like {W/U}: counted as a full pip of each color
            // (conservative; each color needs the source).
            if pip.len() == 3 && pip.contains('/') {
                let parts: Vec<char> = pip.split('/').filter_map(|p| p.chars().next()).collect();
                if parts.contains(&letter) {
                    n += 1; // hybrid pip counts as a full pip of each color (conservative)
                }
            }
        }
        if n > 0 {
            out.push((letter, n));
        }
    }
    out
}

/// Generic pips, total colored pips, and the max same-color pips of a
/// cost — the three-way key into the 60-card table.
fn pip_shape(cost: &str) -> (u8, u8, u8) {
    let generic: u8 = cost
        .split(['{', '}'])
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .filter_map(|s| s.parse::<u8>().ok())
        .sum();
    let pips = cost_pips(cost);
    let total: usize = pips.iter().map(|(_, n)| n).sum();
    let same = pips.iter().map(|(_, n)| *n).max().unwrap_or(0);
    (
        generic,
        u8::try_from(total.min(255)).expect("pip count is capped"),
        u8::try_from(same.min(255)).expect("pip count is capped"),
    )
}

/// Requirement floor for one color of one card's cost. `pips_in_color`
/// is that color's pip count in the cost (a {W}{W}{U} cost needs the
/// 2-pip floor for W, the 1-pip floor for U).
fn requirement_for(cost: &str, lands: f64, is_commander: bool, pips: u8) -> f64 {
    let (generic, total, _) = pip_shape(cost);
    let mut base = if is_commander {
        COMMANDER_REQUIREMENTS[pips.min(3) as usize]
    } else {
        lookup_sixty(generic, total, pips)
    };
    // Gold cards: +1 per additional color requirement beyond the first.
    let colors_required = cost_pips(cost).len();
    if colors_required > 1 {
        base +=
            f64::from(u32::try_from(colors_required - 1).expect("color count is bounded by WUBRG"));
    }
    base + land_count_correction(quantity_count(lands), is_commander)
}

fn lookup_sixty(generic: u8, total: u8, same: u8) -> f64 {
    for (g, t, s_, need) in SIXTY_REQUIREMENTS {
        if *g == generic && *t == total && *s_ == same {
            return *need;
        }
    }
    // Unlisted shape: interpolate between the nearest colored-pip rows
    // (the pip-count column dominates; the generic count lowers the
    // floor one step per pip). Clamp to the deepest row.
    let same_floor = [14.0, 13.0, 21.0, 23.0, 24.0];
    same_floor[(same as usize).min(4)] - f64::from(generic.min(3))
}

/// One requirement line of the report.
#[derive(Debug, Clone)]
pub struct RequirementRow {
    /// Card name.
    pub name: String,
    /// Printed mana cost.
    pub mana_cost: String,
    /// Mana value.
    pub cmc: f64,
    /// Colored sources the card needs, by color.
    pub needs: Vec<(char, f64)>,
    /// Colored sources the deck has, by color.
    pub have: Vec<(char, f64)>,
    /// Shortfall per color.
    pub deficit: Vec<(char, f64)>,
    /// True when every color meets its requirement.
    pub ok: bool,
}

/// Full audit output for a deck.
#[derive(Debug, Clone)]
pub struct Audit {
    /// Effective source count per color.
    pub sources: Vec<f64>,
    /// Weighted source census.
    pub census: SourceCensus,
    /// Per-card requirement lines.
    pub requirements: Vec<RequirementRow>,
    /// Number of lands that enter tapped.
    pub tapland_count: usize,
    /// Untapped sources available on turn one, per color.
    pub untapped_t1: Vec<f64>,
    /// Whether the deck is commander-shaped.
    pub is_commander: bool,
    /// Effective land count.
    pub lands: f64,
}

/// Run the audit: census + per-card requirements + deficits.
#[must_use]
pub fn audit(rows: &[(crate::db::CardRow, f64)], deck_colors: &str, is_commander: bool) -> Audit {
    let lands = effective_lands(rows);
    let commander_colors = if is_commander { deck_colors } else { "" };
    let c = census_with_commander_colors(rows, deck_colors, commander_colors);
    let sources = c.sources.clone();
    let tapland_count = c.tapland_count;
    let untapped_t1 = c.untapped_t1.clone();
    let mut requirements = Vec::new();
    for (card, _) in rows {
        if crate::deck::stats::is_land(card) {
            continue;
        }
        let pips = cost_pips(&card.mana_cost);
        if pips.is_empty() {
            continue;
        }
        let mut needs = Vec::new();
        let mut have = Vec::new();
        let mut deficit = Vec::new();
        let mut ok = true;
        for (letter, n) in &pips {
            let need = requirement_for(
                &card.mana_cost,
                lands,
                is_commander,
                u8::try_from(*n).expect("mana pip count is bounded"),
            );
            let have_n = source_for(&c, *letter);
            let d = (need - have_n).max(0.0);
            if d > 0.0 {
                ok = false;
            }
            needs.push((*letter, need));
            have.push((*letter, have_n));
            deficit.push((*letter, d));
        }
        requirements.push(RequirementRow {
            name: card.name.clone(),
            mana_cost: card.mana_cost.clone(),
            cmc: card.cmc,
            needs,
            have,
            deficit,
            ok,
        });
    }
    requirements.sort_by(|a, b| {
        let sum = |r: &RequirementRow| r.deficit.iter().map(|d| d.1).sum::<f64>();
        sum(b)
            .partial_cmp(&sum(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Audit {
        sources,
        census: c,
        requirements,
        tapland_count,
        untapped_t1,
        is_commander,
        lands,
    }
}

fn source_for(c: &SourceCensus, letter: char) -> f64 {
    WUBRG
        .iter()
        .position(|w| *w == letter)
        .map_or(0.0, |i| c.sources[i])
}

/// The worst deficits, formatted for display ("name: need 16 W, have 14.0").
#[must_use]
pub fn worst_deficits(audit: &Audit, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    for row in &audit.requirements {
        for (letter, d) in &row.deficit {
            if *d > 0.0 {
                let need = row
                    .needs
                    .iter()
                    .find(|n| n.0 == *letter)
                    .map_or(0.0, |n| n.1);
                let have = row
                    .have
                    .iter()
                    .find(|h| h.0 == *letter)
                    .map_or(0.0, |h| h.1);
                out.push(format!(
                    "{}: need {} {}, have {:.1}",
                    row.name, need, letter, have
                ));
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out.truncate(limit);
    out
}
#[cfg(test)]
#[path = "tests/mana_audit_tests.rs"]
mod mana_audit_tests;

/// Typed colored-source audit shared by `deck mana` and simulator reports.
#[must_use]
pub fn colored_sources_report(
    a: &Audit,
) -> crate::deck::simulator::report_schema::ColoredSourcesReport {
    use crate::deck::simulator::report_schema::{ColorRequirement, ColorValues};
    let map = |v: &[f64]| ColorValues {
        white: v[0],
        blue: v[1],
        black: v[2],
        red: v[3],
        green: v[4],
    };
    let pair_values = |values: &[(char, f64)]| {
        let mut colors = [0.0; 5];
        for (color, value) in values {
            if let Some(index) = WUBRG.iter().position(|candidate| candidate == color) {
                colors[index] = *value;
            }
        }
        map(&colors)
    };
    let requirements = a
        .requirements
        .iter()
        .map(|row| ColorRequirement {
            name: row.name.clone(),
            mana_cost: row.mana_cost.clone(),
            mana_value: row.cmc,
            needs: pair_values(&row.needs),
            have: pair_values(&row.have),
            deficit: pair_values(&row.deficit),
            ok: row.ok,
        })
        .collect();
    crate::deck::simulator::report_schema::ColoredSourcesReport {
        format: if a.is_commander {
            "commander".to_string()
        } else {
            "constructed".to_string()
        },
        lands: a.lands,
        sources: map(&a.sources),
        credits: crate::deck::simulator::report_schema::SourceCredits {
            lands: map(&a.census.lands),
            creature_mana_sources: map(&a.census.dorks),
            artifact_mana_sources: map(&a.census.rocks),
            cantrips: map(&a.census.cantrips),
        },
        requirements,
        worst_deficits: worst_deficits(a, 3),
        tapland_count: a.tapland_count,
        untapped_t1_sources: map(&a.untapped_t1),
    }
}
