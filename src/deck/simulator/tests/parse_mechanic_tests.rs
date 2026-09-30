//! New-mechanics parse tests: whenever-ETB, landfall family, haste,
//! tokens, X-costs, per-cast mana, kicker, sagas, loyalty activations,
//! and the truth-fix grammar (interaction, treasure banking,
//! X-scaling draws).

use super::deck_test_support::card;
use super::model::*;
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::cost::parse_cost;
use crate::db::CardRow;

/// A card row with keywords.
fn card_kw(name: &str, mana_cost: &str, type_line: &str, keywords: &str, text: &str) -> CardRow {
    let mut row = card(name, mana_cost, type_line, text);
    row.keywords = keywords.to_string();
    row
}

// New-mechanics parse tests: whenever-ETB, landfall family, haste,
// tokens, X-costs, per-cast mana, kicker, sagas, loyalty activations.

#[test]
fn whenever_etb_parses() {
    let sim = parse_sim_card(&card(
        "Chulane-class Plant",
        "{2}{G}{U}",
        "Creature — Human Druid",
        "Whenever another creature you control enters, draw a card.",
    ));
    let etb = sim
        .unlocked_abilities(0)
        .find(|a| a.trigger == SimTrigger::Enters)
        .expect("whenever-ETB parses");
    assert!(matches!(etb.effect, SimEffect::Draw(1)));
}

#[test]
fn etb_scry_is_awareness_not_draw() {
    let sim = parse_sim_card(&card(
        "Prescient",
        "{1}{U}",
        "Creature — Bird Wizard",
        "When this creature enters, scry 2.",
    ));
    let etb = sim
        .unlocked_abilities(0)
        .find(|a| a.trigger == SimTrigger::Enters)
        .expect("scry ETB parsed");
    assert!(matches!(etb.effect, SimEffect::Scry(2)));
}

/// Preserve surveil as a distinct runtime effect rather than scry.
#[test]
fn etb_surveil_keeps_a_distinct_runtime_effect() {
    let sim = parse_sim_card(&card(
        "Graveyard Lookout",
        "{1}{U}",
        "Creature — Bird",
        "When this creature enters, surveil 2.",
    ));
    let trigger = sim
        .unlocked_abilities(0)
        .find(|ability| ability.trigger == SimTrigger::Enters)
        .expect("surveil trigger");
    assert!(matches!(trigger.effect, SimEffect::Surveil(2)));
}

#[test]
fn landfall_engine_parses_draw_and_tokens() {
    let draw = parse_sim_card(&card(
        "Tatyova's Kite",
        "{2}{G}",
        "Creature — Elemental",
        "Landfall — Whenever a land you control enters, you gain 1 life and draw a card.",
    ));
    assert!(
        draw.unlocked_abilities(0).any(|a| {
            a.trigger == SimTrigger::LandEnters
                && a.effect_sequence()
                    .iter()
                    .any(|effect| matches!(effect, SimEffect::Draw(1)))
        }),
        "landfall draw parses"
    );
    let tokens = parse_sim_card(&card(
        "Lotus Cobra",
        "{1}{G}{U}",
        "Creature — Snake",
        "Landfall — Whenever a land you control enters, create a 2/2 green Beast creature token.",
    ));
    assert!(
        tokens.unlocked_abilities(0).any(
            |a| a.trigger == SimTrigger::LandEnters && matches!(a.effect, SimEffect::Tokens(_))
        ),
        "landfall tokens parse"
    );
    let mana = parse_sim_card(&card(
        "Cobra Mana",
        "{1}{G}{U}",
        "Creature — Snake",
        "Landfall — Whenever a land you control enters, add one mana of any color.",
    ));
    assert!(
        mana.unlocked_abilities(0)
            .any(|a| a.trigger == SimTrigger::LandEnters
                && matches!(a.effect, SimEffect::Mana(ref yield_) if yield_.any_pips == 1)),
        "landfall mana keeps its parsed yield"
    );
}

#[test]
fn haste_parses_into_the_flag() {
    let hasted = parse_sim_card(&card_kw(
        "Swift Body",
        "{R}",
        "Creature — Human",
        r#"["Haste"]"#,
        "Haste",
    ));
    assert!(hasted.flags.has_haste);
    let plain = parse_sim_card(&card("Slow Body", "{R}", "Creature — Human", ""));
    assert!(!plain.flags.has_haste);
}

#[test]
fn token_counts_parse() {
    let two = parse_sim_card(&card(
        "Breya",
        "{W}{U}{B}{R}",
        "Legendary Artifact Creature",
        "When Breya enters, create two 1/1 blue Thopter artifact creature tokens with flying.",
    ));
    let t = two
        .unlocked_abilities(0)
        .find(|a| {
            a.trigger == SimTrigger::Enters && matches!(a.effect, SimEffect::Tokens(n) if n == 2)
        })
        .expect("two-token ETB");
    assert!(matches!(t.effect, SimEffect::Tokens(2)));
    let scaled = parse_sim_card(&card(
        "Swarm Host",
        "{3}{G}",
        "Creature — Insect",
        "Whenever a land you control enters, create a 1/1 Insect creature token for each land you control.",
    ));
    assert!(
        scaled
            .unlocked_abilities(0)
            .any(|a| matches!(a.effect, SimEffect::Tokens(8))),
        "for-each tokens cap at 8"
    );
    let one = parse_sim_card(&card(
        "Solo Maker",
        "{1}{W}",
        "Creature — Soldier",
        "When Solo Maker enters, create a 1/1 Soldier creature token.",
    ));
    assert!(
        one.unlocked_abilities(0)
            .any(|a| matches!(a.effect, SimEffect::Tokens(1))),
        "bare 'a token' counts 1"
    );
}

#[test]
fn x_cost_class_parses() {
    let drain = parse_sim_card(&card(
        "Torment of Hailfire",
        "{X}{B}{B}",
        "Sorcery",
        "Target player loses X life for each artifact and creature you control...",
    ));
    assert_eq!(drain.spell_data.x_class, Some(XClass::Drain));
    let draw = parse_sim_card(&card(
        "Blue Sun's Zenith",
        "{X}{U}{U}",
        "Instant",
        "Draw X cards.",
    ));
    assert_eq!(draw.spell_data.x_class, Some(XClass::Draw));
    let tokens = parse_sim_card(&card(
        "March of Woe",
        "{X}{W}{W}",
        "Sorcery",
        "Create X 1/1 white Soldier creature tokens.",
    ));
    assert_eq!(tokens.spell_data.x_class, Some(XClass::Tokens));
    let none = parse_sim_card(&card(
        "Bonfire Lite",
        "{X}{R}",
        "Sorcery",
        "Exile the top card.",
    ));
    assert_eq!(none.spell_data.x_class, None);
}

#[test]
fn x_clause_fixed_rider_is_not_double_counted() {
    // The X amount is paid and applied by the X conversion, so the fixed
    // rider for the matching clause must be zero. Otherwise the effect
    // resolves as X + 1.
    let drain = parse_sim_card(&card(
        "X Drain",
        "{X}{B}{B}",
        "Sorcery",
        "Target player loses X life.",
    ));
    assert_eq!(drain.spell_data.x_class, Some(XClass::Drain));
    assert_eq!(drain.spell_data.life_loss_on_resolve, 0);

    let draw = parse_sim_card(&card("X Draw", "{X}{U}{U}", "Instant", "Draw X cards."));
    assert_eq!(draw.spell_data.x_class, Some(XClass::Draw));
    assert_eq!(draw.spell_data.draws_on_cast, 0);

    let tokens = parse_sim_card(&card(
        "X Tokens",
        "{X}{W}{W}",
        "Sorcery",
        "Create X 1/1 white Soldier creature tokens.",
    ));
    assert_eq!(tokens.spell_data.x_class, Some(XClass::Tokens));
    assert_eq!(tokens.spell_data.tokens_on_cast, 0);
}

#[test]
fn per_cast_mana_engine_parses() {
    let vivi = parse_sim_card(&card(
        "Vivi Ornitier",
        "{1}{U}{R}",
        "Legendary Creature — Wizard",
        "{T}: Add one mana of any color for each instant or sorcery spell you've cast this turn.",
    ));
    assert!(
        vivi.spell_data.mana_per_cast.is_some(),
        "per-cast mana parses"
    );
    let plain = parse_sim_card(&card("Bear", "{1}{G}", "Creature — Bear", "A bear."));
    assert!(plain.spell_data.mana_per_cast.is_none());
}

#[test]
fn kicker_parses() {
    let kicked = parse_sim_card(&card(
        "Kicked Bolt",
        "{1}{R}",
        "Sorcery",
        "Kicker {2}\nKicked Bolt deals 3 damage to target player.",
    ));
    assert_eq!(kicked.spell_data.kicker, Some(parse_cost("{2}")));
    let plain = parse_sim_card(&card("Bolt", "{1}{R}", "Sorcery", "Bolt deals 3."));
    assert_eq!(plain.spell_data.kicker, None);
}

#[test]
fn kicker_colored_pips_parse() {
    let kicked = parse_sim_card(&card(
        "Kicked Prism",
        "{2}{R}",
        "Sorcery",
        "Kicker {1}{G}\nKicked Prism deals 4 damage to any target.",
    ));
    let kicker = kicked.spell_data.kicker.expect("colored kicker parses");
    assert_eq!(kicker.generic, 1);
    assert_eq!(kicker.pips, [0, 0, 0, 0, 1]);
}

#[test]
fn saga_chapters_parse_into_abilities() {
    let saga = parse_sim_card(&card(
        "Tales of Master",
        "{1}{U}",
        "Enchantment — Saga",
        "Read ahead (Choose a chapter and start with that many lore counters.)\nI — Draw a card.\nII — Draw two cards.\nIII — Mill three cards.",
    ));
    assert!(saga.is_saga);
    let chapter_effects: &[Vec<SimEffect>] = &saga.saga.chapters;
    assert_eq!(chapter_effects.len(), 3, "three chapters parse");
    assert!(
        chapter_effects[0]
            .iter()
            .any(|e| matches!(e, SimEffect::Draw(1)))
    );
    assert!(
        chapter_effects[1]
            .iter()
            .any(|e| matches!(e, SimEffect::Draw(2)))
    );
    assert!(
        chapter_effects[2]
            .iter()
            .any(|e| matches!(e, SimEffect::Mill(3)))
    );
}

#[test]
fn loyalty_minus_and_plus_costs_parse() {
    let mut row = card(
        "Test Walker",
        "{2}{W}",
        "Legendary Planeswalker — Test",
        "+1: Draw a card.\n−3: Draw two cards.\n−7: Draw five cards.",
    );
    row.loyalty = Some("4".into());
    let sim = parse_sim_card(&row);
    let abilities: Vec<_> = sim
        .unlocked_abilities(0)
        .filter(|a| a.kind.is_activated())
        .collect();
    assert!(
        abilities.iter().any(|ability| {
            ability
                .activation
                .as_ref()
                .is_some_and(|costs| costs.loyalty_change() == 1)
        }),
        "plus ability gains loyalty"
    );
    assert!(
        abilities.iter().any(|ability| {
            ability
                .activation
                .as_ref()
                .is_some_and(|costs| costs.loyalty_change() == -3)
        }),
        "minus ability spends loyalty"
    );
    assert!(
        abilities.iter().any(|ability| {
            ability
                .activation
                .as_ref()
                .is_some_and(|costs| costs.loyalty_change() == -7)
        }),
        "ultimate parses as minus"
    );
}

#[test]
fn text_flying_joins_evasion_and_flashback_not_flash() {
    let flier = parse_sim_card(&card(
        "Sky Body",
        "{2}{U}",
        "Creature — Bird",
        "This creature has flying.",
    ));
    assert!(
        has_grant(&flier, super::model::Keyword::Flying),
        "text flying counts as a keyword grant"
    );
    let flashback = parse_sim_card(&card(
        "Past Spell",
        "{1}{R}",
        "Sorcery",
        "Deal 1 damage to any target.\nFlashback {3}{R}",
    ));
    assert!(
        !flashback.flags.is_instant_speed,
        "flashback text does not read as flash"
    );
    let flash = parse_sim_card(&card(
        "Quick Spell",
        "{1}{U}",
        "Instant",
        "Counter target spell.",
    ));
    assert!(flash.flags.is_instant_speed);
}

#[test]
fn extra_land_drops_flag_parses() {
    let aesi = parse_sim_card(&card(
        "Aesi",
        "{4}{G}{U}",
        "Legendary Creature — Merfolk",
        "You may play an additional land on each of your turns.",
    ));
    assert!(aesi.flags.extra_land_drops);
    let plain = parse_sim_card(&card("Bear", "{1}{G}", "Creature — Bear", "A bear."));
    assert!(!plain.flags.extra_land_drops);
}

#[test]
fn removal_beats_draw_when_both_match() {
    let both = parse_sim_card(&card(
        "Murky Looting",
        "{1}{B}",
        "Instant",
        "Destroy target creature. Draw a card.",
    ));
    assert_eq!(both.role, Role::Removal, "removal wins over the draw rider");
}

// Truth fixes: ETB-draw double count, interaction grammar,
// wipes, protection, X-classes, ability words, split cards.

#[test]
fn etb_draw_is_trigger_not_cast_rider() {
    let sim = parse_sim_card(&card(
        "ETB Drawer",
        "{2}{U}",
        "Creature — Bird",
        "Flying\nWhen this creature enters, draw a card.",
    ));
    assert_eq!(
        sim.spell_data.draws_on_cast, 0,
        "ETB draw must not double count"
    );
    assert!(
        sim.unlocked_abilities(0)
            .any(|a| a.trigger == SimTrigger::Enters && matches!(a.effect, SimEffect::Draw(1)))
    );
}

#[test]
fn spell_draw_is_cast_rider() {
    let sim = parse_sim_card(&card("Two Cards", "{2}{U}", "Sorcery", "Draw two cards."));
    assert_eq!(sim.spell_data.draws_on_cast, 2);
    assert!(
        !sim.unlocked_abilities(0)
            .any(|a| a.trigger == SimTrigger::Enters),
        "no ETB trigger on a plain draw spell"
    );
}

#[test]
fn multi_target_cost_rider_is_not_lock() {
    let sim = parse_sim_card(&card(
        "Area Burn",
        "{X}{R}",
        "Sorcery",
        "Area Burn deals X damage to any target. This spell costs {1} more to cast for each target beyond the first.",
    ));
    assert_ne!(sim.role, Role::Lock, "X burn is not a tax piece");
    let tax = parse_sim_card(&card(
        "Tax Piece",
        "{2}{W}",
        "Enchantment",
        "Spells your opponents cast cost {1} more to cast.",
    ));
    assert_eq!(tax.role, Role::Lock);
}

#[test]
fn two_damage_removal_counts_as_interaction() {
    let sim = parse_sim_card(&card(
        "Small Burn",
        "{1}{R}",
        "Instant",
        "Small Burn deals 2 damage to target creature.",
    ));
    assert!(sim.flags.is_interaction);
    assert_eq!(sim.role, Role::Removal);
}

#[test]
fn player_burn_stays_drain_not_interaction() {
    let sim = parse_sim_card(&card(
        "Face Burn",
        "{R}",
        "Sorcery",
        "Face Burn deals 2 damage to target player or planeswalker.",
    ));
    assert!(
        !sim.flags.is_interaction,
        "player burn is not removal capacity"
    );
}

#[test]
fn board_wipe_counts_as_interaction() {
    let sim = parse_sim_card(&card(
        "Sweep",
        "{2}{W}{W}",
        "Sorcery",
        "Destroy all creatures.",
    ));
    assert!(sim.flags.sweeps);
    assert!(sim.flags.is_interaction);
    assert_eq!(sim.role, Role::Removal);
}

#[test]
fn targeted_removal_is_not_wipe() {
    let sim = parse_sim_card(&card(
        "Precise Strike",
        "{1}{W}",
        "Instant",
        "Exile target creature.",
    ));
    assert!(!sim.flags.sweeps);
    assert!(sim.flags.is_interaction);
}

#[test]
fn bounce_removal_counts_as_interaction() {
    let sim = parse_sim_card(&card(
        "Bounce Spell",
        "{1}{U}",
        "Instant",
        "Return target creature to its owner's hand.",
    ));
    assert!(sim.flags.is_interaction);
}

#[test]
fn once_each_turn_trigger_grammar() {
    let row = card(
        "Once Engine",
        "{3}{U}",
        "Artifact",
        "Whenever you cast a spell, draw a card. This ability triggers only once each turn.",
    );
    let sim = parse_sim_card(&row);
    let ab = sim
        .unlocked_abilities(0)
        .find(|a| a.trigger == SimTrigger::SpellCast)
        .expect("cast trigger parses");
    assert!(
        ab.once_per_turn,
        "modern once-each-turn phrasing must bound the trigger"
    );
}

#[test]
fn treasure_banking_requires_own_creator() {
    // Treasure and creature tokens stay distinct on their own effects.
    let exec = parse_sim_card(&card(
        "Pitiless Plunderer",
        "{3}{B}",
        "Creature — Zombie Pirate",
        "Whenever Pitiless Plunderer dies, create a Treasure token.",
    ));
    assert!(
        exec.unlocked_abilities(0)
            .any(|ability| matches!(ability.effect, SimEffect::Treasures(1)))
    );
    let unrelated = parse_sim_card(&card(
        "Goblin Token Maker",
        "{1}{R}",
        "Creature — Goblin",
        "When this creature enters, create a 1/1 red Goblin creature token.",
    ));
    assert!(
        unrelated
            .unlocked_abilities(0)
            .any(|ability| matches!(ability.effect, SimEffect::Tokens(1)))
    );
    assert!(
        !unrelated
            .unlocked_abilities(0)
            .any(|ability| matches!(ability.effect, SimEffect::Treasures(_)))
    );

    let mixed = parse_sim_card(&card(
        "Mixed Token Maker",
        "{2}{R}",
        "Creature — Goblin",
        "When this creature enters, create a Treasure token. When it dies, create a 1/1 Goblin creature token.",
    ));
    assert!(
        mixed
            .unlocked_abilities(0)
            .any(|ability| matches!(ability.effect, SimEffect::Treasures(1)))
    );
    assert!(
        mixed
            .unlocked_abilities(0)
            .any(|ability| matches!(ability.effect, SimEffect::Tokens(1)))
    );
}

#[test]
fn draws_x_grammar() {
    let sim = parse_sim_card(&card(
        "Blue X Draw",
        "{X}{U}{U}",
        "Sorcery",
        "Target player draws X cards.",
    ));
    assert_eq!(
        sim.spell_data.x_class,
        Some(XClass::Draw),
        "draws x parses as Draw"
    );
}

#[test]
fn reveal_x_permanents_parses() {
    let sim = parse_sim_card(&card(
        "Wave Spell",
        "{X}{G}{G}",
        "Sorcery",
        "Reveal the top X cards of your library. You may put any number of permanent cards with mana value X or less from among them onto the battlefield, then put the rest into your graveyard.",
    ));
    assert_eq!(sim.spell_data.x_class, Some(XClass::RevealPermanents));
}

#[test]
fn x_counters_parse() {
    let sim = parse_sim_card(&card(
        "Counter Ball",
        "{X}{X}",
        "Artifact Creature — Construct",
        "Walking Ballista enters the battlefield with X +1/+1 counters on it.\nRemove a +1/+1 counter: This creature deals 1 damage to any target.",
    ));
    assert_eq!(sim.spell_data.x_class, Some(XClass::Counters));
}

/// Spell data (additional cost, alternative cost, reveal, self-exile,
/// sacrifice-search, graveyard exchange, flashback/escape grants) lowers
/// from a typed `OracleSpellData` node read once from the Oracle text.
#[test]
fn spell_data_lowers_from_the_typed_spell_node() {
    let additional = parse_sim_card(&card(
        "Cathartic Reunion",
        "{1}{R}",
        "Sorcery",
        "As an additional cost to cast this spell, discard two cards.\nDraw three cards.",
    ));
    assert_eq!(additional.spell_data.additional_cost_discards, 2);
    assert_eq!(additional.spell_data.additional_cost_creatures, 0);

    let sacrifice = parse_sim_card(&card(
        "Culling the Weak",
        "{B}",
        "Instant",
        "As an additional cost to cast this spell, sacrifice a creature.\nAdd {B}{B}{B}{B}.",
    ));
    assert_eq!(sacrifice.spell_data.additional_cost_creatures, 1);

    let life = parse_sim_card(&card(
        "Redirect Lightning",
        "{1}{R}",
        "Instant",
        "As an additional cost to cast this spell, pay 5 life or pay {2}.",
    ));
    assert_eq!(life.spell_data.additional_cost_life, 5);

    let alternative = parse_sim_card(&card(
        "Force of Will",
        "{3}{U}{U}",
        "Instant",
        "You may pay 1 life and exile a blue card from your hand rather than pay this spell's mana cost.",
    ));
    assert!(alternative.spell_data.alternative_cast_cost.is_some());
    assert_eq!(
        alternative.spell_data.alternative_cast_cost.unwrap().count,
        1
    );

    let reveal = parse_sim_card(&card(
        "Ad Nauseam",
        "{3}{B}{B}",
        "Instant",
        "Reveal the top card of your library and put that card into your hand. You lose life equal to its mana value. You may repeat this process any number of times.",
    ));
    assert!(reveal.spell_data.reveal_rule.is_some());

    let self_exile = parse_sim_card(&card(
        "Eldritch Evolution",
        "{1}{G}{G}",
        "Sorcery",
        "Search your library for a creature card with mana value X or less. Exile Eldritch Evolution.",
    ));
    assert!(self_exile.exile_on_resolve);

    let exchange = parse_sim_card(&card(
        "Living End",
        "{2}{B}{B}",
        "Sorcery",
        "Each player exiles all creature cards from their graveyard, then sacrifices all creatures they control, then puts all cards they exiled this way onto the battlefield.",
    ));
    assert!(exchange.spell_data.graveyard_creature_exchange);

    let flashback_grant = parse_sim_card(&card(
        "Past in Flames",
        "{3}{R}",
        "Sorcery",
        "Each instant and sorcery card in your graveyard gains flashback until end of turn. The flashback cost is equal to its mana cost.",
    ));
    assert!(flashback_grant.spell_data.grants_flashback);

    let escape_grant = parse_sim_card(&card(
        "Underworld Breach",
        "{1}{R}",
        "Enchantment",
        "Each nonland card in your graveyard has escape. The escape cost is equal to the card's mana cost plus exile three other cards from your graveyard.",
    ));
    assert!(escape_grant.spell_data.grants_escape);
}

/// A card whose text mentions these phrases without the matching shape
/// stays inert: the typed node carries no spell data.
#[test]
fn spell_data_stays_inert_without_the_matching_shape() {
    let plain = parse_sim_card(&card("Bear", "{1}{G}", "Creature — Bear", "A bear."));
    assert_eq!(plain.spell_data.additional_cost_discards, 0);
    assert_eq!(plain.spell_data.additional_cost_creatures, 0);
    assert_eq!(plain.spell_data.additional_cost_life, 0);
    assert!(plain.spell_data.alternative_cast_cost.is_none());
    assert!(plain.spell_data.reveal_rule.is_none());
    assert!(!plain.exile_on_resolve);
    assert!(!plain.spell_data.graveyard_creature_exchange);
    assert!(!plain.spell_data.grants_flashback);
    assert!(!plain.spell_data.grants_escape);
}

#[test]
fn x_board_buff_flag_parses() {
    let sim = parse_sim_card(&card(
        "Hoof Beast",
        "{5}{G}{G}{G}",
        "Creature — Rhino",
        "Trample\nHoof Beast gets +X/+X where X is the number of creatures you control.",
    ));
    assert!(sim.buffs_battlefield_on_entry);
    let fixed = parse_sim_card(&card(
        "Static Bear",
        "{1}{G}",
        "Creature — Bear",
        "Creatures you control get +1/+1.",
    ));
    assert!(!fixed.buffs_battlefield_on_entry);
}

#[test]
fn ability_word_prefix_stripped() {
    let sim = parse_sim_card(&card(
        "Blossom Eidolon",
        "{3}{G}{U}",
        "Creature — Elf Druid",
        "Constellation — Whenever an enchantment you control enters, draw a card.",
    ));
    assert!(
        sim.unlocked_abilities(0)
            .any(|a| a.trigger == SimTrigger::Enters && matches!(a.effect, SimEffect::Draw(1))),
        "ability word must not block the trigger family"
    );
}

#[test]
fn scaling_draw_engine_flag_parses() {
    let sim = parse_sim_card(&card(
        "Enchantress",
        "{3}{G}",
        "Creature — Human Druid",
        "Whenever an enchantment enters the battlefield under your control, draw a card.",
    ));
    // Not scaling (no "for each"); the plain ETB trigger still parses.
    assert!(sim.draws_per_matching.is_none());
    let scaler = parse_sim_card(&card(
        "Scaling Eidolon",
        "{3}{G}{U}",
        "Enchantment",
        "Whenever an enchantment enters the battlefield under your control, draw a card for each enchantment you control.",
    ));
    assert_eq!(
        scaler.draws_per_matching,
        Some(DrawMatch::Enchantments),
        "'draw a card for each enchantment you control' scales"
    );
}

/// The no-maximum-hand-size flag and the X-counters flag lower from the
/// typed `OracleStaticData` node.
#[test]
fn no_max_hand_size_and_x_counters_lower_from_static_data() {
    let reliquary = parse_sim_card(&card(
        "Reliquary Tower",
        "",
        "Land",
        "You have no maximum hand size.\n{T}: Add {C}.",
    ));
    assert!(reliquary.flags.no_max_hand_size);
    let plain = parse_sim_card(&card(
        "Plains",
        "",
        "Basic Land — Plains",
        "({T}: Add {W}.)",
    ));
    assert!(!plain.flags.no_max_hand_size);

    let x_counters = parse_sim_card(&card(
        "Walking Ballista",
        "{X}{X}",
        "Artifact Creature — Construct",
        "Walking Ballista enters the battlefield with X +1/+1 counters on it.\nRemove a +1/+1 counter: This creature deals 1 damage to any target.",
    ));
    assert!(x_counters.counters_are_power);
    let fixed = parse_sim_card(&card(
        "Fixed Bear",
        "{1}{G}",
        "Creature — Bear",
        "This creature enters with a +1/+1 counter on it.",
    ));
    assert!(!fixed.counters_are_power);
}

#[test]
fn split_card_no_on_cast_credits() {
    let sim = parse_sim_card(&card(
        "Fire // Ice",
        "{1}{R} // {1}{U}",
        "Instant // Instant",
        "Fire deals 2 damage divided as you choose to one or two targets.\n//\nIce tap target permanent, then draw a card.",
    ));
    assert_eq!(sim.spell_data.draws_on_cast, 0, "split cards cast one face");
    assert!(sim.flags.is_interaction, "the damage face qualifies");
    assert_eq!(sim.role, Role::Removal);
    assert!(!sim.draws_per_matching.is_some());
}

#[test]
fn mdfc_spell_face_keeps_on_cast_credits() {
    // A land/spell MDFC is not a split card: casting the spell face is a
    // real cast, so its on-cast draw stays.
    let sim = parse_sim_card(&card(
        "Valakut Awakening // Valakut Stoneforge",
        "{2}{R} // ",
        "Instant // Land",
        "Draw two cards, then discard a card. Landfall — // {T}: Add {R}.",
    ));
    assert!(sim.is_mdfc_spell, "Valakut Awakening is a land/spell MDFC");
    assert_eq!(
        sim.spell_data.draws_on_cast, 2,
        "the MDFC spell face keeps its on-cast draw"
    );
}

#[test]
fn transform_card_keeps_on_cast_credits() {
    // A transform card is not a split card: only the front face is cast,
    // so its on-cast spell data stays.
    let mut row = card(
        "Delver of Secrets // Insectile Aberration",
        "{1}{U} // ",
        "Creature — Human // Creature — Insect",
        "When you cast this spell, draw a card.\nAt the beginning of your upkeep, look at the top card of your library. You may reveal an instant or sorcery card. If you do, transform this creature.\n//\nFlying",
    );
    row.keywords = r#"["Transform"]"#.into();
    let sim = parse_sim_card(&row);
    assert!(
        sim.spell_data.draws_on_cast > 0,
        "the transform card's front-face rider stays"
    );
}

/// A land's own ETB line ("…enters the battlefield tapped, …") with no
/// real landfall rider parses no landfall engine.
#[test]
fn land_tapped_clause_is_not_landfall_engine() {
    let sim = parse_sim_card(&card(
        "Sejiri Glacier Lookalike",
        "",
        "Land",
        "When Sejiri Glacier Lookalike enters the battlefield tapped, \
         you gain 1 life.",
    ));
    assert!(
        !sim.unlocked_abilities(0)
            .any(|a| a.trigger == SimTrigger::LandEnters),
        "land-clause text must not parse as a landfall engine"
    );
}

#[test]
fn static_keyword_grants_drive_evasion_and_self_haste() {
    // Matching grant: an evasion-keyword grant puts the granter in the
    // evasive-body census.
    let grant = parse_sim_card(&card(
        "Evasion Granter",
        "{3}{U}",
        "Enchantment",
        "Creatures you control have flying.",
    ));
    assert!(
        has_grant(&grant, super::model::Keyword::Flying),
        "flying grant parses into the runtime grant list"
    );
    // Nonmatching grant: a non-evasion keyword stays out.
    let vigilance = parse_sim_card(&card(
        "Vigilance Granter",
        "{3}{W}",
        "Enchantment",
        "Creatures you control have vigilance.",
    ));
    assert!(
        has_grant(&vigilance, super::model::Keyword::Vigilance),
        "vigilance grant parses, but is not evasion"
    );
    // Matching self grant: a grant that targets the source sets haste.
    let self_haste = parse_sim_card(&card(
        "Hasty Body",
        "{2}{R}",
        "Creature — Human",
        "This creature has haste.",
    ));
    assert!(self_haste.flags.has_haste, "self haste grant sets haste");
    // Nonmatching: a grant to other creatures never hastes the granter.
    let grant_haste = parse_sim_card(&card(
        "Haste Granter",
        "{2}{R}",
        "Creature — Goblin",
        "Creatures you control have haste.",
    ));
    assert!(
        !grant_haste.flags.has_haste,
        "a grant to other creatures does not haste the granter"
    );
}

#[test]
fn comma_keyword_lines_join_the_ast_keyword_list() {
    let listed = parse_sim_card(&card(
        "Flier",
        "{2}{U}",
        "Creature — Bird",
        "Flying, lifelink",
    ));
    assert!(
        listed
            .printed_keywords
            .contains(super::model::Keyword::Flying),
        "comma keyword line parses into the AST"
    );
    let face2 = parse_sim_card(&card(
        "Double Flier",
        "{1}{U}",
        "Creature — Bird // Instant",
        "Flying\n// Reach, trample",
    ));
    assert!(
        face2
            .printed_keywords
            .contains(super::model::Keyword::Flying)
            && face2
                .printed_keywords
                .contains(super::model::Keyword::Reach),
        "face-two keyword list parses"
    );
    // Nonmatching: an ability reference to flying is not a keyword grant.
    let mention = parse_sim_card(&card(
        "Gainer",
        "{2}{U}",
        "Creature — Bird",
        "Exile three cards from your graveyard: This creature gains flying \
         until end of turn.",
    ));
    assert!(
        !has_grant(&mention, super::model::Keyword::Flying),
        "gains-flying text is not a grant"
    );
}

/// True when the parsed card carries a runtime keyword grant.
fn has_grant(card: &super::model::SimCard, keyword: super::model::Keyword) -> bool {
    card.flags
        .keyword_grants
        .iter()
        .any(|grant| grant.keyword == keyword)
}

#[test]
fn self_cost_reduction_lowers_min_cost() {
    let reduced = parse_sim_card(&card(
        "Discounted Spell",
        "{5}",
        "Sorcery",
        "This spell costs {2} less to cast.",
    ));
    assert_eq!(
        reduced.min_cost.generic, 3,
        "the AST reduction lowers the card's own floor"
    );
    let full = parse_sim_card(&card("Full Price Spell", "{5}", "Sorcery", ""));
    assert_eq!(full.min_cost.generic, 5, "no reduction line, no discount");
    // A grant reduction discounts other spells and stays inert.
    let granter = parse_sim_card(&card(
        "Discount Granter",
        "{5}",
        "Creature — Goblin",
        "Artifact spells you cast cost {1} less to cast.",
    ));
    assert_eq!(
        granter.min_cost.generic, 5,
        "grant reductions do not cheapen the granter"
    );
}
