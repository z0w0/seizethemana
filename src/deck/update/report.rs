//! Typed update-preview reports and price impact calculations.

use crate::deck::grammar::Deck;

/// The cost delta between two deck states: what you would buy and what
/// removals free up.
#[derive(Debug, serde::Serialize)]
pub(super) struct CostImpact {
    /// Missing cards to buy.
    pub(super) to_buy: Buylist,
    /// Value of copies removed from owned deck slots.
    pub(super) freed_usd: f64,
    /// Net spend after the freed value.
    pub(super) net_usd: f64,
}

/// One card to buy.
#[derive(Debug, serde::Serialize)]
pub(super) struct BuyItem {
    /// Card name.
    pub(super) name: String,
    /// Copies needed.
    pub(super) quantity: i64,
    /// Unit price when known.
    pub(super) price_usd: Option<f64>,
    /// Copies already owned across the collection.
    pub(super) owned: i64,
}

/// Grouped list of cards to buy.
#[derive(Debug, serde::Serialize)]
pub(super) struct Buylist {
    /// Cards and quantities.
    pub(super) items: Vec<BuyItem>,
    /// Total price of missing cards.
    pub(super) total_usd: f64,
}

/// Compute the buy gap and value of owned copies freed by a deck change.
pub(super) fn cost_delta(
    conn: &rusqlite::Connection,
    before: &Deck,
    after: &Deck,
) -> anyhow::Result<CostImpact> {
    let owned = crate::collection::owned_counts_all(conn)?;
    let is_basic = crate::collection::is_basic_name;
    let needed = |deck: &Deck| -> std::collections::BTreeMap<String, i64> {
        let mut needed = std::collections::BTreeMap::new();
        for entry in deck.entries() {
            if is_basic(&entry.name) {
                continue;
            }
            *needed.entry(entry.name.clone()).or_insert(0) += entry.quantity;
        }
        needed
    };
    let before_needed = needed(before);
    let after_needed = needed(after);
    let mut items = Vec::new();
    let mut buy_total = 0.0f64;
    let mut buy_names = Vec::new();
    for (card, quantity) in &after_needed {
        let extra = quantity - before_needed.get(card).copied().unwrap_or(0);
        if extra <= 0 {
            continue;
        }
        let short = extra.min((quantity - owned.get(card).copied().unwrap_or(0)).max(0));
        if short <= 0 {
            continue;
        }
        items.push(BuyItem {
            name: card.clone(),
            quantity: short,
            price_usd: None,
            owned: owned.get(card).copied().unwrap_or(0),
        });
        buy_names.push(card.clone());
    }
    let ranges = crate::prints::price_ranges(conn, &buy_names)?;
    for item in &mut items {
        let price = ranges
            .get(&item.name)
            .and_then(crate::prints::PrintRange::price);
        if let Some(price) = price {
            buy_total += price * item.quantity as f64;
        }
        item.price_usd = price;
    }

    let mut freed_names = Vec::new();
    let mut freed = std::collections::BTreeMap::new();
    for (card, quantity) in &before_needed {
        let released = quantity - after_needed.get(card).copied().unwrap_or(0);
        if released > 0 {
            let owned_released = released.min(owned.get(card).copied().unwrap_or(0));
            if owned_released > 0 {
                freed.insert(card.clone(), owned_released);
                freed_names.push(card.clone());
            }
        }
    }
    let freed_ranges = crate::prints::price_ranges(conn, &freed_names)?;
    let mut freed_total = 0.0f64;
    for (card, quantity) in &freed {
        if let Some(price) = freed_ranges
            .get(card)
            .and_then(crate::prints::PrintRange::price)
        {
            freed_total += price * *quantity as f64;
        }
    }
    Ok(CostImpact {
        to_buy: Buylist {
            items,
            total_usd: buy_total,
        },
        freed_usd: freed_total,
        net_usd: buy_total - freed_total,
    })
}

/// Simulate two deck states at the same seed and return their typed diff.
pub(super) fn sim_delta(
    conn: &rusqlite::Connection,
    name: &str,
    before: &Deck,
    after: &Deck,
) -> anyhow::Result<crate::deck::simulator::report_view::ReportDiff> {
    use crate::deck::simulator::report_view::diff_reports;
    let cards = crate::deck::stats::lookup_names(conn, before)?;
    let after_cards = crate::deck::stats::lookup_names(conn, after)?;
    let mut all_cards = cards.clone();
    for (key, card) in after_cards {
        all_cards.entry(key).or_insert(card);
    }
    let seed = 42u64;
    let runs = 2_000u32;
    let before_report =
        crate::deck::simulator::sim_report_for(before, &all_cards, name, runs, None, seed, None)?;
    let after_report =
        crate::deck::simulator::sim_report_for(after, &all_cards, name, runs, None, seed, None)?;
    diff_reports(&before_report, &after_report).map_err(anyhow::Error::msg)
}

/// Render a quantity in the human deck-update preview.
pub(super) fn card_quantity_text(card: &str, quantity: i64) -> String {
    format!("{quantity}x {card}")
}

/// Print the cost impact of a deck update.
pub(super) fn print_cost_block(
    conn: &rusqlite::Connection,
    out: &crate::output::Output,
    before: &Deck,
    after: &Deck,
) -> anyhow::Result<()> {
    let cost = cost_delta(conn, before, after)?;
    let styles = out.styles();
    println!("{}", styles.header("Cost impact"));
    if cost.to_buy.items.is_empty() {
        println!("    {}", styles.dim("Nothing new to buy."));
    } else {
        for item in &cost.to_buy.items {
            let owned_note = if item.owned > 0 {
                format!(" ({} owned)", item.owned)
            } else {
                String::new()
            };
            println!(
                "    {} {}",
                styles.card_name(&item.name),
                styles.dim(&format!(
                    "x{} @ {}{}",
                    item.quantity,
                    item.price_usd
                        .map(|price| format!("${price:.2}"))
                        .unwrap_or_else(|| "unpriced".into()),
                    owned_note
                ))
            );
        }
    }
    println!(
        "    net spend: {} (buy {}, freed {})",
        styles.money(cost.net_usd),
        styles.money(cost.to_buy.total_usd),
        styles.money(cost.freed_usd)
    );
    Ok(())
}
