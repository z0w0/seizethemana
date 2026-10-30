//! Check real simulator reports for the frozen Commander precon fixtures.
//!
//! The selected card row is a draw-independent mana-readiness measure. The
//! turn-eight signal is a deck-level measure; it does not claim that the
//! selected card was drawn, cast, or caused the aggregate result.
use super::deck_test_support::{COMMANDER_FIXTURES, fixture, fixture_deck, fixture_map};
use super::report_schema::SimReport;
use super::*;

/// The report signal that best matches a supported line in a precon.
#[derive(Clone, Copy)]
enum ReportSignal {
    Creatures,
    CardsSeen,
    ManaAvailable,
}

/// Check that each precon produces its expected real-deck report signal.
#[test]
fn sourced_precons_produce_deck_specific_simulator_output() {
    let cases = [
        (
            "mtgjson dance of the elements",
            "Springleaf Parade",
            ReportSignal::Creatures,
        ),
        (
            "mtgjson blight curse",
            "Sinister Gnarlbark",
            ReportSignal::CardsSeen,
        ),
        (
            "mtgjson family matters",
            "Agate Instigator",
            ReportSignal::Creatures,
        ),
        (
            "mtgjson buckle up",
            "Shorikai, Genesis Engine",
            ReportSignal::CardsSeen,
        ),
        (
            "mtgjson enduring enchantments",
            "Sythis, Harvest's Hand",
            ReportSignal::CardsSeen,
        ),
        (
            "mtgjson ahoy mateys",
            "Enterprising Scallywag",
            ReportSignal::ManaAvailable,
        ),
        (
            "mtgjson blast from the past",
            "Vrestin, Menoptra Leader",
            ReportSignal::Creatures,
        ),
        (
            "mtgjson quantum quandrix",
            "Biomathematician",
            ReportSignal::Creatures,
        ),
    ];

    for (fixture_id, signal_card, signal) in cases {
        assert!(
            COMMANDER_FIXTURES.contains(&fixture_id),
            "{fixture_id} is in the Commander sweep"
        );
        let source = fixture(fixture_id);
        let cards = fixture_map(fixture_id);
        let deck = fixture_deck(fixture_id);
        let main_count: i64 = source.list.iter().map(|(_, count)| count).sum();
        assert_eq!(main_count + source.commanders.len() as i64, 100);

        let report = sim_report_for(&deck, &cards, fixture_id, 64, Some(8), 20261019, None)
            .expect("precon report runs");
        assert_precon_report(&report, &source, &cards, fixture_id, signal_card, signal);
    }
}

/// Check report values against source cards and the expected deckbuilding signal.
fn assert_precon_report(
    report: &SimReport,
    source: &super::deck_test_support::DeckFixture,
    cards: &std::collections::HashMap<String, crate::db::CardRow>,
    fixture_id: &str,
    signal_card: &str,
    signal: ReportSignal,
) {
    assert_eq!(report.name, fixture_id);
    assert_eq!(report.format, "commander");
    assert_eq!(report.runs, 64);
    assert_eq!(report.turns, 8);
    assert_eq!(report.deck_shape.total_cards, 100);
    assert_eq!(
        report.commander.as_ref().map(|row| row.name.as_str()),
        source.commanders.first().map(|row| row.name.as_str())
    );

    let card = cards
        .get(signal_card)
        .unwrap_or_else(|| panic!("{fixture_id} source is missing {signal_card}"));
    let printed = super::oracle_lower::parse_sim_card(card);
    let row = report
        .card_castability
        .iter()
        .find(|row| row.name == signal_card)
        .unwrap_or_else(|| panic!("{fixture_id} report omits {signal_card} castability"));
    assert_eq!(row.mana_value, f64::from(printed.cost.total()));
    assert_eq!(row.target_turn, printed.min_cost.total().max(1));
    let castable = row
        .percent_castable_by_target
        .expect("target turn is within the horizon")
        .value();
    assert!(
        (0.0..=100.0).contains(&castable),
        "{fixture_id} {signal_card} castability is invalid: {castable}"
    );
    assert!(
        castable > 0.0,
        "{fixture_id} shows no on-curve mana readiness for {signal_card}"
    );

    let value = match signal {
        ReportSignal::Creatures => report.creatures_by_turn.get(&8),
        ReportSignal::CardsSeen => report.velocity.avg_cards_seen.get(&8),
        ReportSignal::ManaAvailable => report.mana.avg_available_by_turn.get(&8),
    }
    .copied()
    .unwrap_or_else(|| panic!("{fixture_id} report omits its expected turn-eight signal"));
    let minimum = match signal {
        ReportSignal::Creatures => 0.5,
        ReportSignal::CardsSeen => 10.0,
        ReportSignal::ManaAvailable => 1.0,
    };
    assert!(
        value.is_finite() && value >= minimum,
        "{fixture_id} {signal_card} report signal is too low: {value}"
    );
}
