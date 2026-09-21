// `stm deck cuts`: guided cutting — rank the deck's incumbents by
// expendability, with `--for <role N>` pairing each cut to a fill.
//
// Ranking (pins first, then by score):
// 1. Pins: Game Changers over the bracket cap and format-illegal cards go
//    to the top regardless of score.
// 2. Sim castability: cards rarely castable on curve (dead weight), with
//    reactive removal and improvise/affinity cards exempt (the sim
//    exempts them from `dead_cards` for the same reason).
// 3. Curve: CMC outliers vs the deck's own median.
// 4. Price: expensive one-offs with no other fault, as a tiebreaker.
//
// Basics and the commander are never suggested (the sim's `mana_base`
// verdict owns land counts). `--for` discounts incumbents serving the
// deficit role from the cut list and pairs every cut with a fill
// candidate owned first.

use rusqlite::Connection;

use super::role::Role;
use super::store::load_deck;
use crate::db::CardRow;
use anyhow::Context;

/// One cut row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CutRow {
    /// Card name.
    pub name: String,
    /// Copies in the deck.
    pub qty: i64,
    /// Copies the paired `deck update --remove` should take (full qty for
    /// pins, a partial count for scored cuts).
    pub remove_qty: i64,
    /// Ranked cut reasons, most decisive first.
    pub reasons: Vec<CutReason>,
    /// Composite expendability score in `[0, 1]` (higher = safer to cut).
    pub score: f64,
    /// True when the card must go first regardless of score
    /// (format-illegal, or Game Changer over the bracket cap).
    pub pinned: bool,
    /// 1-based rank: pinned rows first, then by score descending.
    pub rank: usize,
    /// The fill pairing for `--for` cuts: the deficit role's top
    /// owned-first candidates.
    pub replace_with: Option<FillPair>,
}

/// One decisive reason.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CutReason {
    /// Kind ("castability", "game_changer", "illegal", "price",
    /// "curve").
    pub kind: &'static str,
    /// The human detail with numbers.
    pub detail: String,
}

/// The `--for` fill pairing.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FillPair {
    /// The deficit role name.
    pub role: String,
    /// Top candidate names (owned first), capped at 3.
    pub candidates: Vec<String>,
}

/// Knobs for a `deck cuts` run (the command takes 9 flags; one struct
/// keeps the entry point under the argument-count lint).
#[derive(Debug, Clone, Copy, Default)]
pub struct CutOptions<'a> {
    /// Maximum cut rows.
    pub count: usize,
    /// Pair cuts with fills for this role.
    pub for_role: Option<&'a str>,
    /// Game Changer cap pinning bracket.
    pub bracket: Option<u8>,
    /// Pinned format (legality and cut-quantity rules).
    pub format: Option<&'a str>,
    /// Emit JSON.
    pub json: bool,
}

/// Compute the ranked cut rows for a deck.
///
/// The core of `stm deck cuts`: resolve the deck, run the fast sim for
/// castability, score every incumbent, and pair fills for `--for`.
/// Returns `Ok(None)` when the deck has no resolvable cards (the caller
/// renders the error).
pub fn cut_rows(
    conn: &Connection,
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, CardRow>,
    role: Option<Role>,
    count: usize,
    bracket: Option<u8>,
    format: Option<&str>,
) -> anyhow::Result<Vec<CutRow>> {
    // The deck's format: the pinned flag, else commander when the deck is
    // commander-shaped. The legality pin and copy-count rules read it.
    let is_commander = match format {
        Some(f) => matches!(
            f.to_ascii_lowercase().as_str(),
            "commander" | "brawl" | "oathbreaker"
        ),
        None => super::legal::is_commander(deck, None),
    };
    // Castability: a fast sim on the deck (small runs; the ranking needs
    // signal, not precision). Turn count follows the format's default.
    let format_key = format.unwrap_or(if is_commander {
        "commander"
    } else {
        "constructed"
    });
    let sim_deck = super::simulator::deck::build_sim_deck(deck, cards_by_name, Some(format_key));
    let turns = sim_deck.rules.default_turns;
    let mut rng = rand::SeedableRng::seed_from_u64(42);
    let logs: Vec<_> = (0..2000)
        .map(|_| super::simulator::game::run_game(&sim_deck, &mut rng, turns))
        .collect();
    let stats = super::simulator::aggregate::aggregate(&logs, &sim_deck, turns);
    // Castability signal with the dead-cards exemptions applied: reactive
    // removal never fires in a goldfish and improvise/affinity cards cast
    // far earlier in real games, so both measure the mana base, not the
    // card. Their rows stay in `card_castability`; the ranking skips them.
    let exempt: std::collections::HashSet<&str> = sim_deck
        .cards
        .iter()
        .filter(|c| c.board_discount || c.role == super::simulator::model::Role::Removal)
        .map(|c| c.name.as_str())
        .collect();
    let castability: std::collections::HashMap<&str, f64> = stats
        .card_castability
        .iter()
        .filter(|c| !exempt.contains(c.name.as_str()))
        .map(|c| (c.name.as_str(), c.pct_by_target))
        .collect();

    // Game Changer census for bracket pinning (maindeck only; the
    // sideboard is a commander wishlist — see `game_changer_names` in
    // legal.rs).
    let gc_in_deck: Vec<&str> = deck
        .sections
        .iter()
        // Maindeck only: the sideboard is a commander wishlist — the same
        // rule `game_changer_names` in legal.rs applies.
        .filter(|(s, _)| !s.eq_ignore_ascii_case("SIDEBOARD"))
        .flat_map(|(_, e)| e.iter())
        .filter(|e| {
            cards_by_name
                .get(&e.name)
                .is_some_and(|c| c.game_changer == Some(true))
        })
        .map(|e| e.name.as_str())
        .collect();
    // Caps mirror `game_changer_limit` in legal.rs: none for brackets 4–5.
    let gc_cap = match bracket {
        Some(1 | 2) => 0usize,
        Some(3) => 3,
        _ => usize::MAX,
    };
    let gc_over_cap: std::collections::BTreeSet<&str> = if gc_in_deck.len() > gc_cap {
        gc_in_deck[gc_cap.min(gc_in_deck.len())..]
            .iter()
            .copied()
            .collect()
    } else {
        Default::default()
    };

    // The deck's aggregate curve center (filler outlier signal).
    let mut cmcs: Vec<f64> = deck
        .entries()
        .filter_map(|e| cards_by_name.get(&e.name).map(|c| c.cmc))
        .collect();
    cmcs.sort_by(f64::total_cmp);
    let median_cmc = cmcs.get(cmcs.len() / 2).copied().unwrap_or(2.0);

    // The commander is never suggested.
    let commander_names: std::collections::HashSet<&str> = deck
        .section_index("COMMANDER")
        .map(|i| deck.sections[i].1.iter().map(|e| e.name.as_str()).collect())
        .unwrap_or_default();

    // The deck's colors from its card faces (loop-invariant; off-color
    // land detection below reads it).
    let deck_colors = super::suggest::deck_color_letters(deck, cards_by_name);

    // Filler spell prices (one batched lookup).
    let names: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        deck.entries()
            .filter_map(|e| {
                if seen.insert(e.name.clone()) {
                    Some(e.name.clone())
                } else {
                    None
                }
            })
            .collect()
    };
    let prices = crate::prints::price_ranges(conn, &names)?;

    // Role census for `--for`: which incumbents already serve the deficit
    // role (they are discounted from cutting, and their count is context).
    let for_role_tagged = role.map(|r| role_census(deck, cards_by_name, r));

    // Illegal-for-format and over-cap Game Changer cards pin to the top
    // regardless of score: they must go before any discretionary cut.
    let mut rows: Vec<CutRow> = Vec::new();
    let mut seen: std::collections::HashSet<String> = Default::default();
    for entry in deck.entries() {
        let card = match cards_by_name.get(&entry.name) {
            Some(card) => card,
            None => continue,
        };
        if !seen.insert(entry.name.clone()) {
            continue;
        }
        if super::stats::is_basic_land(card) || commander_names.contains(entry.name.as_str()) {
            continue;
        }
        let mut reasons: Vec<CutReason> = Vec::new();
        let mut score = 0.0f64;
        let mut pinned = false;

        if gc_over_cap.contains(entry.name.as_str()) {
            reasons.push(CutReason {
                kind: "game_changer",
                detail: format!(
                    "Game Changer over the {} bracket allowance ({} in deck, cap {gc_cap})",
                    bracket.unwrap_or(3),
                    gc_in_deck.len()
                ),
            });
            pinned = true;
        }
        // Legality pin on the deck's actual format: a commander-banned
        // card is only a must-cut in a commander deck. With no pin and a
        // 60-card deck, the gate is "legal in some 60-card format" (the
        // same default `deck suggest` uses).
        let illegal = if is_commander {
            !crate::deck::suggest::card_is_commander_legal(card)
        } else {
            match format {
                Some(_) => !crate::deck::suggest::card_legal_in(card, Some(format_key)),
                None => !crate::deck::suggest::card_legal_in_any_60(card),
            }
        };
        if illegal {
            reasons.push(CutReason {
                kind: "illegal",
                detail: if is_commander {
                    "banned in commander".to_string()
                } else if format.is_some() {
                    format!("not legal in {format_key}")
                } else {
                    "not legal in any 60-card format".to_string()
                },
            });
            pinned = true;
        }
        // Off-color lands: a land producing nothing the deck can use (or
        // a partial fetch in a mono-color deck) is a basic's worse twin.
        if crate::deck::land_colors::land_is_off_color(card, &deck_colors) {
            reasons.push(CutReason {
                kind: "off_color_land",
                detail: "produces none of this deck's colors; a basic is strictly better"
                    .to_string(),
            });
            score += 0.5;
        } else if crate::deck::land_colors::land_fetches_off_color(card, &deck_colors) {
            reasons.push(CutReason {
                kind: "off_color_land",
                detail: "fetches colors this deck does not use; run a basic unless duals of the other color are present"
                    .to_string(),
            });
            score += 0.2;
        }
        if castability
            .get(entry.name.as_str())
            .is_some_and(|pct| *pct < 0.5)
            && let Some(pct) = castability.get(entry.name.as_str())
        {
            reasons.push(CutReason {
                kind: "castability",
                detail: format!("cast on curve in only {:.0}% of games", pct * 100.0),
            });
            score += (1.0 - pct) * 0.4;
        }
        // Curve outlier: CMC far above the deck's median with no cheap
        // interaction role.
        if card.cmc >= median_cmc + 3.0 {
            reasons.push(CutReason {
                kind: "curve",
                detail: format!("CMC {:.0} vs deck median {:.0}", card.cmc, median_cmc),
            });
            score += 0.15;
        }
        // Price: an unpriced one-off is neutral; a pricey one-off cuts
        // before a cheap one (same fit).
        if let Some(range) = prices
            .get(&entry.name)
            .and_then(|r| r.cheapest.as_ref())
            .and_then(|p| p.usd)
            && range > 10.0
            && reasons.is_empty()
        {
            reasons.push(CutReason {
                kind: "price",
                detail: format!("a {range:.2} USD one-off with no castability fault"),
            });
            score += 0.1;
        }
        // `--for`: incumbents serving the deficit role are discounted.
        if let Some(serving) = &for_role_tagged
            && serving.contains(&entry.name)
        {
            score -= 0.3;
        }
        if reasons.is_empty() {
            continue;
        }
        // Copy-count rule: pins always take the full qty; scored cuts take
        // 1 copy of a 1-2-of and half (rounded up) of a 3-4-of. Commander
        // decks are singleton, so qty 1 always.
        let remove_qty = if pinned || !is_commander {
            // 60-card non-commander: remove 1 of a 2-of, half of a 3-4-of.
            let qty = entry.quantity;
            if pinned {
                qty
            } else if qty <= 2 {
                1
            } else {
                (qty + 1) / 2
            }
        } else {
            entry.quantity
        };
        rows.push(CutRow {
            name: entry.name.clone(),
            qty: entry.quantity,
            remove_qty,
            reasons,
            score: score.clamp(0.0, 1.0),
            pinned,
            rank: 0,
            replace_with: None,
        });
    }
    // Hard pins first (score order inside each group), then scored rows.
    rows.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.score.total_cmp(&a.score))
            .then(a.name.cmp(&b.name))
    });
    rows.truncate(count);
    for (i, row) in rows.iter_mut().enumerate() {
        row.rank = i + 1;
    }

    // `--for` fills: top owned-first candidates for the deficit role, one
    // pairing per cut row (reuses the suggest scoring leg).
    if let Some(role) = role {
        let fills = top_fills(conn, deck, role).unwrap_or_default();
        for row in &mut rows {
            row.replace_with = Some(FillPair {
                role: format!("{role:?}"),
                candidates: fills.clone(),
            });
        }
    }
    Ok(rows)
}

/// Entry point for `stm deck cuts <name>`.
pub fn cuts(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    name: &str,
    options: &CutOptions<'_>,
) -> anyhow::Result<i32> {
    let CutOptions {
        count,
        for_role,
        bracket,
        format,
        json,
    } = *options;
    // Parse the requested role for `--for` discounting + fills (before
    // any deck work: an unknown role is a usage error even for an empty
    // deck).
    let role = match for_role {
        None => None,
        Some(text) => match Role::parse(text) {
            Some(r) => Some(r),
            None => {
                out.error(&format!("unknown role {text:?}"));
                out.hint("see 'stm deck suggest --role' for the known roles");
                return Ok(crate::cli::codes::USAGE);
            }
        },
    };

    let (_path, deck) = load_deck(paths, name)?;
    let cards_by_name = super::stats::lookup_names(conn, &deck);
    if cards_by_name.is_empty() {
        out.error("deck has no resolvable cards");
        out.hint("check card names: stm deck show <name>");
        return Ok(crate::cli::codes::NO_RESULTS);
    }

    let rows = cut_rows(conn, &deck, &cards_by_name, role, count, bracket, format)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(crate::cli::codes::OK);
    }

    let styles = out.styles();
    if rows.is_empty() {
        println!(
            "{}",
            styles.success("no expendable cards found (nothing scored)")
        );
        return Ok(crate::cli::codes::OK);
    }
    println!(
        "{} {}",
        styles.header("Cuts"),
        styles.dim(&format!(
            "ranked by expendability{}",
            for_role
                .map(|r| format!(" (fills: {r})"))
                .unwrap_or_default()
        ))
    );
    for row in &rows {
        let copies_note = if row.remove_qty == row.qty {
            format!("({}x)", row.qty)
        } else {
            format!("(cut {} of {} copies)", row.remove_qty, row.qty)
        };
        println!(
            "  {:>2}. {:<30} score {:>4.2}  {}{}",
            row.rank,
            styles.card_name(&row.name),
            row.score,
            styles.dim(&copies_note),
            if row.pinned {
                styles.dim("  pinned")
            } else {
                String::new()
            }
        );
        for reason in &row.reasons {
            println!(
                "      {}",
                styles.dim(&format!("{}: {}", reason.kind, reason.detail))
            );
        }
        if let Some(pair) = &row.replace_with {
            println!(
                "      {} {}",
                styles.dim("→ fills:"),
                pair.candidates.join(", ")
            );
        }
    }
    Ok(crate::cli::codes::OK)
}

/// Map the suggest-side role onto the sim's coarse role classes (the
/// census only needs the broad buckets: draw/removal/ramp/wincon/lock).
fn sim_role(role: Role) -> super::simulator::model::Role {
    use super::simulator::model::Role as Sim;
    match role {
        Role::Draw | Role::CardSelection | Role::Mill | Role::Discard => Sim::Draw,
        Role::Removal | Role::Counterspell | Role::BoardWipe | Role::Protection => Sim::Removal,
        Role::Ramp => Sim::RampSpell,
        Role::Wincon | Role::Combo | Role::Storm | Role::ExtraTurn => Sim::Wincon,
        Role::Stax => Sim::Lock,
        Role::Token => Sim::Other,
        _ => Sim::Other,
    }
}

/// Deck entries serving one role: re-parse each card through the sim's
/// role classifier (`parse_sim_card`) and keep the matching role.
fn role_census(
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, CardRow>,
    role: Role,
) -> Vec<String> {
    let want = sim_role(role);
    deck.entries()
        .filter_map(|e| cards_by_name.get(&e.name))
        .filter(|card| super::simulator::parse::parse_sim_card(card).role == want)
        .map(|card| card.name.clone())
        .collect()
}

/// Top fill candidates for a role: owned cards matching the role's tag
/// labels, cheapest first. A light version of `deck suggest --role`'s
/// scoring; the pairing is a hint, not a ranked purchase list.
fn top_fills(conn: &Connection, deck: &super::Deck, role: Role) -> anyhow::Result<Vec<String>> {
    // Token-level label matching (the same semantics as `deck suggest`):
    // the role word may appear anywhere in a label ("card draw", "draw
    // engine"), never as a substring of a longer word ("drawback").
    let tag_ids: Vec<String> = super::suggest::matched_tag_ids(conn, Some(role), None)?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    if tag_ids.is_empty() {
        return Ok(Vec::new());
    }
    let deck_names: Vec<String> = deck
        .entries()
        .map(|e| e.name.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let tag_placeholders = vec!["?"; tag_ids.len()].join(",");
    let deck_placeholders = vec!["?"; deck_names.len().max(1)].join(",");
    let sql = format!(
        "SELECT k.name, COALESCE(o.qty, 0) AS owned
         FROM cards k
         LEFT JOIN (SELECT name, SUM(quantity) AS qty FROM collection GROUP BY name) o
           ON o.name = k.name
         JOIN card_tags ct ON ct.oracle_id = k.oracle_id
         WHERE ct.tag_id IN ({tag_placeholders})
           AND k.name NOT IN ({deck_placeholders})
         GROUP BY k.name
         ORDER BY owned DESC, k.edhrec_rank ASC NULLS LAST
         LIMIT 3"
    );
    let mut stmt = conn.prepare(&sql).context("preparing fill query")?;
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = tag_ids
        .iter()
        .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    for name in deck_names {
        params.push(Box::new(name));
    }
    let rows = stmt.query_map(
        rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
        |row| row.get::<_, String>(0),
    )?;
    rows.collect::<Result<Vec<_>, _>>().context("reading fills")
}
#[cfg(test)]
#[path = "tests/cuts_tests.rs"]
mod cuts_tests;
