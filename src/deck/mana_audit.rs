// Karsten colored-source audit: static weighted-source census against
// published requirement floors. No simulation — pure math on the deck's
// card census. Powers `stm deck mana` and the `colored_sources` block in
// the simulate report. Source: Frank Karsten, "How Many Sources Do You
// Need to Consistently Cast Your Spells? A 2022 Update".

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
/// MDFC land weight in the deck's land count (non-mythic / mythic).
/// Requirement floors per number of pips, commander (99-card deck).
/// Stored table, not extrapolated (Karsten 2022).
const COMMANDER_REQUIREMENTS: [f64; 4] = [0.0, 12.0, 17.0, 21.0];

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
fn mana_ability_colors(card: &crate::db::CardRow, deck_colors: &str) -> Vec<char> {
    let text = card.oracle_text.to_lowercase();
    let mut out = Vec::new();
    for letter in WUBRG {
        let l = letter.to_lowercase().to_string();
        if text.contains(&format!("add {{{l}}}")) || text.contains("add one mana of any color") {
            out.push(letter);
        }
    }
    if text.contains("mana of any color") || text.contains("add one mana of any color") {
        return deck_colors.chars().collect();
    }
    out
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
    /// Number of lands that enter tapped (tap lands).
    pub tapland_count: usize,
    /// Untapped turn-1 sources per color.
    pub untapped_t1: Vec<f64>,
}

fn add(map: &mut [f64], colors: &[char], weight: f64) {
    for c in colors {
        if let Some(i) = WUBRG.iter().position(|w| w == c) {
            map[i] += weight;
        }
    }
}

/// Compute the census. `rows` pairs maindeck card rows with their copy
/// counts; `deck_colors` is the deck's WUBRG letter set.
pub fn census(rows: &[(crate::db::CardRow, f64)], deck_colors: &str) -> SourceCensus {
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
                let produced = crate::deck::land_colors::land_producible_colors(card, deck_colors);
                let colors: Vec<char> = if produced.any {
                    deck_colors.chars().collect()
                } else {
                    produced.letters.chars().collect()
                };
                let enters_tapped = enters_tapped(card);
                if enters_tapped {
                    out.tapland_count += *qty as usize;
                }
                for c in &colors {
                    let weight = if enters_tapped { 0.0 } else { 1.0 };
                    add(&mut out.untapped_t1, &[*c], weight * qty);
                }
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
                for c in mana_ability_colors(card, deck_colors) {
                    add(&mut out.dorks, &[c], DORK_CREDIT * qty);
                }
            }
            CardClass::Rock => {
                for c in mana_ability_colors(card, deck_colors) {
                    add(&mut out.rocks, &[c], ROCK_CREDIT * qty);
                }
            }
            CardClass::Cantrip => {
                if cantrip_effects >= CANTRIP_CAP {
                    continue;
                }
                let credit_qty = (*qty as usize).min(CANTRIP_CAP - cantrip_effects);
                cantrip_effects += credit_qty;
                // Flat 0.25 per cantrip effect to each deck color
                // (Karsten's approximation of the producing fraction).
                for letter in deck_colors.chars() {
                    add(
                        &mut out.cantrips,
                        &[letter],
                        CANTRIP_CREDIT * credit_qty as f64,
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

/// True when the land enters tapped (from oracle text).
fn enters_tapped(card: &crate::db::CardRow) -> bool {
    let text = card.oracle_text.to_lowercase();
    text.contains("enters the battlefield tapped")
        || text.contains("enters tapped")
        || (text.contains("enters") && text.contains("tapped") && !text.contains("unless"))
            && !text.contains("pay 1 life")
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
            // Hybrid pips like {W/U} count half toward each color.
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
    (generic, total.min(255) as u8, same.min(255) as u8)
}

/// Requirement floor for one color of one card's cost.
fn requirement_for(cost: &str, lands: f64, is_commander: bool) -> f64 {
    let (generic, total, same) = pip_shape(cost);
    let mut base = if is_commander {
        COMMANDER_REQUIREMENTS[same.min(3) as usize]
    } else {
        lookup_sixty(generic, total, same)
    };
    // Gold cards: +1 per additional color requirement beyond the first.
    let colors_required = cost_pips(cost).len();
    if colors_required > 1 {
        base += (colors_required - 1) as f64;
    }
    base + land_count_correction(lands as usize, is_commander)
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
    pub name: String,
    pub mana_cost: String,
    pub cmc: f64,
    pub needs: Vec<(char, f64)>,
    pub have: Vec<(char, f64)>,
    pub deficit: Vec<(char, f64)>,
    pub ok: bool,
}

/// Full audit output for a deck.
#[derive(Debug, Clone)]
pub struct Audit {
    pub sources: Vec<f64>,
    pub census: SourceCensus,
    pub requirements: Vec<RequirementRow>,
    pub tapland_count: usize,
    pub untapped_t1: Vec<f64>,
    pub is_commander: bool,
    pub lands: f64,
}

/// Run the audit: census + per-card requirements + deficits.
pub fn audit(rows: &[(crate::db::CardRow, f64)], deck_colors: &str, is_commander: bool) -> Audit {
    let lands = effective_lands(rows);
    let c = census(rows, deck_colors);
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
        for (letter, _) in &pips {
            let need = requirement_for(&card.mana_cost, lands, is_commander);
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
        .map(|i| c.sources[i])
        .unwrap_or(0.0)
}

/// The worst deficits, formatted for display ("name: need 16 W, have 14.0").
pub fn worst_deficits(audit: &Audit, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    for row in &audit.requirements {
        for (letter, d) in &row.deficit {
            if *d > 0.0 {
                let need = row
                    .needs
                    .iter()
                    .find(|n| n.0 == *letter)
                    .map(|n| n.1)
                    .unwrap_or(0.0);
                let have = row
                    .have
                    .iter()
                    .find(|h| h.0 == *letter)
                    .map(|h| h.1)
                    .unwrap_or(0.0);
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

/// The `colored_sources` JSON block (shared by `deck mana` and the
/// simulate report).
pub fn colored_sources_json(a: &Audit) -> serde_json::Value {
    let letters = ['W', 'U', 'B', 'R', 'G'];
    let map = |v: &Vec<f64>| -> serde_json::Map<String, serde_json::Value> {
        let mut m = serde_json::Map::new();
        for (i, letter) in letters.iter().enumerate() {
            m.insert(letter.to_string(), serde_json::json!(v[i]));
        }
        m
    };
    let requirements: Vec<serde_json::Value> = a
        .requirements
        .iter()
        .map(|row| {
            serde_json::json!({
                "name": row.name,
                "mana_cost": row.mana_cost,
                "cmc": row.cmc,
                "needs": pairs_map(&row.needs),
                "have": pairs_map(&row.have),
                "deficit": pairs_map(&row.deficit),
                "ok": row.ok,
            })
        })
        .collect();
    serde_json::json!({
        "format": if a.is_commander { "commander" } else { "constructed" },
        "lands": a.lands,
        "sources": map(&a.sources),
        "credits": {
            "lands": map(&a.census.lands),
            "dorks": map(&a.census.dorks),
            "rocks": map(&a.census.rocks),
            "cantrips": map(&a.census.cantrips),
        },
        "requirements": requirements,
        "worst_deficits": worst_deficits(a, 3),
        "tapland_count": a.tapland_count,
        "untapped_t1_sources": map(&a.untapped_t1),
    })
}

/// A per-color `(letter, value)` list as a JSON map.
fn pairs_map(values: &[(char, f64)]) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    for (letter, v) in values {
        m.insert(letter.to_string(), serde_json::json!(v));
    }
    m
}
