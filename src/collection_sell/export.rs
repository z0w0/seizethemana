//! ManaBox-compatible list exports containing only allocated sale copies.

use std::collections::BTreeMap;
use std::io::Write;

use super::{Printing, SellReport};

/// Machine-readable sale list formats, separate from the normal human view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SellOutput {
    /// ManaBox text entries with quantity, printing, and foil marker.
    Txt,
    /// ManaBox CSV rows in a non-owning list named Sell.
    Csv,
}

/// One sale printing, combined across its source binders.
struct ExportEntry {
    name: String,
    set_code: String,
    collector_number: String,
    foil: String,
    quantity: i64,
}

/// Merge identical printing allocations without adding retained or review copies.
fn entries(report: &SellReport) -> Vec<ExportEntry> {
    let allocations: Vec<(&str, &[Printing])> = match &report.fund {
        Some(fund) => fund
            .allocations
            .iter()
            .map(|p| (p.name.as_str(), p.printings.as_slice()))
            .collect(),
        None => report
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.printings.as_slice()))
            .collect(),
    };
    let mut entries = Vec::new();
    for (name, prints) in allocations {
        let mut quantities = BTreeMap::new();
        for print in prints.iter().filter(|p| p.sell_quantity > 0) {
            *quantities
                .entry((
                    print.set_code.as_str(),
                    print.collector_number.as_str(),
                    print.foil.as_str(),
                ))
                .or_insert(0) += print.sell_quantity;
        }
        for ((set, number, foil), quantity) in quantities {
            entries.push(ExportEntry {
                name: name.to_string(),
                set_code: set.to_ascii_uppercase(),
                collector_number: number.to_string(),
                foil: foil.to_string(),
                quantity,
            });
        }
    }
    entries
}

/// Write an importable sell list and return its number of printing entries.
///
/// # Errors
/// Propagates writer failures without substituting a partial human result.
pub(super) fn write(
    report: &SellReport,
    format: SellOutput,
    writer: impl Write,
) -> anyhow::Result<usize> {
    let entries = entries(report);
    match format {
        SellOutput::Txt => write_txt(&entries, writer)?,
        SellOutput::Csv => write_csv(&entries, writer)?,
    }
    Ok(entries.len())
}

/// Use the shared ManaBox text grammar without deck headers or human summaries.
fn write_txt(entries: &[ExportEntry], mut writer: impl Write) -> anyhow::Result<()> {
    for entry in entries {
        let line = crate::deck::grammar::DeckEntry {
            name: entry.name.clone(),
            quantity: entry.quantity,
            set_code: Some(entry.set_code.clone()),
            collector_number: Some(entry.collector_number.clone()),
            foil: entry.foil != "normal",
        }
        .to_line();
        writeln!(writer, "{line}")?;
    }
    writer.flush()?;
    Ok(())
}

/// Preserve finish distinctions in CSV, without inventing purchase prices or IDs.
fn write_csv(entries: &[ExportEntry], writer: impl Write) -> anyhow::Result<()> {
    let mut writer = csv::Writer::from_writer(writer);
    writer.write_record([
        "Binder Name",
        "Binder Type",
        "Name",
        "Set code",
        "Collector number",
        "Foil",
        "Quantity",
    ])?;
    for entry in entries {
        writer.write_record([
            "Sell",
            "list",
            &entry.name,
            &entry.set_code,
            &entry.collector_number,
            &entry.foil,
            &entry.quantity.to_string(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}
