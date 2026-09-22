// Combo and hypgeo human output, plus baseline diffing for
// `deck simulate --baseline`: the print helpers and the delta view,
// split from `report` to keep files small.

use crate::output::Output;

/// Human output for `--combo` pair access.
pub fn print_combo_access(out: &Output, rows: &[super::findings::ComboAccess]) {
    let s = out.styles();
    println!();
    println!("{}", s.header("Combo assembly (both pieces in hand)"));
    for row in rows {
        println!(
            "  {}  {}  {:.0}% of games",
            s.card_name(&row.pair),
            s.dim(&format!("by t{}", row.target_turn)),
            row.pct_games * 100.0
        );
    }
}

/// JSON payload for store-backed combo assembly, capped per direction.
pub fn combos_json(assembly: &super::combos::Assembly, limit: usize) -> serde_json::Value {
    serde_json::json!({
        "source": "commanderspellbook",
        "variants_considered": assembly.variants_considered,
        "complete": &assembly.complete[..assembly.complete.len().min(limit)],
        "near_misses": &assembly.near_misses[..assembly.near_misses.len().min(limit)],
    })
}

/// Human output for store-backed combos: complete combos with assembly
/// rates, then near-misses (one card away). Combo rows never create
/// problems: they are opportunities, not violations.
/// Spellbook features that win the game, for the win-path filter.
const WIN_FEATURES: [&str; 6] = [
    "Win the game",
    "Infinite damage",
    "Infinite turns",
    "Infinite mana",
    "Infinite card draw",
    "Infinite storm",
];

/// True when the combo produces a win feature.
fn is_win_path(produces: &[String]) -> bool {
    produces
        .iter()
        .any(|p| WIN_FEATURES.iter().any(|f| p.contains(f)))
}

/// Win-path rows: complete combos that produce a win feature.
pub fn win_paths_json(assembly: &super::combos::Assembly, limit: usize) -> serde_json::Value {
    let paths: Vec<&super::combos::ComboAccess> = assembly
        .complete
        .iter()
        .filter(|r| is_win_path(&r.produces))
        .take(limit)
        .collect();
    serde_json::json!({
        "count": paths.len(),
        "paths": paths,
    })
}

/// Human win-path block: compact, only when win paths exist.
pub fn print_win_paths(out: &Output, assembly: &super::combos::Assembly, limit: usize) {
    let s = out.styles();
    let paths: Vec<&super::combos::ComboAccess> = assembly
        .complete
        .iter()
        .filter(|r| is_win_path(&r.produces))
        .take(limit)
        .collect();
    if paths.is_empty() {
        return;
    }
    println!();
    println!("{}", s.header("Win paths"));
    for row in paths {
        let feature = row
            .produces
            .iter()
            .find(|p| is_win_path(std::slice::from_ref(p)))
            .map(String::as_str)
            .unwrap_or("");
        println!(
            "  {}  {}  {:.0}% of games",
            s.card_name(&row.combo),
            s.dim(&format!("{feature} by t{}", row.target_turn)),
            row.pct_games * 100.0
        );
    }
}

/// Human view of the store-backed combo assembly: complete combos and
/// one-card-away near misses, each capped at `limit` rows.
pub fn print_store_combos(out: &Output, assembly: &super::combos::Assembly, limit: usize) {
    let s = out.styles();
    println!();
    println!(
        "{} {}",
        s.header("Combo assembly (Spellbook)"),
        s.dim(&format!(
            "{} variants in the deck",
            assembly.variants_considered
        ))
    );
    for row in assembly.complete.iter().take(limit) {
        let tags = row
            .produces
            .first()
            .map(|p| format!(" → {p}"))
            .unwrap_or_default();
        let bracket = row
            .bracket_tag
            .as_deref()
            .map(|b| format!(" [{b}]"))
            .unwrap_or_default();
        println!(
            "  {}  {}  {:.0}% of games{}{}",
            s.card_name(&row.combo),
            s.dim(&format!("by t{}", row.target_turn)),
            row.pct_games * 100.0,
            s.dim(&tags),
            s.dim(&bracket)
        );
    }
    let misses: Vec<&super::combos::ComboAccess> =
        assembly.near_misses.iter().take(limit).collect();
    if !misses.is_empty() {
        println!("{}", s.header("One card away"));
        for row in misses {
            println!(
                "  {}  {}  {}{}",
                s.card_name(row.missing.first().map(String::as_str).unwrap_or("?")),
                s.dim(&format!("completes {}", row.combo)),
                s.dim(&format!("by t{}", row.target_turn)),
                row.bracket_tag
                    .as_deref()
                    .map(|b| s.dim(&format!(" [{b}]")))
                    .unwrap_or_default()
            );
        }
    }
}

// Exact-probability ceilings (`--hypgeo`): print the top gaps between the
// Monte Carlo castability and the hypergeometric ceiling, so a mana-base
// problem separates from a draw problem.
pub fn print_hypgeo(out: &Output, payload: &serde_json::Value) {
    let s = out.styles();
    let Some(cards) = payload.get("cards").and_then(|c| c.as_array()) else {
        return;
    };
    println!();
    println!(
        "{}",
        s.header("Cast-on-curve ceilings (exact hypergeometric)")
    );
    for row in cards.iter().take(5) {
        let name = row
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let target = row.get("target_turn").and_then(|v| v.as_u64()).unwrap_or(0);
        let ceiling = row
            .get("pct_castable_ceiling")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        println!(
            "    {}  {}  ceiling {:.0}%",
            s.card_name(&name),
            s.dim(&format!("by t{target}")),
            ceiling
        );
    }
    println!(
        "{}",
        s.note("ceiling = exact upper bound on the real cast rate; sim castability is draw-agnostic and sits above it")
    );
}

/// One metric delta: dot path, old value, new value.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Delta {
    /// Dot path into the report (e.g. "commander.on_curve_pct").
    pub path: String,
    /// Prior value.
    pub old: serde_json::Value,
    /// New value.
    pub new: serde_json::Value,
}

/// Problem kinds that appeared or disappeared between the two runs.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProblemDelta {
    /// "resolved" (in baseline, gone now) or "new" (absent before).
    pub change: &'static str,
    /// The problem's kind + detail.
    pub kind: String,
    pub detail: String,
}

/// The full diff between two reports.
#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct ReportDiff {
    pub metrics: Vec<Delta>,
    pub problems: Vec<ProblemDelta>,
    /// Deck-shape count changes (name, old, new).
    pub shape: Vec<ShapeDelta>,
}

/// One deck-shape count change between the baseline and current reports.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ShapeDelta {
    /// Deck-shape key (e.g. "lands", "draw_sources").
    pub name: String,
    /// Prior value.
    pub old: String,
    /// New value.
    pub new: String,
}

/// Diff two report payloads: scalar metrics at known paths, problem lists,
/// and deck-shape counts. Identical inputs yield an empty diff.
pub fn diff_reports(baseline: &serde_json::Value, current: &serde_json::Value) -> ReportDiff {
    let mut diff = ReportDiff::default();

    // Scalar metrics: (dot path) pairs worth tracking.
    for path in METRIC_PATHS {
        if let (Some(old), Some(new)) = (lookup(baseline, path), lookup(current, path))
            && old != new
        {
            diff.metrics.push(Delta {
                path: (*path).to_string(),
                old: old.clone(),
                new: new.clone(),
            });
        }
    }

    // Deck shape counts.
    let old_shape = baseline.get("deck_shape");
    let new_shape = current.get("deck_shape");
    if let (Some(old), Some(new)) = (old_shape, new_shape)
        && (old.is_object() && new.is_object())
    {
        for key in [
            "total_cards",
            "lands",
            "rocks",
            "dorks",
            "ramp_spells",
            "draw_sources",
            "removal",
            "wincons",
        ] {
            let old_v = old.get(key).cloned().unwrap_or_default();
            let new_v = new.get(key).cloned().unwrap_or_default();
            if old_v != new_v {
                diff.shape.push(ShapeDelta {
                    name: key.to_string(),
                    old: value_display(&old_v),
                    new: value_display(&new_v),
                });
            }
        }
    }

    // Problems: match by kind+color; report kind+detail changes and
    // appear/disappear as problems entries.
    let old_problems = baseline
        .get("problems")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let new_problems = current
        .get("problems")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let key = |p: &serde_json::Value| {
        format!(
            "{}|{}",
            p.get("kind").and_then(|k| k.as_str()).unwrap_or(""),
            p.get("color").and_then(|c| c.as_str()).unwrap_or("")
        )
    };
    for p in &new_problems {
        if !old_problems.iter().any(|o| key(o) == key(p)) {
            diff.problems.push(ProblemDelta {
                change: "new",
                kind: string_field(p, "kind"),
                detail: string_field(p, "detail"),
            });
        }
    }
    for p in &old_problems {
        if !new_problems.iter().any(|n| key(n) == key(p)) {
            diff.problems.push(ProblemDelta {
                change: "resolved",
                kind: string_field(p, "kind"),
                detail: string_field(p, "detail"),
            });
        }
    }
    // Detail changes for still-present kinds (severity shifts).
    for p in &new_problems {
        if let Some(old) = old_problems.iter().find(|o| key(o) == key(p)) {
            let old_detail = string_field(old, "detail");
            let new_detail = string_field(p, "detail");
            if old_detail != new_detail {
                diff.metrics.push(Delta {
                    path: format!("problems[{}].detail", string_field(p, "kind")),
                    old: serde_json::json!(old_detail),
                    new: serde_json::json!(new_detail),
                });
            }
        }
    }
    diff
}

/// Tracked scalar metric paths (dot-separated).
const METRIC_PATHS: &[&str] = &[
    "commander.on_curve_pct",
    "commander.avg_first_cast_turn",
    "land_drops.screw_pct_2_or_fewer_by_t4",
    "land_drops.flood_pct_6plus_lands_seen_in_11",
    "land_drops.flood_expectation",
    "mana.pct_games_floated_3plus_t6",
    "draw.pct_starved_0_by_t6",
    "role_access.removal_pct_seen_by_5",
    "role_access.draw_pct_seen_by_6",
    "role_access.creature_pct_seen_by_3",
    "role_access.wincon_pct_seen_by_8",
    "color_screw.W",
    "color_screw.U",
    "color_screw.B",
    "color_screw.R",
    "color_screw.G",
];

/// Follow a dot path through JSON objects.
fn lookup(value: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current.clone())
}

/// String field of a problem object.
fn string_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Display string for a JSON value in diff output.
fn value_display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Render the diff for humans: shape changes, metric deltas, problems.
pub fn print_diff(out: &Output, diff: &ReportDiff) {
    let s = out.styles();
    if diff.is_empty() {
        println!("{}", s.success("no changes vs baseline"));
        return;
    }
    println!("{}", s.header("Deltas vs baseline"));
    if !diff.shape.is_empty() {
        println!("{}", s.header("Shape"));
        for d in &diff.shape {
            println!("    {}: {} → {}", d.name, d.old, d.new);
        }
    }
    if !diff.metrics.is_empty() {
        println!("{}", s.header("Metrics"));
        for d in &diff.metrics {
            println!("    {}: {} → {}", d.path, d.old, d.new);
        }
    }
    if !diff.problems.is_empty() {
        println!("{}", s.header("Problems"));
        for p in &diff.problems {
            match p.change {
                // Bare signs: "+" = new problem (red), "-" = resolved
                // (green). error()/success() would print "error: +".
                "new" => println!(
                    "    {} {}: {}",
                    s.glyph("+", crate::output::GlyphKind::Bad),
                    p.kind,
                    p.detail
                ),
                _ => println!(
                    "    {} {}: {}",
                    s.glyph("-", crate::output::GlyphKind::Good),
                    p.kind,
                    p.detail
                ),
            }
        }
    }
}

impl ReportDiff {
    /// True when nothing changed between the two reports.
    pub fn is_empty(&self) -> bool {
        self.shape.is_empty() && self.metrics.is_empty() && self.problems.is_empty()
    }
}
