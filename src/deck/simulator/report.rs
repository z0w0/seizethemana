// Report renders for the simulator: the JSON payload and the human
// stdout view. JSON adds station/bodies/engines metrics; the human view
// gains a station line when the commander is a spacecraft.

use super::aggregate::SimStats;
use super::model::{Role, SimDeck};
use crate::output::Output;

/// Total cards in the deck (library + commanders).
fn lock_count(deck: &SimDeck) -> i64 {
    deck.cards.iter().filter(|c| c.role == Role::Lock).count() as i64
}

fn booster_count(deck: &SimDeck) -> i64 {
    deck.cards
        .iter()
        .filter(|c| c.role == Role::Booster)
        .count() as i64
}

fn total_cards(deck: &SimDeck) -> usize {
    deck.cards.len() + deck.commanders.len()
}

/// Average nonland CMC across the library.
fn avg_cmc(deck: &SimDeck) -> f64 {
    let nonlands: Vec<&super::model::SimCard> =
        deck.cards.iter().filter(|c| c.role != Role::Land).collect();
    if nonlands.is_empty() {
        return 0.0;
    }
    nonlands.iter().map(|c| c.cost.total() as f64).sum::<f64>() / nonlands.len() as f64
}

/// Curve bucket label for a CMC ("0".."6", "7+").
fn curve_bucket(cmc: u32) -> &'static str {
    match cmc {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        _ => "7+",
    }
}

/// The model-limit list shipped with every report.
pub(super) fn assumptions(deck: &SimDeck) -> Vec<String> {
    let mut list = vec![
        "lands enter per their oracle text (enters-tapped honored, untapped when none)".to_string(),
        "no opponents, no counters or protection spells fire".to_string(),
        "draw engines fire once per turn on a fixed delay, not from full board state".to_string(),
        "opponent-dependent mana sources (Fellwar Stone) read as any-color from turn 2, nothing on turn 1".to_string(),
        "hybrid pips pay from any of their colors".to_string(),
        "body power uses the printed power when known, else flat 2 (tokens stay flat)".to_string(),
        "improvise/affinity/warp approximate to cost cuts; improvise/affinity discounts grow with the artifact count on the battlefield".to_string(),
        "X-cost spells pay the leftover mana pool as X (drain/draw/mill/tokens scale with it)".to_string(),
        "kicker pays from leftover mana when affordable; only drain amounts scale with the kick; multikicker pays once".to_string(),
        "additional costs (sacrifice a creature, pay N life) execute with the cast; only the first sacrifice shape is modeled".to_string(),
        "drain resolves x3 life in commander (three opponents), x1 in 60-card formats (combat-damage drains included)".to_string(),
        "token effects create their stated count of flat bodies (capped at 8); Treasure creators bank pips instead of bodies".to_string(),
        "mill fills the graveyard census and counts as cards seen; graveyard replay fires once per card, no recursion chains".to_string(),
        "wheels discard the hand into the graveyard census, then draw seven; loot is draw-N discard-N".to_string(),
        "spend-restricted mana (creature-only lands) pays creature casts only".to_string(),
        "sacrifice outlets (Ashnod's Altar class) consume real untapped bodies; with no body available they do not fire".to_string(),
        "blink effects re-fire the host's ETB triggers once, the turn after".to_string(),
        "planeswalkers fire one loyalty ability per turn; plus abilities gain loyalty, minus abilities spend it; ultimates only flag online".to_string(),
        "sagas advance one chapter per turn; parsed chapter abilities fire (combined numeral lines fill every chapter); the saga leaves the battlefield after its final chapter".to_string(),
        "per-cast mana engines (Vivi class) add their yield per spell cast every turn the host is on the battlefield; their own tap clause does not double count; 'for each' token counts cap at 8".to_string(),
        "zero-cost mana engines that out-produce their cost are capped and flagged (infinite_mana_pct); 'activate only once each turn' engines fire once per turn without flagging".to_string(),
        "equipment buffs only their equipped host after the equip cost is paid; aura buffs are not modeled".to_string(),
        "extra turns replay a land drop, a draw, and upkeep engines; no full-turn replay".to_string(),
        "energy, metalcraft, converge, proliferate, and replay mechanics (flashback, rebound) are not modeled".to_string(),
        "static type-grant abilities on other lands (The World Tree) are not modeled".to_string(),
        "seed baselines are version-local: an upgrade may reshuffle identically-seeded decks, so regenerate the baseline JSON after upgrading".to_string(),
    ];
    if deck.commanders.is_empty() {
        list.push("no commander zone; the whole deck is shuffled".to_string());
        list.push(
            "London mulligan: ship when the opener has fewer than 1 land, redraw and bottom one random card per mulligan (no keep choice)"
                .to_string(),
        );
    } else {
        list.push("commander starts in the command zone, no recast tax".to_string());
        list.push(
            "one free mulligan when the opener has <2 or >6 lands (commander family)".to_string(),
        );
        list.push(
            "unconditional commander draw triggers (upkeep, end step) fire once per turn from the turn after it is cast; attack-gated draws wait for animation and combat".to_string(),
        );
    }
    list.push(
        "combo assembly (Spellbook) measures how often pieces reach their zones; exile-zone pieces are excluded, library-zone pieces read as hand-seen".to_string(),
    );
    list
}

/// Human output for `--combo` pair access.
pub fn print_combo_access(out: &Output, rows: &[super::aggregate::ComboAccess]) {
    let s = out.styles();
    println!();
    println!("{}", s.header("Combo assembly (both pieces in hand)"));
    for row in rows {
        println!(
            "  {}  {}  {:.0}% of games",
            s.card_name(&row.pair),
            s.dim(&format!("by t{}", row.target_turn)),
            row.pct_games * 100.0
        );
    }
}

/// JSON payload for store-backed combo assembly, capped per direction.
pub fn combos_json(assembly: &super::combos::Assembly, limit: usize) -> serde_json::Value {
    serde_json::json!({
        "source": "commanderspellbook",
        "variants_considered": assembly.variants_considered,
        "complete": &assembly.complete[..assembly.complete.len().min(limit)],
        "near_misses": &assembly.near_misses[..assembly.near_misses.len().min(limit)],
    })
}

/// Human output for store-backed combos: complete combos with assembly
/// rates, then near-misses (one card away). Combo rows never create
/// problems: they are opportunities, not violations.
/// Spellbook features that win the game, for the win-path filter.
const WIN_FEATURES: [&str; 6] = [
    "Win the game",
    "Infinite damage",
    "Infinite turns",
    "Infinite mana",
    "Infinite card draw",
    "Infinite storm",
];

/// True when the combo produces a win feature.
fn is_win_path(produces: &[String]) -> bool {
    produces
        .iter()
        .any(|p| WIN_FEATURES.iter().any(|f| p.contains(f)))
}

/// Win-path rows: complete combos that produce a win feature.
pub fn win_paths_json(assembly: &super::combos::Assembly, limit: usize) -> serde_json::Value {
    let paths: Vec<&super::combos::ComboAccess> = assembly
        .complete
        .iter()
        .filter(|r| is_win_path(&r.produces))
        .take(limit)
        .collect();
    serde_json::json!({
        "count": paths.len(),
        "paths": paths,
    })
}

/// Human win-path block: compact, only when win paths exist.
pub fn print_win_paths(out: &Output, assembly: &super::combos::Assembly, limit: usize) {
    let s = out.styles();
    let paths: Vec<&super::combos::ComboAccess> = assembly
        .complete
        .iter()
        .filter(|r| is_win_path(&r.produces))
        .take(limit)
        .collect();
    if paths.is_empty() {
        return;
    }
    println!();
    println!("{}", s.header("Win paths"));
    for row in paths {
        let feature = row
            .produces
            .iter()
            .find(|p| is_win_path(std::slice::from_ref(p)))
            .map(String::as_str)
            .unwrap_or("");
        println!(
            "  {}  {}  {:.0}% of games",
            s.card_name(&row.combo),
            s.dim(&format!("{feature} by t{}", row.target_turn)),
            row.pct_games * 100.0
        );
    }
}

pub fn print_store_combos(out: &Output, assembly: &super::combos::Assembly, limit: usize) {
    let s = out.styles();
    println!();
    println!(
        "{} {}",
        s.header("Combo assembly (Spellbook)"),
        s.dim(&format!(
            "{} variants in the deck",
            assembly.variants_considered
        ))
    );
    for row in assembly.complete.iter().take(limit) {
        let tags = row
            .produces
            .first()
            .map(|p| format!(" → {p}"))
            .unwrap_or_default();
        let bracket = row
            .bracket_tag
            .as_deref()
            .map(|b| format!(" [{b}]"))
            .unwrap_or_default();
        println!(
            "  {}  {}  {:.0}% of games{}{}",
            s.card_name(&row.combo),
            s.dim(&format!("by t{}", row.target_turn)),
            row.pct_games * 100.0,
            s.dim(&tags),
            s.dim(&bracket)
        );
    }
    let misses: Vec<&super::combos::ComboAccess> =
        assembly.near_misses.iter().take(limit).collect();
    if !misses.is_empty() {
        println!("{}", s.header("One card away"));
        for row in misses {
            println!(
                "  {}  {}  {}{}",
                s.card_name(row.missing.first().map(String::as_str).unwrap_or("?")),
                s.dim(&format!("completes {}", row.combo)),
                s.dim(&format!("by t{}", row.target_turn)),
                row.bracket_tag
                    .as_deref()
                    .map(|b| s.dim(&format!(" [{b}]")))
                    .unwrap_or_default()
            );
        }
    }
}

// Exact-probability ceilings (`--hypgeo`): print the top gaps between the
// Monte Carlo castability and the hypergeometric ceiling, so a mana-base
// problem separates from a draw problem.
pub fn print_hypgeo(out: &Output, payload: &serde_json::Value) {
    let s = out.styles();
    let Some(cards) = payload.get("cards").and_then(|c| c.as_array()) else {
        return;
    };
    println!();
    println!(
        "{}",
        s.header("Cast-on-curve ceilings (exact hypergeometric)")
    );
    for row in cards.iter().take(5) {
        let name = row
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let target = row.get("target_turn").and_then(|v| v.as_u64()).unwrap_or(0);
        let ceiling = row
            .get("pct_castable_ceiling")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        println!(
            "    {}  {}  ceiling {:.0}%",
            s.card_name(&name),
            s.dim(&format!("by t{target}")),
            ceiling * 100.0
        );
    }
    println!(
        "{}",
        s.note("ceiling = exact upper bound on the real cast rate; sim castability is draw-agnostic and sits above it")
    );
}

/// Round to 2 decimals.
fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Round a 0-1 share to 0-100 percent, 2 decimals.
///
/// The JSON contract reports every `pct_*` field on the same 0-100 scale;
/// internal shares stay 0-1.
fn pct2(share: f64) -> f64 {
    round2(share * 100.0)
}

/// serde helper: emit a 0-1 share as 0-100 percent.
pub(super) fn serialize_pct<S: serde::Serializer>(share: &f64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(pct2(*share))
}

/// Turn-indexed map of 0-1 shares, emitted as 0-100 percent.
fn pct_turn_map(values: &[f64]) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (i, v) in values.iter().enumerate() {
        map.insert((i + 1).to_string(), serde_json::json!(pct2(*v)));
    }
    map
}

/// Turn-indexed JSON object with 1-based string keys, rounded to 2 decimals.
fn turn_map(values: &[f64]) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (i, v) in values.iter().enumerate() {
        map.insert((i + 1).to_string(), serde_json::json!(round2(*v)));
    }
    map
}

/// One-line summary of the run.
fn summary_line(
    stats: &SimStats,
    deck: &SimDeck,
    problems: &[super::aggregate::Problem],
) -> String {
    let cmd_part = deck.commanders.first().map(|cmd| {
        let by = stats.commander_castable_by[(cmd.cost.total() as usize).min(12)];
        format!(" · commander on curve {:.0}%", by * 100.0)
    });
    let base = format!(
        "{} games · {} problem{}",
        stats.runs,
        problems.len(),
        if problems.len() == 1 { "" } else { "s" }
    );
    match cmd_part {
        Some(part) => format!("{base}{part}"),
        None => base,
    }
}

/// Build the JSON payload for the report.
pub fn json_report(
    stats: &SimStats,
    deck: &SimDeck,
    name: &str,
    seed: u64,
    problems: &[super::aggregate::Problem],
    sideboard_cards: i64,
) -> serde_json::Value {
    let turns = stats.turns as usize;
    let lands = deck.cards.iter().filter(|c| c.role == Role::Land).count() as i64;
    let rocks = deck.cards.iter().filter(|c| c.role == Role::Rock).count() as i64;
    let dorks = deck.cards.iter().filter(|c| c.role == Role::Dork).count() as i64;
    let ramp = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::RampSpell)
        .count() as i64;
    let mut curve = serde_json::Map::new();
    for bucket in ["0", "1", "2", "3", "4", "5", "6", "7+"] {
        let count = deck
            .cards
            .iter()
            .filter(|c| c.role != Role::Land && curve_bucket(c.cost.total()) == bucket)
            .count() as i64;
        curve.insert(bucket.to_string(), serde_json::json!(count));
    }
    let format_name = deck.rules.key;
    let commander_json = deck.commanders.first().map(|cmd| {
        let cmc = cmd.cost.total() as usize;
        serde_json::json!({
            "name": cmd.name,
            "cmc": cmd.cost.total() as f64,
            "pct_castable_by_turn": pct_turn_map(&stats.commander_castable_by[1..=turns.min(12)]),
            "avg_first_cast_turn": round2(stats.avg_commander_cast_turn),
            "p50_cast_turn": stats.p50_commander_cast_turn,
            "p95_cast_turn": stats.p95_commander_cast_turn,
            "on_curve_pct": pct2(stats.commander_castable_by[cmc.clamp(1, turns.min(12))]),
        })
    });
    let problems_json: Vec<serde_json::Value> = problems
        .iter()
        .map(|p| {
            serde_json::json!({
                "kind": p.kind,
                "severity": p.severity,
                "pct_games": p.pct_games.map(round2),
                "color": p.color,
                "detail": p.detail,
                "suggestion": p.suggestion,
            })
        })
        .collect();
    let station_json = if deck
        .commanders
        .first()
        .is_some_and(|cmd| cmd.animate_at().is_some())
    {
        serde_json::json!({
            "online_by_t6": pct2(stats.station_online_pct),
            "p50_online_turn": stats.station_p50_turn,
        })
    } else {
        serde_json::Value::Null
    };
    serde_json::json!({
        "name": name,
        "format": format_name,
        "runs": stats.runs,
        "turns": stats.turns,
        "seed": seed,
        "deck_shape": {
            "total_cards": total_cards(deck) as i64,
            "sideboard_cards": sideboard_cards,
            "lands": lands,
            "rocks": rocks,
            "dorks": dorks,
            "ramp_spells": ramp,
            "draw_sources": stats.draw_count as i64,
            "removal": stats.removal_count as i64,
            "wincons": stats.wincon_count as i64,
            "locks": lock_count(deck),
            "boosters": booster_count(deck),
            "curve": curve,
        },
        "assumptions": assumptions(deck),
        "opening_hand": {
            "avg_lands": round2(stats.avg_opener_lands),
            "pct_0": pct2(stats.opener_pct[0]),
            "pct_1": pct2(stats.opener_pct[1]),
            "pct_2": pct2(stats.opener_pct[2]),
            "pct_3": pct2(stats.opener_pct[3]),
            "pct_4": pct2(stats.opener_pct[4]),
            "pct_5plus": pct2(stats.opener_pct[5]),
            "mulligan_rate": pct2(stats.mulligan_rate),
        },
        "land_drops": {
            "hit_all_by_turn": pct_turn_map(&stats.hit_all_drops_by[1..=turns.min(5)]),
            "screw_pct_2_or_fewer_by_t4": pct2(stats.screw_pct),
            "flood_pct_5plus_by_t4": pct2(stats.flood_pct),
            "p50_drops_by_4": stats.p50_drops_by_4,
            "p95_drops_by_4": stats.p95_drops_by_4,
        },
        "commander": commander_json,
        "station": station_json,
        "bodies_by_turn": turn_map(&stats.bodies_by_turn[..turns]),
        "engines_online_by_turn": turn_map(&stats.engines_by_turn[..turns]),
        "mana": {
            "avg_unused_by_turn": turn_map(&stats.unused_mana[..turns]),
            "pct_games_floated_3plus_t6": pct2(stats.floated_pct),
        },
        "draw": {
            "pct_seen_by_turn": pct_turn_map(&stats.draw_sources[..turns]),
            "pct_starved_0_by_t6": pct2(stats.starved_pct),
        },
        "role_access": {
            "removal_pct_seen_by_5": pct2(stats.removal_access_5),
            "draw_pct_seen_by_6": pct2(stats.draw_access_6),
            "creature_pct_seen_by_3": pct2(stats.creature_access_3),
            "wincon_pct_seen_by_8": pct2(stats.wincon_access_8),
            "lock_pct_seen_by_3": pct2(stats.lock_access_3),
        },
        "velocity": {
            "avg_cards_seen": turn_map(&stats.cards_seen[..turns]),
            "library_awareness_by_turn": pct_turn_map(
                &stats.library_awareness_by_turn[..turns],
            ),
            "self_milled_by_turn": turn_map(&stats.self_milled_by_turn[..turns]),
            "opp_milled_by_turn": turn_map(&stats.opp_milled_by_turn[..turns]),
            "library_remaining_by_turn": turn_map(&stats.library_by_turn[..turns]),
        },
        "combat": {
            "attack_power_avg_by_turn": turn_map(&stats.attack_power_by_turn[..turns]),
            "attack_power_p90_by_t8": stats.attack_power_p90,
            "attackers_by_turn": turn_map(&stats.attackers_by_turn[..turns]),
            "evasive_by_turn": turn_map(&stats.evasive_by_turn[..turns]),
        },
        "wincons": {
            "drain_total_by_turn": turn_map(&stats.drain_total_by_turn[..turns]),
            "extra_turns_pct": pct2(stats.extra_turns_pct),
            "win_threshold_p50_turn": stats.win_threshold_p50_turn,
            "win_threshold_pct": pct2(stats.win_threshold_pct),
            "ultimate_online_pct": pct2(stats.ultimate_online_pct),
            "infinite_mana_pct": pct2(stats.infinite_mana_pct),
        },
        "interaction": {
            "ready_pct_by_turn": pct_turn_map(&stats.interaction_ready_by_turn[..turns]),
            "mana_held_avg": round2(stats.interaction_mana_held),
            "instant_speed_count": stats.interaction_instant_count,
            "note": "capacity, not events",
        },
        "color_screw": {
            "W": pct2(stats.color_screw[0]),
            "U": pct2(stats.color_screw[1]),
            "B": pct2(stats.color_screw[2]),
            "R": pct2(stats.color_screw[3]),
            "G": pct2(stats.color_screw[4]),
        },
        "pip_blocks": stats.pip_blocks.iter().map(|p| {
            serde_json::json!({
                "name": p.name,
                "color": p.color,
                "pct_games": pct2(p.pct_games),
            })
        }).collect::<Vec<_>>(),
        "graveyard": {
            "avg_size_by_turn": turn_map(&stats.graveyard_by_turn[..turns.min(stats.graveyard_by_turn.len())]),
        },
        "color_sources": color_source_census(deck),
        "card_castability": stats.card_castability.iter().map(|c| {
            serde_json::json!({
                "name": c.name,
                "cmc": round2(c.cmc),
                "target_turn": c.target_turn,
                "pct_castable_by_target": pct2(c.pct_by_target),
                "avg_first_castable_turn": round2(c.avg_first_castable_turn),
            })
        }).collect::<Vec<_>>(),
        "problems": problems_json,
        "summary": summary_line(stats, deck, problems),
    })
}

/// Per-color mana-source census over the deck's lands and rocks.
///
/// Static analysis (no simulation): for each WUBRG color, how many
/// sources can produce it and by which shape (fixed pips, choice,
/// any-color). The `fixing` array counts sources producing 2+ colors.
fn color_source_census(deck: &SimDeck) -> serde_json::Value {
    use super::model::Role;
    let mut fixed = [0u32; 5];
    let mut choice = [0u32; 5];
    let mut flexible_nonland = 0u32;
    let mut scaling = 0u32;
    for card in deck.cards.iter() {
        if card.role == Role::Land {
            let Some(y) = &card.tap else { continue };
            for (i, p) in y.fixed.iter().enumerate() {
                if *p > 0 {
                    fixed[i] += 1;
                }
            }
            if y.any_pips > 0 || y.opponent_any {
                // Any-color choice lands (Command Tower): every tracked color.
                for c in choice.iter_mut() {
                    *c += 1;
                }
            } else {
                let colors = y.choice.iter().filter(|c| **c).count();
                if colors >= 2 {
                    for (i, c) in y.choice.iter().enumerate() {
                        if *c {
                            choice[i] += 1;
                        }
                    }
                } else if colors == 1
                    && let Some(i) = y.choice.iter().position(|c| *c)
                {
                    fixed[i] += 1;
                }
            }
        } else if card.role == Role::Rock || card.role == Role::Dork {
            // Non-land mana sources: flexible output per turn (rocks tap
            // for one; dorks join on the first body turn).
            if let Some(y) = &card.tap
                && (y.any_pips > 0 || y.choice.iter().any(|c| *c) || y.fixed.iter().any(|p| *p > 0))
            {
                flexible_nonland += 1;
            }
        }
        if card.tap.as_ref().is_some_and(|y| y.scaling.is_some()) {
            scaling += 1;
        }
    }
    serde_json::json!({
        "fixed_source_lands": {
            "W": fixed[0], "U": fixed[1], "B": fixed[2], "R": fixed[3], "G": fixed[4],
        },
        "choice_source_lands": {
            "W": choice[0], "U": choice[1], "B": choice[2], "R": choice[3], "G": choice[4],
        },
        "flexible_nonland_sources": flexible_nonland,
        "scaling_sources": scaling,
        "note": "static census of land tap yields; a choice land serves every color it can pick; flexible_nonland_sources counts rocks and dorks",
    })
}

/// Human report on stdout: overview block, worst castability, problems, note.
pub fn print_report(
    out: &Output,
    name: &str,
    deck: &SimDeck,
    stats: &SimStats,
    problems: &[super::aggregate::Problem],
) {
    let s = out.styles();
    let turns = stats.turns as usize;
    println!(
        "{}  {}  {}",
        s.header(name),
        s.dim(&format!(
            "{} games · {} turns",
            s.thousands(stats.runs as i64),
            stats.turns
        )),
        s.dim(&format!(
            "{} cards · {} lands · {} ramp · avg CMC {:.1}",
            total_cards(deck),
            stats.land_count,
            deck.cards
                .iter()
                .filter(|c| matches!(c.role, Role::Rock | Role::Dork | Role::RampSpell))
                .count(),
            avg_cmc(deck)
        ))
    );
    println!();
    println!(
        "  {}  {}  {:.1} lands avg · mulligan {:.1}%",
        s.bar(stats.avg_opener_lands / 7.0, 10),
        s.dim("opening hand"),
        stats.avg_opener_lands,
        stats.mulligan_rate * 100.0
    );
    if turns >= 4 {
        println!(
            "  {}  {}  {:.1}% hit all 4 · screw {:.1}% · flood {:.1}%",
            s.bar(stats.hit_all_drops_by[4], 10),
            s.dim("land drops by t4"),
            stats.hit_all_drops_by[4] * 100.0,
            stats.screw_pct * 100.0,
            stats.flood_pct * 100.0
        );
    }
    if let Some(cmd) = deck.commanders.first() {
        let cmc = cmd.cost.total();
        let by = stats.commander_castable_by[(cmc as usize).min(12)];
        println!(
            "  {}  {}  {:.1}% by turn {cmc} · p50 t{} · p95 t{}",
            s.bar(by, 10),
            s.dim(&format!("commander (CMC {cmc})")),
            by * 100.0,
            stats.p50_commander_cast_turn,
            stats.p95_commander_cast_turn
        );
        if cmd.animate_at().is_some() && turns >= 6 {
            println!(
                "  {}  {}  {:.1}% by t6 · p50 t{}",
                s.bar(stats.station_online_pct, 10),
                s.dim("station online"),
                stats.station_online_pct * 100.0,
                stats.station_p50_turn
            );
        }
    }
    let t6 = 6.min(turns) - 1;
    println!(
        "  {}  {}  {:.1} avg unspent · {:.1} cards seen · {:.1} bodies",
        s.bar((stats.unused_mana[t6] / 5.0).min(1.0), 10),
        s.dim(&format!("mana thru t{}", t6 + 1)),
        stats.unused_mana[t6],
        stats.cards_seen[t6],
        stats.bodies_by_turn[t6]
    );
    if turns >= 5 {
        println!(
            "  {}  {}  {:.1}% of games",
            s.bar(stats.removal_access_5, 10),
            s.dim("removal seen by t5"),
            stats.removal_access_5 * 100.0
        );
    }
    if turns >= 6 {
        println!(
            "  {}  {}  {:.1}% of games · {:.1}% starved",
            s.bar(stats.draw_access_6, 10),
            s.dim("draw source by t6"),
            stats.draw_access_6 * 100.0,
            stats.starved_pct * 100.0
        );
    }
    if stats.interaction_instant_count > 0 && turns >= 5 {
        println!(
            "  {}  {}  {:.1}% · instant-speed {} · held {:.1}",
            s.bar(stats.interaction_ready_by_turn[4], 10),
            s.dim("interaction ready by t5"),
            stats.interaction_ready_by_turn[4] * 100.0,
            stats.interaction_instant_count,
            stats.interaction_mana_held
        );
    }
    if stats.attack_power_by_turn[t6] > 0.0 {
        println!(
            "  {}  {}  {:.0} avg power · {:.0} p90 t8 · {:.1}/{:.0} evasion",
            s.bar((stats.attack_power_by_turn[t6] / 40.0).min(1.0), 10),
            s.dim("attack power"),
            stats.attack_power_by_turn[t6],
            stats.attack_power_p90,
            stats.evasive_by_turn[t6],
            stats.bodies_by_turn[t6]
        );
    }
    if stats.drain_total_by_turn[t6] > 0.0 {
        println!(
            "  {}  {}  {:.1} life drained by t{}",
            s.bar((stats.drain_total_by_turn[t6] / 30.0).min(1.0), 10),
            s.dim("drain"),
            stats.drain_total_by_turn[t6],
            t6 + 1
        );
    }
    if stats.extra_turns_pct > 0.0 {
        println!(
            "  {}  {}  {:.1}% of games",
            s.bar(stats.extra_turns_pct, 10),
            s.dim("extra turns taken"),
            stats.extra_turns_pct * 100.0
        );
    }
    if stats.win_threshold_pct > 0.0 {
        println!(
            "  {}  {}  {:.1}% by t10 · p50 t{}",
            s.bar(stats.win_threshold_pct, 10),
            s.dim("win threshold online"),
            stats.win_threshold_pct * 100.0,
            stats.win_threshold_p50_turn
        );
    }
    if stats.ultimate_online_pct > 0.0 {
        println!(
            "  {}  {}  {:.1}% by t10",
            s.bar(stats.ultimate_online_pct, 10),
            s.dim("ultimate online"),
            stats.ultimate_online_pct * 100.0
        );
    }
    let mut worst: Vec<&super::aggregate::CardCast> = stats.card_castability.iter().collect();
    worst.sort_by(|a, b| {
        a.pct_by_target
            .partial_cmp(&b.pct_by_target)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    worst.truncate(3);
    if !worst.is_empty() {
        println!();
        println!("{}", s.header("Slow to cast (worst 3)"));
        for card in worst {
            println!(
                "    {}  {}  cast by t{} in {:.0}%",
                s.card_name(&card.name),
                s.dim(&format!("CMC {:.0}", card.cmc)),
                card.target_turn,
                card.pct_by_target * 100.0
            );
        }
    }
    // Mana-source census + worst pip blockers: the color-screw story in
    // two lines (what the deck has, which card got hurt).
    if stats.color_screw.iter().any(|p| *p >= 0.10) && !stats.pip_blocks.is_empty() {
        println!();
        println!("{}", s.header("Mana sources & pip blocks"));
        for p in stats.pip_blocks.iter().take(3) {
            println!(
                "    {}  {} pips missed in {:.0}% of games",
                s.card_name(&p.name),
                p.color,
                p.pct_games * 100.0
            );
        }
    }
    if !problems.is_empty() {
        println!();
        println!("{}", s.header("Problems"));
        for p in problems {
            println!("  {}", s.error(&format!("{}: {}", p.kind, p.detail)));
            println!("    {}", s.dim(&format!("→ {}", p.suggestion)));
        }
    }
    println!();
    println!(
        "{}",
        s.note(
            "solitaire sim: no opponents, no counters; enters-tapped honored; full detail in --json"
        )
    );
}

// Baseline diffing for `deck simulate --baseline`: only the deltas between
// a prior JSON report and the fresh run print, so a deck edit's effect is
// readable at a glance.

/// One metric delta: dot path, old value, new value.
#[derive(Debug, Clone, PartialEq)]
pub struct Delta {
    /// Dot path into the report (e.g. "commander.on_curve_pct").
    pub path: String,
    /// Prior value.
    pub old: serde_json::Value,
    /// New value.
    pub new: serde_json::Value,
}

/// Problem kinds that appeared or disappeared between the two runs.
#[derive(Debug, Clone, PartialEq)]
pub struct ProblemDelta {
    /// "resolved" (in baseline, gone now) or "new" (absent before).
    pub change: &'static str,
    /// The problem's kind + detail.
    pub kind: String,
    pub detail: String,
}

/// The full diff between two reports.
#[derive(Debug, Default, PartialEq)]
pub struct ReportDiff {
    pub metrics: Vec<Delta>,
    pub problems: Vec<ProblemDelta>,
    /// Deck-shape count changes (name, old, new).
    pub shape: Vec<(String, String, String)>,
}

/// Diff two report payloads: scalar metrics at known paths, problem lists,
/// and deck-shape counts. Identical inputs yield an empty diff.
pub fn diff_reports(baseline: &serde_json::Value, current: &serde_json::Value) -> ReportDiff {
    let mut diff = ReportDiff::default();

    // Scalar metrics: (dot path) pairs worth tracking.
    for path in METRIC_PATHS {
        if let (Some(old), Some(new)) = (lookup(baseline, path), lookup(current, path))
            && old != new
        {
            diff.metrics.push(Delta {
                path: (*path).to_string(),
                old: old.clone(),
                new: new.clone(),
            });
        }
    }

    // Deck shape counts.
    let old_shape = baseline.get("deck_shape");
    let new_shape = current.get("deck_shape");
    if let (Some(old), Some(new)) = (old_shape, new_shape)
        && (old.is_object() && new.is_object())
    {
        for key in [
            "total_cards",
            "lands",
            "rocks",
            "dorks",
            "ramp_spells",
            "draw_sources",
            "removal",
            "wincons",
        ] {
            let old_v = old.get(key).cloned().unwrap_or_default();
            let new_v = new.get(key).cloned().unwrap_or_default();
            if old_v != new_v {
                diff.shape.push((
                    key.to_string(),
                    value_display(&old_v),
                    value_display(&new_v),
                ));
            }
        }
    }

    // Problems: match by kind+color; report kind+detail changes and
    // appear/disappear as problems entries.
    let old_problems = baseline
        .get("problems")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let new_problems = current
        .get("problems")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let key = |p: &serde_json::Value| {
        format!(
            "{}|{}",
            p.get("kind").and_then(|k| k.as_str()).unwrap_or(""),
            p.get("color").and_then(|c| c.as_str()).unwrap_or("")
        )
    };
    for p in &new_problems {
        if !old_problems.iter().any(|o| key(o) == key(p)) {
            diff.problems.push(ProblemDelta {
                change: "new",
                kind: string_field(p, "kind"),
                detail: string_field(p, "detail"),
            });
        }
    }
    for p in &old_problems {
        if !new_problems.iter().any(|n| key(n) == key(p)) {
            diff.problems.push(ProblemDelta {
                change: "resolved",
                kind: string_field(p, "kind"),
                detail: string_field(p, "detail"),
            });
        }
    }
    // Detail changes for still-present kinds (severity shifts).
    for p in &new_problems {
        if let Some(old) = old_problems.iter().find(|o| key(o) == key(p)) {
            let old_detail = string_field(old, "detail");
            let new_detail = string_field(p, "detail");
            if old_detail != new_detail {
                diff.metrics.push(Delta {
                    path: format!("problems[{}].detail", string_field(p, "kind")),
                    old: serde_json::json!(old_detail),
                    new: serde_json::json!(new_detail),
                });
            }
        }
    }
    diff
}

/// Tracked scalar metric paths (dot-separated).
const METRIC_PATHS: &[&str] = &[
    "commander.on_curve_pct",
    "commander.avg_first_cast_turn",
    "land_drops.screw_pct_2_or_fewer_by_t4",
    "land_drops.flood_pct_5plus_by_t4",
    "mana.pct_games_floated_3plus_t6",
    "draw.pct_starved_0_by_t6",
    "role_access.removal_pct_seen_by_5",
    "role_access.draw_pct_seen_by_6",
    "role_access.creature_pct_seen_by_3",
    "role_access.wincon_pct_seen_by_8",
    "color_screw.W",
    "color_screw.U",
    "color_screw.B",
    "color_screw.R",
    "color_screw.G",
];

/// Follow a dot path through JSON objects.
fn lookup(value: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current.clone())
}

/// String field of a problem object.
fn string_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Display string for a JSON value in diff output.
fn value_display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Render the diff for humans: shape changes, metric deltas, problems.
pub fn print_diff(out: &Output, diff: &ReportDiff) {
    let s = out.styles();
    if diff.is_empty() {
        println!("{}", s.success("no changes vs baseline"));
        return;
    }
    println!("{}", s.header("Deltas vs baseline"));
    if !diff.shape.is_empty() {
        println!("{}", s.header("Shape"));
        for (name, old, new) in &diff.shape {
            println!("    {name}: {old} → {new}");
        }
    }
    if !diff.metrics.is_empty() {
        println!("{}", s.header("Metrics"));
        for d in &diff.metrics {
            println!("    {}: {} → {}", d.path, d.old, d.new);
        }
    }
    if !diff.problems.is_empty() {
        println!("{}", s.header("Problems"));
        for p in &diff.problems {
            match p.change {
                // Bare signs: "+" = new problem (red), "-" = resolved
                // (green). error()/success() would print "error: +".
                "new" => println!(
                    "    {} {}: {}",
                    s.glyph("+", crate::output::GlyphKind::Bad),
                    p.kind,
                    p.detail
                ),
                _ => println!(
                    "    {} {}: {}",
                    s.glyph("-", crate::output::GlyphKind::Good),
                    p.kind,
                    p.detail
                ),
            }
        }
    }
}

impl ReportDiff {
    /// True when nothing changed between the two reports.
    pub fn is_empty(&self) -> bool {
        self.shape.is_empty() && self.metrics.is_empty() && self.problems.is_empty()
    }
}

#[cfg(test)]
mod diff_tests {
    use super::*;

    fn base_report() -> serde_json::Value {
        serde_json::json!({
            "deck_shape": {"total_cards": 100, "lands": 44, "removal": 4, "wincons": 3},
            "commander": {"on_curve_pct": 0.77, "avg_first_cast_turn": 4.0},
            "role_access": {"removal_pct_seen_by_5": 0.5, "draw_pct_seen_by_6": 0.94},
            "color_screw": {"W": 0.0, "U": 0.31, "B": 0.0, "R": 0.0, "G": 0.22},
            "problems": [
                {"kind": "color_screw", "severity": "high", "color": "U", "detail": "U pips missed 31%"},
                {"kind": "dead_cards", "severity": "high", "detail": "3 cards slow"}
            ],
        })
    }

    #[test]
    fn identical_reports_diff_to_empty() {
        let base = base_report();
        let diff = diff_reports(&base, &base);
        assert!(diff.is_empty());
    }

    #[test]
    fn metric_deltas_and_shape_changes_report() {
        let mut current = base_report();
        current["commander"]["on_curve_pct"] = serde_json::json!(0.82);
        current["color_screw"]["U"] = serde_json::json!(0.20);
        current["deck_shape"]["lands"] = serde_json::json!(42);
        let diff = diff_reports(&base_report(), &current);
        assert_eq!(diff.metrics.len(), 2);
        assert_eq!(diff.metrics[0].path, "commander.on_curve_pct");
        assert_eq!(diff.shape.len(), 1);
        assert_eq!(diff.shape[0].0, "lands");
        assert_eq!(diff.shape[0].1, "44");
        assert_eq!(diff.shape[0].2, "42");
        assert!(diff.problems.is_empty());
    }

    #[test]
    fn new_and_resolved_problems_report() {
        let mut current = base_report();
        // Resolved: dead_cards gone. New: mana_flood appears.
        current["problems"] = serde_json::json!([
            {"kind": "color_screw", "severity": "high", "color": "U", "detail": "U pips missed 31%"},
            {"kind": "mana_flood", "severity": "medium", "detail": "22% flooded"}
        ]);
        let diff = diff_reports(&base_report(), &current);
        let kinds: Vec<(&str, &str)> = diff
            .problems
            .iter()
            .map(|p| (p.change, p.kind.as_str()))
            .collect();
        assert!(kinds_contains(&kinds, &("resolved", "dead_cards")));
    }

    fn kinds_contains(list: &[(&str, &str)], want: &(&str, &str)) -> bool {
        list.iter().any(|p| p == want)
    }
}
