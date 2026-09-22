// Oracle-text parsing for the simulator: the whole intelligence of the
// model lives here. Each card's oracle text is scanned for the shapes the
// model executes (tap yields, station tiers, crew, ETB/upkeep/attack/cast
// triggers, activations, cost reductions, enters-tapped, verge gates,
// Leyline openers, sagas). Shapes the parser cannot read become plain
// cards with no abilities, which keeps unknown cards playable but passive.
//
// Deliberately not parsed (documented in output assumptions): improvise
// beyond a flat discount, converge, energy, metalcraft, X-scaling,
// proliferate chains, and replay mechanics (flashback, rebound, warp
// beyond the one cheaper cost).

use super::super::stats::{is_dork, is_land, is_rock};
use super::model::{Ability, Cost, Effect, SimCard, TapYield, Tier, draw_amount};
pub use super::parse_cost::{parse_activation_cost, parse_cost, parse_cost_faces};
use super::parse_land::{charge_counters_on_cast, enters_tapped, parse_enter_counters};
pub use super::parse_land::{parse_gates, parse_tap, parse_tap_filtered, parse_tap_yield};
use super::triggers::{mill_amount, parse_triggers};
use crate::db::CardRow;

/// Parse station tiers and the station-card flag from oracle text.
///
/// The reminder text lists striations as `N+ |` segments; each segment's
/// abilities belong to that tier. The animation threshold comes from the
/// "It's an artifact creature at N+" note in the station reminder (planets
/// never animate: no P/T box).
pub fn parse_station_tiers(oracle_text: &str, type_line: &str) -> (Vec<Tier>, bool) {
    let is_station_card = type_line.contains("Spacecraft") || type_line.contains("Planet");
    let mut tiers: Vec<Tier> = Vec::new();
    for segment in oracle_text.split('\n') {
        let seg = segment.trim();
        let Some((head, rest)) = seg.split_once('|') else {
            continue;
        };
        let head_trim = head.trim().trim_end_matches('+').trim();
        let Ok(n) = head_trim.parse::<u32>() else {
            continue;
        };
        let mut tier = Tier {
            at: n,
            animate: false,
            abilities: Vec::new(),
        };
        for part in rest.split('\n') {
            if let Some(ab) = parse_ability(part.trim()) {
                tier.abilities.push(ab);
            }
        }
        tiers.push(tier);
    }
    if let Some(animate_at) = animate_threshold(oracle_text) {
        if let Some(tier) = tiers.iter_mut().find(|t| t.at == animate_at) {
            tier.animate = true;
        } else {
            tiers.push(Tier {
                at: animate_at,
                animate: true,
                abilities: Vec::new(),
            });
        }
    }
    tiers.sort_by_key(|t| t.at);
    (tiers, is_station_card)
}

/// Find the station animation threshold from the reminder text
/// ("It's an artifact creature at 12+."), None when absent (planets).
fn animate_threshold(oracle_text: &str) -> Option<u32> {
    let lower = oracle_text.to_ascii_lowercase();
    let idx = lower.find("artifact creature at ")?;
    let tail = &lower[idx + "artifact creature at ".len()..];
    let num: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse::<u32>().ok()
}

/// True when the oracle text carries a creature/permanent-target damage
/// spell shape ("deals N damage to target creature", N >= 2): removal
/// capacity for the interaction grammar. Player-targeted burn is drain,
/// trigger sentences are not one-shot spells, "each creature" burn is a
/// sweep (not targeted capacity), and activated shapes ("{T}: ...") are
/// not cast-time removal.
pub fn damage_removal_shape(oracle_text: &str) -> bool {
    oracle_text.split(['.', '\n', ',']).any(|seg| {
        let lower = seg.trim().to_ascii_lowercase();
        if lower.starts_with("whenever") || lower.starts_with("when ") || lower.contains(": ") {
            return false;
        }
        lower
            .split_once(" deals ")
            .and_then(|(_, tail)| {
                let (amount, after) = tail.split_once(" damage ")?;
                let n: u32 = amount
                    .trim()
                    .split(' ')
                    .next()
                    .and_then(|w| w.parse().ok())
                    .unwrap_or(0);
                if n < 2 {
                    return Some(false);
                }
                // Player-only burn is drain, creature/permanent-target
                // damage is removal capacity. "Any target" can hit a
                // creature, so it counts as capacity too; multi-target
                // divided damage does as well.
                let player_only = after.starts_with("to target player")
                    || after.starts_with("to target opponent")
                    || after.starts_with("to each opponent")
                    || after.starts_with("to each player")
                    || after.starts_with("to each creature")
                    || after.starts_with("to each permanent");
                let qualifies = after.contains("target") || after.starts_with("to any target");
                Some(qualifies && !player_only)
            })
            .unwrap_or(false)
    })
}

/// Parse one oracle segment into an executable ability when it matches a
/// known shape. Everything else returns None (dropped silently).
pub fn parse_ability(segment: &str) -> Option<Ability> {
    // Activated/loyalty shape: "{1}, {T}: Draw two cards", "−3: Search…".
    let (cost_part, effect_part) = segment.split_once(':')?;
    let cost = parse_activation_cost(cost_part.trim());
    // Loyalty cost: a leading minus ("−3", "-2") spends loyalty. A plus
    // ("+1") gains it.
    let cleaned = cost_part.replace(['−', '–'], "-").trim().to_string();
    let loyalty_cost = cleaned
        .strip_prefix('-')
        .and_then(|d| d.parse::<u32>().ok())
        .filter(|_| cleaned.chars().all(|c| c.is_ascii_digit() || c == '-'))
        .unwrap_or(0);
    let loyalty_gain = cleaned
        .strip_prefix('+')
        .and_then(|d| d.parse::<u32>().ok())
        .filter(|_| cleaned.chars().all(|c| c.is_ascii_digit() || c == '+'))
        .unwrap_or(0);
    let lower_effect = effect_part.to_ascii_lowercase();
    let effect = if lower_effect.contains("add ") || lower_effect.contains("add {") {
        Effect::Mana(parse_tap_yield(effect_part)?)
    } else if lower_effect.contains("draw") && lower_effect.contains("discard") {
        // Loot activations ("Draw a card, then discard a card").
        Effect::Loot(draw_amount(&lower_effect).max(1))
    } else if lower_effect.contains("mill ") {
        Effect::Mill(mill_amount(&lower_effect))
    } else if lower_effect.contains("from your graveyard") && lower_effect.contains("return") {
        Effect::ReturnFromGraveyard {
            to_hand: lower_effect.contains("to your hand")
                || lower_effect.contains("onto the battlefield"),
            count: 1,
        }
    } else if lower_effect.contains("draw") {
        Effect::Draw(draw_amount(&lower_effect).max(1))
    } else if lower_effect.contains("search your library")
        || lower_effect.contains("put it into your hand")
    {
        Effect::Tutor
    } else if lower_effect.contains("create") && lower_effect.contains("token") {
        Effect::Tokens(super::triggers::token_amount(&lower_effect))
    } else if lower_effect.contains("put a charge counter") {
        // "Put a charge counter": one counter per activation (Coalition
        // Relic). The Drill Too Deep shape ("put five charge counters") is
        // a spell, handled by `charge_counters_on_cast`.
        Effect::Counters(1)
    } else if lower_effect.contains("take an extra turn") {
        Effect::ExtraTurn
    } else if lower_effect.contains("scry") {
        // Activated scry ("{T}: Scry 2"). Awareness credit, not draw.
        Effect::Scry(super::model::amount_after(&lower_effect, "scry"))
    } else if lower_effect.contains("surveil") {
        Effect::Scry(super::model::amount_after(&lower_effect, "surveil"))
    } else if (lower_effect.contains("target player loses")
        || lower_effect.contains("each opponent loses")
        || lower_effect.contains("opponent loses"))
        && lower_effect.contains("life")
        && !(lower_effect.contains("sacrifice") && lower_effect.contains("opponent"))
    {
        Effect::Drain(super::model::amount_after(&lower_effect, "loses").max(1))
    } else if lower_effect.contains("remove a charge counter") && lower_effect.contains("add") {
        // Banked activation (Pentad Prism): one counter buys one pip.
        parse_tap_yield(effect_part).map_or(Effect::None, Effect::Mana)
    } else if lower_effect.contains("put x")
        && (lower_effect.contains("counter") || lower_effect.contains("tower"))
        && cost.total() == 1
    {
        // Mana-sink counters ("{X}: Put X tower counters on Helix
        // Pinnacle"): the leftover pool converts to counters so the
        // win threshold can be reached. Modeled as Counters(0): the
        // game loop substitutes the paid amount ({X} parses as 1).
        Effect::Counters(0)
    } else {
        return None;
    };
    Some(Ability {
        trigger: super::model::Trigger::Activated,
        cost,
        effect,
        taps: cost_part.to_ascii_lowercase().contains("{t}"),
        uses_counters: cost_part
            .to_ascii_lowercase()
            .contains("remove a charge counter"),
        // "Sacrifice a creature" / "sacrifice this creature" in the cost
        // consumes a body (aristocrats outlets).
        sacrifice_bodies: u32::from(
            cost_part.to_ascii_lowercase().contains("sacrifice")
                && cost_part.to_ascii_lowercase().contains("creature"),
        ),
        // "Activate only once each turn" on the card bounds the free
        // activation: no looping, no infinite-mana census flag.
        once_per_turn: segment.to_ascii_lowercase().contains("only once each turn")
            || segment
                .to_ascii_lowercase()
                .contains("only once each of your turns")
            || segment
                .to_ascii_lowercase()
                .contains("triggers only once each turn"),
        loyalty_cost,
        loyalty_gain,
    })
}

/// Token count from a "create N …tokens" tail. Delegates to the
/// trigger-family counter; 0 when the text creates nothing.
fn token_amount_after(text: &str) -> u32 {
    let lower = text.to_ascii_lowercase();
    if !lower.contains("token") {
        return 0;
    }
    super::triggers::token_amount(&lower)
}

/// Parse a card row into the simulator's data model.
pub fn parse_sim_card(row: &CardRow) -> SimCard {
    let text = row.oracle_text.to_ascii_lowercase();
    let type_line = row.type_line.to_string();
    let land = is_land(row);
    // Land/spell MDFC: one face is a Land, the other a castable spell.
    // The spell face is the playable cast; the land face plays when the
    // hand holds no other land.
    let mythic = row.rarity == "mythic";
    let is_mdfc_spell = row
        .type_line
        .split(" // ")
        .any(|face| face.split('—').next().unwrap_or("").contains("Land"))
        && row
            .type_line
            .split(" // ")
            .any(|face| !face.split('—').next().unwrap_or("").contains("Land"));
    // MDFC spell faces keep their cast cost; plain lands cost nothing.
    let cost = if land && !is_mdfc_spell {
        Cost::default()
    } else {
        parse_cost_faces(&row.mana_cost)
    };

    // Crew: keyword list plus the "Crew N" reminder text.
    let crew = if row.keywords.contains("Crew") {
        Some(
            text.find("crew ")
                .and_then(|i| {
                    text[i + 5..]
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse::<u32>()
                        .ok()
                })
                .unwrap_or(1),
        )
    } else {
        None
    };

    // Tap yields: `{T}: Add …` segments, merged across abilities since a
    // permanent taps once per turn (Plaza of Heroes, Relic of Legends).
    let tap = parse_tap(row);

    // Station tiers (Spacecraft and Planet cards).
    let (tiers, is_station_card) = if text.contains("station") {
        parse_station_tiers(&row.oracle_text, &row.type_line)
    } else {
        (Vec::new(), false)
    };

    // Enters-tapped: oracle shapes; "unless" conditions that self-solve
    // early ("unless you control two or fewer other lands") stay untapped
    // early, so they are not marked tapped.
    let enters_tapped = land && enters_tapped(&text);

    // Verge gates: "Activate only if you control a Swamp or a Mountain."
    let gate_types = if land {
        parse_gates(&row.oracle_text)
    } else {
        Vec::new()
    };

    // Leyline-style opener.
    let opens_in_play = text.contains("begin the game with it on the battlefield");

    // Abilities outside station tiers: triggers from oracle text.
    let abilities = parse_triggers(&row.oracle_text);
    let mut station_tiers = tiers;
    // Saga chapters parse into tier-0 Activated abilities, one per
    // chapter, consumed by the saga staging in the turn loop.
    let is_saga = type_line.contains("Saga") && !land;
    if is_saga {
        let chapters = parse_saga_chapters(&row.oracle_text);
        // Non-chapter abilities (ETB and other triggers on the saga
        // card) register alongside the chapters; `chapter_count` counts
        // only Activated abilities, so extra triggers stay out of the
        // chapter index. Shapes the model cannot express are dropped at
        // parse time like any other card.
        let mut tier_abilities: Vec<Ability> = chapters
            .into_iter()
            .map(|effect| Ability {
                trigger: super::model::Trigger::Activated,
                effect,
                ..Ability::default()
            })
            .collect();
        tier_abilities.extend(abilities);
        if !tier_abilities.is_empty() {
            station_tiers.insert(
                0,
                Tier {
                    at: 0,
                    animate: false,
                    abilities: tier_abilities,
                },
            );
        }
    } else if !abilities.is_empty() {
        station_tiers.insert(
            0,
            Tier {
                at: 0,
                animate: false,
                abilities,
            },
        );
    }

    // Cast-time reductions (approximations documented in assumptions).
    // Improvise and affinity scale with the artifact count on the board;
    // the min_cost here keeps a conservative flat −2 as the floor.
    let mut min_cost = cost.clone();
    let mut board_discount = false;
    if text.contains("warp ") {
        let warp = text
            .split("warp ")
            .nth(1)
            .and_then(|rest| rest.split(['(', '.', '\n']).next())
            .map(str::trim)
            .map(parse_cost);
        if let Some(warp) = warp
            && warp.total() < min_cost.total()
        {
            min_cost = warp;
        }
    }
    // Reductions stack with affinity: the reminder text of "Affinity for
    // X" reads "costs {1} less to cast for each X", which would fire both
    // branches. Affinity wins (it scales with the board); the flat floor
    // applies only when affinity did not.
    let affinity = text.contains("improvise") || text.contains("affinity");
    if (text.contains("costs {1} less") || text.contains("costs {x} less")) && !affinity {
        min_cost.generic = min_cost.generic.saturating_sub(2);
    }
    if affinity {
        min_cost.generic = min_cost.generic.saturating_sub(2);
        board_discount = true;
    }

    let enter_counters = parse_enter_counters(&text);

    // One-shot effects on cast. The gates read `!land || is_mdfc_spell`:
    // a land/spell MDFC's spell face is a real cast, so its riders fire.
    let cast_face = !land || is_mdfc_spell;
    let mana_on_cast = if cast_face && tap.is_none() && text.contains("add ") {
        row.oracle_text
            .split(['.', '\n'])
            .map(str::trim)
            .find_map(|seg| {
                let lower = seg.to_ascii_lowercase();
                (lower.contains("add ") && !lower.contains("draw") && !lower.contains("search"))
                    .then(|| parse_tap_yield(seg))
                    .flatten()
            })
    } else {
        None
    };
    let draws_on_cast = if cast_face {
        row.oracle_text
            .split(['.', '\n'])
            .map(str::trim)
            .find_map(|seg| {
                let lower = seg.to_ascii_lowercase();
                // Enter-trigger shapes ("When this creature enters, draw")
                // are ETB triggers, not cast riders: they already parse as
                // OnEnter abilities and double count if credited here.
                let enter_trigger = (lower.starts_with("when ") || lower.starts_with("whenever "))
                    && lower.contains("enters");
                // "When you cast this spell, draw a card" self-shapes are
                // one-shot riders: the cast itself resolves them.
                let self_cast = lower.starts_with("when you cast this spell");
                let draws = ((lower.starts_with("draw ")
                    || lower.contains(", draw ")
                    || lower.starts_with("investigate")
                    || lower.contains("then draw"))
                    && !lower.contains("whenever")
                    && !lower.contains("at the beginning")
                    && !lower.contains(": ")
                    && !enter_trigger)
                    || self_cast;
                if draws {
                    Some(draw_amount(&lower))
                } else {
                    None
                }
            })
            .unwrap_or(0)
    } else {
        0
    };
    let counters_on_cast = charge_counters_on_cast(&text);

    // One-shot scry/surveil on cast ("Scry 2", "Surveil 1") — awareness
    // credit, not draw.
    let scry_on_cast = if cast_face {
        row.oracle_text
            .split(['.', '\n'])
            .map(str::trim)
            .find_map(|seg| {
                let lower = seg.to_ascii_lowercase();
                // Enter-trigger shapes ("When this creature enters,
                // surveil 2") are ETB triggers, not cast riders: they
                // already parse as OnEnter Scry abilities and would
                // double-count awareness if credited here too.
                let enter_trigger = lower.starts_with("when ") || lower.starts_with("whenever ");
                if lower.contains("surveil") && !lower.contains("whenever") && !enter_trigger {
                    return Some(super::model::amount_after(&lower, "surveil").max(1));
                }
                if lower.starts_with("scry ") && !lower.contains("whenever") && !enter_trigger {
                    return Some(super::model::amount_after(&lower, "scry"));
                }
                None
            })
            .unwrap_or(0)
    } else {
        0
    };
    let surveils = cast_face && text.contains("surveil");

    // One-shot extra-turn spells ("Take an extra turn after this one").
    let extra_turns_on_cast = cast_face && text.contains("take an extra turn");

    // Mill direction: opponent mills name a target player ("target
    // player mills N", "each opponent mills N").
    let mills_opponent = super::model::mills_opponent(&text);

    // One-shot drain spells ("Deals N damage to target player/opponent",
    // "each opponent loses N life"). Creature-target burn stays removal.
    let drain_on_cast = if cast_face {
        row.oracle_text
            .split(['.', '\n'])
            .map(str::trim)
            .find_map(|seg| {
                let lower = seg.to_ascii_lowercase();
                let player_scope = lower.contains("target player")
                    || lower.contains("target opponent")
                    || lower.contains("each opponent");
                let loses = (lower.contains("loses") && lower.contains("life"))
                    || lower.contains("deals") && lower.contains("damage to");
                if player_scope && loses {
                    let from_loses = super::model::amount_after(&lower, "loses ");
                    let from_deals = super::model::amount_after(&lower, "deals ");
                    Some(from_loses.max(from_deals).max(1))
                } else {
                    None
                }
            })
            .unwrap_or(0)
    } else {
        0
    };

    // X-cost effect class: "target player loses X life" (Drain), "draw X
    // cards" (Draw), "mill X" (Mill), "create X … creature tokens"
    // (Tokens). Oracle text writes the X bare ("loses X life"), so the
    // check is the mana cost carrying {X} plus the effect word.
    let has_x_cost = row.mana_cost.to_ascii_uppercase().contains("{X}");
    let x_class = if cast_face && has_x_cost {
        if (text.contains("loses x life")
            || text.contains("each opponent loses x")
            || text.contains("deals x damage"))
            && (text.contains("target player") || text.contains("opponent"))
        {
            Some(super::model::XClass::Drain)
        } else if text.contains("draw x") || text.contains("draws x") {
            Some(super::model::XClass::Draw)
        } else if text.contains("mill x") {
            Some(super::model::XClass::Mill)
        } else if text.contains("create x") && text.contains("token") {
            Some(super::model::XClass::Tokens)
        } else if text.contains("reveal the top x")
            && text.contains("permanent")
            && (text.contains("put any number") || text.contains("onto the battlefield"))
        {
            Some(super::model::XClass::RevealPermanents)
        } else if enters_with_x_counters(&text) {
            Some(super::model::XClass::Counters)
        } else {
            None
        }
    } else {
        None
    };

    // Repeatable per-cast mana: "add {N} for each spell you've cast this
    // turn". Fires per spell cast while the host
    // is untapped, in the cast phase.
    let mana_per_cast = if cast_face
        && text.contains("add ")
        && (text.contains("for each spell you've cast")
            || text.contains("spell you've cast this turn"))
    {
        row.oracle_text
            .split(['.', '\n'])
            .map(str::trim)
            .find_map(|seg| {
                let lower = seg.to_ascii_lowercase();
                (lower.contains("spell you've cast") && lower.contains("add "))
                    .then(|| parse_tap_yield(seg))
                    .flatten()
            })
    } else {
        None
    };
    // The per-cast engine's own tap segment reads as a plain "{T}: add N"
    // tap too; when the engine parsed, drop the plain tap so the two
    // modes do not double count (the real card's tap yields the
    // per-cast amount, not one plus it).
    let tap = if mana_per_cast.is_some() {
        parse_tap_filtered(row)
    } else {
        tap
    };

    // Kicker/multikicker: an optional extra cost ("Kicker {2}"). The
    // game loop pays it from leftover mana when affordable; the drain
    // rider bumps the amount.
    let kicker = if text.contains("kicker ") {
        text.split("kicker ")
            .nth(1)
            .and_then(|rest| rest.split(['(', '.', '\n', ',']).next())
            .map(str::trim)
            .map(parse_cost)
            .map(|c| c.total())
            .filter(|n| *n > 0)
    } else {
        None
    };

    // Printed power for crew/station math; "*" and unknowns stay None
    // (flat body power). Tokens never carry a row.
    let printed_power = row
        .power
        .as_deref()
        .and_then(|p| p.trim_end_matches('*').parse::<u32>().ok());

    // Cascade (and battle-cascade wording): one free cast of the
    // cheapest cheaper card from the library, no chaining.
    let has_cascade = text.contains("cascade") || text.contains("battle-cascade");

    // Starting loyalty for planeswalkers (the CardRow loyalty column).
    let starting_loyalty = row
        .loyalty
        .as_deref()
        .and_then(|l| l.trim().parse::<u32>().ok());

    // One-shot mill on entering ("mill N" ETB without a trigger segment).
    let mills_on_enter = if cast_face && parse_triggers(&row.oracle_text).is_empty() {
        mill_amount(&text).max(super::model::amount_after(&text, "mills "))
    } else {
        0
    };
    // One-shot token spells ("Create four 1/1 Soldier creature tokens"):
    // no trigger prefix, so the trigger families skip them. The cast
    // resolves the creation.
    let tokens_on_cast =
        if cast_face && parse_triggers(&row.oracle_text).is_empty() && text.contains("token") {
            text.find("create ")
                .map(|i| token_amount_after(&text[i..]))
                .unwrap_or(0)
        } else {
            0
        };

    // One-shot wheel spells ("each player discards their hand, then
    // draws seven"): the cast resolves a full wheel. Requires no
    // trigger prefix (trigger wheels parse as OnUpkeep/Loot abilities).
    let wheel_on_cast = cast_face
        && text.contains("each player")
        && text.contains("discard")
        && text.contains("draw")
        && !text.starts_with("whenever ")
        && !text.starts_with("when ")
        && !text.starts_with("at the beginning");

    // Additional costs: "As an additional cost to cast this spell,
    // sacrifice a creature / pay N life / discard a card". Best case:
    // the agent pays the cost, so the cast consumes the resource.
    let additional_cost_bodies = if text.contains("additional cost")
        && (text.contains("sacrifice a creature") || text.contains("sacrifice any number"))
    {
        1
    } else {
        0
    };
    let additional_cost_life = if text.contains("additional cost") {
        text.split("pay ")
            .nth(1)
            .and_then(|rest| rest.split([' ', '.', ',']).next().map(str::to_string))
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0)
    } else {
        0
    };

    // A land/spell MDFC is one card with two uses: the sim models it as
    // a spell (its dominant non-land role), and the land rule plays the
    // land face when no other land is in hand.
    let role = if is_mdfc_spell {
        classify(row, &text, &tap, &mana_on_cast, false, is_station_card)
    } else {
        classify(row, &text, &tap, &mana_on_cast, land, is_station_card)
    };

    // Printed colors for "per color among permanents" scaling.
    let mut colors = [false; 5];
    for (i, ch) in super::model::COLORS.iter().enumerate() {
        colors[i] = row.colors.contains(*ch);
    }

    // Treasure creation: "create a Treasure token" / "create N Treasure
    // tokens". Each treasure is a banked flexible pip.
    let treasures_on_token = text.contains("treasure token");

    // Static mana grants (Enduring Vitality, Chromatic Lantern).
    let grant = if text.contains("lands you control have \"") {
        Some(super::model::Grant::Lands)
    } else if text.contains("creatures you control have \"") {
        Some(super::model::Grant::Creatures)
    } else {
        None
    };

    // Static creature buff ("creatures you control get +2/+2").
    let buff = super::parse_keywords::parse_creature_buff(&text);

    // Equipment: equip cost, equipped buff, Skullclamp death-draws.
    let equipment = if row.type_line.contains("Equipment") {
        super::parse_keywords::parse_equipment(&text)
    } else {
        None
    };

    // Keywords that read from either the keywords array or the text.
    let double_strike = row.keywords.contains("Double strike") || text.contains("double strike");
    let prowess = row.keywords.contains("Prowess") || text.contains("prowess");
    let landfall = text.contains("landfall");
    // Text "flying" joins the evasion census too ("has flying", "gains
    // flying"); keyword-array Flying covered above.
    let evasion = row.keywords.contains("Trample")
        || row.keywords.contains("Flying")
        || row.keywords.contains("Menace")
        || text.contains("trample")
        || text.contains("menace")
        || text.contains("flying");
    // Haste: keyword array or reminder text. "Creatures you control have
    // haste" is a grant to other bodies — the granter itself does not
    // attack on its entry turn — so the oracle mention does not set haste
    // on this card.
    let has_haste =
        row.keywords.contains("Haste") || text.contains("has haste") || text.contains("with haste");
    // "You may play an additional land" / "an additional land on each of
    // your turns" (Aesi, Wayward Swordtooth, Burrowing Power).
    let extra_land_drops = text.contains("additional land");

    // Instant speed: Instant type or flash. "Flashback" contains
    // "flash" as a substring; exclude it.
    let is_instant_speed = row.type_line.contains("Instant")
        || row.keywords.contains("Flash")
        || (text.contains("flash") && !text.contains("flashback"));

    // Interaction: removal or counterspell shapes (readiness metric).
    // Wipes count too (capacity, not events). Damage shapes match any
    // "deals N damage to target …" with N >= 2;
    // player-targeted damage stays drain. Bounce ("return target … to
    // its owner's hand") answers a threat the same way removal does.
    let wipe = cast_face
        && (text.contains("destroy all")
            || text.contains("exile all")
            || text.contains("return all")
            || text.contains("sacrifice all")
            || (text.contains("each creature") && text.contains("-x/-x")));
    let damage_removal = damage_removal_shape(&row.oracle_text);
    let is_interaction = row.type_line.contains("Instant") || row.type_line.contains("Sorcery");
    let is_interaction = is_interaction
        && (text.contains("destroy target")
            || text.contains("exile target")
            || text.contains("counter target")
            || text.contains("return target")
                && (text.contains("to its owner's hand")
                    || text.contains("to their owner's hand"))
            || damage_removal
            || wipe);

    // Split cards ("Fire // Ice"): the cast pays the cheaper face, so
    // one-shot on-cast credits (draw, mana, tokens) must not fire for a
    // face that was not cast. Triggers and roles still read from the
    // union text. Land/spell MDFCs are not split cards: casting the
    // spell face is a real cast, so its on-cast effects stay.
    let is_split =
        !is_mdfc_spell && (row.oracle_text.contains("//") || row.mana_cost.contains("//"));
    let draws_on_cast = if is_split { 0 } else { draws_on_cast };
    let mana_on_cast = if is_split { None } else { mana_on_cast };
    let tokens_on_cast = if is_split { 0 } else { tokens_on_cast };
    let drain_on_cast = if is_split { 0 } else { drain_on_cast };

    SimCard {
        name: row.name.clone(),
        cost,
        min_cost,
        tap,
        station_tiers,
        crew,
        is_creature: type_line.contains("Creature"),
        is_artifact: type_line.contains("Artifact"),
        is_station_card,
        enter_counters,
        role,
        enters_tapped,
        gate_types,
        opens_in_play,
        is_saga,
        counters_on_cast,
        mana_on_cast,
        mana_per_cast,
        draws_on_cast,
        mills_on_enter,
        tokens_on_cast,
        scry_on_cast,
        surveils,
        mills_opponent,
        extra_turns_on_cast,
        drain_on_cast,
        additional_cost_bodies,
        additional_cost_life,
        wheel_on_cast,
        x_class,
        has_cascade,
        printed_power,
        starting_loyalty,
        board_discount,
        colors,
        treasures_on_token,
        grant,
        buff,
        equipment,
        double_strike,
        prowess,
        landfall,
        evasion,
        is_instant_speed,
        is_interaction,
        has_haste,
        extra_land_drops,
        kicker,
        wipe,
        is_enchantment: type_line.contains("Enchantment") && !type_line.contains("Aura"),
        buffs_board_on_enter: text.contains("+x/+x")
            && (text.contains("where x is") || text.contains("equal to the number")),
        counters_are_power: enters_with_x_counters(&text),
        draws_per_matching: if !land {
            scaling_draw_match(&text)
        } else {
            None
        },
        is_mdfc_spell,
        mythic,
    }
}

/// "Enters with X +1/+1/charge counters" (any "enters [the battlefield]
/// with X … counters" shape).
fn enters_with_x_counters(text: &str) -> bool {
    (text.contains("enters with x") || text.contains("the battlefield with x"))
        && text.contains("counters")
}

/// Detect a board-count-gated draw engine ("draw a card for each
/// enchantment you control"). Returns the counted permanent class, None
/// when the text is not a scaling draw.
fn scaling_draw_match(text: &str) -> Option<super::model::DrawMatch> {
    if !(text.contains("draw") && text.contains("for each")) {
        return None;
    }
    if text.contains("for each enchantment you control") {
        Some(super::model::DrawMatch::Enchantments)
    } else if text.contains("for each artifact you control") {
        Some(super::model::DrawMatch::Artifacts)
    } else if text.contains("for each land you control") {
        Some(super::model::DrawMatch::Lands)
    } else if text.contains("for each creature you control") {
        Some(super::model::DrawMatch::Creatures)
    } else {
        None
    }
}

/// Saga chapter abilities: lines "I — Loot.", "II — Draw two cards."
/// parse through the same effect shapes as triggers (the roman numeral
/// prefix is stripped). Returns one effect per chapter in order.
/// Combined numeral lines ("I, II, III — Create a 3/3 token") give every
/// listed chapter the same effect.
fn parse_saga_chapters(oracle_text: &str) -> Vec<Effect> {
    let mut chapters = Vec::new();
    for line in oracle_text.split('\n') {
        let line = line.trim();
        let Some((numeral, body)) = line.split_once('—').or_else(|| line.split_once(" - "))
        else {
            continue;
        };
        // The numeral part is one or more romans ("I", "II, III"), the
        // effect body follows the em dash.
        let numeral = numeral.trim().trim_end_matches('.');
        let mut ordinals: Vec<u32> = Vec::new();
        for piece in numeral.split([',', ' ']).filter(|p| !p.is_empty()) {
            let ordinal = match piece {
                "I" => 1,
                "II" => 2,
                "III" => 3,
                "IV" => 4,
                _ => continue,
            };
            ordinals.push(ordinal);
        }
        let Some(top) = ordinals.pop() else {
            continue;
        };
        let lower = body.to_ascii_lowercase();
        let effect = if lower.contains("draw") && !lower.contains("discard") {
            Effect::Draw(draw_amount(&lower).max(1))
        } else if lower.contains("create") && lower.contains("token") {
            Effect::Tokens(super::triggers::token_amount(&lower))
        } else if lower.contains("mill ") {
            Effect::Mill(mill_amount(&lower))
        } else if lower.contains("from your graveyard") && lower.contains("return") {
            Effect::ReturnFromGraveyard {
                to_hand: lower.contains("to your hand"),
                count: 1,
            }
        } else if lower.contains("add ") {
            Effect::ExtraLand
        } else if lower.contains("search") {
            Effect::Tutor
        } else if lower.contains("exile") && lower.contains("battlefield") {
            Effect::ExtraLand
        } else {
            Effect::None
        };
        // Pad gaps: chapter II of a saga whose chapter I did not parse
        // still lands at index 1. Combined numeral lines fill every
        // listed chapter with the same effect.
        while chapters.len() < top as usize {
            chapters.push(Effect::None);
        }
        for &o in &ordinals {
            chapters[o as usize - 1] = effect.clone();
        }
        chapters[top as usize - 1] = effect;
    }
    chapters
}

/// Functional role classification.
pub fn classify(
    row: &CardRow,
    text: &str,
    tap: &Option<TapYield>,
    mana_on_cast: &Option<TapYield>,
    land: bool,
    station: bool,
) -> super::model::Role {
    use super::model::Role;
    if land {
        return Role::Land;
    }
    if is_rock(row) {
        return Role::Rock;
    }
    if is_dork(row) {
        return Role::Dork;
    }
    if tap.is_some() || mana_on_cast.is_some() {
        return Role::RampSpell;
    }
    // Removal first: "Destroy target creature. Draw a card." is a
    // removal spell with a rider, not a draw engine. Draw-role counts
    // would skew without this order. Protection grants ("gains
    // hexproof") are not removal: they answer nothing in a goldfish.
    let removal = text.contains("destroy target")
        || text.contains("destroy all")
        || text.contains("exile target")
        || text.contains("counter target")
        || text.contains("return target")
            && (text.contains("to its owner's hand") || text.contains("to their owner's hand"))
        || text.contains("prevent all combat damage")
        || text.contains("prevent the next") && text.contains("damage")
        || text.contains("creatures with power") && text.contains("can't attack")
        || text.contains("can't attack or block")
        || text.contains("regenerate target")
        || damage_removal_shape(&row.oracle_text);
    if removal {
        return Role::Removal;
    }
    // Draw: repeatable engines and one-shot draws; tutors and look-at-top
    // effects refuel the hand.
    let draws = text.contains("draw ")
        || text.contains("drawn")
        || text.contains("investigate")
        || text.contains("surveil")
        || text.contains("search your library")
        || (text.contains("look at the top") && text.contains("into your hand"));
    if draws && !text.contains("opponent") {
        return Role::Draw;
    }
    if station {
        // Spacecraft/planets with no draw text are wincons when big.
        if row.cmc >= 5.0 {
            return Role::Wincon;
        }
        return Role::Other;
    }
    // Static tax/restriction pieces (stax): their timing is the question.
    // "This spell costs {1} more to cast for each target" is a multi-target
    // rider (multi-target X spells), not a tax: exclude "for each
    // target" shapes.
    let lock = (text.contains("cost")
        && text.contains("more to cast")
        && !text.contains("for each target")
        && !text.contains("for each additional target"))
        || text.contains("players can't cast more than")
        || text.contains("can't untap")
        || text.contains("doesn't untap")
        || text.contains("don't untap")
        || text.contains("skip your draw step") && !row.type_line.contains("Creature")
        || (text.contains("activated abilities")
            && text.contains("unless")
            && !row.type_line.contains("Creature"));
    if lock {
        return Role::Lock;
    }
    // Equipment, auras, and pump spells: the suit-up cadence question.
    let booster = row.type_line.contains("Equipment")
        || row.type_line.contains("Aura")
        || (text.contains("target creature")
            && text.contains("gets +")
            && (text.contains("until end of turn") || row.type_line.contains("Enchantment")));
    if booster {
        return Role::Booster;
    }
    let big_threat = row.cmc >= 5.0
        || text.contains("win the game")
        || text.contains("loses the game")
        || (row.type_line.contains("Creature")
            && row
                .power
                .as_deref()
                .and_then(|p| p.trim_end_matches('*').parse::<i32>().ok())
                .is_some_and(|p| p >= 5));
    if big_threat {
        return Role::Wincon;
    }
    Role::Other
}
