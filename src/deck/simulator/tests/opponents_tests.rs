use super::*;

/// Every modeled seat shares the format's starting life with the player.
#[test]
fn opponent_life_uses_format_rules() {
    for (format, life) in [
        ("commander", 40),
        ("brawl", 25),
        ("oathbreaker", 20),
        ("standard", 20),
    ] {
        let table = OpponentTable::for_rules(crate::deck::simulator::format::rules_for(format));
        assert!(table.seats.iter().all(|seat| seat.life == life));
    }
}

/// Reject payments after the shared sources or discretionary budget run out.
#[test]
fn taxes_share_sources_and_willingness() {
    let mut table = OpponentTable::for_format(Format::Commander);
    table.seats[0].unspent_sources = 4;
    assert!(table.pay_tax(0, 1));
    assert!(table.pay_tax(0, 2));
    assert!(!table.pay_tax(0, 2));
    assert_eq!(table.seats[0].unspent_sources, 1);
    assert_eq!(table.seats[0].taxes_paid, 3);
}

/// Protect the planned action while allowing spare draw-step tax payments.
#[test]
fn draw_step_taxes_preserve_scheduled_action() {
    let mut table = OpponentTable::for_format(Format::Commander);
    table.seats[0].lands = 4;
    table.seats[0].turns = 2;
    assert_eq!(table.begin_turn(0), Some(OpponentEvent::Draw));
    assert!(table.pay_tax(0, 2));
    assert!(!table.pay_tax(0, 1));
    assert_eq!(table.seats[0].unspent_sources, 2);
    assert!(matches!(
        table.main_action(0),
        Some(OpponentEvent::Cast { .. })
    ));
    assert_eq!(table.seats[0].unspent_sources, 0);
    assert_eq!(table.casts, 1);
}

/// A finite land target preserves cards for later scheduled spells.
#[test]
fn land_growth_stops_and_late_spells_remain_funded() {
    let mut table = OpponentTable::for_format(Format::Commander);
    for _ in 0..30 {
        table.begin_turn(0);
        if table.main_action(0).is_some() {
            table.resolve_main_spell(0);
        }
    }
    assert_eq!(table.seats[0].lands, 6);
    assert_eq!(table.seats[0].hand, 2);
    assert_eq!(table.casts, 29);
    assert_eq!(table.draws, 30);
    assert_eq!(table.seats[0].library, 62);
}

/// Optional willingness can run out before the opponent's sources do.
#[test]
fn willingness_cap_is_independent_of_available_sources() {
    let mut table = OpponentTable::for_format(Format::Commander);
    table.seats[0].unspent_sources = 10;
    assert!(table.pay_tax(0, 2));
    assert!(table.pay_tax(0, 2));
    assert!(!table.pay_tax(0, 1));
    assert_eq!(table.seats[0].unspent_sources, 6);
}

/// Cast triggers observe the board before the new permanent resolves.
#[test]
fn cast_event_precedes_permanent_entry() {
    let mut table = OpponentTable::for_format(Format::Commander);
    table.seats[0].lands = 2;
    table.seats[0].turns = 2;
    table.begin_turn(0);
    assert!(matches!(
        table.main_action(0),
        Some(OpponentEvent::Cast {
            noncreature: false,
            ..
        })
    ));
    assert_eq!(table.seats[0].creatures, 0);
    assert_eq!(table.seats[0].hand, 6);
    assert_eq!(table.seats[0].unspent_sources, 0);
    table.resolve_main_spell(0);
    assert_eq!(table.seats[0].creatures, 1);
}

/// A single abstract card cannot be both a land play and a spell.
#[test]
fn depleted_hand_cannot_supply_land_and_spell() {
    let mut table = OpponentTable::for_format(Format::Commander);
    table.seats[0].turns = 2;
    table.seats[0].hand = 0;
    table.seats[0].lands = 2;
    table.begin_turn(0);
    assert_eq!(table.main_action(0), None);
    assert_eq!(table.seats[0].hand, 0);
    assert_eq!(table.seats[0].lands, 3);
    assert_eq!(table.skipped_casts, 1);
}

/// Disabled or eliminated seats do not supply events or optional payments.
#[test]
fn inactive_or_eliminated_opponents_emit_nothing() {
    let mut table = OpponentTable::for_format(Format::Commander);
    table.seats[0].active = false;
    assert_eq!(table.begin_turn(0), None);
    assert_eq!(table.main_action(0), None);
    assert!(!table.pay_tax(0, 0));
    table.config.active = false;
    assert_eq!(table.begin_turn(1), None);
    assert_eq!(table.main_action(1), None);
    assert_eq!(table.draws, 0);
}

/// Targeted pressure does not reduce every opponent's life total.
#[test]
fn targeted_and_table_life_loss_have_distinct_recipients() {
    let mut table = OpponentTable::for_format(Format::Commander);
    assert_eq!(table.lose_life(10, LifeLossScope::EachOpponent), 30);
    assert!(table.seats.iter().all(|seat| seat.life == 30));
    assert_eq!(table.lose_life(30, LifeLossScope::TargetPlayer), 30);
    table.settle_losses();
    assert!(!table.seats[0].active);
    assert_eq!(table.begin_turn(0), None);
    assert_eq!(table.lose_life(5, LifeLossScope::EachOpponent), 10);
    assert_eq!(table.seats[1].life, 25);
    assert_eq!(table.seats[2].life, 25);
}

/// Declared capability survives tapping but disappears when its seat leaves.
#[test]
fn known_colors_differ_from_unknown_and_ignore_source_tap_state() {
    let mut table = OpponentTable::for_format(Format::Commander);
    table.seats[0].lands = 1;
    table.seats[0].unspent_sources = 0;
    assert_eq!(table.known_land_colors(), None);
    table.config.land_colors = Some(vec!["U".into(), "".into(), "".into()]);
    assert_eq!(
        table.known_land_colors(),
        Some([false, true, false, false, false])
    );
    table.seats[0].active = false;
    assert_eq!(table.known_land_colors(), Some([false; 5]));
}
