use super::*;
use crate::deck::simulator::game_mana::pay_cost;
use crate::deck::simulator::oracle_parser::cost::parse_cost;

/// One cost symbol for the independent, unit-by-unit reference solver.
#[derive(Clone, Copy)]
enum Symbol {
    Mana(u8),
    Twobrid(u8),
}

/// Try each physical mana unit that can pay a required symbol.
fn reference_one(types: u8, rest: &[Symbol], mana: &[(u8, bool)]) -> bool {
    for (index, &(available, generic)) in mana.iter().enumerate() {
        let eligible = if types == 0 {
            generic
        } else {
            types & available != 0
        };
        if eligible {
            let mut remaining = mana.to_vec();
            remaining.remove(index);
            if reference_symbols(rest, &remaining) {
                return true;
            }
        }
    }
    false
}

/// Try every distinct pair of mana units for a twobrid's generic option.
fn reference_two_generic(rest: &[Symbol], mana: &[(u8, bool)]) -> bool {
    for first in 0..mana.len() {
        if !mana[first].1 {
            continue;
        }
        for second in first + 1..mana.len() {
            if mana[second].1 {
                let mut remaining = mana.to_vec();
                remaining.remove(second);
                remaining.remove(first);
                if reference_symbols(rest, &remaining) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check alternative costs by trying their legal payments directly.
fn reference_symbols(symbols: &[Symbol], mana: &[(u8, bool)]) -> bool {
    let Some((&symbol, rest)) = symbols.split_first() else {
        return true;
    };
    match symbol {
        Symbol::Mana(types) => reference_one(types, rest, mana),
        Symbol::Twobrid(types) => {
            reference_one(types, rest, mana) || reference_two_generic(rest, mana)
        }
    }
}

/// Expand actual source choices before trying individual unit payments.
fn reference_bundles(
    symbols: &[Symbol],
    lots: &[&super::super::ManaLot],
    mana: &mut Vec<(u8, bool)>,
) -> bool {
    let Some((&lot, rest)) = lots.split_first() else {
        return reference_symbols(symbols, mana);
    };
    let types: Vec<u8> = if lot.one_color {
        (0..6)
            .map(|bit| 1 << bit)
            .filter(|bit| lot.types & bit != 0)
            .collect()
    } else {
        vec![lot.types]
    };
    let start = mana.len();
    for types in types {
        mana.extend(std::iter::repeat_n(
            (types, !lot.cannot_pay_generic),
            lot.amount as usize,
        ));
        let payable = reference_bundles(symbols, rest, mana);
        mana.truncate(start);
        if payable {
            return true;
        }
    }
    false
}

/// Build larger-than-one mana lots with independently varied spend rules.
fn sample_lot(seed: usize, first: bool) -> super::super::ManaLot {
    let masks = [1, 2, 3, COLORLESS, COLORLESS | 1, COLORED];
    super::super::ManaLot {
        amount: if first {
            2 + (seed / 36 % 4) as u32
        } else {
            2 + (seed / 144) as u32
        },
        types: masks[if first { seed % 6 } else { seed / 6 % 6 }],
        restriction: (first && (seed / 8).is_multiple_of(2)).then_some(SpendRestriction::Artifact),
        cannot_pay_generic: first && (seed / 4).is_multiple_of(2),
        one_color: if first {
            seed.is_multiple_of(2)
        } else {
            (seed / 2).is_multiple_of(2)
        },
    }
}

/// Compare grouped quantities, restrictions, and twobrid against separate unit choices.
#[test]
fn mixed_lots_and_twobrid_match_an_independent_unit_solver() {
    use Symbol::{Mana, Twobrid};
    let costs: &[(&str, &[Symbol])] = &[
        ("{2/W}", &[Twobrid(1)]),
        ("{1}{2/W}{2/U}", &[Mana(0), Twobrid(1), Twobrid(2)]),
        ("{W/U}{2/W}{C}", &[Mana(3), Twobrid(1), Mana(COLORLESS)]),
        ("{W}{2/U}{2/U}", &[Mana(1), Twobrid(2), Twobrid(2)]),
        ("{2}{W/U}{2/W}", &[Mana(0), Mana(0), Mana(3), Twobrid(1)]),
        (
            "{U}{U}{2/W}{2/U}",
            &[Mana(2), Mana(2), Twobrid(1), Twobrid(2)],
        ),
    ];
    for seed in 0..288 {
        let mut lots = vec![sample_lot(seed, true), sample_lot(seed, false)];
        if seed.is_multiple_of(4) {
            lots.push(lots[0].clone());
        }
        let pool = ManaPool {
            lots,
            generic_credit: (seed / 32 % 2) as u32,
            colorless: (seed / 64 % 2) as u32,
            treasures: (seed / 128 % 2) as u32,
            ..ManaPool::default()
        };
        let classes = if (seed / 16).is_multiple_of(2) {
            &[][..]
        } else {
            &[SpendRestriction::Artifact][..]
        };
        let eligible: Vec<_> = pool
            .lots
            .iter()
            .filter(|lot| lot.restriction.is_none_or(|class| classes.contains(&class)))
            .collect();
        let mut units = vec![(0, true); pool.generic_credit as usize];
        units.extend(std::iter::repeat_n(
            (COLORLESS, true),
            pool.colorless as usize,
        ));
        units.extend(std::iter::repeat_n(
            (COLORED, true),
            pool.treasures as usize,
        ));
        for &(text, symbols) in costs {
            let expected = reference_bundles(symbols, &eligible, &mut units);
            let mut after = pool.clone();
            let plan = payment_plan(&parse_cost(text), &after, classes);
            assert_eq!(plan.is_some(), expected, "seed={seed}, cost={text}");
            if let Some(plan) = plan {
                assert!(plan.commit(&mut after));
                let spent = pool.total() - after.total();
                let maximum: u32 = symbols
                    .iter()
                    .map(|symbol| match symbol {
                        Symbol::Mana(_) => 1,
                        Symbol::Twobrid(_) => 2,
                    })
                    .sum();
                assert!((symbols.len() as u32..=maximum).contains(&spent));
            } else {
                assert_eq!(after, pool);
            }
        }
    }
}

/// Validate large capacities with an independent count-based twobrid calculation.
#[test]
fn large_lots_and_twobrid_match_independent_resource_counts() {
    let cost = Cost {
        generic: 4090,
        pips: [3, 0, 0, 0, 0],
        colorless: 2,
        twobrid: [6, 0, 0, 0, 0],
        ..Cost::default()
    };
    for white in [0, 1, 3, 5, 4096] {
        for colorless in [0, 4090, 4102] {
            for credit in [0, 1, 9] {
                for no_generic in [false, true] {
                    let expected = colorless >= 2
                        && (0..=6).any(|colored| {
                            let needed_white = 3 + colored;
                            let needed_generic = 4090 + 2 * (6 - colored);
                            white >= needed_white
                                && credit + colorless - 2
                                    + if no_generic { 0 } else { white - needed_white }
                                    >= needed_generic
                        });
                    let mut pool = ManaPool {
                        lots: vec![super::super::ManaLot {
                            amount: white,
                            types: 1,
                            restriction: None,
                            cannot_pay_generic: no_generic,
                            one_color: false,
                        }],
                        generic_credit: credit,
                        colorless,
                        ..ManaPool::default()
                    };
                    let before = pool.clone();
                    assert_eq!(
                        pay_cost(&cost, &mut pool),
                        expected,
                        "white={white}, colorless={colorless}, credit={credit}, no_generic={no_generic}"
                    );
                    if expected {
                        assert!((4101..=4107).contains(&(before.total() - pool.total())));
                    } else {
                        assert_eq!(pool, before);
                    }
                }
            }
        }
    }
}
