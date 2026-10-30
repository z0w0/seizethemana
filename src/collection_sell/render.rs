//! Compact sale tables with optional printing allocations and evidence.

use super::{Printing, SellOptions, SellReport, SellRow};
use crate::output::{GlyphKind, Styles};

/// Print a compact table using the terminal's existing color policy.
pub(super) fn text(out: &crate::output::Output, report: &SellReport, options: &SellOptions<'_>) {
    println!(
        "{}",
        human(
            report,
            options,
            &out.styles(),
            crate::output::terminal_width()
        )
    );
}

/// Format human output independently of terminal I/O for layout checks.
pub(super) fn human(
    report: &SellReport,
    options: &SellOptions<'_>,
    styles: &Styles,
    width: usize,
) -> String {
    let bulk = report.view == "bulk";
    let mut lines = Vec::new();
    if bulk {
        lines.push(styles.header("Bulk sell plan"));
        lines.push(format!(
            "Copies: {} · Card names: {}{}",
            report.bulk.copies,
            report.bulk.cards,
            report
                .bulk
                .estimated_proceeds
                .map_or_else(String::new, |v| format!(
                    " · {} estimated proceeds",
                    styles.money(v)
                ))
        ));
        if let Some(rate) = report.bulk.rate_per_1000 {
            lines.push(format!(
                "Bulk rate: {} per 1,000 copies (your estimate).",
                styles.money(rate)
            ));
        }
    } else {
        lines.push(format!(
            "Sell plan: {} market value · Card names: {} · Copies: {}",
            styles.money(report.singles.market_value),
            report.singles.cards,
            report.singles.copies
        ));
    }
    if !report.excluded_binders.is_empty() {
        lines.push(styles.dim(&format!("Excluded: {}", report.excluded_binders.join(", "))));
    }
    if let Some(fund) = &report.fund {
        lines.push(format!(
            "\n{}",
            styles.header(&format!(
                "Target: ${:.2} USD at market value",
                fund.target_usd
            ))
        ));
        lines.push(format!(
            "{} market value selected{}",
            styles.money(fund.achieved_usd),
            if fund.shortfall_usd > 0.0 {
                format!(" · {} short", styles.money(fund.shortfall_usd))
            } else {
                String::new()
            }
        ));
        for pick in &fund.allocations {
            lines.push(format!(
                "  {} ×{}  {}",
                styles.card_name(&pick.name),
                pick.sell_quantity,
                styles.money(pick.market_value)
            ));
            if options.details {
                allocations(&mut lines, styles, &pick.printings);
            }
        }
    }
    if !report.rows.is_empty() {
        table(
            &mut lines,
            styles,
            &report.rows,
            bulk,
            options.details,
            width,
        );
    } else {
        lines.push(format!(
            "\nNo {} candidates matched. Check filters or run 'stm collection' to review your inventory.",
            report.view
        ));
    }
    if options.review && !report.review.is_empty() {
        lines.push(format!(
            "\n{}",
            styles.glyph("Review before selling", GlyphKind::Warn)
        ));
        table(
            &mut lines,
            styles,
            &report.review,
            bulk,
            options.details,
            width,
        );
    }
    footer(&mut lines, styles, report, options);
    lines
        .into_iter()
        .flat_map(|line| Styles::wrap(&line, width))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Align card names and numeric columns before applying ANSI styles.
fn table(
    lines: &mut Vec<String>,
    styles: &Styles,
    rows: &[SellRow],
    bulk: bool,
    details: bool,
    width: usize,
) {
    let last_width = if bulk {
        width.saturating_sub(26).clamp(10, 18)
    } else {
        12
    };
    let card_width = width.saturating_sub(last_width + 14).clamp(12, 60);
    let stacked = width < 60
        || rows.iter().any(|row| {
            console::measure_text_width(&row.name) > card_width
                || row.sell_quantity.to_string().len() > 4
                || row.keep_quantity.to_string().len() > 4
                || (bulk && console::measure_text_width(&sale_binders(row)) > last_width)
                || (!bulk && format!("${:.2}", row.market_value).len() > last_width)
        });
    lines.push(String::new());
    if stacked {
        for row in rows {
            lines.push(styles.card_name(&row.name));
            lines.push(format!(
                "  Sell: {} · Keep: {}",
                row.sell_quantity, row.keep_quantity
            ));
            lines.push(if bulk {
                format!("  Binder: {}", sale_binders(row))
            } else {
                format!("  Value: {}", styles.money(row.market_value))
            });
            if details {
                row_details(lines, styles, row);
            } else if row.action == "review" {
                lines.push(format!("  {}", hold_reason(row)));
            }
        }
        return;
    }
    lines.push(styles.dim(&format!(
        "{}  {:>4}  {:>4}  {}",
        cell("Card", card_width),
        "Sell",
        "Keep",
        if bulk {
            cell("Binder", last_width)
        } else {
            format!("{:>last_width$}", "Value (USD)")
        }
    )));
    for row in rows {
        let last = if bulk {
            styles.dim(&cell(&sale_binders(row), last_width))
        } else {
            styles.glyph(
                &format!("{:>last_width$}", format!("${:.2}", row.market_value)),
                GlyphKind::Good,
            )
        };
        lines.push(format!(
            "{}  {}  {}  {}",
            styles.card_name(&cell(&row.name, card_width)),
            styles.glyph(&format!("{:>4}", row.sell_quantity), GlyphKind::Info),
            styles.dim(&format!("{:>4}", row.keep_quantity)),
            last
        ));
        if details {
            row_details(lines, styles, row);
        } else if row.action == "review" {
            lines.push(format!("  {}", styles.dim(&hold_reason(row))));
        }
    }
}

/// List every binder that supplies sale copies, in stable order.
fn sale_binders(row: &SellRow) -> String {
    row.printings
        .iter()
        .filter(|printing| printing.sell_quantity > 0)
        .map(|printing| printing.binder.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ")
}

/// Pad a cell by visible columns without clipping its value.
fn cell(value: &str, width: usize) -> String {
    format!(
        "{}{}",
        value,
        " ".repeat(width.saturating_sub(console::measure_text_width(value)))
    )
}

/// Show only present demand evidence and the actual sale printings.
fn row_details(lines: &mut Vec<String>, styles: &Styles, row: &SellRow) {
    let mut evidence = Vec::new();
    if let Some(rank) = row.edhrec_rank {
        evidence.push(format!("EDHREC #{rank}"));
    }
    if let Some(rank) = row.penny_rank {
        evidence.push(format!("Penny Dreadful #{rank}"));
    }
    if let Some(n) = row.combo_piece_sets.filter(|n| *n > 0) {
        evidence.push(format!("{n} combo sets"));
    }
    if row.owned_combo_options > 0 {
        evidence.push(format!("{} owned combo options", row.owned_combo_options));
    }
    if row.action == "review" {
        evidence.push(hold_reason(row));
    }
    if row.legal_in_format == Some(false) {
        evidence.push("not legal in selected format".to_string());
    }
    let label = if row.keep_quantity > 0 {
        "Retains binder copies"
    } else {
        "No binder copies retained"
    };
    lines.push(format!(
        "  {}",
        styles.dim(&format!(
            "{label} · {}{}",
            row.name,
            if evidence.is_empty() {
                String::new()
            } else {
                format!(" · {}", evidence.join(" · "))
            }
        ))
    ));
    allocations(lines, styles, &row.printings);
}

/// Explain actionable personal hold reasons without repeating data disclaimers.
fn hold_reason(row: &SellRow) -> String {
    row.hold_warnings
        .iter()
        .map(|reason| match *reason {
            "reserved_list" => "Reserved List",
            "maybeboard_interest" => "in your maybeboard",
            "unchecked_deck_demand" => "decklist needs checking",
            "unknown_card_metadata" => "card data unavailable",
            _ => reason,
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Identify copies to pull without listing every retained printing.
fn allocations(lines: &mut Vec<String>, styles: &Styles, prints: &[Printing]) {
    for p in prints.iter().filter(|p| p.sell_quantity > 0) {
        lines.push(format!(
            "    ×{} {} · {} {}{}{}",
            p.sell_quantity,
            p.binder,
            p.set_code.to_ascii_uppercase(),
            p.collector_number,
            if p.foil == "normal" {
                String::new()
            } else {
                format!(" {}", p.foil)
            },
            p.price.map_or_else(String::new, |v| format!(
                " · {} each",
                styles.glyph(&format!("${v:.2}"), GlyphKind::Good)
            ))
        ));
    }
}

/// Offer navigation hints and show when the sale evidence needs review.
fn footer(
    lines: &mut Vec<String>,
    styles: &Styles,
    report: &SellReport,
    options: &SellOptions<'_>,
) {
    let total = if options.bulk {
        report.bulk.cards
    } else {
        report.singles.cards
    };
    let mut hints = Vec::new();
    if report.rows.len() < total {
        hints.push(format!(
            "Showing {} of {total} cards. Use --limit {} (maximum 500).",
            report.rows.len(),
            total.min(500)
        ));
    }
    if !options.bulk && report.bulk.copies > 0 {
        hints.push(format!(
            "Bulk: {} copies. View with --bulk.",
            report.bulk.copies
        ));
    }
    if !options.review && report.review_cards > 0 {
        hints.push(format!(
            "Review: {} cards. View with --review.",
            report.review_cards
        ));
    }
    if options.review && report.review.len() < report.review_cards {
        hints.push(format!(
            "{} of {} review cards shown",
            report.review.len(),
            report.review_cards
        ));
    }
    if !options.details {
        hints.push("Use --details for printings and demand evidence.".to_string());
        if !report.warnings.is_empty() {
            lines.push(styles.warning("Deck or combo evidence needs review. The sell plan may miss demand. Read --details before selling."));
        }
    }
    if !options.bulk {
        hints.push("Market value uses stored prices for English printings before selling costs. Missing foil or etched prices use another finish of the same printing. Sale prices may differ.".to_string());
    }
    if !hints.is_empty() {
        lines.push(format!("\n{}", styles.dim(&hints.join("\n"))));
    }
    if options.details {
        for warning in &report.warnings {
            lines.push(styles.dim(warning));
        }
    }
}
