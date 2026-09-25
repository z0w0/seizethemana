// New-mechanics parse tests: whenever-ETB, landfall family, haste,
// tokens, X-costs, per-cast mana, kicker, sagas, loyalty activations,
// and the Phase 1 truth-fix grammar (interaction, treasure banking,
// X-scaling draws).

use super::model::*;
use super::parse::*;
use crate::db::CardRow;

/// A minimal card row for tests.
fn card(name: &str, mana_cost: &str, type_line: &str, text: &str) -> CardRow {
    CardRow {
        name: name.to_string(),
        oracle_id: String::new(),
        mana_cost: mana_cost.to_string(),
        cmc: parse_cost(mana_cost).total() as f64,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.to_string(),
        rarity: "common".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: String::new(),
        collector_number: String::new(),
        scryfall_id: String::new(),
        released_at: String::new(),
        game_changer: None,
    }
}

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
        .abilities()
        .find(|a| a.trigger == Trigger::OnEnter)
        .expect("whenever-ETB parses");
    assert!(matches!(etb.effect, Effect::Draw(1)));
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
        .abilities()
        .find(|a| a.trigger == Trigger::OnEnter)
        .expect("scry ETB parsed");
    assert!(matches!(etb.effect, Effect::Scry(2)));
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
        draw.abilities()
            .any(|a| a.trigger == Trigger::OnEnter && matches!(a.effect, Effect::Draw(_))),
        "landfall draw parses"
    );
    let tokens = parse_sim_card(&card(
        "Lotus Cobra",
        "{1}{G}{U}",
        "Creature — Snake",
        "Landfall — Whenever a land you control enters, create a 2/2 green Beast creature token.",
    ));
    assert!(
        tokens
            .abilities()
            .any(|a| a.trigger == Trigger::OnEnter && matches!(a.effect, Effect::Tokens(_))),
        "landfall tokens parse"
    );
    let mana = parse_sim_card(&card(
        "Cobra Mana",
        "{1}{G}{U}",
        "Creature — Snake",
        "Landfall — Whenever a land you control enters, add one mana of any color.",
    ));
    assert!(
        mana.abilities()
            .any(|a| a.trigger == Trigger::OnEnter && matches!(a.effect, Effect::ExtraLand)),
        "landfall mana reads as ramp"
    );
}

#[test]
fn haste_skips_sickness_in_game() {
    let hasted = parse_sim_card(&card_kw(
        "Swift Body",
        "{R}",
        "Creature — Human",
        "Haste",
        "Haste",
    ));
    assert!(hasted.has_haste);
    let plain = parse_sim_card(&card("Slow Body", "{R}", "Creature — Human", ""));
    assert!(!plain.has_haste);
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
        .abilities()
        .find(|a| a.trigger == Trigger::OnEnter && matches!(a.effect, Effect::Tokens(n) if n == 2))
        .expect("two-token ETB");
    assert!(matches!(t.effect, Effect::Tokens(2)));
    let scaled = parse_sim_card(&card(
        "Swarm Host",
        "{3}{G}",
        "Creature — Insect",
        "Whenever a land you control enters, create a 1/1 Insect creature token for each land you control.",
    ));
    assert!(
        scaled
            .abilities()
            .any(|a| matches!(a.effect, Effect::Tokens(8))),
        "for-each tokens cap at 8"
    );
    let one = parse_sim_card(&card(
        "Solo Maker",
        "{1}{W}",
        "Creature — Soldier",
        "When Solo Maker enters, create a 1/1 Soldier creature token.",
    ));
    assert!(
        one.abilities()
            .any(|a| matches!(a.effect, Effect::Tokens(1))),
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
    assert_eq!(drain.x_class, Some(XClass::Drain));
    let draw = parse_sim_card(&card(
        "Blue Sun's Zenith",
        "{X}{U}{U}",
        "Instant",
        "Draw X cards.",
    ));
    assert_eq!(draw.x_class, Some(XClass::Draw));
    let tokens = parse_sim_card(&card(
        "March of Woe",
        "{X}{W}{W}",
        "Sorcery",
        "Create X 1/1 white Soldier creature tokens.",
    ));
    assert_eq!(tokens.x_class, Some(XClass::Tokens));
    let none = parse_sim_card(&card(
        "Bonfire Lite",
        "{X}{R}",
        "Sorcery",
        "Exile the top card.",
    ));
    assert_eq!(none.x_class, None);
}

#[test]
fn per_cast_mana_engine_parses() {
    let vivi = parse_sim_card(&card(
        "Vivi Ornitier",
        "{1}{U}{R}",
        "Legendary Creature — Wizard",
        "{T}: Add one mana of any color for each instant or sorcery spell you've cast this turn.",
    ));
    assert!(vivi.mana_per_cast.is_some(), "per-cast mana parses");
    let plain = parse_sim_card(&card("Bear", "{1}{G}", "Creature — Bear", "A bear."));
    assert!(plain.mana_per_cast.is_none());
}

#[test]
fn kicker_parses() {
    let kicked = parse_sim_card(&card(
        "Kicked Bolt",
        "{1}{R}",
        "Sorcery",
        "Kicker {2}\nKicked Bolt deals 3 damage to target player.",
    ));
    assert_eq!(kicked.kicker, Some(parse_cost("{2}")));
    let plain = parse_sim_card(&card("Bolt", "{1}{R}", "Sorcery", "Bolt deals 3."));
    assert_eq!(plain.kicker, None);
}

#[test]
fn kicker_colored_pips_parse() {
    let kicked = parse_sim_card(&card(
        "Kicked Prism",
        "{2}{R}",
        "Sorcery",
        "Kicker {1}{G}\nKicked Prism deals 4 damage to any target.",
    ));
    let kicker = kicked.kicker.expect("colored kicker parses");
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
    let chapter_effects: Vec<&Effect> = saga
        .abilities()
        .filter(|a| a.trigger == Trigger::Activated)
        .map(|a| &a.effect)
        .collect();
    assert_eq!(chapter_effects.len(), 3, "three chapters parse");
    assert!(matches!(chapter_effects[0], Effect::Draw(1)));
    assert!(matches!(chapter_effects[1], Effect::Draw(2)));
    assert!(matches!(chapter_effects[2], Effect::Mill(3)));
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
        .abilities()
        .filter(|a| a.trigger == Trigger::Activated)
        .collect();
    assert!(
        abilities.iter().any(|a| a.loyalty_gain == 1),
        "plus ability gains loyalty"
    );
    assert!(
        abilities.iter().any(|a| a.loyalty_cost == 3),
        "minus ability spends loyalty"
    );
    assert!(
        abilities.iter().any(|a| a.loyalty_cost == 7),
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
    assert!(flier.evasion, "text flying counts as evasion");
    let flashback = parse_sim_card(&card(
        "Past Spell",
        "{1}{R}",
        "Sorcery",
        "Deal 1 damage to any target.\nFlashback {3}{R}",
    ));
    assert!(
        !flashback.is_instant_speed,
        "flashback text does not read as flash"
    );
    let flash = parse_sim_card(&card(
        "Quick Spell",
        "{1}{U}",
        "Instant",
        "Counter target spell.",
    ));
    assert!(flash.is_instant_speed);
}

#[test]
fn extra_land_drops_flag_parses() {
    let aesi = parse_sim_card(&card(
        "Aesi",
        "{4}{G}{U}",
        "Legendary Creature — Merfolk",
        "You may play an additional land on each of your turns.",
    ));
    assert!(aesi.extra_land_drops);
    let plain = parse_sim_card(&card("Bear", "{1}{G}", "Creature — Bear", "A bear."));
    assert!(!plain.extra_land_drops);
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

// Phase 1 truth fixes: ETB-draw double count, interaction grammar,
// wipes, protection, X-classes, ability words, split cards.

#[test]
fn etb_draw_is_trigger_not_cast_rider() {
    let sim = parse_sim_card(&card(
        "ETB Drawer",
        "{2}{U}",
        "Creature — Bird",
        "Flying\nWhen this creature enters, draw a card.",
    ));
    assert_eq!(sim.draws_on_cast, 0, "ETB draw must not double count");
    assert!(
        sim.abilities()
            .any(|a| a.trigger == Trigger::OnEnter && matches!(a.effect, Effect::Draw(1)))
    );
}

#[test]
fn spell_draw_is_cast_rider() {
    let sim = parse_sim_card(&card("Two Cards", "{2}{U}", "Sorcery", "Draw two cards."));
    assert_eq!(sim.draws_on_cast, 2);
    assert!(
        !sim.abilities().any(|a| a.trigger == Trigger::OnEnter),
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
    assert!(sim.is_interaction);
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
    assert!(!sim.is_interaction, "player burn is not removal capacity");
}

#[test]
fn board_wipe_counts_as_interaction() {
    let sim = parse_sim_card(&card(
        "Sweep",
        "{2}{W}{W}",
        "Sorcery",
        "Destroy all creatures.",
    ));
    assert!(sim.wipe);
    assert!(sim.is_interaction);
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
    assert!(!sim.wipe);
    assert!(sim.is_interaction);
}

#[test]
fn bounce_removal_counts_as_interaction() {
    let sim = parse_sim_card(&card(
        "Bounce Spell",
        "{1}{U}",
        "Instant",
        "Return target creature to its owner's hand.",
    ));
    assert!(sim.is_interaction);
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
        .abilities()
        .find(|a| a.trigger == Trigger::OnCastSpell)
        .expect("cast trigger parses");
    assert!(
        ab.once_per_turn,
        "modern once-each-turn phrasing must bound the trigger"
    );
}

#[test]
fn treasure_banking_requires_own_creator() {
    // The fallback that converted any unsourced token effect to pips
    // when any deck card creates Treasures is gone: only the Treasure
    // creator's own tokens bank.
    let mut deck = SimDeck {
        cards: Vec::new(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let exec = parse_sim_card(&card(
        "Pitiless Plunderer",
        "{3}{B}",
        "Creature — Zombie Pirate",
        "Whenever Pitiless Plunderer dies, create a Treasure token.",
    ));
    assert!(exec.treasures_on_token);
    deck.cards.push(exec);
    let unrelated = parse_sim_card(&card(
        "Goblin Token Maker",
        "{1}{R}",
        "Creature — Goblin",
        "When this creature enters, create a 1/1 red Goblin creature token.",
    ));
    assert!(!unrelated.treasures_on_token);
    deck.cards.push(unrelated);
}

#[test]
fn draws_x_grammar() {
    let sim = parse_sim_card(&card(
        "Blue X Draw",
        "{X}{U}{U}",
        "Sorcery",
        "Target player draws X cards.",
    ));
    assert_eq!(sim.x_class, Some(XClass::Draw), "draws x parses as Draw");
}

#[test]
fn reveal_x_permanents_parses() {
    let sim = parse_sim_card(&card(
        "Wave Spell",
        "{X}{G}{G}",
        "Sorcery",
        "Reveal the top X cards of your library. You may put any number of permanent cards with mana value X or less from among them onto the battlefield, then put the rest into your graveyard.",
    ));
    assert_eq!(sim.x_class, Some(XClass::RevealPermanents));
}

#[test]
fn x_counters_parse() {
    let sim = parse_sim_card(&card(
        "Counter Ball",
        "{X}{X}",
        "Artifact Creature — Construct",
        "Walking Ballista enters the battlefield with X +1/+1 counters on it.\nRemove a +1/+1 counter: This creature deals 1 damage to any target.",
    ));
    assert_eq!(sim.x_class, Some(XClass::Counters));
}

#[test]
fn x_board_buff_flag_parses() {
    let sim = parse_sim_card(&card(
        "Hoof Beast",
        "{5}{G}{G}{G}",
        "Creature — Rhino",
        "Trample\nHoof Beast gets +X/+X where X is the number of creatures you control.",
    ));
    assert!(sim.buffs_board_on_enter);
    let fixed = parse_sim_card(&card(
        "Static Bear",
        "{1}{G}",
        "Creature — Bear",
        "Creatures you control get +1/+1.",
    ));
    assert!(!fixed.buffs_board_on_enter);
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
        sim.abilities()
            .any(|a| a.trigger == Trigger::OnEnter && matches!(a.effect, Effect::Draw(1))),
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

#[test]
fn split_card_no_on_cast_credits() {
    let sim = parse_sim_card(&card(
        "Fire // Ice",
        "{1}{R} // {1}{U}",
        "Instant // Instant",
        "Fire deals 2 damage divided as you choose to one or two targets.\n//\nIce tap target permanent, then draw a card.",
    ));
    assert_eq!(sim.draws_on_cast, 0, "split cards cast one face");
    assert!(sim.is_interaction, "the damage face qualifies");
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
        sim.draws_on_cast, 2,
        "the MDFC spell face keeps its on-cast draw"
    );
}

#[test]
fn transform_card_keeps_on_cast_credits() {
    // A transform card is not a split card: only the front face is cast,
    // so its on-cast riders stay.
    let mut row = card(
        "Delver of Secrets // Insectile Aberration",
        "{1}{U} // ",
        "Creature — Human // Creature — Insect",
        "When you cast this spell, draw a card.\nAt the beginning of your upkeep, look at the top card of your library. You may reveal an instant or sorcery card. If you do, transform this creature.\n//\nFlying",
    );
    row.keywords = "Transform".into();
    let sim = parse_sim_card(&row);
    assert!(
        sim.draws_on_cast > 0,
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
        !sim.abilities()
            .any(|a| a.trigger == Trigger::OnEnter && matches!(a.effect, Effect::ExtraLand)),
        "land-clause text must not parse as a landfall engine"
    );
}
