use anyhow::Context;

use crate::cli;
use crate::cli::codes;
use crate::db::{self, CardRow};
use crate::embed::{self, VectorStore};
use crate::output::Output;
use crate::paths::Paths;
use crate::search::CardFilters;

/// One ranked search hit: the card plus its similarity score.
pub struct Hit {
    pub card: CardRow,
    /// Hybrid score in `[0, 1]`: normalized reciprocal-rank fusion of the
    /// full-text and vector result lists (see `fuse_rrf`).
    pub score: f32,
}

/// Query-expansion table: player shorthand -> community tag vocabulary.
///
/// Each row is a trigger phrase and extra search terms appended to both
/// retrieval legs — the FTS expression gains them as additional OR terms,
/// the embedded query gains them as trailing words. Appending (never
/// replacing) keeps the original intent terms in play.
///
/// Triggers match anywhere in the query with word boundaries, so "cheap
/// counterspell" expands via `counterspell`. Expansions are grounded in
/// Scryfall Tagger's vocabulary (`ramp`, `sweeper`, `reanimate`,
/// `sacrifice outlet`, `wheel`, `cantrip`, `anthem`, ...) plus the oracle
/// phrasing of the mechanic, so role words match the `tags_text` full-text
/// column and the embedding even when oracle text never uses the shorthand.
const EXPANSIONS: &[(&str, &str)] = &[
    // Mana development
    (
        "ramp",
        "ramp mana acceleration add one mana of any color search your library for a land",
    ),
    (
        "mana ramp",
        "ramp mana acceleration land ramp add one mana of any color",
    ),
    (
        "land ramp",
        "land ramp multi land ramp tutor-land search your library for a land",
    ),
    (
        "mana acceleration",
        "ramp mana acceleration adds multiple mana",
    ),
    (
        "mana rock",
        "mana rock utility mana rock mana artifact adds multiple mana",
    ),
    (
        "mana dork",
        "mana dork ramp adds multiple mana tap for mana",
    ),
    (
        "mana fixing",
        "mana fixing color fixer dual land utility land",
    ),
    ("color fixing", "mana fixing dual land utility land"),
    ("mana sink", "mana sink bottomless mana sink"),
    (
        "mana value reduction",
        "cheaper than mv discount-self alternative cost free-cast",
    ),
    (
        "cheaper to cast",
        "cheaper than mv discount-self alternative cost",
    ),
    (
        "free cast",
        "free-cast-another alternative cost cast on resolution",
    ),
    // Card advantage
    (
        "card draw",
        "draw engine pure draw burst draw card advantage",
    ),
    (
        "draw cards",
        "draw engine pure draw burst draw card advantage",
    ),
    (
        "draw engine",
        "draw engine repeatable pure draw repeatable card advantage",
    ),
    ("cantrip", "cantrip burst draw pure draw"),
    (
        "card advantage",
        "draw engine repeatable card advantage pure draw",
    ),
    ("wheel", "wheel draw discard"),
    ("wheel effect", "wheel draw discard"),
    ("loot", "loot impulse discard draw"),
    ("rummage", "loot impulse discard draw"),
    (
        "impulse draw",
        "impulse repeatable impulsive draw exiled cards",
    ),
    ("scry", "scry surveil card selection"),
    ("surveil", "surveil scry card selection mill-self"),
    ("card selection", "scry surveil card selection"),
    (
        "mill",
        "mill-self mill-opponent mill-any cards in graveyard matter",
    ),
    (
        "self mill",
        "mill-self graveyard fuel cards in graveyard matter",
    ),
    ("discard outlet", "discard outlet loot rummage discard"),
    ("looting", "loot impulse discard draw"),
    // Interaction
    (
        "removal",
        "spot removal removal-creature removal-destroy removal-exile destroy exile",
    ),
    (
        "spot removal",
        "spot removal removal-creature removal-destroy destroy target",
    ),
    (
        "kill spell",
        "spot removal removal-creature removal-destroy",
    ),
    (
        "board wipe",
        "sweeper sweeper-one-sided removal destroy all creatures each creature gets",
    ),
    (
        "wrath effect",
        "sweeper removal destroy all creatures each creature gets",
    ),
    ("wrath", "sweeper removal destroy all creatures"),
    ("board clear", "sweeper removal destroy all creatures"),
    ("mass removal", "sweeper removal destroy all"),
    ("wipe the board", "sweeper removal destroy all creatures"),
    (
        "counterspell",
        "counterspell counter target spell counter target",
    ),
    (
        "counter",
        "counterspell counter target spell counter target",
    ),
    ("counters", "counterspell counter target spell"),
    ("countermagic", "counterspell counter target spell"),
    (
        "bounce",
        "removal-bounce bounce-self return to owner's hand",
    ),
    ("tap down", "tapper-creature freeze-creature tap target"),
    ("tax", "toll group slug rhystic opponent pays"),
    ("stax", "group slug tax toll prevent attack hate"),
    (
        "pillow fort",
        "protects-all damage prevention group slug tax",
    ),
    (
        "protection",
        "damage prevention protects-creature protects-all hexproof ward indestructible",
    ),
    ("hexproof", "gives hexproof hexproof ward"),
    ("ward", "ward gives hexproof"),
    (
        "indestructible",
        "gives indestructible indestructible damage prevention",
    ),
    (
        "hate piece",
        "hate-graveyard hate-artifact hate-attacker hate-blocker",
    ),
    ("graveyard hate", "hate-graveyard exile graveyard"),
    ("anti graveyard", "hate-graveyard exile graveyard"),
    // Sacrifice and graveyard
    (
        "sacrifice outlet",
        "sacrifice outlet repeatable sacrifice outlet sacrifice",
    ),
    (
        "sac outlet",
        "sacrifice outlet repeatable sacrifice outlet sacrifice",
    ),
    (
        "sacrifice payoff",
        "death trigger martyr sacrifice outlet cards in graveyard matter",
    ),
    (
        "aristocrats",
        "death trigger sacrifice outlet martyr drain life opponent loses life",
    ),
    (
        "reanimate",
        "reanimate reanimate-creature return from graveyard to the battlefield",
    ),
    (
        "reanimator",
        "reanimate reanimate-creature graveyard return from graveyard to the battlefield",
    ),
    (
        "graveyard recursion",
        "reanimate recursion castable from graveyard cards in graveyard matter return from graveyard",
    ),
    (
        "recursion",
        "reanimate castable from graveyard cards in graveyard matter return from graveyard",
    ),
    (
        "graveyard fill",
        "graveyard fuel mill-self cards in graveyard matter",
    ),
    (
        "graveyard matters",
        "cards in graveyard matter graveyard fuel castable from graveyard",
    ),
    (
        "return from graveyard",
        "reanimate castable from graveyard return from graveyard",
    ),
    // Board presence
    (
        "tokens",
        "repeatable creature tokens creature token creates tokens",
    ),
    (
        "token generators",
        "repeatable creature tokens creature token creates tokens",
    ),
    (
        "token",
        "repeatable creature tokens creature token creates tokens",
    ),
    (
        "go wide",
        "repeatable creature tokens multiple bodies anthem",
    ),
    (
        "anthem",
        "anthem keyword anthem power boost to all toughness boost to all",
    ),
    (
        "pump team",
        "anthem power boost to all toughness boost to all",
    ),
    (
        "buff all",
        "anthem power boost to all toughness boost to all",
    ),
    // Combat
    ("evasion", "evasion gives flying unblockable menace"),
    ("flying", "gives flying evasion flying"),
    ("unblockable", "unblockable gives unblockable evasion"),
    ("menace", "gives menace evasion menace"),
    ("trample", "gives trample evasion trample"),
    (
        "deathtouch",
        "gives deathtouch deathtouch removal-toughness",
    ),
    ("combat trick", "combat trick enlarge giant growth burst"),
    ("pump", "combat trick enlarge giant growth"),
    (
        "voltron",
        "equipment aura enlarge scales with power power matters",
    ),
    ("equipment", "equipment synergy-equipment equip"),
    ("equips", "equipment synergy-equipment equip"),
    ("aura", "aura enchantment bestow"),
    ("enchantments", "enchantment aura disenchant/naturalize"),
    ("burn", "burn burn creature burn player direct damage"),
    (
        "direct damage",
        "burn burn player burn any opponent loses life",
    ),
    ("pinger", "pinger burn any deal 1 damage"),
    ("damage", "burn damage opponent loses life"),
    ("lifegain", "lifegain repeatable lifegain gain life"),
    ("life gain", "lifegain repeatable lifegain gain life"),
    ("gain life", "lifegain repeatable lifegain gain life"),
    ("drain", "drain life opponent loses life lifegain"),
    ("lifedrain", "drain life opponent loses life lifegain"),
    // Tutors
    ("tutor", "tutor tutor-to-hand search your library"),
    ("search library", "tutor tutor-to-hand search your library"),
    ("toolbox", "tutor modal search your library"),
    // Themes
    ("landfall", "landfall land ramp cards in graveyard matter"),
    ("lands matter", "landfall utility land land ramp"),
    (
        "artifacts matter",
        "synergy-artifact synergy-artifact-creature artifact",
    ),
    (
        "artifact deck",
        "synergy-artifact synergy-artifact-creature artifact",
    ),
    ("typal", "noncreature typal creature count matters synergy"),
    ("tribal", "noncreature typal creature count matters synergy"),
    (
        "kindred",
        "noncreature typal creature count matters synergy",
    ),
    (
        "storm",
        "copy-spell copy-instant castable from exile cheap spells",
    ),
    (
        "spellslinger",
        "copy-spell copy-instant cast trigger-you impulse",
    ),
    ("copy", "copy-spell copy-instant copy-sorcery copy-creature"),
    ("spell copy", "copy-spell copy-instant copy-sorcery"),
    ("extra turn", "extra turn take an extra turn"),
    ("time walk", "extra turn take an extra turn"),
    ("combo piece", "requires another card combo win the game"),
    ("two card combo", "combo win the game"),
    ("combo", "combo win the game"),
    (
        "win the game",
        "win the game you win the game alternate win",
    ),
    (
        "alt wincon",
        "win the game you win the game alternate win mill-opponent",
    ),
    (
        "alternate win",
        "win the game you win the game mill-opponent",
    ),
    (
        "mill win",
        "mill-opponent win the game cards in graveyard matter",
    ),
    ("theft", "theft-creature theft-cast gain control"),
    ("steal", "theft-creature theft-cast gain control"),
    ("aikido", "aikido combat trick redirect damage"),
    (
        "group hug",
        "group hug selective group hug symmetrical draw",
    ),
    ("politics", "group hug selective group hug per-player"),
    (
        "blink",
        "bounce-self enters-the-battlefield multiple bodies rescue-creature",
    ),
    (
        "flicker",
        "bounce-self enters-the-battlefield multiple bodies rescue-creature",
    ),
    (
        "etb",
        "enters-the-battlefield cast trigger death trigger multiple bodies",
    ),
    (
        "enters the battlefield",
        "enters-the-battlefield cast trigger multiple bodies",
    ),
    (
        "death trigger",
        "death trigger death trigger-self sacrifice",
    ),
    ("attack trigger", "attack trigger attacking matters"),
    (
        "planeswalker",
        "planeswalker loyalty counters synergy-planeswalker",
    ),
    (
        "counters matter",
        "pp counters matter counters matter gains pp counters",
    ),
    (
        "plus one plus one",
        "gains pp counters pp counters matter counters matter",
    ),
    (
        "counters plus one",
        "gains pp counters pp counters matter counters matter",
    ),
    ("energy", "mm counters energy gains mm counters"),
    ("regenerate", "regenerates self damage prevention"),
    ("untap", "untapper-creature untaps self untap"),
    ("crew", "crew vehicles"),
    ("vehicle", "crew vehicles"),
    (
        "fight",
        "one-sided fight removal-fight deals damage equal to",
    ),
    (
        "freeze",
        "freeze-creature tap target creature doesn't untap",
    ),
    ("discard", "discard outlet loot rummage"),
    (
        "big creature",
        "enlarge scales with power creates bigger body high power",
    ),
    (
        "finisher",
        "enlarge scales with power win the game creates bigger body",
    ),
    (
        "cheap spells",
        "cheaper than mv low mana value matters cantrip",
    ),
    ("ritual", "adds multiple mana ramp impulse"),
    ("exile removal", "removal-exile exile-self exile target"),
    ("exile target", "removal-exile exile-self exile target"),
    (
        "exile graveyard",
        "hate-graveyard exile-self exile all graveyards",
    ),
    (
        "regrowth",
        "regrowth-creature regrowth-self return from graveyard to your hand",
    ),
    (
        "rebuy",
        "regrowth-creature regrowth-self return from graveyard to your hand",
    ),
    (
        "flicker enabler",
        "bounce-self rescue-creature enters-the-battlefield",
    ),
    ("tapper", "tapper-creature tap target"),
    ("untapper", "untapper-creature untaps self untap"),
    ("crime", "repeatable crime targeting opponent"),
];

/// Extra search terms for the query text: every trigger phrase found
/// (word-boundary match) contributes its expansion. Public so the quality
/// harness measures the same query the CLI runs.
pub fn expanded_text(text: &str) -> String {
    let normalized: String = text
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    let padded = format!(" {normalized} ");
    let mut extras: Vec<&str> = Vec::new();
    // Cap expansion bulk so one broad trigger cannot flood the full-text
    // leg: expansions add ~24 words at most, keeping the query's own words
    // dominant in the BM25 ranking.
    const MAX_EXPANSION_WORDS: usize = 24;
    let mut used = 0usize;
    for (trigger, expansion) in EXPANSIONS {
        let padded_trigger = format!(" {} ", trigger.replace('+', " "));
        if !padded.contains(&padded_trigger) || extras.contains(expansion) {
            continue;
        }
        extras.push(expansion);
        used += expansion.split_whitespace().count();
        if used >= MAX_EXPANSION_WORDS {
            break;
        }
    }
    if extras.is_empty() {
        return text.to_string();
    }
    format!("{text} {}", extras.join(" "))
}

/// Candidate depth each retrieval leg contributes to the fusion. Public
/// so the quality harness measures the same candidate pools the CLI fuses.
///
/// Deep enough that filters cutting candidates during fusion rarely starve
/// the final `--limit` window.
pub fn fusion_depth(limit: usize) -> usize {
    limit.max(20)
}

/// Reciprocal-rank fusion constant. The original paper's value (60) works
/// without tuning; larger values soften the advantage of top ranks.
const RRF_K: f64 = 60.0;

/// Fuse full-text and vector result lists with reciprocal rank fusion.
///
/// Each list contributes `1 / (k + rank)` per document; a card ranked well by
/// both legs beats a card ranked first by only one. Equal fusion scores break
/// toward lower EDHREC rank (more popular first), then by name for
/// determinism.
///
/// Inputs are `(card name, score)` pairs, best first (scores are ignored;
/// only ranks matter). Returns `(name, score)` with score normalized to
/// `[0, 1]`. `k` is the RRF constant; [`RRF_K`] is the production value,
/// the parameter exists so benchmarks can sweep it.
pub fn fuse_rrf(
    fts_hits: &[(String, f64)],
    vector_hits: &[(String, f64)],
    limit: usize,
    edhrec_rank: impl Fn(&str) -> Option<i64>,
) -> Vec<(String, f32)> {
    fuse_rrf_k(fts_hits, vector_hits, limit, RRF_K, edhrec_rank)
}

/// [`fuse_rrf`] with an explicit fusion constant (bench sweeps).
pub fn fuse_rrf_k(
    fts_hits: &[(String, f64)],
    vector_hits: &[(String, f64)],
    limit: usize,
    k: f64,
    edhrec_rank: impl Fn(&str) -> Option<i64>,
) -> Vec<(String, f32)> {
    let mut scores: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    for (rank, (name, _)) in fts_hits.iter().enumerate() {
        *scores.entry(name.as_str()).or_insert(0.0) += 1.0 / (k + rank as f64 + 1.0);
    }
    for (rank, (name, _)) in vector_hits.iter().enumerate() {
        *scores.entry(name.as_str()).or_insert(0.0) += 1.0 / (k + rank as f64 + 1.0);
    }
    // Normalize to [0, 1]: a card ranked first on both legs scores
    // 2 / (k + 1); everything else sits below that ceiling.
    let max_score = 2.0 / (k + 1.0);
    let mut ranked: Vec<(String, f32)> = scores
        .into_iter()
        .map(|(name, score)| (name.to_string(), (score / max_score) as f32))
        .collect();
    ranked.sort_unstable_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| {
                edhrec_rank(&a.0)
                    .unwrap_or(i64::MAX)
                    .cmp(&edhrec_rank(&b.0).unwrap_or(i64::MAX))
            })
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(limit);
    ranked
}

/// Run a hybrid search: full-text (BM25) + vector legs fused by RRF.
///
/// Shared by `stm query` and `stm collection query` (the latter restricts to
/// owned names via `restrict`).
///
/// # Errors
/// Fails when the vector store or model is missing (not set up) or embedding
/// fails.
pub fn run_search(
    paths: &Paths,
    conn: &rusqlite::Connection,
    _out: &Output,
    text: &str,
    filters: &CardFilters,
    limit: usize,
    restrict: Option<&std::collections::HashSet<String>>,
) -> anyhow::Result<Vec<Hit>> {
    if !paths.is_setup() {
        anyhow::bail!("card index not built yet");
    }
    let store =
        VectorStore::load(paths.root()).context("loading vector index (run 'stm setup' first)")?;
    let cards = db::load_all_cards(conn)?;
    let cards_by_name: std::collections::HashMap<&str, &CardRow> =
        cards.iter().map(|c| (c.name.as_str(), c)).collect();
    let names_by_id: std::collections::HashMap<i64, usize> = cards
        .iter()
        .enumerate()
        .map(|(i, _)| (i as i64 + 1, i))
        .collect();
    let depth = fusion_depth(limit);
    // One filter pass over the store up front: leg closures read this mask
    // instead of re-running the JSON-backed filter checks per card per leg.
    let allowed: Vec<bool> = cards
        .iter()
        .map(|card| {
            restrict.is_none_or(|names| names.contains(&card.name)) && filters.matches(card)
        })
        .collect();

    // Leg 1: vector scan into a scratch buffer, filtered, top-`depth` by
    // partial sort (a full scan is cheap; a full sort is not). The embedded
    // text carries the expansion alias so shorthand phrasing matches the
    // vocabulary the documents actually use.
    let expanded = expanded_text(text);
    let mut model = embed::load_model(&paths.models_dir(), false)?;
    let query = store.embed_query(&mut model, &expanded)?;
    let mut scored: Vec<(usize, f32)> = Vec::with_capacity(cards.len());
    for (i, allowed_i) in allowed.iter().enumerate() {
        if *allowed_i && i < store.meta.names.len() {
            let row = store.row(i);
            let score: f32 = query.iter().zip(row).map(|(q, v)| q * v).sum();
            scored.push((i, score));
        }
    }
    if scored.len() > depth {
        scored.select_nth_unstable_by(depth - 1, |a, b| b.1.total_cmp(&a.1));
        scored.truncate(depth);
    }
    scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
    let vector_hits: Vec<(String, f64)> = scored
        .into_iter()
        .map(|(idx, score)| (cards[idx].name.clone(), score as f64))
        .collect();

    // Leg 2: full-text BM25 over name/tags/type/oracle text. Punctuation-only
    // queries skip the leg (nothing tokenizes). The SQL cut over-selects
    // (4× per round) until `depth` rows survive the filters, so a tight
    // restrict/filter cannot starve the fused window. Expansion alias terms
    // join as extra OR terms so shorthand still reaches the tag vocabulary.
    let Some(expr) = db::fts_query(&expanded) else {
        let fused = fuse_rrf(&[], &vector_hits, limit, |name| {
            cards_by_name.get(name).and_then(|c| c.edhrec_rank)
        });
        return Ok(to_hits(fused, &cards_by_name));
    };
    let mut fts_hits: Vec<(String, f64)> = Vec::new();
    let mut sql_limit = depth.saturating_mul(4).max(50);
    for _ in 0..4 {
        let round: Vec<(String, f64)> = db::fts_search(conn, &expr, sql_limit)?
            .into_iter()
            .filter_map(|(id, _score)| {
                let card_idx = *names_by_id.get(&id)?;
                allowed[card_idx].then(|| (cards[card_idx].name.clone(), 0.0))
            })
            .collect();
        if round.len() >= fts_hits.len() || round.len() >= depth {
            fts_hits = round;
        }
        if fts_hits.len() >= depth {
            break;
        }
        sql_limit = sql_limit.saturating_mul(4);
    }
    fts_hits.truncate(depth);

    // Fuse on rank, keep the requested window, then map back to cards.
    let fused = fuse_rrf(&fts_hits, &vector_hits, limit, |name| {
        cards_by_name.get(name).and_then(|c| c.edhrec_rank)
    });
    Ok(to_hits(fused, &cards_by_name))
}

/// Map a fused `(name, score)` list to `Hit`s, best first.
fn to_hits(
    fused: Vec<(String, f32)>,
    cards_by_name: &std::collections::HashMap<&str, &CardRow>,
) -> Vec<Hit> {
    fused
        .into_iter()
        .filter_map(|(name, score)| {
            let card = cards_by_name.get(name.as_str())?;
            Some(Hit {
                card: (*card).clone(),
                score,
            })
        })
        .collect()
}

/// Entry point for `stm query`.
#[allow(clippy::too_many_arguments)]
pub fn run_query(
    paths: &Paths,
    conn: &mut rusqlite::Connection,
    out: &mut Output,
    text: &str,
    cli_filters: &cli::CardFilters,
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    let filters = CardFilters::from_cli(cli_filters)?;
    let hits = match run_search(paths, conn, out, text, &filters, limit as usize, None) {
        Ok(hits) => hits,
        Err(err) => {
            // Distinguish "not set up" so the agent knows what to run.
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
            println!("[]");
        } else {
            out.error("no cards matched");
            out.hint("try broader words, or drop filters");
        }
        return Ok(codes::NO_RESULTS);
    }
    if json {
        let names: Vec<String> = hits.iter().map(|h| h.card.name.clone()).collect();
        // One batched query per finish kind instead of four per card name.
        let ranges = crate::prints::price_ranges(conn, &names).unwrap_or_default();
        let tag_index = crate::tags::TagIndex::load(conn)?;
        let items: Vec<serde_json::Value> = hits
            .iter()
            .filter_map(|h| {
                let range = ranges.get(&h.card.name)?;
                let mut v = crate::card::card_json(&h.card, &tag_index, range);
                v["score"] = serde_json::json!((f64::from(h.score) * 10_000.0).round() / 10_000.0);
                Some(v)
            })
            .collect();
        print_json(items)?;
    } else {
        print_text(out, &hits);
    }
    Ok(codes::OK)
}

/// Print results as a JSON array.
fn print_json(items: Vec<serde_json::Value>) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
}

/// Print results as a styled table.
fn print_text(out: &Output, hits: &[Hit]) {
    let styles = out.styles();
    for (i, hit) in hits.iter().enumerate() {
        let line = format!(
            "{:>2}. {} {} {} {} {}",
            i + 1,
            styles.card_name(&hit.card.name),
            styles.mana_pips(&hit.card.mana_cost),
            styles.rarity(&hit.card.rarity),
            styles.dim(&hit.card.type_line),
            styles.dim(&format!("({:.3})", hit.score)),
        );
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_appends_tag_vocabulary() {
        let expanded = expanded_text("mana ramp");
        assert!(expanded.starts_with("mana ramp"), "{expanded}");
        assert!(expanded.contains("ramp"), "{expanded}");
        assert!(expanded.contains("mana acceleration"), "{expanded}");
        // No trigger: text passes through unchanged.
        assert_eq!(expanded_text("Lightning Bolt"), "Lightning Bolt");
        // Multiple triggers stack, each expansion once.
        let expanded = expanded_text("cheap counterspell");
        assert!(expanded.contains("counter target spell"), "{expanded}");
        // A trigger inside a larger phrase still expands.
        let expanded = expanded_text("board wipe");
        assert!(expanded.contains("sweeper"), "{expanded}");
        assert!(expanded.contains("destroy all creatures"), "{expanded}");
        // Word boundaries: "ramp" must not match inside "trample"; a query
        // of "trample" does trigger its own entry.
        let expanded = expanded_text("gives trample");
        assert!(!expanded.contains("mana acceleration"), "{expanded}");
        assert!(expanded.contains("evasion"), "{expanded}");
        // Punctuation and case do not block matching.
        let expanded = expanded_text("Board-Wipe!!");
        assert!(expanded.contains("sweeper"), "{expanded}");
    }

    #[test]
    fn fuse_rrf_rewards_consensus() {
        // A card ranked well on both legs beats a card ranked first on one.
        let fts = vec![("shared".to_string(), 0.0), ("fts_only".to_string(), 0.0)];
        let vector = vec![
            ("vector_only".to_string(), 0.0),
            ("shared".to_string(), 0.0),
        ];
        let fused = fuse_rrf(&fts, &vector, 10, |_| None);
        assert_eq!(fused[0].0, "shared");
        // Scores are normalized to [0, 1].
        assert!((0.0..=1.0).contains(&fused[0].1));
    }

    #[test]
    fn fuse_rrf_normalizes_top_score() {
        // First on both legs: (1/(k+1) + 1/(k+1)) / (2/(k+1)) == 1.0.
        let fts = vec![("bolt".to_string(), 0.0)];
        let vector = vec![("bolt".to_string(), 0.0)];
        let fused = fuse_rrf(&fts, &vector, 1, |_| None);
        assert!((fused[0].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fuse_rrf_ties_break_toward_popularity_then_name() {
        // zebra (fts #1) and apple (vector #1) tie at 1/(k+1); loved
        // (vector #2) and plain (fts #2) tie at 1/(k+2).
        let fts = vec![("zebra".to_string(), 0.0), ("plain".to_string(), 0.0)];
        let vector = vec![("apple".to_string(), 0.0), ("loved".to_string(), 0.0)];
        let fused = fuse_rrf(&fts, &vector, 10, |name| match name {
            "loved" => Some(5),
            _ => None,
        });
        // Top tie: no EDHREC data either side -> alphabetical.
        assert_eq!(fused[0].0, "apple");
        assert_eq!(fused[1].0, "zebra");
        // Second tie: EDHREC-ranked card first.
        assert_eq!(fused[2].0, "loved");
        assert_eq!(fused[3].0, "plain");
    }

    #[test]
    fn fuse_rrf_respects_limit() {
        let fts: Vec<(String, f64)> = (0..30).map(|i| (format!("card{i}"), 0.0)).collect();
        assert_eq!(fuse_rrf(&fts, &[], 5, |_| None).len(), 5);
    }

    #[test]
    fn fuse_rrf_k_softens_top_ranks() {
        // A larger k compresses the score gap between rank 1 and rank 2.
        let fts: Vec<(String, f64)> = (0..5).map(|i| (format!("card{i}"), 0.0)).collect();
        let tight = fuse_rrf_k(&fts, &[], 5, 1.0, |_| None);
        let soft = fuse_rrf_k(&fts, &[], 5, 500.0, |_| None);
        let gap = |list: &[(String, f32)]| list[0].1 - list[1].1;
        assert!(gap(&tight) > gap(&soft), "larger k narrows the gap");
    }

    #[test]
    fn query_json_is_the_full_card_shape_plus_score() {
        let card = CardRow {
            name: "Bolt".into(),
            oracle_id: "oid".into(),
            mana_cost: "{R}".into(),
            cmc: 1.0,
            type_line: "Instant".into(),
            colors: r#"["R"]"#.into(),
            color_identity: r#"["R"]"#.into(),
            keywords: "[]".into(),
            power: None,
            toughness: None,
            loyalty: None,
            oracle_text: "Deal 3".into(),
            rarity: "uncommon".into(),
            edhrec_rank: None,
            legalities: "{}".into(),
            set_code: "TST".into(),
            collector_number: "1".into(),
            scryfall_id: "sid-1".into(),
            released_at: String::new(),
            game_changer: None,
        };
        let empty_tags = crate::tags::TagIndex::default_empty();
        let range = crate::prints::PrintRange {
            cheapest: Some(crate::prints::Print {
                scryfall_id: "sid-1".into(),
                name: "Bolt".into(),
                set_code: "tst".into(),
                set_name: "Test".into(),
                collector_number: "1".into(),
                lang: "en".into(),
                finishes: vec!["nonfoil".into()],
                released_at: "2020-01-01".into(),
                usd: Some(0.99),
                usd_foil: Some(4.5),
                usd_etched: None,
            }),
            priciest: None,
            cheapest_foil: None,
            priciest_foil: None,
        };
        let v = crate::card::card_json(&card, &empty_tags, &range);
        // The full contract: every CardRow field an agent joins on.
        for key in [
            "name",
            "oracle_id",
            "mana_cost",
            "cmc",
            "type_line",
            "colors",
            "color_identity",
            "keywords",
            "power",
            "toughness",
            "loyalty",
            "oracle_text",
            "rarity",
            "edhrec_rank",
            "legalities",
            "game_changer",
            "set",
            "collector_number",
            "scryfall_id",
            "released_at",
            "tags",
            "price_usd",
            "price_usd_foil",
            "max_price_usd",
            "max_price_usd_foil",
        ] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
        assert_eq!(v["oracle_id"], "oid");
        assert_eq!(v["price_usd"], 0.99);
        // An unpriced card renders null, not a missing field.
        let v = crate::card::card_json(&card, &empty_tags, &Default::default());
        assert_eq!(v["price_usd"], serde_json::Value::Null);
    }
}
