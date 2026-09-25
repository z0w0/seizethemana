// Universe and franchise mapping for sets and cards.
//
// Universes Beyond (UB) is Magic's crossover product family: sets printed
// under an outside IP (Marvel, Final Fantasy, Fallout, …). Two levels:
//
// - `universe`: is a card/print from beyond Magic's own multiverse?
//   Detected per print from Scryfall's `promo_types` "universesbeyond"
//   flag; a card is UB only when *all* of its prints are UB-flagged.
// - `franchise`: which outside IP a UB set belongs to ("Marvel",
//   "Final Fantasy", …). Only UB families get one; Secret Lair drops are
//   UB but have no single franchise.
//
// Franchises apply only to UB sets. In-universe sets never carry one. D&D
// is honorary UB: Wizards owns D&D, so Scryfall does not flag its prints,
// but players read AFR/CLB as crossover product.

use rusqlite::OptionalExtension;

use anyhow::Context;

/// Sets in a UB franchise, curated by set code (lowercase).
const FRANCHISE_CODES: &[(&str, &str)] = &[
    ("ltr", "Middle-earth"),
    ("ltc", "Middle-earth"),
    ("hob", "Middle-earth"),
    ("hoc", "Middle-earth"),
    ("msh", "Marvel"),
    ("msc", "Marvel"),
    ("spm", "Marvel"),
    ("spe", "Marvel"),
    ("mar", "Marvel"),
    ("lmar", "Marvel"),
    ("pspm", "Marvel"),
    ("afr", "Dungeons & Dragons"),
    ("afc", "Dungeons & Dragons"),
    ("clb", "Dungeons & Dragons"),
    ("fin", "Final Fantasy"),
    ("fic", "Final Fantasy"),
    ("fca", "Final Fantasy"),
    ("tla", "Avatar: The Last Airbender"),
    ("tle", "Avatar: The Last Airbender"),
    ("tmt", "Teenage Mutant Ninja Turtles"),
    ("tmc", "Teenage Mutant Ninja Turtles"),
    ("pza", "Teenage Mutant Ninja Turtles"),
    ("trk", "Star Trek"),
    ("trc", "Star Trek"),
    ("sds", "Star Trek"),
    ("who", "Doctor Who"),
    ("pip", "Fallout"),
    ("acr", "Assassin's Creed"),
    ("rex", "Jurassic World"),
    ("40k", "Warhammer 40,000"),
    ("bot", "Transformers"),
];

/// Secret Lair set codes: UB-flagged prints, but no single franchise. They
/// group by their set name ("Secret Lair Drop") in censuses.
const SECRET_LAIR_CODES: &[&str] = &["sld", "slc", "slu", "slp", "pssc"];

/// UB-flagged promo and organized-play set codes: crossover promos,
/// convention and WPN prints. No single franchise; each groups by its own
/// set name in censuses. Known so the sync does not warn.
const UB_PROMO_CODES: &[&str] = &[
    "pmei", // Media and Collaboration Promos
    "pf23", // MagicFest 2023
    "pf25", // MagicFest 2025
    "pf26", // MagicFest 2026
    "ph21", // 2021 Heroes of the Realm
    "ppro", // Pro Tour Promos
    "pspl", // Spotlight Series
    "pss5", // FIN Standard Showdown
    "purl", // URL/Convention Promos
    "pw23", // Wizards Play Network 2023
    "pw25", // Wizards Play Network 2025
    "pw26", // Wizards Play Network 2026
    "sch",  // Store Championships
    "clu",  // Ravnica: Clue Edition (Clue board-game crossover)
];

/// Name-keyword rules for future sets (checked in order; first hit wins).
const FRANCHISE_KEYWORDS: &[(&str, &str)] = &[
    ("Lord of the Rings", "Middle-earth"),
    ("Tales of Middle-earth", "Middle-earth"),
    ("The Hobbit", "Middle-earth"),
    ("Marvel", "Marvel"),
    ("Forgotten Realms", "Dungeons & Dragons"),
    ("Baldur's Gate", "Dungeons & Dragons"),
    ("Dungeons & Dragons", "Dungeons & Dragons"),
    ("Final Fantasy", "Final Fantasy"),
    ("Avatar: The Last Airbender", "Avatar: The Last Airbender"),
    ("Ninja Turtles", "Teenage Mutant Ninja Turtles"),
    ("Star Trek", "Star Trek"),
    ("Doctor Who", "Doctor Who"),
    ("Fallout", "Fallout"),
    ("Assassin's Creed", "Assassin's Creed"),
    ("Jurassic World", "Jurassic World"),
    ("Warhammer", "Warhammer 40,000"),
    ("Transformers", "Transformers"),
];

/// The franchise for a UB set, or `None` when the set has no single
/// franchise (Secret Lair) or is not UB.
///
/// Codes match case-insensitively (the store lowercases set codes; the
/// keyword rules accept any case). Unknown-UB-set detection lives in the
/// caller: `universe::franchise_for` returning `None` for a UB-flagged set
/// is the signal to warn.
pub fn franchise_for(set_code: &str, set_name: &str) -> Option<&'static str> {
    let code = set_code.to_ascii_lowercase();
    if let Some((_, franchise)) = FRANCHISE_CODES.iter().find(|(c, _)| *c == code) {
        return Some(franchise);
    }
    if SECRET_LAIR_CODES.contains(&code.as_str()) {
        return None;
    }
    FRANCHISE_KEYWORDS
        .iter()
        .find(|(kw, _)| set_name.contains(kw))
        .map(|(_, f)| *f)
}

/// True when the set is part of the Secret Lair family (no franchise).
pub fn is_secret_lair(set_code: &str) -> bool {
    SECRET_LAIR_CODES.contains(&set_code.to_ascii_lowercase().as_str())
}

/// True when the set is a known UB promo/organized-play set (no franchise,
/// but not a mapping gap).
pub fn is_ub_promo(set_code: &str) -> bool {
    UB_PROMO_CODES.contains(&set_code.to_ascii_lowercase().as_str())
}

/// True when a set is UB by our rules, given Scryfall's per-print flag.
///
/// D&D sets count as UB even though Scryfall leaves their prints unflagged.
pub fn is_universes_beyond(set_code: &str, print_flagged: bool) -> bool {
    print_flagged
        || matches!(
            set_code.to_ascii_lowercase().as_str(),
            "afr" | "afc" | "clb"
        )
}

/// True when the set carries the UB print flag but maps to no franchise
/// and is neither Secret Lair nor a known UB promo set — a curated-table
/// gap the sync should warn about.
pub fn unknown_ub_set(set_code: &str, set_name: &str, print_flagged: bool) -> bool {
    print_flagged
        && franchise_for(set_code, set_name).is_none()
        && !is_secret_lair(set_code)
        && !is_ub_promo(set_code)
}

/// Universe metadata for one card, resolved from the store.
///
/// `universe` is `"beyond"` only when every stored print of the card is
/// UB-flagged; any in-universe print (Universes Within, omenpath reprints)
/// makes the card multiverse.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CardUniverse {
    /// `"beyond"` or `"multiverse"`.
    pub universe: &'static str,
    /// Franchise of the card's representative print's set (UB only).
    pub franchise: Option<String>,
    /// Full set name of the representative print's set.
    pub set_name: Option<String>,
    /// Set type of that set ("expansion", "commander", …).
    pub set_type: Option<String>,
    /// In-universe block of that set, when one exists.
    pub block: Option<String>,
}

/// Resolve one card's universe metadata from its stored prints.
///
/// The card's representative print names the set for display; the UB
/// decision spans every print of the name.
///
/// # Errors
/// Propagates SQLite failures.
pub fn card_universe(
    conn: &rusqlite::Connection,
    name: &str,
    set_code: &str,
) -> anyhow::Result<CardUniverse> {
    let (total, ub): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(universes_beyond), 0)
             FROM card_prints WHERE name = ?1",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .with_context(|| format!("reading universe prints for {name}"))?;
    // A name with no stored prints reads as total = 0: multiverse.
    let universe = if total > 0 && ub == total {
        "beyond"
    } else {
        "multiverse"
    };
    let (set_name, set_type, block, set_franchise) = conn
        .query_row(
            "SELECT set_name, set_type, block, franchise FROM sets WHERE set_code = ?1",
            [set_code.to_ascii_lowercase()],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .with_context(|| format!("reading set metadata for {set_code}"))?
        .unwrap_or((None, None, None, None));
    // A franchise belongs to a universes-beyond card only: the
    // representative print may sit in an in-universe set while other
    // prints are UB, and pairing `universe: "multiverse"` with a
    // franchise reads as a contradiction.
    let franchise = if universe == "beyond" {
        set_franchise
    } else {
        None
    };
    Ok(CardUniverse {
        universe,
        franchise,
        set_name,
        set_type,
        block,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_curated_franchise_codes() {
        assert_eq!(franchise_for("msh", "Marvel Super Heroes"), Some("Marvel"));
        assert_eq!(
            franchise_for("ltr", "The Lord of the Rings"),
            Some("Middle-earth")
        );
        assert_eq!(
            franchise_for("hob", "The Hobbit: Unexpected Journey"),
            Some("Middle-earth")
        );
        assert_eq!(
            franchise_for("afr", "Adventures in the Forgotten Realms"),
            Some("Dungeons & Dragons")
        );
        assert_eq!(
            franchise_for("clb", "Commander Legends: Battle for Baldur's Gate"),
            Some("Dungeons & Dragons")
        );
        assert_eq!(
            franchise_for("40k", "Warhammer 40,000"),
            Some("Warhammer 40,000")
        );
    }

    #[test]
    fn maps_by_name_keywords() {
        assert_eq!(
            franchise_for("xyz", "Marvel Super Heroes: Second Age"),
            Some("Marvel")
        );
        assert_eq!(
            franchise_for("abc", "Jumpstart: Forgotten Realms"),
            Some("Dungeons & Dragons")
        );
    }

    #[test]
    fn secret_lair_has_no_franchise() {
        for code in ["sld", "slc", "slu", "slp", "pssc"] {
            assert_eq!(franchise_for(code, "Secret Lair Drop"), None);
            assert!(is_secret_lair(code));
        }
        assert!(!unknown_ub_set("sld", "Secret Lair Drop", true));
    }

    #[test]
    fn multiverse_sets_have_no_franchise() {
        assert_eq!(franchise_for("mh3", "Modern Horizons 3"), None);
        assert_eq!(franchise_for("blb", "Bloomburrow"), None);
        assert_eq!(franchise_for("mkm", "Murders at Karlov Manor"), None);
        assert!(!unknown_ub_set("mh3", "Modern Horizons 3", false));
    }

    #[test]
    fn dnd_is_honorary_ub() {
        assert!(is_universes_beyond("afr", false));
        assert!(is_universes_beyond("afc", false));
        assert!(is_universes_beyond("clb", false));
        assert!(!is_universes_beyond("mh3", false));
        assert!(is_universes_beyond("msh", true));
    }

    #[test]
    fn unknown_ub_sets_are_flagged_for_warning() {
        assert!(unknown_ub_set("zzz", "Some New Crossover", true));
        assert!(!unknown_ub_set("zzz", "Some New Crossover", false));
        assert!(!unknown_ub_set("msh", "Marvel Super Heroes", true));
    }

    #[test]
    fn card_universe_resolves_from_prints_and_sets() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        // Two prints of the same card: one UB (Marvel), one in-universe.
        conn.execute(
            "INSERT INTO sets (set_code, set_name, set_type, block, franchise)
             VALUES ('msh', 'Marvel Super Heroes', 'expansion', NULL, 'Marvel'),
                    ('mh3', 'Modern Horizons 3', 'draft_innovation', NULL, NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, universes_beyond, updated_at)
             VALUES ('p1', 'Test Card', 'msh', 1, 't'), ('p2', 'Test Card', 'mh3', 0, 't')",
            [],
        )
        .unwrap();

        let meta = card_universe(&conn, "Test Card", "msh").unwrap();
        assert_eq!(meta.universe, "multiverse", "any in-universe print wins");
        // A mixed-print card reports multiverse with no franchise: the
        // franchise belongs to the UB set, not the card.
        assert_eq!(meta.franchise.as_deref(), None);
        assert_eq!(meta.set_name.as_deref(), Some("Marvel Super Heroes"));
        assert_eq!(meta.set_type.as_deref(), Some("expansion"));

        // All-UB card: beyond, and the franchise follows.
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, universes_beyond, updated_at)
             VALUES ('p3', 'Iron Test', 'msh', 1, 't')",
            [],
        )
        .unwrap();
        let meta = card_universe(&conn, "Iron Test", "msh").unwrap();
        assert_eq!(meta.universe, "beyond");
        assert_eq!(meta.franchise.as_deref(), Some("Marvel"));

        // Unknown set / unpriced card degrades to defaults, not errors.
        let meta = card_universe(&conn, "Missing Card", "ghost").unwrap();
        assert_eq!(meta.universe, "multiverse");
        assert_eq!(meta.set_name, None);
    }

    #[test]
    fn card_universe_propagates_set_query_errors() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE card_prints (name TEXT, universes_beyond INTEGER);
             CREATE TABLE sets (set_code TEXT, set_name TEXT);",
        )
        .unwrap();
        conn.execute("INSERT INTO card_prints VALUES ('Bolt', 0)", [])
            .unwrap();
        let error = card_universe(&conn, "Bolt", "tst").unwrap_err();
        assert!(error.to_string().contains("reading set metadata"));
    }
}

#[test]
fn promo_ub_sets_do_not_warn() {
    // Organized-play and crossover promo sets carry the UB print flag but
    // no franchise: known, never a warning.
    for code in [
        "pspl", "pmei", "pf25", "pss5", "pw26", "clu", "pf23", "pw25", "purl", "pf26", "sch",
        "ppro", "ph21", "pw23",
    ] {
        assert!(!unknown_ub_set(code, "Any Name", true), "{code} warns");
    }
}
