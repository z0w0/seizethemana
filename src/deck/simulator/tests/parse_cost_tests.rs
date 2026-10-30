//! Tests for Oracle cost parsing.
use super::oracle_parser::cost::{parse_activation_cost, parse_cost, parse_cost_faces};
use super::oracle_parser::land::parse_tap_yield;

#[test]
fn parses_plain_and_generic_costs() {
    let cost = parse_cost("{2}{W}{W}");
    assert_eq!(cost.generic, 2);
    assert_eq!(cost.pips[0], 2);
    assert_eq!(cost.total(), 4);
    assert_eq!(parse_cost("{7}").total(), 7);
    assert_eq!(parse_cost("{0}").total(), 0);
}

/// Each X symbol uses the same chosen value, without an invented minimum.
#[test]
fn x_is_chosen_before_payment_and_snow_stays_separate() {
    let cost = parse_cost("{X}{R}{R}");
    assert_eq!(cost.generic, 0);
    assert_eq!(cost.x_symbols, 1);
    assert_eq!(cost.pips[3], 2);
    assert_eq!(cost.total(), 2);
    assert_eq!(cost.with_x(3).expect("finite X").total(), 5);
    assert_eq!(parse_cost("{S}").total(), 1);
    assert_eq!(parse_cost("{S}").generic, 0);
    assert_eq!(parse_cost("{S}").snow, 1);
    assert_eq!(parse_cost("{X}").total(), 0);
    assert_eq!(
        parse_cost("{X}{X}{B}").with_x(3).expect("finite X").total(),
        7
    );
}

#[test]
fn hybrid_pip_keeps_its_color_choices() {
    let cost = parse_cost("{W/U}");
    assert_eq!(cost.hybrid[0b00011], 1);
    assert_eq!(cost.total(), 1);
    let both = parse_cost("{W/U}{U/B}");
    assert_eq!(both.hybrid[0b00011], 1);
    assert_eq!(both.hybrid[0b00110], 1);
}

#[test]
fn phyrexian_pip_is_its_color() {
    // {B/P} pays black or 2 life (CR 107.4f): the pip rides in the
    // phyrexian lane, payable by life.
    let cost = parse_cost("{1}{B/P}");
    assert_eq!(cost.generic, 1);
    assert_eq!(cost.pips[2], 0);
    assert_eq!(cost.phyrexian[2], 1);
    assert_eq!(cost.total(), 2);
}

#[test]
fn twobrid_keeps_both_payment_options() {
    let cost = parse_cost("{2/W}");
    assert_eq!(cost.generic, 0);
    assert_eq!(cost.twobrid[0], 1);
    assert_eq!(cost.pips[0], 0);
    assert_eq!(cost.total(), 2);
}

#[test]
fn colorless_only_cost_has_no_pips() {
    let cost = parse_cost("{3}");
    assert_eq!(cost.generic, 3);
    assert_eq!(cost.pips, [0; 5]);
    assert_eq!(cost.hybrid, [0; 32]);
}

#[test]
fn colorless_pip_keeps_its_lane() {
    // {C} must be paid with colorless mana (CR 107.4a), never a color.
    let cost = parse_cost("{3}{C}{C}");
    assert_eq!(cost.generic, 3);
    assert_eq!(cost.colorless, 2);
    assert_eq!(cost.pips, [0; 5]);
    assert_eq!(cost.total(), 5);
    assert_eq!(parse_cost("{C}").colorless, 1);
}

#[test]
fn unknown_symbols_make_payment_unsupported() {
    let cost = parse_cost("{garbage}{2}{G}");
    assert_eq!(cost.generic, 2);
    assert_eq!(cost.pips[4], 1);
    assert!(cost.unsupported);
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
