// Land color-relevance: what colors a land can add or fetch, derived from
// its name, type line, and oracle text. Powers `deck suggest` land ranking
// (zero-overlap lands out) and the `deck cuts` `off_color_land` reason.

/// The five basic land types, in WUBRG color order.
const BASIC_TYPES: &[(&str, char)] = &[
    ("Plains", 'W'),
    ("Island", 'U'),
    ("Swamp", 'B'),
    ("Mountain", 'R'),
    ("Forest", 'G'),
];

/// Fetchland names and the basic types they can fetch. The nine allied
/// and enemy fetches are fixed data (plus Wastes-class colorless).
const FETCH_TABLE: &[(&str, &str)] = &[
    ("Windswept Heath", "WG"),
    ("Flooded Strand", "WU"),
    ("Polluted Delta", "UB"),
    ("Bloodstained Mire", "BR"),
    ("Wooded Foothills", "RG"),
    ("Marsh Flats", "WB"),
    ("Scalding Tarn", "UR"),
    ("Verdant Catacombs", "BG"),
    ("Arid Mesa", "RW"),
    ("Misty Rainforest", "UG"),
    ("Prismatic Vista", "WUBRG"),
    ("Fabled Passage", "WUBRG"),
    ("Terramorphic Expanse", "WUBRG"),
    ("Evolving Wilds", "WUBRG"),
    ("Wastes", ""),
];

/// What colors a land can produce: the letters it can add or fetch.
/// `any` covers any-color sources (Command Tower, rainbow lands); those
/// count as producing every deck color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandColors {
    /// WUBRG letters the land can add or fetch.
    pub letters: String,
    /// True when the land's yield is any color (deck colors apply).
    pub any: bool,
}

/// What colors the land can add or fetch, from oracle text and the type
/// line. Fetchlands resolve by name table; typed lands from the type
/// line's basic subtypes and the tap text's mana pips; any-color lands
/// set `any` (the caller intersects with deck colors). A colorless
/// utility land returns an empty set.
pub fn land_producible_colors(card: &crate::db::CardRow, deck_colors: &str) -> LandColors {
    let name = card.name.trim();
    for (fetch, letters) in FETCH_TABLE {
        if name.eq_ignore_ascii_case(fetch.trim()) {
            return LandColors {
                letters: letters.to_string(),
                any: false,
            };
        }
    }
    let tap_text = card.oracle_text.to_lowercase();
    // Generic basic-fetch text ("search ... for a basic land card") names
    // no type, so the land can fetch a basic of every deck color
    // (Myriad Landscape, Escape Tunnel). Typed fetches ("basic Swamp
    // cards") name their types and fall through to the subtype scan;
    // Wastes never fetches, so it stays colorless.
    if tap_text.contains("basic land card") {
        return LandColors {
            letters: deck_colors.to_string(),
            any: true,
        };
    }
    let mut letters = String::new();
    let mut any = false;
    // Basic subtypes on the type line (typed duals, triomes, basics).
    for (subtype, letter) in BASIC_TYPES {
        if card.type_line.contains(subtype) && !letters.contains(*letter) {
            letters.push(*letter);
        }
    }
    // Fetch-style text ("search … for an Island or Forest card") unions
    // the named basic types.
    for (subtype, letter) in BASIC_TYPES {
        if card.oracle_text.contains(subtype) && !letters.contains(*letter) {
            letters.push(*letter);
        }
    }
    if tap_text.contains("add one mana of any color")
        || tap_text.contains("add {w} or {u} or {b} or {r} or {g}")
        || tap_text.contains("mana of any one color")
        || tap_text.contains("mana of any color")
    {
        any = true;
    }
    // "of any combination of colors" (Triomes, pathway faces) — every
    // printed color of the card joins.
    if tap_text.contains("any combination") {
        for ch in card_colors(card) {
            if !letters.contains(ch) {
                letters.push(ch);
            }
        }
        if letters.is_empty() {
            any = true;
        }
    }
    // Tap-yield pips and basic-type fetch text, per sentence: "{T}: Add
    // {U} or {B}." yields U and B; "search ... for an Island or Forest
    // card" unions the named basic types.
    for segment in card.oracle_text.split(['.', '\n']) {
        let lower = segment.to_lowercase();
        if !(lower.contains("add") || lower.contains("search")) {
            continue;
        }
        // Mana-symbol pips in an Add clause: "Add {U} or {B}" yields U
        // and B; "{T}: Add {R}." yields R.
        for letter in ['W', 'U', 'B', 'R', 'G'] {
            let pip = format!("{{{}}}", letter.to_lowercase());
            if lower.contains(&pip) && !letters.contains(letter) {
                letters.push(letter);
            }
        }
        // Spelled-out yields: "Add one mana of any color" is handled by
        // the `any` flag; "{C}" (colorless) adds nothing.
        for (subtype, letter) in BASIC_TYPES {
            if lower.contains(&subtype.to_lowercase()) && !letters.contains(*letter) {
                letters.push(*letter);
            }
        }
    }
    // Any-color lands yield the deck's colors by definition.
    if any {
        return LandColors {
            letters: deck_colors.to_string(),
            any: true,
        };
    }
    LandColors { letters, any }
}

/// WUBRG letters printed on the card (colors JSON array).
fn card_colors(card: &crate::db::CardRow) -> Vec<char> {
    serde_json::from_str::<Vec<String>>(&card.colors)
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c.chars().next())
        .collect()
}

/// True when the land shares zero colors with the deck: nothing it
/// produces or fetches is a deck color (any-color lands never qualify).
/// The strictly-worse-than-a-basic case.
pub fn land_is_off_color(card: &crate::db::CardRow, deck_colors: &str) -> bool {
    if !crate::deck::stats::is_land(card) || deck_colors.is_empty() {
        return false;
    }
    let produced = land_producible_colors(card, deck_colors);
    if produced.any {
        return false;
    }
    !produced.letters.chars().any(|c| deck_colors.contains(c))
}

/// True when the land's producible set extends past the deck's colors
/// (a partial fetch in a mono-color deck: strictly worse than a basic
/// unless duals of the extra color exist).
pub fn land_fetches_off_color(card: &crate::db::CardRow, deck_colors: &str) -> bool {
    if !crate::deck::stats::is_land(card) {
        return false;
    }
    let produced = land_producible_colors(card, deck_colors);
    !produced.any && produced.letters.chars().any(|c| !deck_colors.contains(c))
}

/// Color-relevance sort rank for suggest rows: 0 = shares deck colors or
/// any-color, 1 = partial fetch in a mono-color deck, 2 = zero overlap.
pub fn land_rank(card: &crate::db::CardRow, deck_colors: &str) -> u8 {
    if !crate::deck::stats::is_land(card) {
        return 0;
    }
    let produced = land_producible_colors(card, deck_colors);
    if produced.any || deck_colors.is_empty() {
        return 0;
    }
    let overlap = produced.letters.chars().any(|c| deck_colors.contains(c));
    if produced.letters.is_empty() {
        // Colorless utility lands are normal in 60-card decks.
        return 1;
    }
    if !overlap {
        2
    } else if produced.letters.chars().any(|c| !deck_colors.contains(c)) {
        1
    } else {
        0
    }
}

#[cfg(test)]
#[path = "tests/land_colors_tests.rs"]
mod land_colors_tests;
