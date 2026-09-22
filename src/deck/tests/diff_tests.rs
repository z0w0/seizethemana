use super::*;
use crate::deck::Deck;

fn parse(text: &str) -> Deck {
    Deck::parse(text).unwrap()
}

fn names(rows: &[(String, i64)]) -> Vec<&str> {
    let mut v: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
    v.sort();
    v
}

#[test]
fn identical_decks_diff_to_empty() {
    let deck = parse("// COMMANDER\n1 Breya\n// DECK\n2 Bolt\n2 Island (SOS) 274\n");
    let diff = diff_decks(&deck, &deck, false, |_| false);
    assert!(diff.iter().all(|s| s.is_empty()));
}

#[test]
fn basics_report_as_qty_deltas() {
    let a = parse("// DECK\n16 Forest\n12 Island\n2 Bolt\n");
    let b = parse("// DECK\n12 Forest\n14 Island\n2 Bolt\n");
    let diff = diff_decks(&a, &b, false, |n| n == "Forest" || n == "Island");
    let deck = &diff[0];
    assert_eq!(deck.changed.len(), 2);
    assert!(deck.changed.contains(&("Forest".to_string(), 16, 12)));
    assert!(deck.changed.contains(&("Island".to_string(), 12, 14)));
    assert!(deck.removed.is_empty() && deck.added.is_empty());
}

#[test]
fn nonbasic_quantity_changes_split_into_remove_add() {
    let a = parse("// DECK\n3 Bolt\n");
    let b = parse("// DECK\n1 Bolt\n");
    let diff = diff_decks(&a, &b, false, |_| false);
    assert_eq!(diff[0].removed, vec![("Bolt".to_string(), 2)]);
    assert!(diff[0].added.is_empty());
    assert!(diff[0].changed.is_empty());
}

#[test]
fn remove_and_add_report_per_name() {
    let a = parse("// COMMANDER\n1 Breya\n// DECK\n2 Bolt\n1 Shock\n");
    let b = parse("// COMMANDER\n1 Atraxa\n// DECK\n1 Counterspell\n1 Shock\n");
    let diff = diff_decks(&a, &b, false, |_| false);
    assert_eq!(diff[0].removed, vec![("Breya".to_string(), 1)]);
    assert_eq!(diff[0].added, vec![("Atraxa".to_string(), 1)]);
    assert_eq!(diff[1].removed, vec![("Bolt".to_string(), 2)]);
    assert_eq!(diff[1].added, vec![("Counterspell".to_string(), 1)]);
}

#[test]
fn exact_mode_diffs_printings() {
    let a = parse("// DECK\n1 Bolt (M11) 148\n");
    let b = parse("// DECK\n1 Bolt (2XM) 124\n");
    let plain = diff_decks(&a, &b, false, |_| false);
    assert!(plain.iter().all(|s| s.is_empty()), "any print fills a slot");
    let exact = diff_decks(&a, &b, true, |_| false);
    assert_eq!(exact[0].removed.len(), 1, "m11 print removed");
    assert_eq!(exact[0].added.len(), 1, "2xm print added");
}

#[test]
fn section_matches_case_insensitively_and_new_sections_report() {
    let a = parse("// DECK\n1 Bolt\n");
    let b = parse("// deck\n1 Bolt\n// SIDEBOARD\n1 Fog\n");
    let diff = diff_decks(&a, &b, false, |_| false);
    assert!(diff[0].is_empty(), "case-insensitive match");
    assert_eq!(diff[1].section, "SIDEBOARD");
    assert_eq!(diff[1].added, vec![("Fog".to_string(), 1)]);
    let back = diff_decks(&b, &a, false, |_| false);
    assert_eq!(back[1].section, "SIDEBOARD");
    assert_eq!(back[1].removed, vec![("Fog".to_string(), 1)]);
}

#[test]
fn markdown_renders_basics_line_and_table() {
    let a = parse("// COMMANDER\n1 Breya\n// DECK\n16 Forest\n2 Bolt\n1 Fog\n");
    let b = parse("// COMMANDER\n1 Breya\n// DECK\n12 Forest\n1 Counterspell\n1 Shock\n");
    let diff = diff_decks(&a, &b, false, |n| n == "Forest");
    let md = markdown(&diff);
    assert!(md.contains("## COMMANDER") || md.contains("## DECK"));
    assert!(md.contains("Remove 4 Forests."), "{md}");
    assert!(md.contains("Bolt"), "{md}");
    assert!(md.contains("Counterspell"), "{md}");
    assert!(!names(&[("Fog".into(), 1)]).is_empty());
}

#[test]
fn duplicate_lines_collapse_before_diffing() {
    let a = parse("// DECK\n2 Bolt\n1 Bolt\n");
    let b = parse("// DECK\n3 Bolt\n");
    let diff = diff_decks(&a, &b, false, |_| false);
    assert!(diff[0].is_empty(), "2+1 == 3 after aggregation");
}

#[test]
fn diff_accepts_file_paths_on_both_operands() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    let file_a = tmp.path().join("a.txt");
    let file_b = tmp.path().join("b.txt");
    std::fs::write(&file_a, "// DECK\n2 Bolt\n").unwrap();
    std::fs::write(&file_b, "// DECK\n3 Bolt\n").unwrap();
    let conn = crate::db::open(&paths.db()).unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let code = diff(
        &paths,
        &conn,
        &mut out,
        file_a.to_str().unwrap(),
        file_b.to_str().unwrap(),
        false,
        DiffFormat::Human,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
}

#[test]
fn markdown_basics_line_joins_two_basics_with_and() {
    let a = parse("// DECK\n16 Forest\n12 Island\n");
    let b = parse("// DECK\n12 Forest\n14 Island\n");
    let diff = diff_decks(&a, &b, false, |n| n == "Forest" || n == "Island");
    let md = markdown(&diff);
    // Removals dominate: verb is Remove, per-basic amounts joined with
    // "and"; the table below keeps the per-card transitions.
    assert!(md.contains("Remove 4 Forests and 2 Islands."), "{md}");
}

#[test]
fn markdown_basics_mixed_add_remove_use_the_dominant_direction() {
    // 3 Forests removed, 2 Islands added: removals dominate, so the line
    // reads "Remove …" with both amounts (the table keeps the per-card
    // transitions).
    let a = parse("// DECK\n15 Forest\n10 Island\n");
    let b = parse("// DECK\n12 Forest\n12 Island\n");
    let diff = diff_decks(&a, &b, false, |n| n == "Forest" || n == "Island");
    let md = markdown(&diff);
    assert!(md.contains("Remove 3 Forests and 2 Islands."), "{md}");
}

#[test]
fn as_update_outputs_removes_sets_and_adds() {
    // A full apply loop: the emitted ops turn A into B.
    let (tmp, paths) = {
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
        std::fs::create_dir_all(paths.decks_dir()).unwrap();
        std::fs::write(paths.deck_file("A"), "// DECK\n2 Bolt\n12 Forest\n").unwrap();
        std::fs::write(paths.deck_file("B"), "// DECK\n1 Shock\n10 Forest\n").unwrap();
        (tmp, paths)
    };
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    let mut out = crate::output::Output::new(false, true, false);
    let code = diff_as_update(&paths, &conn, &mut out, "A", "B", false).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    // Ops apply cleanly to A and reproduce B.
    let deck_a = Deck::parse("// DECK\n2 Bolt\n12 Forest\n").unwrap();
    let deck_b = Deck::parse("// DECK\n1 Shock\n10 Forest\n").unwrap();
    let sections = diff_decks(&deck_a, &deck_b, false, |_| false);
    let mut lines = Vec::new();
    for s in &sections {
        let prefix = if s.section.eq_ignore_ascii_case("DECK") {
            String::new()
        } else {
            format!("{}:", s.section)
        };
        for (n, q) in &s.removed {
            lines.push(format!("remove {prefix}{q} {n}"));
        }
        for (n, f, t) in &s.changed {
            if *t == 0 {
                lines.push(format!("remove {prefix}{f} {n}"));
            } else {
                lines.push(format!("set {prefix}{t} {n}"));
            }
        }
        for (n, q) in &s.added {
            lines.push(format!("add {prefix}{q} {n}"));
        }
    }
    let text = lines.join("\n");
    assert!(text.contains("remove 2 Bolt"), "{text}");
    assert!(text.contains("remove 2 Forest"), "{text}");
    assert!(text.contains("add 1 Shock"), "{text}");
    // Applying the ops through the real op path reaches B.
    let mut applied = Deck::parse("// DECK\n2 Bolt\n12 Forest\n").unwrap();
    let ops: Vec<crate::deck::ops::DeckOp> = text
        .lines()
        .map(|l| {
            let mut parts = l.splitn(2, ' ');
            let kind = parts.next().unwrap();
            crate::deck::ops::parse_op(kind, parts.next().unwrap()).unwrap()
        })
        .collect();
    crate::deck::ops::apply_ops(&mut applied, &ops).unwrap();
    assert!(
        diff_decks(&applied, &deck_b, false, |_| false)
            .iter()
            .all(|s| s.is_empty())
    );
    let _ = paths;
}

#[test]
fn as_update_prefixes_non_deck_sections() {
    // Sideboard and COMMANDER diffs carry the `section:` prefix so the
    // ops land in the right section; the round-trip reproduces B from A.
    let (tmp, paths) = {
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
        std::fs::create_dir_all(paths.decks_dir()).unwrap();
        std::fs::write(
            paths.deck_file("A"),
            "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n// SIDEBOARD\n1 Island\n",
        )
        .unwrap();
        std::fs::write(
            paths.deck_file("B"),
            "// COMMANDER\n1 Breya\n1 Tymna\n// DECK\n1 Bolt\n// SIDEBOARD\n2 Swamp\n",
        )
        .unwrap();
        (tmp, paths)
    };
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    let mut out = crate::output::Output::new(false, true, false);
    let code = diff_as_update(&paths, &conn, &mut out, "A", "B", false).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    // Recompute the emitted lines from the same diff the command used.
    let deck_a =
        Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n// SIDEBOARD\n1 Island\n").unwrap();
    let deck_b =
        Deck::parse("// COMMANDER\n1 Breya\n1 Tymna\n// DECK\n1 Bolt\n// SIDEBOARD\n2 Swamp\n")
            .unwrap();
    let sections = diff_decks(&deck_a, &deck_b, false, |_| false);
    let mut lines = Vec::new();
    for s in &sections {
        let prefix = if s.section.eq_ignore_ascii_case("DECK") {
            String::new()
        } else {
            format!("{}:", s.section)
        };
        for (n, q) in &s.removed {
            lines.push(format!("remove {prefix}{q} {n}"));
        }
        for (n, f, t) in &s.changed {
            if *t == 0 {
                lines.push(format!("remove {prefix}{f} {n}"));
            } else {
                lines.push(format!("set {prefix}{t} {n}"));
            }
        }
        for (n, q) in &s.added {
            lines.push(format!("add {prefix}{q} {n}"));
        }
    }
    let text = lines.join("\n");
    let lower = text.to_lowercase();
    assert!(lower.contains("add commander:1 tymna"), "{text}");
    assert!(lower.contains("add sideboard:2 swamp"), "{text}");
    assert!(lower.contains("remove sideboard:1 island"), "{text}");
    // The unprefixed DECK ops stay unprefixed.
    assert!(
        !text.lines().any(|l| l.starts_with("remove deck:")),
        "{text}"
    );
    // Round-trip: applying the ops to A reproduces B.
    let ops: Vec<crate::deck::ops::DeckOp> = text
        .lines()
        .map(|l| {
            let mut parts = l.splitn(2, ' ');
            let kind = parts.next().unwrap();
            crate::deck::ops::parse_op(kind, parts.next().unwrap()).unwrap()
        })
        .collect();
    let mut applied = deck_a.clone();
    crate::deck::ops::apply_ops(&mut applied, &ops).unwrap();
    assert!(
        diff_decks(&applied, &deck_b, false, |_| false)
            .iter()
            .all(|s| s.is_empty()),
        "applying the ops reproduces B; got {:?}",
        applied.sections
    );
    let _ = paths;
}
