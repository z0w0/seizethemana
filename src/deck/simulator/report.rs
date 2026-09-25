// Report renders for the simulator: the JSON payload and the human
// stdout view. JSON adds station/bodies/engines metrics; the human view
// gains a station line when the commander is a spacecraft.

use super::aggregate::SimStats;
use super::model::{Role, SimDeck};
use crate::output::Output;

/// Lock-pieces in the deck (Role::Lock census for the deck shape).
fn lock_count(deck: &SimDeck) -> i64 {
    deck.cards.iter().filter(|c| c.role == Role::Lock).count() as i64
}

/// Booster-pieces in the deck (equipment, auras, pump spells).
fn booster_count(deck: &SimDeck) -> i64 {
    deck.cards
        .iter()
        .filter(|c| c.role == Role::Booster)
        .count() as i64
}

/// Total cards in the deck (library + commanders).
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
///
/// Reader-facing: an AI agent (or a human) uses these lines to interpret
/// the numbers. Each line states one assumption or constraint of the
/// simulation model, in plain sentences. Implementation names, module
/// names, and code identifiers stay out; the JSON field names the list
/// itself refers to (like `infinite_mana_pct`) are the report's own
/// keys, which the reader sees in the same report.
pub(super) fn assumptions(deck: &SimDeck) -> Vec<String> {
    let mut list = vec![
        "This is a best-case solitaire simulation. There are no opponents: nothing is countered, no removal is cast, no attacker is blocked, and board wipes never fire.".to_string(),
        "Lands enter the battlefield as their card text says. Cards that may enter untapped by paying life do so.".to_string(),
        "A creature's attack power uses its printed power when known; unknown powers and tokens count as 2.".to_string(),
        "Each draw engine draws its stated amount once per turn from the turn after it enters, no matter what is on the battlefield.".to_string(),
        "A draw engine that scales with the board ('draw a card for each enchantment you control') draws the number of matching permanents, at most 8. Shapes it cannot count draw 1.".to_string(),
        "A card that enters and draws ('When this creature enters, draw a card') draws only once per entry. It is a trigger, not an extra cast effect.".to_string(),
        "X-cost spells spend all leftover mana as X. Drain, draw, mill, tokens, reveal-permanents, and counter-power effects scale with that X.".to_string(),
        "X spells the model cannot execute pay X = 1 and do nothing extra.".to_string(),
        "A split card such as 'Fire // Ice' is cast as its cheaper face. The other face counts for deck categories but its cast effects never happen.".to_string(),
        "One optional kicker cost is paid when spare mana covers it. Only drain amounts grow with the kick.".to_string(),
        "Casts that sacrifice a creature or pay life as an extra cost pay it; the body really leaves the battlefield.".to_string(),
        "Cards that create tokens create that many 2/2 bodies, at most 8 per effect. Card effects that create Treasure tokens bank the tokens as mana instead of bodies.".to_string(),
        "Treasure banking needs the Treasure clause on the card that makes the tokens. A Treasure maker elsewhere in the deck does not convert other cards' tokens.".to_string(),
        "Damage and life loss aimed at a player resolves three times in commander (three opponents) and once in 60-card formats.".to_string(),
        "The lethal census sums combat damage and burn/drain effects against the full table life (120 in commander, 20 in 60-card). It is an upper bound: real games have blockers, removal, and life gain.".to_string(),
        "Mill and discard fill the graveyard census and count as cards seen. A card returns from the graveyard at most once; no loops.".to_string(),
        "A wheel discards the whole hand, then draws seven. Loot is draw N, then discard N.".to_string(),
        "Removal is measured as capacity: how often an answer is in hand and affordable. Counterspells, targeted removal, bounce, and board wipes all count; wipes never resolve.".to_string(),
        "The removal count splits into targeted removal and board wipes in the deck shape.".to_string(),
        "Lands that may tap only for creature spells pay creature casts only.".to_string(),
        "A sacrifice outlet consumes a real untapped creature. With no creature available it does nothing.".to_string(),
        "A blink effect ('exile, then return') re-fires the card's enter triggers exactly once, the next turn.".to_string(),
        "Planeswalkers use one loyalty ability per turn: plus abilities gain loyalty, minus abilities spend it. Ultimates only report the turn they become affordable; they do not resolve.".to_string(),
        "Sagas resolve one chapter per turn and leave the battlefield after the last chapter.".to_string(),
        "Mana engines that produce per spell cast ('add one mana for each spell you've cast this turn') pay out once per spell cast each turn while untapped.".to_string(),
        "A zero-cost mana engine that produces more than it costs is capped after 24 activations per turn and flagged in the report (suspected infinite engine). Engines limited to once per turn are not flagged.".to_string(),
        "Equipment boosts only the creature it equips, after the equip cost is paid. Auras and other continuous buffs are not modeled.".to_string(),
        "A one-shot board buff ('creatures you control get +X/+X where X is the number of creatures you control') boosts that combat phase only, on the turn it enters, capped at 20 creatures.".to_string(),
        "Extra turns replay a land drop, a draw, and upkeep engines. They are not full turns: no casts, no combat.".to_string(),
        "Not modeled: energy, metalcraft, converge, proliferate, and replay mechanics such as flashback and rebound. Cards with only these effects play as vanilla.".to_string(),
        "A cascade cast also casts the cheapest cheaper nonland card in the library, once, with no cascade chaining. Only creature hits join the battlefield.".to_string(),
        "A land/spell MDFC (e.g. Valakut Awakening) is played as its land face when no other land drop is available, and cast as its spell face otherwise. Non-mythic MDFCs count 0.4 land, mythic 0.75.".to_string(),
        "Cards whose text the parser cannot read play as plain cards with no abilities. Their mana cost still gates the cast.".to_string(),
        "Seeded baselines change between versions. Regenerate any saved baseline report after upgrading.".to_string(),
    ];
    if deck.commanders.is_empty() {
        list.push("The whole deck is shuffled; there is no commander zone.".to_string());
        list.push(
            "London mulligan: a hand ships only when it has 0, 1, 6, or 7 lands. It redraws a full seven and bottoms one chosen card toward three lands, keeping six. Kept hands stay at seven."
                .to_string(),
        );
    } else {
        list.push(
            "The commander starts the game in the command zone and never costs extra to recast."
                .to_string(),
        );
        list.push(
            "One free mulligan when the opening hand has fewer than 2 or more than 6 lands."
                .to_string(),
        );
        list.push(
            "The commander's upkeep and end-step draw triggers fire once per turn from the turn after it is cast. Draws gated on attacking wait for combat."
                .to_string(),
        );
    }
    list.push(
        "Combo assembly measures how often each combo piece reaches the zone its combo needs. Exile-zone pieces are skipped; library pieces count as seen when drawn."
            .to_string(),
    );
    list
}

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
fn summary_line(stats: &SimStats, deck: &SimDeck, problems: &[super::findings::Problem]) -> String {
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

/// Bench-section counts for the JSON `deck_shape` block.
#[derive(Debug, Clone, Copy, Default)]
pub struct BenchCounts {
    /// SIDEBOARD copies excluded from the simulated library.
    pub sideboard_cards: i64,
    /// MAYBEBOARD copies excluded from the simulated library.
    pub maybeboard_cards: i64,
}

/// Build the JSON payload for the report.
pub fn json_report(
    stats: &SimStats,
    deck: &SimDeck,
    name: &str,
    seed: u64,
    problems: &[super::findings::Problem],
    bench: BenchCounts,
    mana_base: &super::findings::ManaBase,
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
        // "On curve" only means something when the sim ran long enough
        // to reach the commander's curve turn; a clamped short run
        // would read as a misleading 0%. Null past the horizon.
        let on_curve = (cmc <= turns && cmc >= 1)
            .then(|| pct2(stats.commander_castable_by[cmc.clamp(1, turns.min(12))]));
        serde_json::json!({
            "name": cmd.name,
            "cmc": cmd.cost.total() as f64,
            "pct_castable_by_turn": pct_turn_map(&stats.commander_castable_by[1..=turns.min(12)]),
            "avg_first_cast_turn": round2(stats.avg_commander_cast_turn),
            "p50_cast_turn": stats.p50_commander_cast_turn,
            "p95_cast_turn": stats.p95_commander_cast_turn,
            "on_curve_pct": on_curve,
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
                "offenders": p.offenders,
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
            "sideboard_cards": bench.sideboard_cards,
            "maybeboard_cards": bench.maybeboard_cards,
            "lands": lands,
            "rocks": rocks,
            "dorks": dorks,
            "ramp_spells": ramp,
            "draw_sources": stats.draw_count as i64,
            "removal": stats.removal_count as i64,
            "removal_targeted": stats.removal_targeted as i64,
            "removal_wipes": stats.removal_wipes as i64,
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
            "flood_pct_6plus_lands_seen_in_11": pct2(stats.flood_pct),
            "flood_expectation": pct2(stats.flood_expectation),
            "p50_drops_by_4": stats.p50_drops_by_4,
            "p95_drops_by_4": stats.p95_drops_by_4,
        },
        "mana_base": mana_base,
        "mana_base_bracket_inferred": mana_base.bracket_inferred,
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
            "lethal_damage_by_turn": pct_turn_map(
                &stats.lethal_damage_by_turn[..turns],
            ),
            "p50_lethal_turn": stats.p50_lethal_turn,
            "lethal_note": "best-case goldfish, unblocked: an upper bound; real games have blockers, removal, and life gain",
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
    problems: &[super::findings::Problem],
    mana_base: &super::findings::ManaBase,
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
            "  {}  {}  {:.1}% hit all 4 · short {:.1}% · flooded {:.1}%",
            s.bar(stats.hit_all_drops_by[4], 10),
            s.dim("land drops by turn 4"),
            stats.hit_all_drops_by[4] * 100.0,
            stats.screw_pct * 100.0,
            stats.flood_pct * 100.0
        );
        let mana_base_line = match mana_base.bracket {
            Some(bracket) => format!(
                "{} lands · {} ramp · target {}-{} lands + {}-{} ramp · bracket {}",
                mana_base.lands,
                mana_base.rocks + mana_base.dorks + mana_base.ramp_spells,
                mana_base.bracket_target_lands[0],
                mana_base.bracket_target_lands[1],
                mana_base.bracket_target_ramp[0],
                mana_base.bracket_target_ramp[1],
                bracket,
            ),
            None => format!(
                "{} lands · {} ramp · target {}-{} lands",
                mana_base.lands,
                mana_base.rocks + mana_base.dorks + mana_base.ramp_spells,
                mana_base.bracket_target_lands[0],
                mana_base.bracket_target_lands[1],
            ),
        };
        println!(
            "  {}  {}  {}",
            s.dim("mana base"),
            s.dim(&mana_base_line),
            s.note(&mana_base.verdict)
        );
    }
    if let Some(cmd) = deck.commanders.first() {
        let cmc = cmd.cost.total();
        let by = stats.commander_castable_by[(cmc as usize).min(12)];
        println!(
            "  {}  {}  {:.1}% by turn {cmc} · p50 t{} · p95 t{}",
            s.bar(by, 10),
            s.dim(&format!("commander (costs {cmc} mana)")),
            by * 100.0,
            stats.p50_commander_cast_turn,
            stats.p95_commander_cast_turn
        );
        if cmd.animate_at().is_some() && turns >= 6 {
            println!(
                "  {}  {}  {:.1}% by turn 6 · half of games by turn {}",
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
        s.dim(&format!("mana through turn {}", t6 + 1)),
        stats.unused_mana[t6],
        stats.cards_seen[t6],
        stats.bodies_by_turn[t6]
    );
    if turns >= 5 {
        println!(
            "  {}  {}  {:.1}% of games",
            s.bar(stats.removal_access_5, 10),
            s.dim("removal seen by turn 5"),
            stats.removal_access_5 * 100.0
        );
    }
    if turns >= 6 {
        println!(
            "  {}  {}  {:.1}% of games · {:.1}% starved",
            s.bar(stats.draw_access_6, 10),
            s.dim("draw source by turn 6"),
            stats.draw_access_6 * 100.0,
            stats.starved_pct * 100.0
        );
    }
    if stats.interaction_instant_count > 0 && turns >= 5 {
        println!(
            "  {}  {}  {:.1}% · instant-speed {} · held {:.1}",
            s.bar(stats.interaction_ready_by_turn[4], 10),
            s.dim("answers ready by turn 5"),
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
    if let Some(p50) = stats.p50_lethal_turn {
        println!(
            "  {}  {}  {}",
            s.bar(1.0, 10),
            s.dim("best-case lethal"),
            s.note(&format!(
                "half of games by turn {p50} — goldfish, unblocked: an upper bound"
            ))
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
        println!("{}", s.header("Mana sources & color gaps"));
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
            // Plain-English finding line: what is wrong, then the
            // jargon-free read. The kind names a stable JSON key; the
            // human line leads with the meaning.
            println!("  {}", s.error(&format!("{}: {}", p.kind, p.detail)));
            for offender in &p.offenders {
                let share = offender
                    .pct_games
                    .map(|pct| format!(" ({pct:.0}% of games)"))
                    .unwrap_or_default();
                println!(
                    "    {} {}",
                    s.card_name(&offender.name),
                    s.dim(&format!("{}{}", offender.detail, share))
                );
            }
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
