//! Rendering and theme-word helpers for `deck suggest`, split from
//! suggest.rs to keep files small.

use super::Suggestion;
use crate::db::CardRow;
use crate::deck::Deck;

pub(super) fn deck_theme_words(
    deck: &Deck,
    cards_by_name: &std::collections::HashMap<String, CardRow>,
) -> String {
    let stop: std::collections::HashSet<&str> = [
        "the",
        "of",
        "and",
        "a",
        "an",
        "creature",
        "token",
        "legendary",
        "enchantment",
        "instant",
        "sorcery",
        "artifact",
        "land",
        "planeswalker",
        "battle",
        "spell",
    ]
    .into_iter()
    .collect();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for entry in deck.entries() {
        if let Some(card) = cards_by_name.get(&entry.name) {
            for word in card.type_line.split(&[' ', '—', ',']) {
                let w = word.trim();
                if w.len() >= 4 && !stop.contains(w.to_ascii_lowercase().as_str()) {
                    *counts.entry(w.to_string()).or_insert(0) += entry.quantity as usize;
                }
            }
        }
    }
    let mut words: Vec<(String, usize)> = counts.into_iter().collect();
    words.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    words.truncate(4);
    let joined = words
        .iter()
        .map(|(w, _)| w.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        "tribal commander".to_string()
    } else {
        format!("{joined} commander")
    }
}

/// JSON rows for the suggestion list.
pub(super) fn print_json(suggestions: &[Suggestion]) -> anyhow::Result<()> {
    println!("{}", suggestions_json(suggestions)?);
    Ok(())
}

/// The pretty-printed JSON payload for the suggestion list (split from
/// the printer so the shape is assertable).
fn suggestions_json(suggestions: &[Suggestion]) -> anyhow::Result<String> {
    let items: Vec<serde_json::Value> = suggestions
        .iter()
        .map(|s| {
            let identity: serde_json::Value =
                serde_json::from_str(&s.card.color_identity).unwrap_or_default();
            serde_json::json!({
                "name": s.card.name,
                "oracle_id": s.card.oracle_id,
                "mana_cost": s.card.mana_cost,
                "cmc": s.card.cmc,
                "type_line": s.card.type_line,
                "edhrec_rank": s.card.edhrec_rank,
                "game_changer": s.card.game_changer,
                "owned": s.owned,
                "price": s.price_usd,
                "score": (f64::from(s.score) * 10_000.0).round() / 10_000.0,
                "tags": s.tags,
                "oracle_text": s.card.oracle_text,
                "color_identity": identity,
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&items)?)
}

#[cfg(test)]
pub(super) fn theme_words_for_test(
    deck: &Deck,
    cards_by_name: &std::collections::HashMap<String, CardRow>,
) -> String {
    deck_theme_words(deck, cards_by_name)
}

#[cfg(test)]
pub(super) fn json_for_test(suggestions: &[Suggestion]) -> anyhow::Result<String> {
    suggestions_json(suggestions)
}

/// Human table for the suggestion list: owned block first, then the
/// unowned block, each in relevance order. One line per hit: name, cost,
/// type, EDHREC rank, ownership/price, and why it matched.
pub(super) fn print_text(out: &crate::output::Output, suggestions: &[Suggestion], deck_name: &str) {
    let styles = out.styles();
    println!(
        "{} {}",
        styles.header("Suggestions"),
        styles.dim(&format!(
            "for deck {deck_name:?} (owned first, then by fit)"
        ))
    );
    let mut last_owned = true;
    for (i, s) in suggestions.iter().enumerate() {
        if i > 0 && last_owned && s.owned == 0 {
            println!("{}", styles.dim("— not owned —"));
        }
        last_owned = s.owned > 0;
        let own_note = if s.owned > 0 {
            styles.success(&format!("own {}", styles.thousands(s.owned)))
        } else {
            match s.price_usd {
                Some(p) => format!("buy {}", styles.money(p)),
                None => "unpriced".to_string(),
            }
        };
        let gc = if s.card.game_changer == Some(true) {
            " [GC]"
        } else {
            ""
        };
        let why = if s.tags.is_empty() {
            String::new()
        } else {
            format!(
                " {}",
                s.tags
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        println!(
            "{:>2}. {} {} {} {} {}{}{}",
            i + 1,
            styles.card_name(&s.card.name),
            styles.mana_pips(&s.card.mana_cost),
            styles.dim(&s.card.type_line),
            styles.dim(&format!(
                "rank {}",
                s.card
                    .edhrec_rank
                    .map(|r| styles.thousands(r))
                    .unwrap_or_else(|| "—".into())
            )),
            styles.dim(&own_note),
            styles.dim(gc),
            styles.dim(&why),
        );
    }
}

#[cfg(test)]
#[path = "tests/render_tests.rs"]
mod render_tests;
