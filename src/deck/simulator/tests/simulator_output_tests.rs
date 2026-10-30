//! Real-terminal coverage for simulator text, optional rows, and report diffs.

use super::*;
use crate::deck::simulator::{hypgeo, report, report_schema as schema, report_view};
use std::process::{Command, Stdio};

/// Source paths are deliberately unbroken and longer than every tested width.
const SOURCE_PATH: &str = "/saved/漢字/e\u{301}/simulator/source/reports/very-long-unbroken-directory-with-complete-identity-preserved-without-clipping-or-ellipsis/baseline.json";
/// Card identities must stay whole even when one token exceeds the terminal width.
const LONG_TOKEN: &str = "UnbrokenCommanderIdentityThatMustRemainWholeEvenInTheNarrowestTerminalWithoutEllipsisOrTruncationAndStillExceedsOneHundredTwentyDisplayColumns";
/// Both partners contain wide characters, combining marks, and wrap points.
const PARTNERS: [&str; 2] = [
    "Captain 漢字 e\u{301} with a long commander identity and several words",
    "Partner Åsa 漢字 e\u{301} with another long distinct commander identity",
];

/// Start with the production report builder, then cover optional display states.
fn layout_report() -> SimReport {
    let mut report = base_report();
    report.name = "A long 漢字 e\u{301} simulator report title with several words".to_string();
    report.commander = Some(CommanderReport {
        name: PARTNERS[0].to_string(),
        mana_value: 4.0,
        percent_castable_by_turn: Default::default(),
        avg_first_cast_turn: None,
        p50_cast_turn: None,
        p95_cast_turn: None,
        payable_games: 0,
        percent_castable_by_curve: None,
    });
    report.combat.attack_power_avg_by_turn.insert(6, 10.0);
    report.phase_observations.combat_by_turn.insert(6, 1.0);
    report.combat.commander_potential = PARTNERS
        .iter()
        .enumerate()
        .map(|(slot, name)| schema::CommanderCombatPotential {
            slot: u32::try_from(slot).expect("two commander slots fit u32"),
            name: (*name).to_string(),
            damage_by_turn: [(6, 5.0)].into(),
            cumulative_damage_by_turn: [(6, 25.0)].into(),
            percent_games_reaching_21: (slot == 0).then(|| Percent::from_percent(100.0)),
            observed_games: u32::from(slot == 0),
            games_reaching_21: u32::from(slot == 0),
            median_turn_to_21: (slot == 0).then_some(6),
            note: "Potential only, not damage applied to opponents.".to_string(),
        })
        .collect();
    report.card_castability = vec![schema::CardCastability {
        name: PARTNERS[1].to_string(),
        mana_value: 5.0,
        target_turn: 5,
        percent_castable_by_target: Some(Percent::from_percent(25.0)),
        avg_first_castable_turn: Some(5.0),
        payable_games: 1,
    }];
    let mut note = finding(
        "limited_draw_access",
        "limited_draw_access",
        "A long finding explanation with 漢字 and e\u{301} that must remain readable in a narrow terminal.",
    );
    note.suggestion = format!("Inspect the complete saved source path {SOURCE_PATH}");
    note.evidence = vec![schema::FindingEvidence {
        subject: PARTNERS[0].to_string(),
        explanation: "A long evidence label with wide 漢字 and combining e\u{301} text."
            .to_string(),
        percent_of_games: None,
    }];
    report.findings.push(note);
    report.combo_access = Some(vec![
        schema::PairAccessReport {
            pair: PARTNERS.join(" + "),
            target_turn: 6,
            percent_of_games: Some(Percent::from_percent(100.0)),
            access: schema::RateEstimate::observed(1, 1),
        },
        schema::PairAccessReport {
            pair: "Unavailable 漢字 e\u{301} pair with a long access label".to_string(),
            target_turn: 20,
            percent_of_games: None,
            access: schema::RateEstimate::unavailable(schema::ObservationStatus::OutsideHorizon),
        },
    ]);
    let combo = schema::ComboAccessReport {
        combo: PARTNERS.join(" + "),
        id: "synthetic-width-combo".to_string(),
        produces: vec![
            "Infinite damage with a long synthetic 漢字 e\u{301} feature label".to_string(),
        ],
        bracket_tag: Some("Long optional bracket label 漢字 e\u{301}".to_string()),
        target_turn: 6,
        percent_of_games: Some(Percent::from_percent(100.0)),
        access: schema::RateEstimate::observed(1, 1),
        missing: vec![],
    };
    let mut unavailable = combo.clone();
    unavailable.combo = "Unobserved 漢字 e\u{301} combo with a long label".to_string();
    unavailable.id = "synthetic-unobserved-combo".to_string();
    unavailable.percent_of_games = None;
    unavailable.access =
        schema::RateEstimate::unavailable(schema::ObservationStatus::NoObservations);
    let mut near_miss = combo.clone();
    near_miss.missing.push(LONG_TOKEN.to_string());
    near_miss
        .missing
        .push("Another required card 漢字 e\u{301}".to_string());
    report.combos = Some(schema::CombosReport {
        source: "commanderspellbook".to_string(),
        variants_considered: 3,
        complete: vec![combo.clone(), unavailable],
        near_misses: vec![near_miss],
    });
    report.win_paths = Some(schema::WinPathsReport {
        count: 1,
        paths: vec![combo],
    });
    report.hypgeo = Some(hypgeo::HypgeoReport {
        note: format!("Exact uniform-sample baseline with source {SOURCE_PATH}"),
        cards: vec![hypgeo::ModeledAccess {
            name: PARTNERS[0].to_string(),
            copies: 1,
            target_turn: 4,
            percent_modeled_access: Percent::from_percent(25.0),
        }],
    });
    report
}

/// Use production diffing for shape, partner identity, and finding rows.
fn layout_diff(report: &SimReport) -> report_view::ReportDiff {
    let mut baseline = report.clone();
    baseline.deck_shape.lands += 1;
    baseline.findings.clear();
    baseline.combat.commander_potential[1].name = "Prior commander partner".to_string();
    diff_reports(&baseline, report).expect("compatible report diff")
}

/// Exercise the same human renderers used by the simulator command.
fn render_layout_child(mode: &str) {
    let width: usize = std::env::var("STM_SIMULATOR_TEST_WIDTH")
        .expect("width")
        .parse()
        .expect("numeric width");
    assert_eq!(
        crate::output::terminal_width(),
        width,
        "the child must have a real PTY width"
    );
    let report = layout_report();
    println!("SIMULATOR_OUTPUT_BEGIN");
    if mode == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("report JSON")
        );
    } else if mode == "json-diff" {
        println!(
            "{}",
            serde_json::to_string_pretty(&layout_diff(&report)).expect("diff JSON")
        );
    } else {
        let before = serde_json::to_value(&report).expect("original report");
        let out = crate::output::Output::new(false, mode == "plain", false);
        report::print_report(&out, &report);
        report_view::print_combo_access(&out, report.combo_access.as_deref().expect("pair rows"));
        report_view::print_store_combos(&out, report.combos.as_ref().expect("store combos"));
        report_view::print_win_paths(&out, report.win_paths.as_ref().expect("win paths"));
        report_view::print_hypgeo(&out, report.hypgeo.as_ref().expect("modeled access"));
        report_view::print_diff(&out, &layout_diff(&report));
        report_view::print_diff(&out, &Default::default());
        assert_eq!(
            serde_json::to_value(&report).expect("rendered report"),
            before,
            "rendering must not change machine data"
        );
        let mut unavailable = report.clone();
        unavailable.outcomes.finished_games = 0;
        unavailable.outcomes.incomplete_games = 1;
        report::print_report(&out, &unavailable);
    }
    println!("SIMULATOR_OUTPUT_END");
}

/// Capture a real terminal without letting BSD script echo EOF before JSON.
fn capture_layout(width: usize, mode: &str) -> String {
    let temp = tempfile::tempdir().expect("terminal wrapper directory");
    let wrapper = temp.path().join("simulator output wrapper.sh");
    std::fs::write(
        &wrapper,
        r#"#!/bin/bash
set -eu
stty -echo rows 40 cols "$STM_SIMULATOR_TEST_WIDTH"
exec "$STM_SIMULATOR_TEST_BINARY" --exact "$STM_SIMULATOR_TEST_NAME" --nocapture </dev/null
"#,
    )
    .expect("write Bash wrapper");
    let mut command = Command::new("script");
    if cfg!(target_os = "macos") {
        command.args(["-q", "/dev/null", "/bin/bash"]).arg(&wrapper);
    } else {
        command.args([
            "-q",
            "-c",
            "exec /bin/bash \"$STM_SIMULATOR_TEST_WRAPPER\"",
            "/dev/null",
        ]);
    }
    let test_module = module_path!()
        .split_once("::")
        .expect("crate test module")
        .1;
    command
        .env(
            "STM_SIMULATOR_TEST_NAME",
            format!("{test_module}::simulator_output_respects_terminal_widths"),
        )
        .env(
            "STM_SIMULATOR_TEST_BINARY",
            std::env::current_exe().expect("test executable"),
        )
        .env("STM_SIMULATOR_TEST_WRAPPER", &wrapper)
        .env("STM_SIMULATOR_TEST_WIDTH", width.to_string())
        .env("STM_SIMULATOR_TEST_MODE", mode)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env("COLORFGBG", "15;0")
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("FORCE_COLOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("native script utility");
    let input = child.stdin.take().expect("keep script input open");
    let result = child.wait_with_output().expect("renderer capture");
    drop(input);
    let stdout = String::from_utf8(result.stdout)
        .expect("terminal UTF-8")
        .replace('\r', "");
    assert!(
        result.status.success(),
        "{mode} at {width}: {stdout}\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        result.stderr.is_empty(),
        "renderer diagnostics: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    stdout
        .split_once("SIMULATOR_OUTPUT_BEGIN\n")
        .expect("output start")
        .1
        .split_once("SIMULATOR_OUTPUT_END\n")
        .expect("output end")
        .0
        .to_string()
}

/// Allow overflow only when one unbroken token alone exceeds the display width.
fn assert_display_width(text: &str, width: usize) {
    for line in text.lines() {
        if console::measure_text_width(line) > width {
            let words: Vec<_> = line.split_whitespace().collect();
            assert_eq!(
                words.len(),
                1,
                "line exceeds {width} display columns: {line:?}"
            );
            assert!(
                console::measure_text_width(words[0]) > width,
                "unexpected overflow: {line:?}"
            );
        }
    }
}

/// Wrapping may change whitespace, but it must not remove or change any content.
fn normalized_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Summary, optional, partner, unavailable, and diff rows fit real terminal widths.
#[test]
fn simulator_output_respects_terminal_widths() {
    if let Ok(mode) = std::env::var("STM_SIMULATOR_TEST_MODE") {
        render_layout_child(&mode);
        return;
    }
    let mut reference = None;
    for width in [32, 40, 60, 80, 120] {
        let plain = capture_layout(width, "plain");
        assert!(!plain.contains('\u{1b}'), "plain output contains escapes");
        assert_display_width(&plain, width);
        let normalized = normalized_text(&plain);
        for expected in [
            "Commander combat",
            "25.0 average cumulative potential damage",
            "unavailable (no observations) reach 21",
            "unavailable (target turn is beyond the turn limit)",
            "unavailable (no games observed this access window)",
            "Unmet combo requirements",
            "Another required card 漢字 e\u{301}",
            "Changes from baseline",
            "No report changes from baseline",
            "Rates, averages, and timings are unavailable: no finished games.",
        ] {
            assert!(
                normalized.contains(expected),
                "missing {expected:?} at {width}: {plain}"
            );
        }
        for name in PARTNERS {
            assert!(normalized.contains(name), "missing commander {name}");
        }
        assert!(
            normalized.find("Findings").expect("findings heading")
                < normalized
                    .find("Opening and mana")
                    .expect("measurement heading"),
            "findings must appear before supporting measurements"
        );
        assert!(
            plain.contains(SOURCE_PATH),
            "source path was clipped or split"
        );
        assert!(
            plain.contains(LONG_TOKEN),
            "unbroken card identity was clipped or split"
        );
        if let Some(reference) = &reference {
            assert_eq!(&normalized, reference);
        } else {
            reference = Some(normalized.clone());
        }
        let colored = capture_layout(width, "color");
        assert!(
            colored.contains("38;2;255;92;154m"),
            "missing terminal accent"
        );
        let unstyled = console::strip_ansi_codes(&colored);
        assert_display_width(&unstyled, width);
        assert_eq!(normalized_text(&unstyled), normalized, "color changed text");
        let json = capture_layout(width, "json");
        assert!(!json.contains('\u{1b}'), "JSON contains escapes");
        let decoded: SimReport = serde_json::from_str(&json).expect("unwrapped report JSON");
        assert_eq!(
            decoded,
            layout_report(),
            "terminal width changed machine data"
        );
        assert!(
            json.lines()
                .any(|line| console::measure_text_width(line) > width),
            "machine output must not be word-wrapped"
        );
        let json_diff = capture_layout(width, "json-diff");
        assert!(!json_diff.contains('\u{1b}'), "JSON diff contains escapes");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json_diff).expect("unwrapped diff JSON"),
            serde_json::to_value(layout_diff(&layout_report())).expect("expected diff"),
            "terminal width changed diff data"
        );
    }
}
