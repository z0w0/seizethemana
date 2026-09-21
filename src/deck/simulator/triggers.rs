// Trigger-family parsers for the simulator's oracle text: ETB, upkeep,
// attack, death, cast, and wheel shapes. Landfall, token counts, and
// activated abilities live in sibling modules.

use super::model::{Ability, Effect, TapYield, draw_amount};
pub use super::trigger_activated::mill_amount;
use super::trigger_activated::{activated_trigger, etb_shape, once_each_turn, trigger_for};
use super::trigger_landfall::landfall_trigger;
pub use super::trigger_landfall::token_amount;

/// ETB triggers: "When this …enters", "Whenever …enters",
/// "When Cardname enters", "When you cast this…". Opponent-facing
/// triggers stay ignored. Landfall ETBs ("Whenever a land you control
/// enters") route to the landfall family instead.
fn etb_trigger(lower: &str, out: &mut Vec<Ability>) -> bool {
    if lower.contains("whenever a land") && lower.contains("enters") && lower.contains("landfall") {
        return false;
    }
    let etb = (lower.starts_with("when ")
        || lower.starts_with("whenever ")
        || lower.starts_with("when you cast this"))
        && lower.contains("enters")
        && !lower.contains("opponent");
    if !etb {
        return false;
    }
    if lower.contains("draw") || lower.contains("investigate") {
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::Draw(draw_amount(lower).max(1)),
            ..Ability::default()
        });
        return true;
    }
    if lower.contains("scry") || lower.contains("surveil") {
        // Scry/surveil ETBs feed awareness, not draw credit.
        let amount = if lower.contains("surveil") {
            super::model::amount_after(lower, "surveil")
        } else {
            super::model::amount_after(lower, "scry")
        };
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::Scry(amount.max(1)),
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
            effect: Effect::Tokens(token_amount(lower)),
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
            once_per_turn: once_each_turn(lower),
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
                effect: Effect::Tokens(token_amount(lower)),
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
                effect: Effect::Tokens(token_amount(lower)),
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

/// Cast-spell engines: "Whenever you cast a …spell…draw" (loot shape
/// included so the wheel family does not double-push one segment).
/// Self-referential shapes ("When you cast this spell, …") are one-shot
/// riders on the spell itself, not repeatable engines; they stay
/// unclaimed (the cast path reads them as one-shot draws).
fn cast_trigger(lower: &str, out: &mut Vec<Ability>) -> bool {
    if !(lower.starts_with("whenever you cast") || lower.starts_with("when you cast")) {
        return false;
    }
    if lower.contains("enters") || lower.contains("this spell") {
        return false;
    }
    if lower.contains("draw") || lower.contains("investigate") {
        // Draw+discard cast triggers parse as Loot, plain draws as Draw.
        let effect = if lower.contains("discard") {
            Effect::Loot(draw_amount(lower).max(1))
        } else {
            Effect::Draw(draw_amount(lower).max(1))
        };
        out.push(Ability {
            trigger: super::model::Trigger::OnCastSpell,
            effect,
            // "This ability triggers only once each turn" bounds the
            // engine (no per-cast looping past one draw).
            once_per_turn: once_each_turn(lower),
            ..Ability::default()
        });
        return true;
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
    // Wheels: "each player discards … then draws" — only on a
    // trigger-prefixed segment (an upkeep wheel engine). Plain wheel
    // spells resolve on cast (`wheel_on_cast`).
    if (lower.starts_with("at the beginning")
        || lower.starts_with("whenever ")
        || lower.starts_with("when "))
        && lower.contains("discards")
        && lower.contains("draws")
        && lower.contains("each player")
    {
        out.push(Ability {
            trigger: trigger_for(lower),
            effect: Effect::Wheel,
            ..Ability::default()
        });
        return true;
    }
    false
}

/// Strip a leading ability-word prefix ("Constellation — When …") so
/// the trigger families see the real shape. Ability words are italic
/// flavor keywords; they never change the trigger itself.
pub(super) fn strip_ability_word(seg: &str) -> &str {
    if let Some((head, rest)) = seg.split_once(" — ") {
        let head_clean = head.trim();
        let single_word = !head_clean.contains(' ');
        let known = matches!(
            head_clean,
            "Constellation"
                | "Landfall"
                | "Raid"
                | "Revolt"
                | "Battle cry"
                | "Spectacle"
                | "Alliance"
                | "Training"
                | "Heroic"
                | "Inspired"
                | "Delirium"
                | "Magecraft"
                | "Descend"
                | "Forge"
                | "Commit"
                | "Will"
                | "Spellcraft"
                | "Start your engines"
                | "Renown"
                | "Afflict"
                | "Battalion"
                | "Bloodrush"
                | "Channel"
                | "Conspire"
        );
        if (single_word || known)
            && (rest.starts_with("When")
                || rest.starts_with("Whenever")
                || rest.starts_with("At the beginning"))
        {
            return rest.trim();
        }
    }
    seg
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
        let mut seg = strip_ability_word(segments[i].trim()).to_string();
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
            // Loyalty abilities (+N/−N:) are separate segments; merging
            // them glues every planeswalker ability into one blob.
            && !(next.starts_with('+') || next.starts_with('−') || next.starts_with('-'))
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
            || landfall_trigger(&lower, &mut out)
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
