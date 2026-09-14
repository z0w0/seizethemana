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
use super::triggers::{mill_amount, parse_triggers};
use crate::db::CardRow;

/// Parse a Scryfall mana-cost string (`{2}{W}{W}`, `{W/U}`, `{1}{B/P}`,
/// multi-face `{2}{B} // {B}`) into a cost. Multi-face costs union the
/// faces; hybrid pips become flexible pips.
pub fn parse_cost(mana_cost: &str) -> Cost {
    let mut cost = Cost::default();
    for symbol in mana_cost.split(['{', '}']).filter(|s| !s.is_empty()) {
        if symbol == "//" {
            continue;
        }
        let upper = symbol.to_ascii_uppercase();
        if upper.chars().all(|c| c.is_ascii_digit()) {
            cost.generic += upper.parse::<u32>().unwrap_or(0);
            continue;
        }
        if upper == "X" || upper == "S" {
            cost.generic += 1;
            continue;
        }
        let colored: Vec<usize> = upper
            .chars()
            .filter(|c| *c != '/' && *c != 'P')
            .filter_map(|c| super::model::COLORS.iter().position(|w| *w == c))
            .collect();
        if colored.len() == 1 {
            cost.pips[colored[0]] += 1;
        } else if colored.len() > 1 {
            cost.flex_pips += 1;
        }
    }
    cost
}

/// The cheapest castable cost across a card's faces: split cards
/// ("Dusk // Dawn", "Bramble Familiar // Fetch Quest") are one card the
/// deck casts by choosing a face, so the model pays the cheaper face, not
/// the face sum. MDFC spell faces ("Bramble Familiar // Fetch Quest" again:
/// one face is a land) and modal faces follow the same rule. Empty faces
/// (MDFC land side) cost nothing as a land and never gate a cast.
pub fn parse_cost_faces(mana_cost: &str) -> Cost {
    let faces: Vec<&str> = mana_cost.split(" // ").collect();
    if faces.len() <= 1 {
        return parse_cost(mana_cost);
    }
    let nonempty: Vec<Cost> = faces
        .iter()
        .filter(|face| !face.trim().is_empty())
        .map(|face| parse_cost(face))
        .collect();
    if nonempty.len() <= 1 {
        return nonempty.first().cloned().unwrap_or_default();
    }
    nonempty
        .into_iter()
        .min_by_key(|cost| cost.total())
        .unwrap_or_default()
}

/// Parse one "add" clause into a tap yield. Juxtaposed symbols with no
/// "or" produce fixed simultaneous pips (Jegantha's `{W}{U}{B}{R}{G}`);
/// "or" between symbols and "one mana of any color" prose are choices.
pub fn parse_tap_yield(text: &str) -> Option<TapYield> {
    let lower = text.to_ascii_lowercase();
    // Opponent-dependent production ("any color that a land an opponent
    // controls could produce") yields nothing in a goldfish sim.
    if lower.contains("opponent") {
        return None;
    }
    let mut yield_ = TapYield::default();
    if lower.contains("one mana of any color") || lower.contains("mana of any one color") {
        yield_.any = true;
        yield_.choice = [false; 5];
        return Some(yield_);
    }
    if lower.contains(" or ") {
        for clause in lower.split(" or ") {
            for symbol in clause.split(['{', '}']).filter(|s| !s.is_empty()) {
                let upper = symbol.to_ascii_uppercase();
                if upper.len() == 1 {
                    let ch = upper.chars().next().unwrap_or(' ');
                    if let Some(idx) = super::model::COLORS.iter().position(|c| *c == ch) {
                        yield_.choice[idx] = true;
                    } else if ch == 'C' {
                        yield_.colorless += 1;
                    }
                }
            }
        }
    } else {
        // No "or": juxtaposed symbols are one simultaneous set.
        for symbol in lower.split(['{', '}']).filter(|s| !s.is_empty()) {
            let upper = symbol.to_ascii_uppercase();
            if upper.len() == 1 {
                let ch = upper.chars().next().unwrap_or(' ');
                if let Some(idx) = super::model::COLORS.iter().position(|c| *c == ch) {
                    yield_.fixed[idx] += 1;
                } else if ch == 'C' {
                    yield_.colorless += 1;
                }
            }
        }
    }
    if yield_.total() == 0 {
        return None;
    }
    Some(yield_)
}

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

/// Parse one oracle segment into an executable ability when it matches a
/// known shape. Everything else returns None (dropped silently).
pub fn parse_ability(segment: &str) -> Option<Ability> {
    // Activated/loyalty shape: "{1}, {T}: Draw two cards", "−3: Search…".
    let (cost_part, effect_part) = segment.split_once(':')?;
    let cost = parse_activation_cost(cost_part.trim());
    // Loyalty cost: a leading minus ("−3", "-2") spends loyalty.
    let cleaned = cost_part.replace(['−', '–'], "-").trim().to_string();
    let loyalty_cost = cleaned
        .strip_prefix('-')
        .and_then(|d| d.parse::<u32>().ok())
        .filter(|_| cleaned.chars().all(|c| c.is_ascii_digit() || c == '-'))
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
        Effect::Tokens(2)
    } else if lower_effect.contains("put a charge counter") {
        // "Put a charge counter": one counter per activation (Coalition
        // Relic). The Drill Too Deep shape ("put five charge counters") is
        // a spell, handled by `charge_counters_on_cast`.
        Effect::Counters(1)
    } else {
        return None;
    };
    Some(Ability {
        trigger: super::model::Trigger::Activated,
        cost,
        effect,
        taps: cost_part.to_ascii_lowercase().contains("{t}"),
        // "Sacrifice a creature" / "sacrifice this creature" in the cost
        // consumes a body (aristocrats outlets).
        sacrifice_bodies: u32::from(
            cost_part.to_ascii_lowercase().contains("sacrifice")
                && cost_part.to_ascii_lowercase().contains("creature"),
        ),
        loyalty_cost,
    })
}

/// Parse an activation cost prefix ("{1}, {T}", "−3", "{0}").
fn parse_activation_cost(text: &str) -> Cost {
    // Planeswalker loyalty costs ("-3", "0", "−7") cost no mana.
    let cleaned = text.replace(['−', '–'], "-");
    if cleaned.trim_start().starts_with('-')
        || cleaned
            .trim()
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-')
    {
        return Cost::default();
    }
    parse_cost(&cleaned)
}

/// Parse a card row into the simulator's data model.
pub fn parse_sim_card(row: &CardRow) -> SimCard {
    let text = row.oracle_text.to_ascii_lowercase();
    let type_line = row.type_line.to_string();
    let land = is_land(row);
    let cost = if land {
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
    if !abilities.is_empty() {
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

    let is_saga = type_line.contains("Saga") && !land;
    let enter_counters = parse_enter_counters(&text);

    // One-shot effects on cast.
    let mana_on_cast = if !land && tap.is_none() && text.contains("add ") {
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
    let draws_on_cast = if !land {
        row.oracle_text
            .split(['.', '\n'])
            .map(str::trim)
            .find_map(|seg| {
                let lower = seg.to_ascii_lowercase();
                let draws = (lower.starts_with("draw ")
                    || lower.contains(", draw ")
                    || lower.starts_with("investigate")
                    || lower.contains("then draw"))
                    && !lower.contains("whenever")
                    && !lower.contains("at the beginning")
                    && !lower.contains(": ");
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

    // Printed power for crew/station math; "*" and unknowns stay None
    // (flat body power). Tokens never carry a row.
    let printed_power = row
        .power
        .as_deref()
        .and_then(|p| p.trim_end_matches('*').parse::<u32>().ok());

    // Starting loyalty for planeswalkers (the CardRow loyalty column).
    let starting_loyalty = row
        .loyalty
        .as_deref()
        .and_then(|l| l.trim().parse::<u32>().ok());

    // One-shot mill on entering ("mill N" ETB without a trigger segment).
    let mills_on_enter = if !land && parse_triggers(&row.oracle_text).is_empty() {
        mill_amount(&text)
    } else {
        0
    };

    let role = classify(row, &text, &tap, &mana_on_cast, land, is_station_card);

    SimCard {
        name: row.name.clone(),
        cost,
        min_cost,
        tap,
        station_tiers,
        crew,
        is_creature: type_line.contains("Creature"),
        is_station_card,
        enter_counters,
        role,
        enters_tapped,
        gate_types,
        opens_in_play,
        is_saga,
        counters_on_cast,
        mana_on_cast,
        draws_on_cast,
        mills_on_enter,
        printed_power,
        starting_loyalty,
        board_discount,
    }
}

/// Tap yield from `{T}: Add …` segments, merged across abilities (a
/// permanent taps once; later abilities merge as the union of colors).
/// Lands whose oracle grants them a basic type ("This land is the chosen
/// type") tap for that type's color without an explicit add clause.
fn parse_tap(row: &CardRow) -> Option<TapYield> {
    // Basic lands wrap the oracle text in parens: "({T}: Add {G}.)".
    let oracle = row.oracle_text.trim_start_matches('(');
    let segments: Vec<&str> = oracle.split(['\n', '.']).map(str::trim).collect();
    // Type-granted lands ("This land is the chosen type"): no add clause,
    // but the chosen basic type is the player's choice each game, so the
    // tap reads as one mana of any color.
    let lower_all = oracle.to_ascii_lowercase();
    if segments.iter().all(|s| {
        !(s.to_ascii_lowercase().starts_with("{t}")
            || s.to_ascii_lowercase().contains(": add"))
    } && (lower_all.contains("this land is the chosen type")
        || lower_all.contains("this land is every basic land type")))
    {
        return Some(TapYield {
            any: true,
            ..TapYield::default()
        });
    }
    // Station tier lines ("12+ | {U}, {T}: Add …") belong to a tier, not
    // to the base card; parse_tap must not double-count them.
    let tier_lines: Vec<usize> = segments
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.split_once('|').is_some_and(|(head, _)| {
                head.trim()
                    .trim_end_matches('+')
                    .trim()
                    .parse::<u32>()
                    .is_ok()
            })
        })
        .map(|(i, _)| i)
        .collect();
    let mut merged: Option<TapYield> = None;
    for (si, seg) in segments.iter().enumerate() {
        // Skip tier segments and any segment a tier marker precedes.
        if tier_lines.contains(&si)
            || tier_lines
                .iter()
                .any(|t| *t > si && segments[si..*t].iter().all(|s| s.is_empty()))
        {
            continue;
        }
        let lower = seg.to_ascii_lowercase();
        if !lower.starts_with("{t}") && !lower.contains(": add") {
            continue;
        }
        let Some((_, body)) = seg.split_once(':') else {
            continue;
        };
        if let Some(y) = parse_tap_yield(body) {
            // A gated mode ("Activate only if you control …" in the
            // segment itself or the next one) joins as a choice color,
            // not a fixed pip: it does not produce every turn.
            let gated = lower.contains("activate only if")
                || segments.get(si + 1).is_some_and(|next| {
                    tier_lines.contains(&(si + 1))
                        || next.to_ascii_lowercase().contains("activate only if")
                });
            // Spend restriction: "spend this mana only to cast … creature
            // spells" (Secluded Courtyard, Unclaimed Territory). The clause
            // follows the add segment as its own sentence.
            let restriction_window: String = segments[si..]
                .iter()
                .take(3)
                .map(|s| s.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            let creature_only = restriction_window.contains("only to cast")
                && restriction_window.contains("creature");
            let y = if gated {
                let mut choice_only = y.clone();
                choice_only.choice = y.fixed.map(|p| p > 0);
                choice_only.fixed = [0; 5];
                choice_only
            } else {
                y
            };
            let mut y = y;
            y.creature_only = creature_only;
            merged = Some(match merged.take() {
                None => y,
                Some(prev) => {
                    // Several tap abilities on one permanent: one tap
                    // yields one mana of any reachable color. A restricted
                    // mode restricts the merged tap (the unrestricted
                    // colorless mode produces nothing of value).
                    let mut m = TapYield {
                        alternatives: true,
                        creature_only: prev.creature_only || y.creature_only,
                        ..TapYield::default()
                    };
                    m.any = prev.any || y.any;
                    for i in 0..5 {
                        m.choice[i] =
                            prev.choice[i] || y.choice[i] || prev.fixed[i] > 0 || y.fixed[i] > 0;
                    }
                    m.colorless = u32::from(prev.colorless > 0 || y.colorless > 0);
                    m
                }
            });
        }
    }
    merged
}

/// Gate colors of a verge-style land: the types listed after the second
/// `{T}:` ability's "Activate only if you control …".
fn parse_gates(oracle_text: &str) -> Vec<&'static str> {
    let text = oracle_text.to_ascii_lowercase();
    // The gate clause names basic types; any type mentioned after
    // "Activate only if" is gated (the ungated mode is the first).
    let Some(idx) = text.find("activate only if") else {
        return Vec::new();
    };
    let gate_text = &text[idx..];
    let mut gates = Vec::new();
    for (word, kind) in [
        ("plains", "Plains"),
        ("island", "Island"),
        ("swamp", "Swamp"),
        ("mountain", "Mountain"),
        ("forest", "Forest"),
    ] {
        if gate_text.contains(&format!("control a {word}"))
            || gate_text.contains(&format!("control an {word}"))
            // "control a Swamp or a Mountain": the second type follows an
            // "or a" clause.
            || gate_text.contains(&format!("or a {word}"))
            || gate_text.contains(&format!("or an {word}"))
        {
            gates.push(kind);
        }
    }
    gates
}

/// Enters-tapped oracle check for lands.
/// Enters-tapped oracle check for lands. Best-case reading: the shock-dual
/// life-payment clause ("you may pay 2 life") stays untapped; unconditional
/// "enters tapped" texts are tapped.
fn enters_tapped(text: &str) -> bool {
    // Shock duals and MDFC "you may pay 3 life" lands: the sim's
    // best-case agent pays any printed life.
    if text.contains("you may pay 2 life")
        || text.contains("unless you pay 2 life")
        || text.contains("you may pay 3 life")
        || text.contains("unless you pay 3 life")
    {
        return false;
    }
    // "Enters tapped unless …" conditions that self-solve early
    // ("unless you control two or fewer other lands", first turns) are
    // treated untapped; other unless-conditions as tapped.
    if text.contains("enters tapped unless") {
        return !text.contains("two or fewer other lands")
            && !text.contains("it's your first, second, or third turn");
    }
    text.contains("enters tapped") || text.contains("enters the battlefield tapped")
}

/// Charge counters the card enters with ("enters with three charge
/// counters on it" → 3).
/// Charge counters the card enters with ("enters with three charge
/// counters on it" → 3). The search stays inside the same sentence so a
/// later "{2}, {T}" activation does not leak a number.
fn parse_enter_counters(text: &str) -> u32 {
    if !text.contains("enters with") {
        return 0;
    }
    let Some(idx) = text.find("enters with") else {
        return 0;
    };
    let tail: &str = text[idx..].split(['.', '\n', ',']).next().unwrap_or("");
    let digits: String = tail
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if !digits.is_empty() {
        return digits.parse::<u32>().unwrap_or(0);
    }
    for (word, n) in [
        ("one", 1u32),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("six", 6),
    ] {
        if tail.contains(&format!("{word} ")) {
            return n;
        }
    }
    0
}

/// "Put N charge counters" on cast (Drill Too Deep).
fn charge_counters_on_cast(text: &str) -> u32 {
    if !text.contains("charge counter") {
        return 0;
    }
    let Some(idx) = text.find("put ") else {
        return 0;
    };
    let tail = &text[idx + 4..];
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        return digits.parse::<u32>().unwrap_or(0);
    }
    for (word, n) in [
        ("five", 5u32),
        ("four", 4),
        ("three", 3),
        ("two", 2),
        ("one", 1),
    ] {
        if tail.starts_with(word) {
            return n;
        }
    }
    0
}

/// Functional role classification.
fn classify(
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
        || text.contains("gains hexproof")
        || text.contains("gains indestructible");
    if removal {
        return Role::Removal;
    }
    // Static tax/restriction pieces (stax): their timing is the question.
    let lock = text.contains("cost") && text.contains("more to cast")
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
