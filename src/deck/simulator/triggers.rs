// Trigger-family parsers for the simulator's oracle text: ETB, upkeep,
// attack, death, cast, wheel, and plain activated abilities. Split from
// parse.rs to keep files small.

use super::model::{Ability, Effect, TapYield, draw_amount};
use super::parse::parse_ability;

/// ETB triggers: "When this …enters", "When Cardname enters",
/// "When you cast this…". Opponent-facing triggers stay ignored.
fn etb_trigger(lower: &str, out: &mut Vec<Ability>) -> bool {
    let etb = (lower.starts_with("when ") || lower.starts_with("when you cast this"))
        && lower.contains("enters")
        && !lower.contains("opponent");
    if !etb {
        return false;
    }
    if lower.contains("draw") || lower.contains("investigate") || lower.contains("scry") {
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::Draw(draw_amount(lower).max(1)),
            ..Ability::default()
        });
        return true;
    }
    if lower.contains("search your library") || lower.contains("into your hand") {
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::Tutor,
            ..Ability::default()
        });
        return true;
    }
    if lower.contains("create") && lower.contains("token") {
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::Tokens(2),
            ..Ability::default()
        });
        return true;
    }
    if lower.contains("search") && lower.contains("land") {
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::ExtraLand,
            ..Ability::default()
        });
        return true;
    }
    if lower.contains("mill ") {
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::Mill(mill_amount(lower)),
            ..Ability::default()
        });
        return true;
    }
    if lower.contains("from your graveyard") && lower.contains("return") {
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::ReturnFromGraveyard {
                to_hand: lower.contains("to your hand"),
                count: 1,
            },
            ..Ability::default()
        });
        return true;
    }
    if lower.contains("exile") && lower.contains("return it to the battlefield") {
        // Blink: the exiled permanent comes back, so its ETB triggers
        // fire again on the next turn.
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::ExtraLand,
            ..Ability::default()
        });
        return true;
    }
    if lower.contains("you become the monarch") {
        // Monarch acquisition: modeled as an extra card per turn (the
        // Monarch draws at their upkeep).
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::ExtraLand,
            ..Ability::default()
        });
        return true;
    }
    false
}

/// Upkeep and end-step engines: draws, mill, monarch, banked mana.
fn upkeep_trigger(lower: &str, out: &mut Vec<Ability>) -> bool {
    // Upkeep and end-step draw engines.
    if (lower.starts_with("at the beginning of your upkeep")
        || lower.starts_with("at the beginning of your end step"))
        && (lower.contains("draw") || lower.contains("investigate"))
    {
        out.push(Ability {
            trigger: super::model::Trigger::OnUpkeep,
            effect: Effect::Draw(draw_amount(lower).max(1)),
            ..Ability::default()
        });
        return true;
    }
    // Upkeep mill engines ("At the beginning of your upkeep, mill N").
    if lower.starts_with("at the beginning of your upkeep") && lower.contains("mill ") {
        out.push(Ability {
            trigger: super::model::Trigger::OnUpkeep,
            effect: Effect::Mill(mill_amount(lower)),
            ..Ability::default()
        });
        return true;
    }
    // Upkeep monarch draw: the Monarch draws at upkeep once taken.
    if lower.starts_with("at the beginning of your upkeep")
        && lower.contains("draw a card")
        && lower.contains("monarch")
    {
        out.push(Ability {
            trigger: super::model::Trigger::OnUpkeep,
            effect: Effect::Draw(1),
            ..Ability::default()
        });
        return true;
    }
    // Banked-mana engines: "At the beginning of your [first main]
    // phase, remove all charge counters … add one mana of any color
    // for each" (Coalition Relic). Releases counters × N at upkeep.
    if lower.starts_with("at the beginning of your first main phase")
        && lower.contains("remove all charge counters")
        && lower.contains("add one mana of any color")
    {
        out.push(Ability {
            trigger: super::model::Trigger::OnUpkeep,
            effect: Effect::ManaPerCounter(TapYield {
                any_pips: 1,
                ..TapYield::default()
            }),
            ..Ability::default()
        });
        return true;
    }
    // Win thresholds: "if there are 100 or more tower counters on
    // Helix Pinnacle, you win". Anchor on the "N or more … counters"
    // clause directly so the amount is the number before "or more".
    if lower.contains("or more")
        && lower.contains("counter")
        && (lower.contains("you win") || lower.contains("wins the game"))
    {
        let counters = {
            let rel = lower.find(" or more").unwrap_or(0);
            let digits: String = lower[..rel]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            digits.parse::<u32>().unwrap_or(u32::MAX).max(1)
        };
        out.push(Ability {
            trigger: super::model::Trigger::OnUpkeep,
            effect: Effect::WinThreshold { counters },
            ..Ability::default()
        });
        return true;
    }
    // Upkeep drain engines ("At the beginning of each opponent's
    // upkeep, that player loses N life" is opponent-scoped and stays
    // inert; "each opponent loses N" at YOUR upkeep drains).
    if lower.starts_with("at the beginning of your upkeep") && lower.contains("each opponent loses")
    {
        out.push(Ability {
            trigger: super::model::Trigger::OnUpkeep,
            effect: Effect::Drain(super::model::amount_after(lower, "loses").max(1)),
            ..Ability::default()
        });
        return true;
    }
    false
}

/// Attack triggers: "Whenever …attacks…" draws, tokens, tutors.
fn attack_trigger(lower: &str, out: &mut Vec<Ability>) -> bool {
    // Attack triggers.
    if lower.starts_with("whenever ") && lower.contains("attack") {
        if lower.contains("draw") {
            out.push(Ability {
                trigger: super::model::Trigger::OnAttack,
                effect: Effect::Draw(draw_amount(lower).max(1)),
                ..Ability::default()
            });
        } else if lower.contains("create") && lower.contains("token") {
            out.push(Ability {
                trigger: super::model::Trigger::OnAttack,
                effect: Effect::Tokens(2),
                ..Ability::default()
            });
        } else if lower.contains("search") || lower.contains("return target") {
            out.push(Ability {
                trigger: super::model::Trigger::OnAttack,
                effect: Effect::Tutor,
                ..Ability::default()
            });
        }
        return true;
    }
    false
}

/// Combat-damage triggers: "Whenever [Cardname] deals combat damage to a
/// player …". Best case: every attacker connects. Draws, proliferate
/// (+1 counter on the source), and drain map here.
fn combat_damage_trigger(lower: &str, out: &mut Vec<Ability>) -> bool {
    let combat = (lower.starts_with("whenever ") || lower.starts_with("when "))
        && (lower.contains("deals combat damage to a player")
            || lower.contains("deals combat damage to an opponent"));
    if !combat {
        return false;
    }
    if lower.contains("draw") {
        out.push(Ability {
            trigger: super::model::Trigger::OnCombatDamage,
            effect: Effect::Draw(draw_amount(lower).max(1)),
            ..Ability::default()
        });
    } else if lower.contains("proliferate") {
        out.push(Ability {
            trigger: super::model::Trigger::OnCombatDamage,
            effect: Effect::Counters(1),
            ..Ability::default()
        });
    } else if lower.contains("player loses")
        || lower.contains("opponent loses")
        || lower.contains("loses")
    {
        out.push(Ability {
            trigger: super::model::Trigger::OnCombatDamage,
            effect: Effect::Drain(super::model::amount_after(lower, "loses").max(1)),
            ..Ability::default()
        });
    }
    true
}

/// Death triggers: "When this creature dies …" draws, tokens, returns,
/// mill. Sacrificed and destroyed bodies fire these.
fn death_trigger(lower: &str, out: &mut Vec<Ability>) -> bool {
    // Death triggers ("When this creature dies …", "Whenever … dies,
    // draw"). Sacrificed and destroyed bodies fire these.
    if (lower.starts_with("whenever ") || lower.starts_with("when "))
        && (lower.contains("dies")
            || lower.contains("dies,")
            || lower.contains("is put into a graveyard")
            || lower.contains("is put into your graveyard"))
        && !lower.contains("opponent")
    {
        if lower.contains("draw") {
            out.push(Ability {
                trigger: super::model::Trigger::OnDeath,
                effect: Effect::Draw(draw_amount(lower).max(1)),
                ..Ability::default()
            });
            return true;
        }
        if lower.contains("create") && lower.contains("token") {
            out.push(Ability {
                trigger: super::model::Trigger::OnDeath,
                effect: Effect::Tokens(2),
                ..Ability::default()
            });
            return true;
        }
        if lower.contains("return") && lower.contains("graveyard") {
            out.push(Ability {
                trigger: super::model::Trigger::OnDeath,
                effect: Effect::ReturnFromGraveyard {
                    to_hand: true,
                    count: 1,
                },
                ..Ability::default()
            });
            return true;
        }
        if lower.contains("mill ") {
            out.push(Ability {
                trigger: super::model::Trigger::OnDeath,
                effect: Effect::Mill(mill_amount(lower)),
                ..Ability::default()
            });
            return true;
        }
    }
    false
}

/// Cast-spell engines: "Whenever you cast a …spell…draw".
fn cast_trigger(lower: &str, out: &mut Vec<Ability>) -> bool {
    // Cast-spell engines ("Whenever you cast a …spell…draw").
    if lower.starts_with("whenever you cast")
        && !lower.contains("enters")
        && (lower.contains("draw") || lower.contains("investigate"))
    {
        out.push(Ability {
            trigger: super::model::Trigger::OnCastSpell,
            effect: Effect::Draw(draw_amount(lower).max(1)),
            ..Ability::default()
        });
    }
    false
}

/// Wheel-shaped draws: "draw"+"discard" in one trigger, and the
/// "each player discards, then draws" wheel.
fn wheel_trigger(lower: &str, out: &mut Vec<Ability>) -> bool {
    // Wheel-shaped draws: "draw" + "discard" in one trigger, or a
    // "each player discards, then draws" wheel.
    if (lower.starts_with("whenever ") || lower.starts_with("when ") || etb_shape(lower))
        && lower.contains("draw")
        && lower.contains("discard")
    {
        let amount = if lower.contains("each player") {
            7
        } else {
            draw_amount(lower).max(1)
        };
        out.push(Ability {
            trigger: trigger_for(lower),
            effect: Effect::Loot(amount),
            ..Ability::default()
        });
        return true;
    }
    // Wheels: "each player discards … then draws".
    if lower.contains("discards") && lower.contains("draws") && lower.contains("each player") {
        out.push(Ability {
            trigger: trigger_for(lower),
            effect: Effect::Wheel,
            ..Ability::default()
        });
    }
    false
}

/// Activated abilities on plain cards ("{T}: Draw a card",
/// "{1}, {T}: …", "−3: …", "{2}, Sacrifice a creature: …"). The
/// station-tier path handles tiered activations; this covers the rest.
fn activated_trigger(seg: &str, lower: &str, out: &mut Vec<Ability>) -> bool {
    // Activated abilities on plain cards ("{T}: Draw a card",
    // "{1}, {T}: …", "−3: …", "{2}, Sacrifice a creature: …"). The
    // station-tier path handles tiered activations; this covers the
    // rest. Loyalty activations start with a minus sign. Banked
    // activations ("Remove a charge counter …: Add …") start with a
    // remove clause and consume a counter per fire.
    if (seg.starts_with('{')
        || lower.starts_with("tap:")
        || lower.starts_with('−')
        || lower.starts_with('-')
        || lower.starts_with("remove a charge counter"))
        && let Some(ab) = parse_ability(seg.trim())
    {
        out.push(ab);
        return true;
    }
    false
}

/// True when the segment reads as an enters-the-battlefield shape even
/// without the strict "when … enters" prefix (sagas, cast triggers).
pub(super) fn etb_shape(lower: &str) -> bool {
    lower.starts_with("at the beginning of your end step") || lower.starts_with("when you cast")
}

/// The best trigger guess for a segment by its opening words.
fn trigger_for(lower: &str) -> super::model::Trigger {
    if lower.starts_with("whenever ") && lower.contains("attack") {
        super::model::Trigger::OnAttack
    } else if lower.contains("deals combat damage") {
        super::model::Trigger::OnCombatDamage
    } else if lower.starts_with("at the beginning of your upkeep")
        || lower.starts_with("at the beginning of your end step")
    {
        super::model::Trigger::OnUpkeep
    } else if lower.starts_with("whenever you cast") || lower.starts_with("when you cast") {
        super::model::Trigger::OnCastSpell
    } else if lower.contains("enters") {
        super::model::Trigger::OnEnter
    } else {
        super::model::Trigger::OnUpkeep
    }
}

/// The trigger parser the oracle scan calls: segments the text, then runs
/// each family helper.
pub fn parse_triggers(oracle_text: &str) -> Vec<Ability> {
    let mut out = Vec::new();
    // Split into segments, but keep sentences joined so a trigger's
    // effect continues past the first period ("When X enters, reveal…
    // …put one into your hand."). Sentences ending a trigger are hard to
    // detect textually, so segments pair with the next sentence.
    let segments: Vec<&str> = oracle_text.split('\n').map(str::trim).collect();
    let mut i = 0;
    while i < segments.len() {
        let mut seg = segments[i].trim().to_string();
        // Merge following short continuation sentences into the segment.
        while i + 1 < segments.len()
            && let next = segments[i + 1].trim().to_ascii_lowercase()
            && !next.starts_with("when ")
            && !next.starts_with("whenever ")
            && !next.starts_with("at the beginning")
            && !next.contains("station")
            && !next.contains('|')
            && !next.starts_with('{')
            && !next.starts_with("remove a charge counter")
        {
            i += 1;
            seg.push(' ');
            seg.push_str(segments[i].trim());
        }
        let lower = seg.to_ascii_lowercase();
        i += 1;
        if lower.len() < 8 {
            continue;
        }
        if etb_trigger(&lower, &mut out)
            || upkeep_trigger(&lower, &mut out)
            || attack_trigger(&lower, &mut out)
            || combat_damage_trigger(&lower, &mut out)
            || death_trigger(&lower, &mut out)
            || cast_trigger(&lower, &mut out)
            || wheel_trigger(&lower, &mut out)
            || activated_trigger(&seg, &lower, &mut out)
        {
            continue;
        }
    }
    out
}

/// Mill amount from text ("mill three cards", "mill 10"). Scans every
/// "mill " occurrence so card names ("Mill Fiend") do not swallow the
/// real clause.
pub fn mill_amount(text: &str) -> u32 {
    let mut best = 0;
    let mut from = 0;
    while let Some(rel) = text[from..].find("mill ") {
        let start = from + rel + 5;
        let tail = &text[start..];
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        let found = if !digits.is_empty() {
            digits.parse::<u32>().unwrap_or(0)
        } else {
            let mut hit = 0;
            for (word, n) in [
                ("seven", 7u32),
                ("six", 6),
                ("five", 5),
                ("four", 4),
                ("three", 3),
                ("two", 2),
                ("one", 1),
            ] {
                if tail.starts_with(word) {
                    hit = n;
                    break;
                }
            }
            hit
        };
        best = best.max(found);
        from = start;
    }
    best
}
