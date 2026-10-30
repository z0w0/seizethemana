//! Command flow and text or JSON output for `stm query`.

use crate::cli;
use crate::cli::codes;
use crate::output::Output;
use crate::paths::Paths;

/// Full card detail with its semantic search score.
#[derive(Debug, Clone, serde::Serialize)]
struct QueryCardReport {
    /// Card detail shared with `stm card`.
    #[serde(flatten)]
    card: crate::card::CardReport,
    /// Semantic search score, or zero for a filter-only listing.
    score: f64,
}

/// Entry point for `stm query`.
/// Keep filters, search, and JSON detail in one SQL read view.
#[allow(clippy::too_many_arguments)]
pub fn run_query(
    paths: &Paths,
    conn: &mut rusqlite::Connection,
    out: &mut Output,
    text: &str,
    cli_filters: &cli::CardFilters,
    max_price: Option<f64>,
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    let filters = super::CardFilters::from_cli(cli_filters)?;
    let snapshot = if text.trim().is_empty() {
        None
    } else {
        Some(super::RankedSnapshot::open(paths)?)
    };
    let conn = snapshot
        .as_ref()
        .map_or(&*conn, super::RankedSnapshot::connection);
    let _listing_read = if snapshot.is_none() {
        Some(conn.unchecked_transaction()?)
    } else {
        None
    };
    let price_allow: Option<std::collections::HashSet<String>> = match max_price {
        Some(max_price) => Some(
            crate::prints::names_under_price(conn, max_price)?
                .into_iter()
                .collect(),
        ),
        None => None,
    };
    #[cfg(test)]
    super::after_scope_for_test();
    let search = match &snapshot {
        Some(snapshot) => {
            let task = "cards (preparing text search)";
            out.status("Searching", task);
            out.progress_bar("Searching", task, 0);
            let result = snapshot.search(text, &filters, limit as usize, price_allow.as_ref());
            out.clear_progress();
            result
        }
        None => super::filter_only(conn, &filters, limit as usize, price_allow.as_ref()),
    };
    let hits = match search {
        Ok(hits) => hits,
        Err(err) => {
            if !paths.is_setup() {
                out.error(&format!("{err:#}"));
                out.hint("run 'stm setup' first");
                return Ok(codes::ERROR);
            }
            return Err(err);
        }
    };
    if hits.is_empty() {
        if json {
            print_json(Vec::<QueryCardReport>::new())?;
        } else if max_price.is_some() {
            out.error("no cards matched");
            out.hint("remove filters or raise --max-price; unpriced cards stay excluded");
        } else {
            out.error("no cards matched");
            out.hint(if text.trim().is_empty() {
                "remove filters to list more cards"
            } else {
                "try broader words, or drop filters"
            });
        }
        return Ok(codes::NO_RESULTS);
    }
    if json {
        let names: Vec<String> = hits.iter().map(|hit| hit.card.name.clone()).collect();
        let ranges = crate::prints::price_ranges(conn, &names)?;
        let tags = crate::tags::TagIndex::load(conn)?;
        let owned = crate::collection::owned_counts_all(conn)?;
        let available = crate::collection::available_counts_all(conn)?;
        let items = hits
            .iter()
            .map(|hit| -> anyhow::Result<QueryCardReport> {
                let range = ranges.get(&hit.card.name).cloned().unwrap_or_default();
                let universe =
                    crate::universe::card_universe(conn, &hit.card.name, &hit.card.set_code)?;
                let card =
                    crate::card::card_json(&hit.card, &tags, &range, &universe, &owned, &available);
                Ok(QueryCardReport {
                    card,
                    score: (f64::from(hit.score) * 10_000.0).round() / 10_000.0,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        print_json(items)?;
    } else {
        print_text(out, &hits, !text.trim().is_empty());
    }
    Ok(codes::OK)
}

/// Print a sequence of typed results as a JSON array.
fn print_json<T: serde::Serialize>(items: Vec<T>) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
}

/// Print results as numbered lines with score, cost, and type line.
fn print_text(out: &Output, hits: &[super::Hit], ranked: bool) {
    let styles = out.styles();
    let rank_width = hits.len().to_string().len().max(2);
    for (index, hit) in hits.iter().enumerate() {
        let line = format!(
            "{:>width$}. {} {} {} {} {}",
            index + 1,
            styles.card_name(&hit.card.name),
            styles.mana_pips(&hit.card.mana_cost),
            styles.rarity(&hit.card.rarity),
            styles.dim(&hit.card.type_line),
            if ranked {
                styles.dim(&format!("(score {:.3})", hit.score))
            } else {
                String::new()
            },
            width = rank_width,
        );
        out.print_wrapped(&line);
    }
}
