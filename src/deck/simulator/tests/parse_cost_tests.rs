use super::parse_cost::{parse_activation_cost, parse_cost, parse_cost_faces};
use super::parse_land::parse_tap_yield;

#[test]
fn parses_plain_and_generic_costs() {
    let cost = parse_cost("{2}{W}{W}");
    assert_eq!(cost.generic, 2);
    assert_eq!(cost.pips[0], 2);
    assert_eq!(cost.total(), 4);
    assert_eq!(parse_cost("{7}").total(), 7);
    assert_eq!(parse_cost("{0}").total(), 0);
}

#[test]
fn x_and_s_cost_one_generic() {
    // The sim pays X and S as one generic each.
    let cost = parse_cost("{X}{R}{R}");
    assert_eq!(cost.generic, 1);
    assert_eq!(cost.pips[3], 2);
    assert_eq!(cost.total(), 3);
    assert_eq!(parse_cost("{S}").total(), 1);
    assert_eq!(parse_cost("{X}").total(), 1);
}

#[test]
fn hybrid_pip_is_flexible() {
    let cost = parse_cost("{W/U}");
    assert_eq!(cost.flex_pips, 1);
    assert_eq!(cost.total(), 1);
    let both = parse_cost("{W/U}{U/B}");
    assert_eq!(both.flex_pips, 2);
}

#[test]
fn phyrexian_pip_is_its_color() {
    let cost = parse_cost("{1}{B/P}");
    assert_eq!(cost.generic, 1);
    assert_eq!(cost.pips[2], 1);
    assert_eq!(cost.total(), 2);
}

#[test]
fn twobrid_pays_two_generic() {
    // {2/W} models the generic payment only: 2 generic, no pip.
    let cost = parse_cost("{2/W}");
    assert_eq!(cost.generic, 2);
    assert_eq!(cost.pips[0], 0);
    assert_eq!(cost.total(), 2);
}

#[test]
fn colorless_only_cost_has_no_pips() {
    let cost = parse_cost("{3}");
    assert_eq!(cost.generic, 3);
    assert_eq!(cost.pips, [0; 5]);
    assert_eq!(cost.flex_pips, 0);
}

#[test]
fn garbage_symbols_do_not_panic_or_double_count() {
    let cost = parse_cost("{garbage}{2}{G}");
    assert_eq!(cost.generic, 2);
    assert_eq!(cost.pips[4], 1);
}

#[test]
fn multi_face_cost_takes_the_cheaper_face() {
    // Split card: cast either face; the model pays the cheaper.
    let cost = parse_cost_faces("{3}{W}{W} // {1}{B}");
    assert_eq!(cost.total(), 2);
    assert_eq!(cost.pips[2], 1);
}

#[test]
fn mdfc_empty_land_face_costs_nothing() {
    let cost = parse_cost_faces("{3}{G} // ");
    assert_eq!(cost.total(), 4);
    assert_eq!(cost.pips[4], 1);
}

#[test]
fn single_face_cost_passes_through_unchanged() {
    let cost = parse_cost_faces("{2}{U}");
    assert_eq!(cost.generic, 2);
    assert_eq!(cost.pips[1], 1);
}

#[test]
fn loyalty_costs_are_free() {
    let cost = parse_activation_cost("−3");
    assert_eq!(cost.total(), 0);
    assert_eq!(parse_activation_cost("0").total(), 0);
    assert_eq!(parse_activation_cost("−7").total(), 0);
}

#[test]
fn plain_numeric_activation_is_free() {
    // A bare number is a loyalty cost, not generic mana.
    assert_eq!(parse_activation_cost("2").total(), 0);
}

#[test]
fn mana_activations_parse() {
    let cost = parse_activation_cost("{1}, {T}");
    assert_eq!(cost.generic, 1);
    assert_eq!(parse_activation_cost("{0}").total(), 0);
}

#[test]
fn tap_yield_for_basic_and_filter_lands() {
    // Land-tap parsing sits next to cost parsing in the cast gate; pin
    // the common shapes so a parse regression fails loudly here.
    let basic = parse_tap_yield("{T}: Add {U}.").unwrap();
    assert_eq!(basic.fixed[1], 1);
    let filter = parse_tap_yield("{T}: Add {R} or {G}.").unwrap();
    assert!(filter.choice[3] && filter.choice[4]);
}
