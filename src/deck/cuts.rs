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
/// keeps the entry point under the argument-count lint). `Default` exists
/// for tests; the CLI path always sets every field, including `count`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CutOptions<'a> {
    /// Maximum cut rows (5, the CLI default).
    pub count: usize,
    /// Pair cuts with fills for this role.
    pub for_role: Option<&'a str>,
    /// Game Changer cap pinning bracket.
    pub bracket: Option<u8>,
    /// Pinned format (legality and cut-quantity rules).
    pub format: Option<&'a str>,
    /// Budget cap for `--for` fill candidates (USD; unpriced excluded).
    pub max_price: Option<f64>,
    /// Rank maindeck cuts for each card in this deck zone.
    pub make_room_for: Option<crate::cli::MakeRoomFor>,
    /// Emit JSON.
    pub json: bool,
}

/// One `--make-room-for` pairing: a bench card (sideboard or maybeboard)
/// and the maindeck cut that makes room for it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MakeRoomRow {
    /// The bench card wanting a slot.
    pub bench_card: String,
    /// The maindeck card to cut for it.
    pub cut_candidate: String,
    /// Why this pairing (role similarity, expendability).
    pub reasons: Vec<String>,
    /// Expendability score of the cut candidate in `[0, 1]`.
    pub score: f32,
}

/// Compute the ranked cut rows for a deck.
///
/// The core of `stm deck cuts`: resolve the deck, run the fast sim for
/// castability, score every incumbent, and pair fills for `--for`.
/// Returns an empty row list when the deck has no resolvable cards (the
/// caller renders the error).
#[allow(
    clippy::too_many_arguments,
    reason = "one flat argument per CLI flag; the entry-point CutOptions keeps the public surface lint-clean"
)]
pub fn cut_rows(
    conn: &Connection,
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, CardRow>,
    role: Option<Role>,
    count: usize,
    bracket: Option<u8>,
    format: Option<&str>,
    max_price: Option<f64>,
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
    // Seed 42 is load-bearing: the same deck must produce identical
    // castability scores across runs so cut rankings stay reproducible.
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
    // legal.rs). Distinct names: duplicate lines are one card for the cap.
    let gc_names: std::collections::BTreeSet<&str> = deck
        .sections
        .iter()
        // Maindeck only: the sideboard is a commander wishlist — the same
        // rule `game_changer_names` in legal.rs applies.
        .filter(|(s, _)| !super::grammar::is_bench_section(s))
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
    // Pins are the weakest over-cap cards: keep the most-played (best
    // EDHREC rank, low number), pin the rest. Unranked cards sort last
    // (least played), so popular Game Changers survive a cap.
    let mut gc_ranked: Vec<(&str, Option<i64>)> = gc_names
        .iter()
        .map(|n| (*n, cards_by_name.get(*n).and_then(|c| c.edhrec_rank)))
        .collect();
    gc_ranked.sort_unstable_by(|a, b| {
        // Unranked cards rank as i64::MAX (worst), so an over-cap pin
        // lands on the least-played Game Changer, not an unranked one
        // kept by Option ordering.
        let key = |r: &Option<i64>| r.unwrap_or(i64::MAX);
        key(&a.1).cmp(&key(&b.1)).then_with(|| a.0.cmp(b.0))
    });
    let gc_over_cap: std::collections::BTreeSet<&str> = if gc_ranked.len() > gc_cap {
        gc_ranked[gc_cap..].iter().map(|(n, _)| *n).collect()
    } else {
        Default::default()
    };

    // The deck's aggregate curve center (filler outlier signal).
    // Maindeck only: the bench is not part of the curve being measured.
    let mut cmcs: Vec<f64> = deck
        .sections
        .iter()
        .filter(|(s, _)| !super::grammar::is_bench_section(s))
        .flat_map(|(_, e)| e.iter())
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

    // Filler spell prices (one batched lookup). Maindeck names only: bench
    // cards are not cut candidates.
    let names: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        deck.sections
            .iter()
            .filter(|(s, _)| !super::grammar::is_bench_section(s))
            .flat_map(|(_, e)| e.iter())
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
    // Maindeck only: bench cards are not cut candidates (cutting them
    // frees no maindeck slot).
    for entry in deck
        .sections
        .iter()
        .filter(|(s, _)| !super::grammar::is_bench_section(s))
        .flat_map(|(_, e)| e.iter())
    {
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
                    gc_names.len()
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
        if let Some(pct) = castability
            .get(entry.name.as_str())
            .filter(|pct| **pct < 0.5)
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
        let fills = top_fills(
            conn,
            deck,
            cards_by_name,
            role,
            format_key,
            is_commander,
            max_price,
        );
        match fills {
            Ok(fills) => {
                for row in &mut rows {
                    row.replace_with = Some(FillPair {
                        role: format!("{role:?}"),
                        candidates: fills.clone(),
                    });
                }
            }
            Err(err) => {
                // The fill query failed; cuts still render, the pairing
                // column just reads empty. Say so, like `deck show` does
                // for price lookups.
                eprintln!("warning: fill lookup failed; fill pairings omitted: {err:#}");
            }
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
        max_price,
        make_room_for,
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
    let cards_by_name = super::stats::lookup_names(conn, &deck)?;
    if cards_by_name.is_empty() {
        out.error("deck has no resolvable cards");
        out.hint("check card names: stm deck show <name>");
        return Ok(crate::cli::codes::NO_RESULTS);
    }

    // `--make-room-for` runs its own pairing output and returns. The
    // `--for` role and `--max-price` cap only feed plain cuts' fill
    // pairing, which this path does not run; say so rather than ignore
    // the flags silently.
    if let Some(zone) = make_room_for {
        if for_role.is_some() || max_price.is_some() {
            out.warning(
                "--for and --max-price apply to plain cuts only; --make-room-for ignores them",
            );
        }
        let (rows, qualifying) =
            make_room_rows(conn, &deck, &cards_by_name, zone, count, bracket, format)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&rows)?);
            return Ok(crate::cli::codes::OK);
        }
        let styles = out.styles();
        let zone_name = match zone {
            crate::cli::MakeRoomFor::Sideboard => "sideboard",
            crate::cli::MakeRoomFor::Maybeboard => "maybeboard",
        };
        if rows.is_empty() {
            println!(
                "{}",
                styles.success(&format!(
                    "no {zone_name} cards to make room for (empty {zone_name})"
                ))
            );
            return Ok(crate::cli::codes::OK);
        }
        println!(
            "{} {}",
            styles.header("Make-room swaps"),
            styles.dim(&format!("{zone_name} card ← maindeck cut"))
        );
        for row in &rows {
            println!(
                "  {:<30} ← {:<30} score {:>4.2}",
                styles.card_name(&row.bench_card),
                styles.card_name(&row.cut_candidate),
                row.score
            );
            for reason in &row.reasons {
                println!("      {}", styles.dim(reason));
            }
        }
        // The pairing window is the cut-row cap; qualifying bench cards
        // past it go unpaired. Say so instead of dropping them silently.
        if qualifying > rows.len() {
            out.warning(&format!(
                "{} {zone_name} card(s) had no cut to pair with (the cut list holds {} rows)",
                qualifying - rows.len(),
                rows.len()
            ));
        }
        return Ok(crate::cli::codes::OK);
    }

    let rows = cut_rows(
        conn,
        &deck,
        &cards_by_name,
        role,
        count,
        bracket,
        format,
        max_price,
    )?;

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
    deck.sections
        .iter()
        .filter(|(s, _)| !super::grammar::is_bench_section(s))
        .flat_map(|(_, e)| e.iter())
        .filter_map(|e| cards_by_name.get(&e.name))
        .filter(|card| super::simulator::parse::parse_sim_card(card).role == want)
        .map(|card| card.name.clone())
        .collect()
}

/// Pair each bench card (sideboard or maybeboard) with the maindeck cut
/// that makes room for it.
///
/// A bench card qualifies when it fits the deck's color identity and is
/// legal in the deck's format (the same zone rules `deck legal` applies).
/// Each qualifying card pairs with the most expendable maindeck cut from
/// `cut_rows` that shares its sim role (a role swap keeps the deck's
/// shape); with no same-role candidate the top cut overall makes room.
/// `bracket` pins Game Changers the allowance cannot hold, same as plain
/// `deck cuts`.
///
/// Returns the paired rows plus the qualifying bench-card count; the
/// difference is the count of bench cards the cut window could not pair.
#[allow(clippy::too_many_arguments)]
fn make_room_rows(
    conn: &Connection,
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, CardRow>,
    zone: crate::cli::MakeRoomFor,
    count: usize,
    bracket: Option<u8>,
    format: Option<&str>,
) -> anyhow::Result<(Vec<MakeRoomRow>, usize)> {
    let zone_matches = |section: &str| -> bool {
        match zone {
            crate::cli::MakeRoomFor::Sideboard => super::grammar::is_sideboard_section(section),
            crate::cli::MakeRoomFor::Maybeboard => super::grammar::is_maybeboard_section(section),
        }
    };
    let is_commander = match format {
        Some(f) => matches!(
            f.to_ascii_lowercase().as_str(),
            "commander" | "brawl" | "oathbreaker"
        ),
        None => super::legal::is_commander(deck, None),
    };
    let identity = if is_commander {
        super::suggest::commander_identity(deck, cards_by_name)
    } else {
        String::new()
    };
    // Qualifying bench cards: identity + legality checked.
    let bench: Vec<&CardRow> = deck
        .sections
        .iter()
        .filter(|(s, _)| zone_matches(s))
        .flat_map(|(_, e)| e.iter())
        .filter_map(|entry| cards_by_name.get(&entry.name))
        .filter(|card| {
            (!is_commander || identity.is_empty() || super::suggest::identity_ok(card, &identity))
                && match format {
                    Some(f) => super::suggest::card_legal_in(card, Some(f)),
                    None if is_commander => super::suggest::card_is_commander_legal(card),
                    None => super::suggest::card_legal_in_any_60(card),
                }
        })
        .collect();
    if bench.is_empty() {
        return Ok((Vec::new(), 0));
    }
    // Expendability ranking of the maindeck (same scoring as plain cuts).
    let rows = cut_rows(
        conn,
        deck,
        cards_by_name,
        None,
        count.max(20),
        bracket,
        format,
        None,
    )?;
    let mut out = Vec::new();
    // Each cut candidate opens one slot, so a pick is consumed after use:
    // later bench cards fall through to the next-best same-role cut.
    let mut used: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for card in &bench {
        let sim_role = super::simulator::parse::parse_sim_card(card).role;
        // Prefer a same-role cut (the swap keeps the deck's role census).
        let pick = rows
            .iter()
            .enumerate()
            .find(|(i, row)| {
                !used.contains(i)
                    && cards_by_name.get(&row.name).is_some_and(|inc| {
                        super::simulator::parse::parse_sim_card(inc).role == sim_role
                    })
            })
            .or_else(|| rows.iter().enumerate().find(|(i, _)| !used.contains(i)))
            .map(|(i, _)| i);
        let Some(pick) = pick else {
            continue;
        };
        used.insert(pick);
        let pick = &rows[pick];
        let mut reasons = Vec::new();
        let same_role = cards_by_name
            .get(&pick.name)
            .is_some_and(|inc| super::simulator::parse::parse_sim_card(inc).role == sim_role);
        reasons.push(if same_role {
            format!(
                "same role as {}: the swap keeps the deck's shape",
                card.name
            )
        } else {
            format!("most expendable maindeck card for {}", card.name)
        });
        reasons.extend(
            pick.reasons
                .iter()
                .map(|r| format!("{}: {}", r.kind, r.detail)),
        );
        out.push(MakeRoomRow {
            bench_card: card.name.clone(),
            cut_candidate: pick.name.clone(),
            reasons,
            score: pick.score as f32,
        });
    }
    Ok((out, bench.len()))
}

/// Top fill candidates for a role: owned cards matching the role's tag
/// labels, most-owned first then best EDHREC rank. A light version of
/// `deck suggest --role`'s scoring; the pairing is a hint, not a ranked
/// purchase list. Candidates must fit the deck's color identity
/// (commander decks) and be legal in the deck's format. `max_price` caps
/// the final list: candidates are cut to three first, then unpriced and
/// over-cap names are dropped, so the result can be shorter than 3.
#[allow(clippy::too_many_arguments)]
fn top_fills(
    conn: &Connection,
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, CardRow>,
    role: Role,
    format_key: &str,
    is_commander: bool,
    max_price: Option<f64>,
) -> anyhow::Result<Vec<String>> {
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
    // Identity and legality gates mirror `deck suggest`: commander decks
    // fill only within the commander's color identity, and every deck
    // fills only with cards legal in its format.
    let identity = if is_commander {
        super::suggest::commander_identity(deck, cards_by_name)
    } else {
        String::new()
    };
    let deck_names: Vec<String> = deck
        .entries()
        .map(|e| e.name.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    // An empty deck excludes nothing: skip the NOT IN clause entirely
    // instead of binding zero placeholders to it.
    let deck_clause = if deck_names.is_empty() {
        String::new()
    } else {
        let deck_placeholders = vec!["?"; deck_names.len()].join(",");
        format!(" AND k.name NOT IN ({deck_placeholders})")
    };
    let tag_placeholders = vec!["?"; tag_ids.len()].join(",");
    // Identity and legality narrow the pool in Rust after the fetch, so
    // over-fetch before the cut (same depth rule as `deck suggest`: three
    // times the display limit, minimum 60 when capped, 30 otherwise). An
    // over-cap filter narrows too.
    let fetch = super::suggest::fusion_depth(3, max_price.is_some());
    let sql = format!(
        "SELECT k.name, COALESCE(o.qty, 0) AS owned,
                k.oracle_id, k.color_identity, k.legalities
          FROM cards k
          LEFT JOIN (SELECT name, SUM(quantity) AS qty FROM collection GROUP BY name) o
            ON o.name = k.name
          JOIN card_tags ct ON ct.oracle_id = k.oracle_id
          WHERE ct.tag_id IN ({tag_placeholders}){deck_clause}
          GROUP BY k.name
          ORDER BY owned DESC, k.edhrec_rank ASC NULLS LAST
          LIMIT {fetch}"
    );
    struct FillRow {
        name: String,
        identity: String,
        legalities: String,
    }
    let mut stmt = conn.prepare(&sql).context("preparing fill query")?;
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = tag_ids
        .iter()
        .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    for name in &deck_names {
        params.push(Box::new(name.clone()));
    }
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
            |row| {
                Ok(FillRow {
                    name: row.get(0)?,
                    identity: row.get(3)?,
                    legalities: row.get(4)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()
        .context("reading fills")?;
    // Identity + legality gates in Rust (the same checks as
    // `deck suggest`, applied before the cut so they never consume
    // candidate slots).
    let passes_identity = |row: &FillRow| -> bool {
        if is_commander && !identity.is_empty() {
            super::legal::identity_letters(&row.identity)
                .chars()
                .all(|c| identity.contains(c))
        } else {
            true
        }
    };
    let passes_legality = |row: &FillRow| -> bool {
        if format_key == "constructed" {
            let legalities = super::legal::legality_in_map(&row.legalities).unwrap_or_default();
            crate::deck::suggest::SIXTY_CARD_FORMATS.iter().any(|f| {
                legalities
                    .get(*f)
                    .is_some_and(|s| s == "legal" || s == "restricted")
            })
        } else {
            super::legal::legality_in(&row.legalities, format_key)
                .is_some_and(|state| state == "legal" || state == "restricted")
        }
    };
    let filtered: Vec<String> = rows
        .into_iter()
        .filter(|row| passes_identity(row) && passes_legality(row))
        .map(|row| row.name)
        .take(3)
        .collect();
    match max_price {
        None => Ok(filtered),
        Some(cap) => {
            let (kept, _) = crate::prints::retain_by_price::<_>(conn, filtered, |name| name, cap)?;
            Ok(kept)
        }
    }
}
#[cfg(test)]
#[path = "tests/cuts_tests.rs"]
mod cuts_tests;
