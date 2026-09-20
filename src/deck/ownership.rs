// Ownership accounting shared by `deck show`, `deck buylist`, and JSON
// callers: one definition of "this deck's available copies" so the
// ownership and missing numbers can never disagree.
//
// The rule (unchanged from before, now in one place): a deck slot is
// filled by copies assigned to this deck plus copies sitting in binders.
// Copies assigned to other decks never count (that would deconstruct
// those decks). Basic lands are unlimited and never tracked.

use rusqlite::Connection;

/// How a deck slot is covered by the collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// The slot count is met by copies assigned to this deck.
    Deck,
    /// The slot count is met only once binder copies fill in.
    Binder,
    /// Copies are missing at any price (or unowned).
    Missing,
}

/// Copies available to fill this deck's slots, keyed by card name:
/// assigned to the deck plus copies in binders. Other decks' copies do
/// not count.
///
/// # Errors
/// Propagates SQLite failures.
pub fn available_map(
    conn: &Connection,
    deck: &str,
) -> anyhow::Result<std::collections::HashMap<String, i64>> {
    let mut map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT c.name, SUM(c.quantity)
         FROM collection c JOIN cards k ON k.name = c.name
         WHERE (c.binder_type = 'deck' AND c.binder = ?1)
            OR c.binder_type = 'binder'
         GROUP BY c.name",
    )?;
    let rows = stmt.query_map([deck], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (name, qty) = row.map_err(|e| anyhow::anyhow!("reading available copies: {e}"))?;
        map.insert(name, qty);
    }
    Ok(map)
}

/// Copies assigned specifically to this deck, keyed by card name (no
/// binder copies mixed in).
///
/// # Errors
/// Propagates SQLite failures.
pub fn deck_assigned_map(
    conn: &Connection,
    deck: &str,
) -> anyhow::Result<std::collections::HashMap<String, i64>> {
    let mut map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT c.name, SUM(c.quantity)
         FROM collection c JOIN cards k ON k.name = c.name
         WHERE c.binder_type = 'deck' AND c.binder = ?1
         GROUP BY c.name",
    )?;
    let rows = stmt.query_map([deck], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (name, qty) = row.map_err(|e| anyhow::anyhow!("reading deck-assigned copies: {e}"))?;
        map.insert(name, qty);
    }
    Ok(map)
}

/// Ownership for every deck entry name.
///
/// `available` is the [`available_map`] and `assigned` the
/// [`deck_assigned_map`] for this deck. `held_elsewhere` counts copies
/// assigned to *other* decks (visible via the `held_elsewhere_map`); pass
/// it to slot_map so missing slots can explain why they are missing.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotOwnership {
    /// Copies assigned to this deck.
    pub in_deck: i64,
    /// Copies sitting in binders (may fill slots here).
    pub in_binder: i64,
    /// Copies assigned to other decks (never fill slots here).
    pub held_elsewhere: i64,
    /// Copies still needed at the cheapest printing price.
    pub missing: i64,
    /// The slot's coverage classification.
    pub coverage: Coverage,
}

/// Copies assigned to other decks (not this one), keyed by card name,
/// with the names of the holding decks.
///
/// These never fill slots here — they explain `held_elsewhere` missing
/// slots (the card is owned, but parked in another deck), and the human
/// To-buy line names the holder ("1 copy in Froggy").
///
/// # Errors
/// Propagates SQLite failures.
pub fn held_elsewhere_map(
    conn: &Connection,
    deck: &str,
) -> anyhow::Result<std::collections::HashMap<String, (i64, Vec<String>)>> {
    let mut map: std::collections::HashMap<String, (i64, Vec<String>)> =
        std::collections::HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT c.name, c.binder, SUM(c.quantity)
         FROM collection c JOIN cards k ON k.name = c.name
         WHERE c.binder_type = 'deck' AND c.binder != ?1
         GROUP BY c.name, c.binder",
    )?;
    let rows = stmt.query_map([deck], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (name, holder, qty) =
            row.map_err(|e| anyhow::anyhow!("reading other-deck copies: {e}"))?;
        let entry = map.entry(name).or_insert((0, Vec::new()));
        entry.0 += qty;
        entry.1.push(holder);
    }
    Ok(map)
}

/// Ownership for every deck entry name (non-basic only; basics are
/// unlimited).
///
/// `held_elsewhere` may be empty when the caller skips the extra query;
/// missing slots then report the plain `not_owned` reason.
pub fn slot_map(
    deck: &super::Deck,
    available: &std::collections::HashMap<String, i64>,
    assigned: &std::collections::HashMap<String, i64>,
    held_elsewhere: &std::collections::HashMap<String, (i64, Vec<String>)>,
    is_basic: impl Fn(&str) -> bool,
) -> std::collections::HashMap<String, SlotOwnership> {
    let mut needed: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for entry in deck.entries() {
        if is_basic(&entry.name) {
            continue;
        }
        *needed.entry(entry.name.clone()).or_insert(0) += entry.quantity;
    }
    let mut out = std::collections::HashMap::new();
    for (name, qty_needed) in needed {
        let in_deck = assigned.get(&name).copied().unwrap_or(0);
        let total = available.get(&name).copied().unwrap_or(0);
        let in_binder = total - in_deck;
        let held_elsewhere = held_elsewhere.get(&name).map(|(n, _)| *n).unwrap_or(0);
        let missing = (qty_needed - total).max(0);
        let coverage = if missing <= 0 {
            if in_deck >= qty_needed {
                Coverage::Deck
            } else {
                Coverage::Binder
            }
        } else {
            Coverage::Missing
        };
        out.insert(
            name.clone(),
            SlotOwnership {
                in_deck,
                in_binder,
                held_elsewhere,
                missing,
                coverage,
            },
        );
    }
    out
}
