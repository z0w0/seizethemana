// Tests for `mana_audit`: weighted source credits, requirement floors,
// gold-card adjustment, and the deficit report.

use super::super::mana_audit::*;
use crate::db::CardRow;

fn card(name: &str, cost: &str, type_line: &str, text: &str, cmc: f64) -> CardRow {
    CardRow {
        name: name.to_string(),
        oracle_id: format!("oid-{name}"),
        mana_cost: cost.to_string(),
        cmc,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.to_string(),
        rarity: "rare".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: "tst".into(),
        collector_number: "1".into(),
        scryfall_id: format!("sid-{name}"),
        released_at: "2020-01-01".into(),
        game_changer: None,
        penny_rank: None,
        reserved: None,
    }
}

fn set_colors(card: &mut CardRow, letters: &str) {
    card.colors = serde_json::to_string(
        &letters
            .chars()
            .map(|c| c.to_string())
            .collect::<Vec<String>>(),
    )
    .unwrap();
}

fn idx(letter: char) -> usize {
    WUBRG.iter().position(|w| *w == letter).unwrap()
}

#[test]
fn lands_credit_one_per_producible_color() {
    let mut w = card("Plains", "", "Basic Land — Plains", "", 0.0);
    set_colors(&mut w, "W");
    let mut ub = card("Drowned Catacomb", "", "Land", "{T}: Add {U} or {B}.", 0.0);
    set_colors(&mut ub, "UB");
    let any = card(
        "Command Tower",
        "",
        "Land",
        "{T}: Add one mana of any color in your commander's color identity.",
        0.0,
    );
    let rows = vec![(w, 10.0), (ub, 4.0), (any, 3.0)];
    let c = audit(&rows, "WUB", true).census;
    assert_eq!(c.lands[idx('W')], 13.0);
    assert_eq!(c.lands[idx('U')], 7.0);
    assert_eq!(c.lands[idx('B')], 7.0);
    assert_eq!(c.sources[idx('W')], 13.0);
    assert_eq!(c.untapped_t1[idx('W')], 13.0);
    assert_eq!(c.lands[idx('R')], 0.0);
    assert_eq!(c.lands[idx('G')], 0.0);
}

/// Identity-bound sources use only the audit's declared commander color palette.
#[test]
fn commander_identity_sources_keep_empty_and_excluded_colors() {
    let rows = vec![
        (
            card(
                "Identity land",
                "",
                "Land",
                "{T}: Add one mana of any color in your commander's color identity.",
                0.0,
            ),
            1.0,
        ),
        (
            card(
                "Identity rock",
                "{2}",
                "Artifact",
                "{T}: Add one mana of any color in your commander's color identity.",
                2.0,
            ),
            1.0,
        ),
    ];
    let empty = audit(&rows, "", true).census;
    assert_eq!(empty.sources, [0.0; 5]);
    assert_eq!(empty.untapped_t1, [0.0; 5]);
    let blue_red = audit(&rows, "UR", true).census;
    assert_eq!(blue_red.lands, [0.0, 1.0, 0.0, 1.0, 0.0]);
    assert_eq!(blue_red.rocks, [0.0, 0.75, 0.0, 0.75, 0.0]);
    assert_eq!(blue_red.sources, [0.0, 1.75, 0.0, 1.75, 0.0]);
    assert_eq!(blue_red.untapped_t1, [0.0, 1.0, 0.0, 1.0, 0.0]);
}

/// Maindeck colors cannot supply commander identity, while unrestricted sources keep their credit.
#[test]
fn constructed_audit_does_not_credit_commander_identity_mana() {
    let rows = vec![
        (
            card(
                "Command Tower",
                "",
                "Land",
                "{T}: Add one mana of any color in your commander's color identity.",
                0.0,
            ),
            1.0,
        ),
        (
            card(
                "Arcane Signet",
                "{2}",
                "Artifact",
                "{T}: Add one mana of any color in your commander's color identity.",
                2.0,
            ),
            1.0,
        ),
        (card("Blue maindeck spell", "{U}", "Sorcery", "", 1.0), 1.0),
    ];
    let constructed = audit(&rows, "U", false);
    assert_eq!(constructed.sources, [0.0; 5]);
    assert_eq!(constructed.census.lands, [0.0; 5]);
    assert_eq!(constructed.census.rocks, [0.0; 5]);
    assert_eq!(constructed.untapped_t1, [0.0; 5]);
    assert_eq!(census(&rows, "U").sources, [0.0; 5]);
    let commander = audit(&rows, "U", true);
    assert_eq!(commander.sources, [0.0, 1.75, 0.0, 0.0, 0.0]);
    assert_eq!(commander.untapped_t1, [0.0, 1.0, 0.0, 0.0, 0.0]);

    let unrestricted = vec![
        (
            card(
                "Unrestricted land",
                "",
                "Land",
                "{T}: Add one mana of any color.",
                0.0,
            ),
            1.0,
        ),
        (
            card(
                "Unrestricted rock",
                "{2}",
                "Artifact",
                "{T}: Add one mana of any color.",
                2.0,
            ),
            1.0,
        ),
    ];
    let audit = audit(&unrestricted, "U", false);
    assert_eq!(audit.sources, [0.0, 1.75, 0.0, 0.0, 0.0]);
    assert_eq!(audit.untapped_t1, [0.0, 1.0, 0.0, 0.0, 0.0]);
}

#[test]
fn dorks_rocks_and_cantrips_take_partial_credits() {
    let dork = card(
        "Llanowar Elves",
        "{G}",
        "Creature — Elf Druid",
        "{T}: Add {G}.",
        1.0,
    );
    let rock = card(
        "Mind Stone",
        "{2}",
        "Artifact",
        "{T}: Add one mana of any color.",
        2.0,
    );
    let cantrip = card("Opt", "{U}", "Instant", "Scry 1, then draw a card.", 1.0);
    let rows = vec![(dork, 4.0), (rock, 4.0), (cantrip, 4.0)];
    let c = census(&rows, "GU");
    assert_eq!(c.dorks[idx('G')], 2.0);
    assert_eq!(c.rocks[idx('U')], 3.0);
    assert_eq!(c.rocks[idx('G')], 3.0);
    assert_eq!(c.cantrips[idx('U')], 1.0);
    assert_eq!(c.cantrips[idx('G')], 1.0);
}

#[test]
fn cantrip_credit_caps_at_ten_effects() {
    let cantrip = card("Opt", "{U}", "Instant", "Scry 1, then draw a card.", 1.0);
    let rows = vec![(cantrip, 14.0)];
    let c = census(&rows, "U");
    assert!(
        c.cantrips[idx('U')] < 4.0,
        "credit stays near the cap: {}",
        c.cantrips[idx('U')]
    );
}

#[test]
fn commander_requirements_follow_the_stored_table() {
    let mut spell = card(
        "Wrath of God",
        "{2}{W}{W}",
        "Sorcery",
        "Destroy all creatures.",
        4.0,
    );
    set_colors(&mut spell, "W");
    let lands = vec![(card("Plains", "", "Basic Land — Plains", "", 0.0), 20.0)];
    let mut all = lands.clone();
    all.push((spell, 1.0));
    let a = audit(&all, "W", true);
    let row = &a.requirements[0];
    let need = row.needs[0].1;
    assert_eq!(need, 17.0, "2 pips of W in commander wants 17 sources");
}

#[test]
fn sixty_card_requirements_and_gold_cards() {
    let mut bolt = card("Lightning Bolt", "{R}", "Instant", "Deal 3 damage.", 1.0);
    set_colors(&mut bolt, "R");
    let mut gold = card("Siege Rhino", "{W}{B}{G}", "Creature", "", 4.0);
    set_colors(&mut gold, "WBG");
    let rows = vec![
        (card("Plains", "", "Basic Land — Plains", "", 0.0), 12.0),
        (card("Island", "", "Basic Land — Island", "", 0.0), 12.0),
        (bolt, 1.0),
        (gold, 1.0),
    ];
    let a = audit(&rows, "WUR", false);
    let bolt_row = a
        .requirements
        .iter()
        .find(|r| r.name == "Lightning Bolt")
        .unwrap();
    assert_eq!(
        bolt_row.needs[0].1, 13.0,
        "1 pip at baseline lands wants 13"
    );
    let gold_row = a
        .requirements
        .iter()
        .find(|r| r.name == "Siege Rhino")
        .unwrap();
    // 3 pips, 1 same → 10; +1 per extra color (2 extras) → 12.
    assert_eq!(gold_row.needs[0].1, 12.0);
}

#[test]
fn requirement_shifts_with_land_count() {
    let mut spell = card(
        "Wrath of God",
        "{2}{W}{W}",
        "Sorcery",
        "Destroy all creatures.",
        4.0,
    );
    set_colors(&mut spell, "W");
    let base = vec![(card("Plains", "", "Basic Land — Plains", "", 0.0), 24.0)];
    let mut with_spell = base.clone();
    with_spell.push((spell.clone(), 1.0));
    let a = audit(&with_spell, "W", false);
    // {2}{W}{W} (2CCC-class): 22 at the 24-25 land baseline.
    assert_eq!(a.requirements[0].needs[0].1, 22.0);
    // Fewer lands: the floor drops by the correction column.
    let fewer = vec![(card("Plains", "", "Basic Land — Plains", "", 0.0), 20.0)];
    let mut fewer_deck = fewer.clone();
    fewer_deck.push((spell, 1.0));
    let a2 = audit(&fewer_deck, "W", false);
    assert_eq!(a2.requirements[0].needs[0].1, 21.0);
}

#[test]
fn deficits_rank_and_format_for_display() {
    let mut spell = card(
        "Wrath of God",
        "{2}{W}{W}",
        "Sorcery",
        "Destroy all creatures.",
        4.0,
    );
    set_colors(&mut spell, "W");
    let mut rows = vec![(card("Island", "", "Basic Land — Island", "", 0.0), 24.0)];
    rows.push((spell, 1.0));
    let a = audit(&rows, "U", false);
    let worst = worst_deficits(&a, 3);
    assert!(!worst.is_empty());
    assert!(worst[0].contains("Wrath of God"), "{}", worst[0]);
    assert!(a.requirements[0].deficit[0].1 > 0.0);
    assert!(!a.requirements[0].ok);
}

#[test]
fn taplands_count_and_hold_t1_sources() {
    // Unconditional tapland.
    let tap = card(
        "Temple of Silence",
        "",
        "Land",
        "Temple of Silence enters the battlefield tapped.",
        0.0,
    );
    // Shock land: the tapped clause is conditional, so it never counts.
    let shock = card(
        "Hallowed Fountain",
        "",
        "Land — Plains Island",
        "As Hallowed Fountain enters the battlefield, you may pay 2 life. If you don't, it enters the battlefield tapped.",
        0.0,
    );
    let untapped = card("Plains", "", "Basic Land — Plains", "", 0.0);
    let rows = vec![(tap, 3.0), (shock, 3.0), (untapped, 10.0)];
    let c = census(&rows, "W");
    assert_eq!(c.tapland_count, 3);
    assert_eq!(c.untapped_t1[idx('W')], 13.0);
}

#[test]
fn mdfc_lands_weight_the_land_count() {
    let mut mdfc = card(
        "Ketria Cradle",
        "",
        "Land // Creature",
        "{T}: Add {G}. // —",
        0.0,
    );
    set_colors(&mut mdfc, "G");
    let rows = vec![
        (card("Forest", "", "Basic Land — Forest", "", 0.0), 10.0),
        (mdfc, 4.0),
    ];
    let lands = effective_lands(&rows);
    assert!(
        (lands - 11.6).abs() < 0.01,
        "10 + 4x0.4 = 11.6, got {lands}"
    );
}

#[test]
fn sixty_pip_shape_table_matches_karsten() {
    // The Karsten 2022 rows the docs quote, keyed on (generic, total,
    // same): CC 21, 1CC 18, 2CCC 22, CCC 23, CCCC 24.
    // "{C}" in the Karsten rows stands for the required color; the test
    // uses {W} so cost_pips sees the pips.
    let cases: &[(&str, f64)] = &[
        ("{W}", 13.0),
        ("{W}{W}", 21.0),
        ("{1}{W}{W}", 18.0),
        ("{2}{W}{W}{W}", 22.0),
        ("{W}{W}{W}", 23.0),
        ("{W}{W}{W}{W}", 24.0),
    ];
    for (cost, want) in cases {
        let mut spell = card(cost, cost, "Sorcery", "", 4.0);
        set_colors(&mut spell, "W");
        let rows = vec![
            (card("Plains", "", "Basic Land — Plains", "", 0.0), 24.0),
            (spell, 1.0),
        ];
        let a = audit(&rows, "W", false);
        let got = a.requirements[0].needs[0].1;
        assert_eq!(
            got, *want,
            "{cost} wants {got} sources, Karsten says {want}"
        );
    }
}

#[test]
fn sixty_fallback_interpolates_unlisted_shapes() {
    // An unlisted shape uses the fallback: same-pip floor minus one per
    // generic pip. {2}{W} = 1 same pip, 2 generic → 13 - 2 = 11.
    let mut spell = card("{2}{W}", "{2}{W}", "Sorcery", "", 4.0);
    set_colors(&mut spell, "W");
    let rows = vec![
        (card("Plains", "", "Basic Land — Plains", "", 0.0), 24.0),
        (spell, 1.0),
    ];
    let a = audit(&rows, "W", false);
    let got = a.requirements[0].needs[0].1;
    assert_eq!(got, 11.0, "fallback floor 13 minus 2 generic pips");
}

#[test]
fn commander_floors_one_and_three_pips() {
    // 12 sources for 1 pip, 21 for 3 pips (the stored table).
    let mut one = card("Sign in Blood", "{B}", "Sorcery", "Draw two cards.", 2.0);
    set_colors(&mut one, "B");
    let mut three = card("Deep Black", "{B}{B}{B}", "Sorcery", "", 3.0);
    set_colors(&mut three, "B");
    let plains = card("Swamp", "", "Basic Land — Swamp", "", 0.0);
    let rows = vec![(plains.clone(), 20.0), (one, 1.0), (three, 1.0)];
    let a = audit(&rows, "B", true);
    assert_eq!(a.requirements[1].needs[0].1, 12.0, "1 pip wants 12");
    assert_eq!(a.requirements[0].needs[0].1, 21.0, "3 pips want 21");
}

#[test]
fn mdfc_land_face_credits_eight_tenths() {
    // A land/spell MDFC's land face counts 0.8 source of its color.
    let mut mdfc = card(
        "Test Split",
        "",
        "Land // Instant",
        "{T}: Add {U}. // Deal 2 damage.",
        0.0,
    );
    set_colors(&mut mdfc, "U");
    let rows = vec![(mdfc, 4.0)];
    let c = census(&rows, "U");
    assert_eq!(
        c.lands[idx('U')],
        3.2,
        "4 MDFC copies credit 0.8 source each"
    );
}

#[test]
fn mythic_mdfc_land_weight_is_three_quarters() {
    // effective_lands weights a mythic land/spell MDFC at 0.75, a rare
    // one at 0.4.
    let mut rare = card(
        "Test Rare Split",
        "",
        "Land // Instant",
        "{T}: Add {U}. // Draw a card.",
        0.0,
    );
    set_colors(&mut rare, "U");
    let mut mythic = card(
        "Test Mythic Split",
        "",
        "Land // Instant",
        "{T}: Add {U}. // Draw a card.",
        0.0,
    );
    set_colors(&mut mythic, "U");
    mythic.rarity = "mythic".into();
    let rare_lands = effective_lands(&[(rare, 2.0)]);
    let mythic_lands = effective_lands(&[(mythic, 2.0)]);
    assert_eq!(rare_lands, 0.8, "2 rare MDFCs weigh 0.4 each");
    assert_eq!(mythic_lands, 1.5, "2 mythic MDFCs weigh 0.75 each");
}

#[test]
fn cantrip_credit_caps_at_exactly_ten_effects() {
    // 0.25 per effect, capped at 10 effects (2.5 sources).
    let mut cantrip = card("Test Cantrip", "{1}{U}", "Instant", "Draw a card.", 2.0);
    set_colors(&mut cantrip, "U");
    let rows = vec![(cantrip, 10.0)];
    let c = census(&rows, "U");
    assert_eq!(
        c.cantrips[idx('U')],
        2.5,
        "10 effects at 0.25 cap at 2.5 exactly"
    );
}

// mana() wrapper

#[test]
fn mana_wrapper_json_exits_ok_and_needs_setup() {
    // JSON mode prints a payload and exits OK; a missing deck errors with
    // a deck-not-found path rather than a panic.
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let result = super::super::mana::mana(&paths, &conn, &mut out, "Nope", None, true);
    assert!(result.is_err(), "unknown deck must error, not panic");
}

#[test]
fn mana_wrapper_renders_json_payload_for_real_deck() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at)
         VALUES ('Forest', 'oid-F', '', 0, 'Basic Land — Forest', '[]', '[]', '[]',
            '({T}: Add {G}.)', 'common', '{}', 'tst', '1', 'sid-F', '2020-01-01')",
        [],
    )
    .unwrap();
    crate::deck::create(
        &paths,
        &mut crate::output::Output::new(true, false, false),
        "D",
    )
    .unwrap();
    std::fs::write(paths.deck_file("D"), "// DECK\n10 Forest\n").unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let code = super::super::mana::mana(&paths, &conn, &mut out, "D", None, true).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
}

/// Every alternative after Add belongs to the ability's output, not its costs.
#[test]
fn talisman_color_alternatives_share_the_ast_parser() {
    let talisman = card(
        "Renamed Talisman",
        "{2}",
        "Artifact",
        "{T}: Add {C}.\n{T}: Add {B} or {G}. This artifact deals 1 damage to you.",
        2.0,
    );
    let c = census(&[(talisman, 1.0)], "BG");
    assert_eq!(c.rocks[idx('B')], 0.75);
    assert_eq!(c.rocks[idx('G')], 0.75);
    let filter = card(
        "Filter",
        "{2}",
        "Artifact",
        "{R}, {T}: Add {B} or {G}.",
        2.0,
    );
    let c = census(&[(filter, 1.0)], "BRG");
    assert_eq!(c.rocks[idx('R')], 0.0);
    assert_eq!(c.rocks[idx('B')], 0.75);
    assert_eq!(c.rocks[idx('G')], 0.75);
}

/// Unknown opponent colors provide no colored-source credit.
#[test]
fn opponent_dependent_credit_does_not_become_all_colors() {
    let rock = card(
        "Unknown colors",
        "{2}",
        "Artifact",
        "{T}: Add one mana of any color that a land an opponent controls could produce.",
        2.0,
    );
    let c = census(&[(rock, 1.0)], "WUBRG");
    assert_eq!(c.rocks, vec![0.0; 5]);
}

/// Conditions that need other cards are not assumed for the first land drop.
#[test]
fn conditional_lands_use_turn_one_entry_conditions() {
    let cases = [
        (
            "Check",
            "Check enters tapped unless you control a Swamp or a Forest.",
            false,
        ),
        (
            "Fast",
            "Fast enters tapped unless you control two or fewer other lands.",
            true,
        ),
        (
            "Slow",
            "Slow enters tapped unless you control two or more other lands.",
            false,
        ),
        (
            "Reveal",
            "As Reveal enters, you may reveal a Forest card from your hand. If you don't, Reveal enters tapped.",
            false,
        ),
        (
            "Shock",
            "As Shock enters, you may pay 2 life. If you don't, Shock enters tapped.",
            true,
        ),
    ];
    for (name, entry, untapped) in cases {
        let land = card(
            name,
            "",
            "Land",
            &format!("{entry}\n{{T}}: Add {{B}} or {{G}}."),
            0.0,
        );
        let c = census(&[(land, 1.0)], "BG");
        assert_eq!(c.untapped_t1[idx('B')], f64::from(untapped), "{name}");
        assert_eq!(c.untapped_t1[idx('G')], f64::from(untapped), "{name}");
    }
}

/// Fetch credit requires an affordable activation and an untapped matching target.
#[test]
fn fetch_turn_one_credit_depends_on_cost_destination_and_targets() {
    let forest = || card("Forest", "", "Basic Land — Forest", "", 0.0);
    let free = || {
        card(
            "Free fetch",
            "",
            "Land",
            "{T}, Pay 1 life, Sacrifice Free fetch: Search your library for a Forest card, put it onto the battlefield, then shuffle.",
            0.0,
        )
    };
    let paid = card(
        "Paid fetch",
        "",
        "Land",
        "{2}, {T}, Sacrifice Paid fetch: Search your library for a Forest card, put it onto the battlefield, then shuffle.",
        0.0,
    );
    let tapped = card(
        "Tapped fetch",
        "",
        "Land",
        "{T}, Sacrifice Tapped fetch: Search your library for a basic land card, put it onto the battlefield tapped, then shuffle.",
        0.0,
    );
    assert_eq!(census(&[(free(), 1.0)], "G").untapped_t1[idx('G')], 0.0);
    assert_eq!(
        census(&[(free(), 1.0), (forest(), 1.0)], "G").untapped_t1[idx('G')],
        2.0
    );
    assert_eq!(
        census(&[(paid, 1.0), (forest(), 1.0)], "G").untapped_t1[idx('G')],
        1.0
    );
    assert_eq!(
        census(&[(tapped, 1.0), (forest(), 1.0)], "G").untapped_t1[idx('G')],
        1.0
    );
    let swamp = card("Swamp", "", "Basic Land — Swamp", "", 0.0);
    let c = census(&[(free(), 1.0), (swamp, 1.0)], "BG");
    assert_eq!(c.untapped_t1[idx('G')], 0.0);
    assert_eq!(c.untapped_t1[idx('B')], 1.0);
    let tapped_target = card(
        "Tapped Forest",
        "",
        "Land — Forest",
        "Tapped Forest enters tapped.",
        0.0,
    );
    assert_eq!(
        census(&[(free(), 1.0), (tapped_target, 1.0)], "G").untapped_t1[idx('G')],
        0.0
    );
}

/// A matching type can grant multiple colors, but a basic-only fetch rejects duals.
#[test]
fn typed_and_basic_fetch_targets_remain_distinct() {
    let dual = || card("Typed dual", "", "Land — Swamp Forest", "", 0.0);
    let typed = card(
        "Typed fetch",
        "",
        "Land",
        "{T}, Sacrifice Typed fetch: Search your library for a Swamp card, put it onto the battlefield, then shuffle.",
        0.0,
    );
    let basic = card(
        "Basic fetch",
        "",
        "Land",
        "{T}, Sacrifice Basic fetch: Search your library for a basic land card, put it onto the battlefield, then shuffle.",
        0.0,
    );
    let c = census(&[(typed, 1.0), (dual(), 1.0)], "BG");
    assert_eq!(c.untapped_t1[idx('B')], 2.0);
    assert_eq!(c.untapped_t1[idx('G')], 2.0);
    let c = census(&[(basic, 1.0), (dual(), 1.0)], "BG");
    assert_eq!(c.untapped_t1[idx('B')], 1.0);
    assert_eq!(c.untapped_t1[idx('G')], 1.0);
}

/// Land-entry costs and bounce rules are not assumed away for early source credit.
#[test]
fn unsupported_land_entry_has_no_early_mana_modes() {
    let chancery = card(
        "Azorius Chancery",
        "",
        "Land",
        "Azorius Chancery enters tapped.\nWhen Azorius Chancery enters, return a land you control to its owner's hand.\n{T}: Add {W}{U}.",
        0.0,
    );
    let ruins = card(
        "Scorched Ruins",
        "",
        "Land",
        "If Scorched Ruins would enter the battlefield, sacrifice two untapped lands instead. If you do, put Scorched Ruins onto the battlefield. If you don't, put it into its owner's graveyard.\n{T}: Add {C}{C}{C}{C}.",
        0.0,
    );
    for land in [chancery, ruins] {
        assert!(simple_land_mana(&land).is_empty(), "{}", land.name);
        assert!(!land_untapped_on_turn_one(&land), "{}", land.name);
        assert_eq!(census(&[(land, 1.0)], "WU").untapped_t1, vec![0.0; 5]);
    }
}

/// Summoning sickness also applies when a creature land comes from a fetch.
#[test]
fn creature_lands_and_their_fetches_have_no_turn_one_tap_credit() {
    let arbor = card("Dryad Arbor", "", "Land Creature — Forest Dryad", "", 0.0);
    let fetch = card(
        "Forest Fetch",
        "",
        "Land",
        "{T}, Sacrifice Forest Fetch: Search your library for a Forest card, put it onto the battlefield, then shuffle.",
        0.0,
    );
    let c = census(&[(arbor, 1.0), (fetch, 1.0)], "G");
    assert_eq!(c.untapped_t1[idx('G')], 0.0);
    assert_eq!(c.lands[idx('G')], 2.0);
    let haste = card(
        "Hasty Grove",
        "",
        "Land Creature — Forest Dryad",
        "Haste",
        0.0,
    );
    assert_eq!(census(&[(haste, 1.0)], "G").untapped_t1[idx('G')], 1.0);
}

/// Unknown opponent production has no weighted colored credit, even on lands.
#[test]
fn opponent_dependent_land_colors_do_not_receive_weighted_credit() {
    let orchard = card(
        "Exotic Orchard",
        "",
        "Land",
        "{T}: Add one mana of any color that a land an opponent controls could produce.",
        0.0,
    );
    let c = census(&[(orchard, 1.0)], "WUBRG");
    assert_eq!(c.lands, vec![0.0; 5]);
    assert_eq!(c.sources, vec![0.0; 5]);
    let independent = card(
        "Renamed source",
        "",
        "Land",
        "{T}: Add {W}.\n{T}: Add one mana of any color that a land an opponent controls could produce.",
        0.0,
    );
    let c = census(&[(independent, 1.0)], "WUBRG");
    assert_eq!(c.lands, vec![1.0, 0.0, 0.0, 0.0, 0.0]);
}

/// Unsupported graveyard colors do not gain either total or early colored credit.
#[test]
fn graveyard_color_condition_is_not_a_weighted_rainbow_land() {
    let land = card(
        "The Grey Havens",
        "",
        "Legendary Land",
        "{T}: Add {C}.\n{T}: Add one mana of any color among legendary creature cards in your graveyard.",
        0.0,
    );
    let c = census(&[(land, 1.0)], "WUBRG");
    assert_eq!(c.lands, vec![0.0; 5]);
    assert_eq!(c.untapped_t1, vec![0.0; 5]);
}

/// Three comma-separated colors retain their normal land and creature credits.
#[test]
fn comma_color_choices_credit_tri_lands_and_hierarchs() {
    let tri_land = card(
        "Renamed tri-land",
        "",
        "Land",
        "{T}: Add {W}, {U}, or {B}.",
        0.0,
    );
    let hierarch = card(
        "Noble Hierarch",
        "{G}",
        "Creature — Human Druid",
        "{T}: Add {G}, {W}, or {U}.",
        1.0,
    );
    let c = census(&[(tri_land, 1.0), (hierarch, 1.0)], "WUBG");
    assert_eq!(c.lands, vec![1.0, 1.0, 1.0, 0.0, 0.0]);
    assert_eq!(c.dorks, vec![0.5, 0.5, 0.0, 0.0, 0.5]);
}
