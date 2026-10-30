//! Complete draw, life-loss, and top-of-library selection grammar.

use super::super::model::LifeLossScope;
use super::super::oracle_ast::OracleEffect;
use super::effects::parse_oracle_number_word;

/// Keep controller draw and life loss together, with target-player draws aimed at self.
pub(super) fn parse_draw_life(source: &str) -> Option<Vec<OracleEffect>> {
    let body = normalized(source);
    if let Some(draw) = target_draw_count(&body) {
        return Some(vec![OracleEffect::Draw(draw)]);
    }
    let (first, second) = linked_clauses(&body)?;
    let (draw, loss, draw_first) = if let Some(draw) = controller_draw_count(first) {
        (draw, controller_life_loss(second)?, true)
    } else if let Some(draw) = target_draw_count(first) {
        (draw, target_life_loss(second)?, true)
    } else {
        (
            controller_draw_count(second)?,
            controller_life_loss(first)?,
            false,
        )
    };
    let draw = OracleEffect::Draw(draw);
    let loss = OracleEffect::LoseLife {
        amount: loss,
        scope: LifeLossScope::SelfOnly,
    };
    Some(if draw_first {
        vec![draw, loss]
    } else {
        vec![loss, draw]
    })
}

/// Parse complete look-and-return or draw-and-return rules without source-name checks.
pub(super) fn parse_top_selection(source: &str) -> Option<Vec<OracleEffect>> {
    let body = normalized(source);
    if let Some((draw, returned)) = linked_clauses(&body)
        && let Some(draw) = controller_draw_count(draw)
        && let Some(returned) = hand_return_count(returned)
    {
        return Some(vec![
            OracleEffect::Draw(draw),
            OracleEffect::PutHandOnTop(returned),
        ]);
    }
    let mut sentences = body.split(". ");
    let head = sentences.next()?;
    let (look, returned) = head.split_once(", then ")?;
    if returned != "put them back in any order" {
        return None;
    }
    let count = card_count(look.strip_prefix("look at the top ")?, " of your library")?;
    let mut tail = sentences.peekable();
    let may_shuffle = tail.peek().is_some_and(|sentence| {
        matches!(
            *sentence,
            "you may shuffle" | "you may shuffle your library"
        )
    });
    if may_shuffle {
        tail.next();
    }
    let mut effects = vec![OracleEffect::LookAtTop { count, may_shuffle }];
    if let Some(draw) = tail.next() {
        effects.push(OracleEffect::Draw(controller_draw_count(draw)?));
    }
    tail.next().is_none().then_some(effects)
}

/// Ignore case and layout while retaining every rule word and sentence boundary.
fn normalized(source: &str) -> String {
    let body = source.split_whitespace().collect::<Vec<_>>().join(" ");
    body.strip_suffix('.').unwrap_or(&body).to_ascii_lowercase()
}

/// Split one complete pair of instructions without dropping later clauses.
fn linked_clauses(body: &str) -> Option<(&str, &str)> {
    [", then ", " and then ", " and ", ". "]
        .into_iter()
        .find_map(|connector| body.split_once(connector))
}

/// Read a draw whose subject is the controller, not another player.
fn controller_draw_count(body: &str) -> Option<u32> {
    let body = body.strip_prefix("you ").unwrap_or(body);
    card_count(body.strip_prefix("draw ")?, "")
}

/// Read a target-player draw; the solitaire policy selects the controller.
fn target_draw_count(body: &str) -> Option<u32> {
    card_count(body.strip_prefix("target player draws ")?, "")
}

/// Read fixed card counts only when the entire noun phrase matches.
fn card_count(body: &str, suffix: &str) -> Option<u32> {
    let body = body.strip_suffix(suffix)?;
    let amount = body
        .strip_suffix(" cards")
        .or_else(|| body.strip_suffix(" card"))?;
    fixed_count(amount)
}

/// Require one complete count rather than accepting a number followed by a condition.
fn fixed_count(amount: &str) -> Option<u32> {
    if amount.split_whitespace().count() != 1
        || !amount
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return None;
    }
    parse_oracle_number_word(amount)
}

/// Keep controller life loss distinct from opponent life loss.
fn controller_life_loss(body: &str) -> Option<u32> {
    let body = body.strip_prefix("you ").unwrap_or(body);
    fixed_count(body.strip_prefix("lose ")?.strip_suffix(" life")?)
}

/// Keep a shared or repeated target-player subject attached to its own life loss.
fn target_life_loss(body: &str) -> Option<u32> {
    let amount = body
        .strip_prefix("loses ")
        .or_else(|| body.strip_prefix("that player loses "))
        .or_else(|| body.strip_prefix("they lose "))?;
    fixed_count(amount.strip_suffix(" life")?)
}

/// Require a hand-to-library move rather than a discard or a library reorder.
fn hand_return_count(body: &str) -> Option<u32> {
    let body = body.strip_suffix(" in any order").unwrap_or(body);
    card_count(
        body.strip_prefix("put ")?,
        " from your hand on top of your library",
    )
}

/// Parser checks cover captured sources, changed counts, and rejected subjects.
#[cfg(test)]
#[path = "tests/selection_tests.rs"]
mod selection_tests;
