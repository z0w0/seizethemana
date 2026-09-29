//! Report renders for the simulator: the JSON payload and the human
//! stdout view. JSON adds station/bodies/engines metrics; the human view
//! gains a station line when the commander is a spacecraft.

use super::aggregate::SimStats;
use super::model::{Role, SimDeck};
use super::report_schema::*;
use crate::output::Output;

/// Lock-pieces in the deck (Role::Lock census for the deck shape).
fn lock_count(deck: &SimDeck) -> i64 {
    deck.library_cards()
        .filter(|c| c.role == Role::Lock)
        .count() as i64
}

/// Booster-pieces in the deck (equipment, auras, pump spells).
fn booster_count(deck: &SimDeck) -> i64 {
    deck.library_cards()
        .filter(|c| c.role == Role::Booster)
        .count() as i64
}

/// Total cards in the deck (library + commanders).
fn total_cards(deck: &SimDeck) -> usize {
    deck.library_len() + deck.commanders.len()
}

/// Average nonland CMC across the library.
fn avg_cmc(deck: &SimDeck) -> f64 {
    let nonlands: Vec<&super::model::SimCard> = deck
        .library_cards()
        .filter(|c| c.role != Role::Land)
        .collect();
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
/// itself refers to are report keys, which the reader sees in the same
/// response.
pub(super) fn assumptions(deck: &SimDeck) -> Vec<String> {
    let mut list = vec![
        "This is a best-case solitaire simulation. There are no opponents: nothing is countered, no removal is cast, no attacker is blocked, and board wipes never fire.".to_string(),
        "Lands enter the battlefield as their card text says. Cards that may enter untapped by paying life do so.".to_string(),
        "A creature's attack power uses its printed power when known; unknown powers and tokens count as 2.".to_string(),
        "Summoning sickness follows CR 302.6: a creature that entered this turn cannot attack or use its own tap-symbol abilities unless it has haste. A creature can still crew or station while sick. A Vehicle that becomes a creature this turn cannot attack unless it has haste.".to_string(),
        "Phyrexian pips ({W/P}) pay with 2 life each — on casts, commanders, graveyard replays, activations, kicker, and cycling; the goldfish preserves mana and pays life when it has room.".to_string(),
        "A card that doesn't untap during the untap step stays tapped; only its untap activations clear the tap (Basalt Monolith class).".to_string(),
        "A static mana grant (lands you control have {T}: Add one mana of any color) turns each matching permanent's tap into one any-color pip; the land's printed production does not stack on top.".to_string(),
        "Improvise and affinity cut the printed generic by one per artifact on the battlefield, never below zero.".to_string(),
        "Triggers limited to once each turn fire once even when the event repeats (two land drops in one turn).".to_string(),
        "Each draw engine draws its stated amount once per turn from the turn after it enters, no matter what is on the battlefield.".to_string(),
        "A draw engine that scales with the board ('draw a card for each enchantment you control') draws the number of matching permanents, at most 8. Shapes it cannot count draw 1.".to_string(),
        "A card that enters and draws ('When this creature enters, draw a card') draws only once per entry. It is a trigger, not an extra cast effect.".to_string(),
        "Landfall draw, token, life_loss, and search effects resolve when a land enters. Mana-producing landfall triggers are parsed but do not add mana to the pool.".to_string(),
        "Deck-shape counts use card roles from the card index. They are not raw counts of card types or a count of game events.".to_string(),
        "Unknown activation conditions and opponent-scoped triggers stay inert. Metalcraft remains modeled for supported mana sources.".to_string(),
        "Intervening 'if' clauses (rule 603.4) evaluate for descend, threshold, raid, ferocious, and metalcraft. Triggers with unsupported condition shapes stay inert.".to_string(),
        "A mana mode pays only its own supported costs and adds only its own mana. Unknown mandatory costs make the mode unavailable. Supported non-library effects on the mode also resolve.".to_string(),
        "Mana that cannot pay generic costs (Jegantha) can still pay its printed colored pips. It does not pay generic, hybrid, or X costs.".to_string(),
        "An additional land-play effect adds to the turn's land allowance and can be used later that turn. The game tracks how many lands have already been played.".to_string(),
        "The Monarch draws at the beginning of the end step when they are the Monarch, including the turn they gain the title before that step.".to_string(),
        "Saga chapters use scheduled lore counters. Other lore-counter placements and stack timing are not modeled. The postcombat main phase has no actions.".to_string(),
        "X-cost spells spend all leftover mana as X. Drain, draw, mill, tokens, reveal-permanents, and counter-power effects scale with that X. Spend-restricted mana that cannot legally fund the spell stays out of the X.".to_string(),
        "X spells the model cannot execute pay X = 1 and do nothing extra.".to_string(),
        "A split card such as 'Fire // Ice' is cast as its cheaper face. The other face counts for deck categories but its cast effects never happen.".to_string(),
        "One optional kicker cost is paid when spare mana covers it and life stays above any phyrexian kicker pips. Only drain amounts grow with the kick.".to_string(),
        "Casts that sacrifice a creature or pay life as an extra cost pay it; the body really leaves the battlefield.".to_string(),
        "Cards that create tokens create that many 2/2 bodies, at most 8 per effect. Card effects that create Treasure tokens bank the tokens as mana instead of bodies.".to_string(),
        "Treasure creation is tracked on its own Oracle effect. A Treasure effect banks mana; a separate creature-token effect on the same card still creates bodies.".to_string(),
        "Life loss follows the target shape (CR 119.3): 'each opponent' hits every opponent, 'target player' hits one player, and 'each player' also costs the goldfish. The table has three opponents in commander and one in 60-card formats.".to_string(),
        "Player damage and direct life loss use separate measures. Damage to creatures, planeswalkers, battles, and ambiguous 'any target' effects stays inert. Damage prevention and replacement effects, including infect, are not modeled.".to_string(),
        "The lethal census sums combat damage and burn/drain effects against the full table life (120 in commander, 20 in 60-card). It is an upper bound: real games have blockers, removal, and life gain.".to_string(),
        "Mill and discard fill the graveyard census and count as cards seen. A card returns from the graveyard at most once; no loops.".to_string(),
        "A wheel discards the whole hand, then draws seven. Loot is draw N, then discard N.".to_string(),
        "Removal is measured as capacity: how often an answer is in hand and affordable. Counterspells, targeted removal, bounce, and board wipes all count; wipes never resolve.".to_string(),
        "The removal count splits into targeted removal and board wipes in the deck shape.".to_string(),
        "Lands that may tap only for creature spells pay creature casts only. An artifact creature may be paid with creature-restricted or artifact-restricted mana alike.".to_string(),
        "A sacrifice outlet consumes a real untapped creature. With no creature available it does nothing.".to_string(),
        "A blink effect ('exile, then return') re-fires the card's enter triggers exactly once, the next turn.".to_string(),
        "Planeswalkers use one loyalty ability per turn: plus abilities gain loyalty, minus abilities spend it. Ultimates only report the turn they become affordable; they do not resolve.".to_string(),
        "Sagas resolve chapter I on entry, then one chapter per turn added at the start of the precombat main phase; unsupported chapter text does nothing. The saga leaves after its last chapter.".to_string(),
        "Mana engines that produce per spell cast ('add one mana for each spell you've cast this turn') pay out once per spell cast each turn while untapped.".to_string(),
        format!(
            "A capped {}-activation sequence is flagged as suspected infinite mana only when it adds mana. Free draws and other actions without positive mana do not count.",
            super::model::MAX_LOOP_PASSES
        ),
        "Equipment boosts only the creature it equips, after the equip cost is paid. Auras and other continuous buffs are not modeled.".to_string(),
        "A one-shot board buff ('creatures you control get +X/+X where X is the number of creatures you control') boosts that combat phase only, on the turn it enters, capped at 20 creatures.".to_string(),
        "An extra turn uses the next configured turn slot and runs the full turn pipeline, including untap, upkeep, draw, land, cast, activations, and combat. Chained turns are bounded by the configured turn count; per-turn metrics mark extra-turn slots.".to_string(),
        "Opponent-dependent mana, such as Fellwar Stone, is generic-only from turn 2. Kinnan adds one pip of the type its nonland source produced; the opponent-dependent approximation adds colorless.".to_string(),
        "Convoke taps untapped creatures for {1} each (or a matching colored pip); delve exiles graveyard cards for {1} generic each; twobrid {2/W} reads as 2 generic; snow {S} pays as colorless.".to_string(),
        "Storm copies the spell once per other spell cast this turn; the copies resolve the same draw, life-loss, and token effects.".to_string(),
        "Mobilize creates tapped-and-attacking Warriors that join the swing and are sacrificed at the end step; amass grows one Army body with +1/+1 counters; afterlife creates Spirit bodies on death.".to_string(),
        "The Ring temptations raise the emblem level: the strongest body is the Ring-bearer; level 2 attack-loots and level 4 combat damage drains each opponent for 3. Level 3 needs blockers and stays inert.".to_string(),
        "Empower Jace creates a Jace planeswalker token with '[-1]: Surveil 1' and '[-3]: Draw a card'; a board with no blockers never sees it attacked.".to_string(),
        "Power-up and exhaust activations fire once per game; Power-up's cost is reduced by the permanent's mana cost the turn it entered (CR 702.193b). Teamwork taps bodies with total power N when available.".to_string(),
        "Offspring pays its extra cost from leftover mana and adds a 1/1 body on entry; plot exiles a card now and casts it free on a later turn.".to_string(),
        "Explore draws a land off the library top or adds a +1/+1 counter to the best body; connive loots and adds a +1/+1 counter when a nonland card was discarded.".to_string(),
        "Living metal makes a Vehicle an artifact creature during your turn, so it attacks without crew. Read ahead Sagas start at their first chapter with a parsed effect.".to_string(),
        "The end step discards the highest-mana-value cards down to seven; a permanent with 'no maximum hand size' skips the discard.".to_string(),
        "Metalcraft gates mana abilities on three controlled artifacts. A card's own printed flashback or escape cost casts it from the graveyard, then exiles it (flashback) or exiles three other cards (escape); converge and rebound are not modeled.".to_string(),
        "Counters are split by kind: +1/+1 and -1/-1 annihilate in pairs (CR 122.3); charge counters feed station tiers and banked mana; loyalty feeds planeswalker abilities. Unmodeled counter kinds stay inert.".to_string(),
        "Keyword counters (flying, lifelink, deathtouch, …) grant their keyword. Granted keywords merge with printed ones and static grants at combat time; temporary keyword loss is not modeled.".to_string(),
        "Energy counters are player counters. They accrue from parsed 'you get {E}' clauses and pay parsed activation costs; energy sinks beyond their activation cost stay inert.".to_string(),
        "Proliferate adds one counter of each kind already present on every counter-bearing permanent and on the player's energy. Keyword counters do not stack.".to_string(),
        "A companion is the first bench card with the Companion keyword. Its condition is assumed legal; once per game, in a main phase, {3} puts it into hand.".to_string(),
        "Reconfigure gear counts as a creature body until it attaches, then stops being a creature (CR 702.151b). The generic mana option is the modeled reconfigure cost.".to_string(),
        "Cascade reveals cards from the library top and free-casts the first nonland card with lower printed mana value. It uses the normal spell-resolution path and does not chain. Uncast revealed cards go to the library bottom in reveal order.".to_string(),
        "Supported cycling pays its mana or life cost, discards the card, then draws. Landcycling searches for the named basic land type. The graveyard-creature exchange applies only to the player's graveyard and battlefield.".to_string(),
        "A land/spell MDFC (e.g. Valakut Awakening) is played as its land face when no other land drop is available, and cast as its spell face otherwise. Non-mythic MDFCs count 0.4 land, mythic 0.75.".to_string(),
        "Unsupported Oracle clauses stay inert. Other parsed costs, keywords, and abilities on the same card still apply.".to_string(),
        "Keywords that need an opponent or blockers stay inert by design: ward, shroud, protection, defender, exalted, provoke, and changeling. Temporary keyword loss and imprint are not modeled.".to_string(),
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

/// Round to two decimals, the report's standard precision.
fn round2(v: f64) -> f64 {
    super::report_schema::round2(v)
}

/// Turn-indexed averages with one-based typed keys.
fn turn_values(values: &[f64]) -> TurnValues {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| ((index + 1) as u32, round2(*value)))
        .collect()
}

/// Turn-indexed shares converted to public percentages at the report boundary.
fn percent_turn_values(values: &[f64]) -> std::collections::BTreeMap<u32, Percent> {
    values
        .iter()
        .enumerate()
        .map(|(index, share)| ((index + 1) as u32, Percent::from_share(*share)))
        .collect()
}

/// Convert the internal five-color array to the report's named color fields.
fn color_values(values: [f64; 5]) -> ColorValues {
    ColorValues {
        white: values[0],
        blue: values[1],
        black: values[2],
        red: values[3],
        green: values[4],
    }
}

/// Convert internal color shortage shares to public percentages.
fn color_percents(values: [f64; 5]) -> ColorPercents {
    ColorPercents {
        white: Percent::from_share(values[0]),
        blue: Percent::from_share(values[1]),
        black: Percent::from_share(values[2]),
        red: Percent::from_share(values[3]),
        green: Percent::from_share(values[4]),
    }
}

/// One-line summary of the run.
fn summary_line(stats: &SimStats, deck: &SimDeck, findings: &[super::findings::Finding]) -> String {
    let cmd_part = deck.commanders.first().map(|cmd| {
        let by = stats.commander_castable_by[(cmd.cost.total() as usize).min(12)];
        format!(" · commander on curve {:.0}%", by * 100.0)
    });
    let base = format!(
        "{} games · {} finding{}",
        stats.runs,
        findings.len(),
        if findings.len() == 1 { "" } else { "s" }
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
    findings: &[super::findings::Finding],
    bench: BenchCounts,
    mana_base: &super::findings::ManaBase,
) -> SimReport {
    let turns = stats.turns as usize;
    let lands = deck
        .library_cards()
        .filter(|c| c.role == Role::Land)
        .count() as i64;
    let artifact_mana_sources = deck
        .library_cards()
        .filter(|c| c.role == Role::Rock)
        .count() as i64;
    let creature_mana_sources = deck
        .library_cards()
        .filter(|c| c.role == Role::Dork)
        .count() as i64;
    let ramp = deck
        .library_cards()
        .filter(|c| c.role == Role::RampSpell)
        .count() as i64;
    let curve = ["0", "1", "2", "3", "4", "5", "6", "7+"]
        .into_iter()
        .map(|bucket| {
            let count = deck
                .library_cards()
                .filter(|c| c.role != Role::Land && curve_bucket(c.cost.total()) == bucket)
                .count() as i64;
            (bucket.to_string(), count)
        })
        .collect();
    let commander = deck.commanders.first().map(|cmd| {
        let cmc = cmd.cost.total() as usize;
        // "On curve" only means something when the sim ran long enough
        // to reach the commander's curve turn; a clamped short run
        // would read as a misleading 0%. Null past the horizon.
        let on_curve = (cmc <= turns && cmc >= 1)
            .then(|| Percent::from_share(stats.commander_castable_by[cmc.clamp(1, turns.min(12))]));
        CommanderReport {
            name: cmd.name.clone(),
            mana_value: cmd.cost.total() as f64,
            percent_castable_by_turn: percent_turn_values(
                &stats.commander_castable_by[1..=turns.min(12)],
            ),
            avg_first_cast_turn: round2(stats.avg_commander_cast_turn),
            p50_cast_turn: stats.p50_commander_cast_turn,
            p95_cast_turn: stats.p95_commander_cast_turn,
            percent_castable_by_curve: on_curve,
        }
    });
    let report_findings: Vec<Finding> = findings
        .iter()
        .map(|p| Finding {
            kind: p.kind.to_string(),
            identity: p.color.map_or_else(
                || p.kind.to_string(),
                |color| format!("{}:{}", p.kind, color.symbol()),
            ),
            severity: p.severity.to_string(),
            percent_of_games: p.game_share.map(Percent::from_share),
            color: p.color.map(|color| color.symbol().to_string()),
            explanation: p.explanation.clone(),
            suggestion: p.suggestion.clone(),
            evidence: p
                .evidence
                .iter()
                .map(|item| FindingEvidence {
                    subject: item.name.clone(),
                    explanation: item.explanation.clone(),
                    percent_of_games: item.game_share.map(Percent::from_share),
                })
                .collect(),
        })
        .collect();
    let station = if deck
        .commanders
        .first()
        .is_some_and(|cmd| cmd.animate_at().is_some())
    {
        Some(StationReport {
            online_by_t6: Percent::from_share(stats.station_online_pct),
            p50_online_turn: stats.station_p50_turn,
        })
    } else {
        None
    };
    let companion = if deck.companion.is_some() {
        Some(CompanionReport {
            in_hand_by_t6: Percent::from_share(stats.companion_online_pct),
            p50_online_turn: stats.companion_p50_turn,
        })
    } else {
        None
    };
    SimReport {
        name: name.to_string(),
        format: deck.rules.key.to_string(),
        runs: stats.runs,
        turns: stats.turns,
        seed,
        deck_shape: DeckShape {
            total_cards: total_cards(deck),
            average_nonland_mana_value: round2(avg_cmc(deck)),
            sideboard_cards: bench.sideboard_cards,
            maybeboard_cards: bench.maybeboard_cards,
            lands,
            artifact_mana_sources,
            creature_mana_sources,
            ramp_spells: ramp,
            draw_sources: stats.draw_count as i64,
            removal_spells: stats.removal_count as i64,
            targeted_removal_spells: stats.removal_targeted as i64,
            mass_removal_spells: stats.removal_wipes as i64,
            win_conditions: stats.wincon_count as i64,
            locks: lock_count(deck),
            boosters: booster_count(deck),
            curve,
        },
        assumptions: assumptions(deck),
        opening_hand: OpeningHand {
            avg_lands: round2(stats.avg_opener_lands),
            percent_0_lands: Percent::from_share(stats.opener_pct[0]),
            percent_1_land: Percent::from_share(stats.opener_pct[1]),
            percent_2_lands: Percent::from_share(stats.opener_pct[2]),
            percent_3_lands: Percent::from_share(stats.opener_pct[3]),
            percent_4_lands: Percent::from_share(stats.opener_pct[4]),
            percent_five_or_more_lands: Percent::from_share(stats.opener_pct[5]),
            percent_mulliganed: Percent::from_share(stats.mulligan_rate),
        },
        land_drops: LandDrops {
            percent_games_hitting_all_land_drops_by_turn:
                percent_turn_values(&stats.hit_all_drops_by[1..=turns.min(5)]),
            percent_games_with_two_or_fewer_lands_by_turn_4:
                Percent::from_share(stats.screw_pct),
            percent_games_with_six_or_more_lands_by_turn_4:
                Percent::from_share(stats.flood_pct),
            expected_percent_with_six_or_more_lands_by_turn_4:
                Percent::from_share(stats.flood_expectation),
            p50_drops_by_4: stats.p50_drops_by_4,
            p95_drops_by_4: stats.p95_drops_by_4,
        },
        mana_base: mana_base.clone(),
        commander,
        station,
        companion,
        creatures_by_turn: turn_values(&stats.creatures_by_turn[..turns]),
        repeatable_sources_by_turn: turn_values(&stats.repeatable_sources_by_turn[..turns]),
        mana: ManaMeasures {
            avg_unused_by_turn: turn_values(&stats.unused_mana[..turns]),
            percent_games_with_three_or_more_unused_mana_by_turn_6:
                Percent::from_share(stats.floated_pct),
        },
        draw: DrawMeasures {
            draw_source_percent_seen_by_turn: percent_turn_values(&stats.draw_sources[..turns]),
            percent_games_with_no_draw_source_by_turn_6: Percent::from_share(stats.starved_pct),
            avg_life_paid: round2(stats.life_paid_avg),
            avg_life_gained: round2(stats.life_gained_avg),
            avg_life_funded_draws: round2(stats.life_funded_draws_avg),
        },
        role_access: RoleAccess {
            removal_spell_percent_seen_by_turn_5: Percent::from_share(stats.removal_access_5),
            draw_source_percent_seen_by_turn_6: Percent::from_share(stats.draw_access_6),
            creature_role_percent_seen_by_turn_3: Percent::from_share(stats.creature_access_3),
            win_condition_role_percent_seen_by_turn_8: Percent::from_share(stats.wincon_access_8),
            lock_role_percent_seen_by_turn_3: Percent::from_share(stats.lock_access_3),
        },
        velocity: VelocityMeasures {
            avg_cards_seen: turn_values(&stats.cards_seen[..turns]),
            library_awareness_by_turn: percent_turn_values(
                &stats.library_awareness_by_turn[..turns],
            ),
            self_milled_by_turn: turn_values(&stats.self_milled_by_turn[..turns]),
            opponent_milled_by_turn: turn_values(&stats.opp_milled_by_turn[..turns]),
            library_remaining_by_turn: turn_values(&stats.library_by_turn[..turns]),
        },
        combat: CombatMeasures {
            attack_power_avg_by_turn: turn_values(&stats.attack_power_by_turn[..turns]),
            player_damage_avg_by_turn: turn_values(&stats.player_damage_by_turn[..turns]),
            attack_power_p90_by_t8: f64::from(stats.attack_power_p90),
            attackers_by_turn: turn_values(&stats.attackers_by_turn[..turns]),
            evasive_by_turn: turn_values(&stats.evasive_by_turn[..turns]),
        },
        win_conditions: WinMeasures {
            opponent_life_loss_by_turn: turn_values(&stats.opponent_life_loss_by_turn[..turns]),
            percent_games_with_extra_turn: Percent::from_share(stats.extra_turns_pct),
            win_threshold_p50_turn: stats.win_threshold_p50_turn,
            percent_games_reaching_counter_win_threshold:
                Percent::from_share(stats.win_threshold_pct),
            percent_games_with_affordable_ultimate: Percent::from_share(stats.ultimate_online_pct),
            percent_games_with_suspected_infinite_mana:
                Percent::from_share(stats.infinite_mana_pct),
            percent_games_at_or_above_table_life_by_turn:
                percent_turn_values(&stats.lethal_damage_by_turn[..turns]),
            p50_lethal_turn: stats.p50_lethal_turn,
            lethal_note: "best-case goldfish, unblocked: an upper bound; real games have blockers, removal, and life gain".to_string(),
        },
        interaction: InteractionMeasures {
            percent_games_with_ready_interaction_by_turn:
                percent_turn_values(&stats.interaction_ready_by_turn[..turns]),
            mana_held_avg: round2(stats.interaction_mana_held),
            instant_speed_count: stats.interaction_instant_count,
            note: "capacity, not events".to_string(),
        },
        color_mana_shortage: color_percents(stats.color_screw),
        color_pip_blocks: stats
            .pip_blocks
            .iter()
            .map(|block| ColorPipBlock {
                name: block.name.clone(),
                color: block.color.symbol().to_string(),
                percent_of_games: Percent::from_share(block.game_share),
            })
            .collect(),
        graveyard: GraveyardMeasures {
            avg_size_by_turn: turn_values(
                &stats.graveyard_by_turn[..turns.min(stats.graveyard_by_turn.len())],
            ),
            replay_casts_avg: round2(stats.replay_casts_avg),
        },
        milestones: Milestones {
            free_cast_permanents_entered_avg_by_turn:
                turn_values(&stats.free_cast_permanents_by_turn[..turns]),
            dredge_uses_avg_by_turn: turn_values(&stats.dredge_uses_by_turn[..turns]),
            graveyard_casts_avg_by_turn: turn_values(&stats.graveyard_casts_by_turn[..turns]),
            life_funded_draws_avg_by_turn: turn_values(&stats.life_funded_draws_by_turn[..turns]),
            percent_games_with_positive_mana_loop_by_turn:
                percent_turn_values(&stats.positive_mana_loop_by_turn[..turns]),
            note: "Action counts are events in that turn. Positive mana loop is the share of games that reached one during that turn.".to_string(),
        },
        color_sources: color_source_census(deck),
        card_castability: stats
            .card_castability
            .iter()
            .map(|card| CardCastability {
                name: card.name.clone(),
                mana_value: round2(card.cmc),
                target_turn: card.target_turn,
                percent_castable_by_target: Percent::from_share(card.pct_by_target),
                avg_first_castable_turn: round2(card.avg_first_castable_turn),
            })
            .collect(),
        findings: report_findings,
        summary: summary_line(stats, deck, findings),
        combo_access: None,
        colored_sources: None,
        combos: None,
        win_paths: None,
        hypgeo: None,
    }
}

/// Per-color mana-source census over the deck's lands and rocks.
///
/// Static analysis (no simulation): for each WUBRG color, how many
/// sources can produce it and by which shape (fixed pips, choice,
/// any-color). The `fixing` array counts sources producing 2+ colors.
fn color_source_census(deck: &SimDeck) -> ColorSourceCensus {
    use super::model::Role;
    let mut fixed = [0u32; 5];
    let mut choice = [0u32; 5];
    let mut flexible_nonland = 0u32;
    let mut scaling = 0u32;
    for card in deck.library_cards() {
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
    ColorSourceCensus {
        fixed_source_lands: color_values(fixed.map(f64::from)),
        choice_source_lands: color_values(choice.map(f64::from)),
        flexible_nonland_sources: flexible_nonland,
        scaling_sources: scaling,
        note: "static census of land tap yields; a choice land serves every color it can pick; flexible nonland sources count rocks and dorks".to_string(),
    }
}

/// Human report on stdout: overview block, worst castability, findings, note.
pub fn print_report(out: &Output, report: &SimReport) {
    let s = out.styles();
    let turns = report.turns as usize;
    let turn = turns.clamp(1, 6) as u32;
    let get_turn = |values: &TurnValues, by: u32| values.get(&by).copied().unwrap_or_default();
    println!(
        "{}  {}  {}",
        s.header(&report.name),
        s.dim(&format!(
            "{} games · {} turns",
            s.thousands(i64::from(report.runs)),
            report.turns
        )),
        s.dim(&format!(
            "{} cards · {} lands · {} ramp · average mana value {:.1}",
            report.deck_shape.total_cards,
            report.deck_shape.lands,
            report.deck_shape.artifact_mana_sources
                + report.deck_shape.creature_mana_sources
                + report.deck_shape.ramp_spells,
            report.deck_shape.average_nonland_mana_value
        ))
    );
    println!();
    println!(
        "  {}  {}  {:.1} lands avg · mulligan {:.1}%",
        s.bar(report.opening_hand.avg_lands / 7.0, 10),
        s.dim("opening hand"),
        report.opening_hand.avg_lands,
        report.opening_hand.percent_mulliganed.value()
    );
    if turns >= 4 {
        println!(
            "  {}  {}  {:.1}% hit all 4 · short {:.1}% · flooded {:.1}%",
            s.bar(
                report
                    .land_drops
                    .percent_games_hitting_all_land_drops_by_turn
                    .get(&4)
                    .map_or(0.0, |percent| percent.value() / 100.0),
                10,
            ),
            s.dim("land drops by turn 4"),
            report
                .land_drops
                .percent_games_hitting_all_land_drops_by_turn
                .get(&4)
                .map_or(0.0, |percent| percent.value()),
            report
                .land_drops
                .percent_games_with_two_or_fewer_lands_by_turn_4
                .value(),
            report
                .land_drops
                .percent_games_with_six_or_more_lands_by_turn_4
                .value()
        );
        let mana_base_line = match report.mana_base.bracket {
            Some(bracket) => format!(
                "{} lands · {} ramp · target {}-{} lands + {}-{} ramp · bracket {}",
                report.mana_base.lands,
                report.mana_base.artifact_mana_sources
                    + report.mana_base.creature_mana_sources
                    + report.mana_base.ramp_spells,
                report.mana_base.bracket_target_lands[0],
                report.mana_base.bracket_target_lands[1],
                report.mana_base.bracket_target_ramp[0],
                report.mana_base.bracket_target_ramp[1],
                bracket,
            ),
            None => format!(
                "{} lands · {} ramp · target {}-{} lands",
                report.mana_base.lands,
                report.mana_base.artifact_mana_sources
                    + report.mana_base.creature_mana_sources
                    + report.mana_base.ramp_spells,
                report.mana_base.bracket_target_lands[0],
                report.mana_base.bracket_target_lands[1],
            ),
        };
        println!(
            "  {}  {}  {}",
            s.dim("mana base"),
            s.dim(&mana_base_line),
            s.note(&report.mana_base.verdict)
        );
    }
    if let Some(cmd) = &report.commander {
        let mana_value = cmd.mana_value as u32;
        let by = cmd
            .percent_castable_by_turn
            .get(&mana_value)
            .map_or(0.0, |percent| percent.value() / 100.0);
        println!(
            "  {}  {}  {:.1}% by turn {mana_value} · p50 t{} · p95 t{}",
            s.bar(by, 10),
            s.dim(&format!("commander (costs {mana_value} mana)")),
            by * 100.0,
            cmd.p50_cast_turn,
            cmd.p95_cast_turn
        );
        if turns >= 6
            && let Some(station) = &report.station
        {
            println!(
                "  {}  {}  {:.1}% by turn 6 · half of games by turn {}",
                s.bar(station.online_by_t6.value() / 100.0, 10),
                s.dim("station online"),
                station.online_by_t6.value(),
                station.p50_online_turn
            );
        }
    }
    if let Some(companion) = &report.companion
        && turns >= 6
    {
        println!(
            "  {}  {}  {:.1}% by turn 6 · half of games by turn {}",
            s.bar(companion.in_hand_by_t6.value() / 100.0, 10),
            s.dim("companion in hand"),
            companion.in_hand_by_t6.value(),
            companion.p50_online_turn
        );
    }
    let unused = get_turn(&report.mana.avg_unused_by_turn, turn);
    let seen = get_turn(&report.velocity.avg_cards_seen, turn);
    let creatures = get_turn(&report.creatures_by_turn, turn);
    println!(
        "  {}  {}  {:.1} avg unspent · {:.1} cards seen · {:.1} creatures",
        s.bar((unused / 5.0).min(1.0), 10),
        s.dim(&format!("mana through turn {turn}")),
        unused,
        seen,
        creatures
    );
    if turns >= 5 {
        println!(
            "  {}  {}  {:.1}% of games",
            s.bar(
                report
                    .role_access
                    .removal_spell_percent_seen_by_turn_5
                    .value()
                    / 100.0,
                10
            ),
            s.dim("removal seen by turn 5"),
            report
                .role_access
                .removal_spell_percent_seen_by_turn_5
                .value()
        );
    }
    if turns >= 6 {
        println!(
            "  {}  {}  {:.1}% of games · {:.1}% starved",
            s.bar(
                report
                    .role_access
                    .draw_source_percent_seen_by_turn_6
                    .value()
                    / 100.0,
                10
            ),
            s.dim("draw source by turn 6"),
            report
                .role_access
                .draw_source_percent_seen_by_turn_6
                .value(),
            report
                .draw
                .percent_games_with_no_draw_source_by_turn_6
                .value()
        );
    }
    if report.interaction.instant_speed_count > 0 && turns >= 5 {
        println!(
            "  {}  {}  {:.1}% · instant-speed {} · held {:.1}",
            s.bar(
                report
                    .interaction
                    .percent_games_with_ready_interaction_by_turn
                    .get(&5)
                    .map_or(0.0, |percent| percent.value() / 100.0),
                10
            ),
            s.dim("answers ready by turn 5"),
            report
                .interaction
                .percent_games_with_ready_interaction_by_turn
                .get(&5)
                .map_or(0.0, |percent| percent.value()),
            report.interaction.instant_speed_count,
            report.interaction.mana_held_avg
        );
    }
    let attack_power = get_turn(&report.combat.attack_power_avg_by_turn, turn);
    let evasive = get_turn(&report.combat.evasive_by_turn, turn);
    if attack_power > 0.0 {
        println!(
            "  {}  {}  {:.0} avg power · {:.0} p90 t8 · {:.1}/{:.0} evasion",
            s.bar((attack_power / 40.0).min(1.0), 10),
            s.dim("attack power"),
            attack_power,
            report.combat.attack_power_p90_by_t8,
            evasive,
            creatures
        );
    }
    let opponent_life_loss = get_turn(&report.win_conditions.opponent_life_loss_by_turn, turn);
    if opponent_life_loss > 0.0 {
        println!(
            "  {}  {}  {:.1} opponent life lost by t{}",
            s.bar((opponent_life_loss / 30.0).min(1.0), 10),
            s.dim("drain"),
            opponent_life_loss,
            turn
        );
    }
    if let Some(p50) = report.win_conditions.p50_lethal_turn {
        println!(
            "  {}  {}  {}",
            s.bar(1.0, 10),
            s.dim("best-case lethal"),
            s.note(&format!(
                "half of games by turn {p50} — goldfish, unblocked: an upper bound"
            ))
        );
    }
    if report.win_conditions.percent_games_with_extra_turn.value() > 0.0 {
        println!(
            "  {}  {}  {:.1}% of games",
            s.bar(
                report.win_conditions.percent_games_with_extra_turn.value() / 100.0,
                10
            ),
            s.dim("games taking an extra turn"),
            report.win_conditions.percent_games_with_extra_turn.value()
        );
    }
    if report
        .win_conditions
        .percent_games_reaching_counter_win_threshold
        .value()
        > 0.0
    {
        println!(
            "  {}  {}  {:.1}% by t10 · p50 t{}",
            s.bar(
                report
                    .win_conditions
                    .percent_games_reaching_counter_win_threshold
                    .value()
                    / 100.0,
                10
            ),
            s.dim("win threshold online"),
            report
                .win_conditions
                .percent_games_reaching_counter_win_threshold
                .value(),
            report.win_conditions.win_threshold_p50_turn
        );
    }
    if report
        .win_conditions
        .percent_games_with_affordable_ultimate
        .value()
        > 0.0
    {
        println!(
            "  {}  {}  {:.1}% by t10",
            s.bar(
                report
                    .win_conditions
                    .percent_games_with_affordable_ultimate
                    .value()
                    / 100.0,
                10
            ),
            s.dim("ultimate online"),
            report
                .win_conditions
                .percent_games_with_affordable_ultimate
                .value()
        );
    }
    let mut worst: Vec<&CardCastability> = report.card_castability.iter().collect();
    worst.sort_by(|a, b| {
        a.percent_castable_by_target
            .value()
            .partial_cmp(&b.percent_castable_by_target.value())
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
                s.dim(&format!("mana value {:.0}", card.mana_value)),
                card.target_turn,
                card.percent_castable_by_target.value()
            );
        }
    }
    // Mana-source census + worst pip blockers: the color-screw story in
    // two lines (what the deck has, which card got hurt).
    if [
        report.color_mana_shortage.white,
        report.color_mana_shortage.blue,
        report.color_mana_shortage.black,
        report.color_mana_shortage.red,
        report.color_mana_shortage.green,
    ]
    .iter()
    .any(|p| p.value() >= 10.0)
        && !report.color_pip_blocks.is_empty()
    {
        println!();
        println!("{}", s.header("Mana sources & color gaps"));
        for p in report.color_pip_blocks.iter().take(3) {
            println!(
                "    {}  {} pips missed in {:.0}% of games",
                s.card_name(&p.name),
                p.color,
                p.percent_of_games.value()
            );
        }
    }
    if !report.findings.is_empty() {
        println!();
        println!("{}", s.header("Findings"));
        for p in &report.findings {
            // Plain-English finding line: what is wrong, then the
            // jargon-free read. The kind names a stable JSON key; the
            // human line leads with the meaning.
            println!("  {}", s.error(&p.explanation));
            for offender in &p.evidence {
                let share = offender
                    .percent_of_games
                    .map(|percent| format!(" ({:.0}% of games)", percent.value()))
                    .unwrap_or_default();
                println!(
                    "    {} {}",
                    s.card_name(&offender.subject),
                    s.dim(&format!("{}{}", offender.explanation, share))
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
