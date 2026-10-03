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
        lines.push(styles.header("Bulk sell"));
        lines.push(format!(
            "{} copies · {} cards{}",
            report.bulk.copies,
            report.bulk.cards,
            report
                .bulk
                .estimated_proceeds
                .map_or_else(String::new, |v| format!(" · {} estimated", styles.money(v)))
        ));
    } else {
        lines.push(format!(
            "{} · {} cards · {} copies",
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
            styles.header(&format!("Funding ${:.2} USD", fund.target_usd))
        ));
        lines.push(format!(
            "{} selected{}",
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
        lines.push(format!("\nNo {} candidates matched.", report.view));
    }
    if options.review && !report.review.is_empty() {
        lines.push(format!("\n{}", styles.glyph("Review", GlyphKind::Warn)));
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
    lines.join("\n")
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
    lines.push(String::new());
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
            let binders = row
                .printings
                .iter()
                .filter(|p| p.sell_quantity > 0)
                .map(|p| p.binder.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            styles.dim(&cell(
                &binders.into_iter().collect::<Vec<_>>().join(", "),
                last_width,
            ))
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

/// Fit a Unicode cell to its display width without counting ANSI escape codes.
fn cell(value: &str, width: usize) -> String {
    let value = console::truncate_str(value, width, "…");
    format!(
        "{}{}",
        value,
        " ".repeat(width.saturating_sub(console::measure_text_width(&value)))
    )
}

/// Show only present demand evidence and the actual sale printings.
fn row_details(lines: &mut Vec<String>, styles: &Styles, row: &SellRow) {
    let mut evidence = Vec::new();
    if let Some(rank) = row.edhrec_rank {
        evidence.push(format!("EDHREC #{rank}"));
    }
    if let Some(rank) = row.penny_rank {
        evidence.push(format!("Penny #{rank}"));
    }
    if let Some(n) = row.combo_piece_sets.filter(|n| *n > 0) {
        evidence.push(format!("{n} combo sets"));
    }
    if row.owned_combo_options > 0 {
        evidence.push(format!("{} owned options", row.owned_combo_options));
    }
    if row.action == "review" {
        evidence.push(hold_reason(row));
    }
    if row.legal_in_format == Some(false) {
        evidence.push("not legal in selected format".to_string());
    }
    let label = if row.keep_quantity > 0 {
        "Surplus"
    } else {
        "Idle"
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

/// Offer short navigation hints; diagnostic notes are opt-in.
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
            "Showing {} of {total}. Use --limit {} to see more.",
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
        hints.push("For more details: --details".to_string());
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
