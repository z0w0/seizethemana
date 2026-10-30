//! Offline setup transitions on real terminals, including redirected streams.

#![cfg(unix)]

use super::*;
use std::io::{IsTerminal, Write};
use std::time::Duration;

/// Capture a real terminal and apply its cursor commands to a small screen.
#[path = "setup_terminal_capture.rs"]
mod terminal_capture;
use terminal_capture::TerminalCapture;

/// Mark a screen state without moving the terminal cursor or printing a line.
fn checkpoint(name: &str) {
    eprint!("\x1b]777;STM_SETUP:{name}\x07");
    std::io::stderr().flush().expect("flush screen checkpoint");
}

/// Make each active screen stable before recording its checkpoint.
fn draw_checkpoint(out: &Output, name: &str) {
    if let Some(bar) = &out.progress {
        bar.disable_steady_tick();
        bar.force_draw();
    }
    checkpoint(name);
}

/// Count fixture bytes through the same update methods used by downloads.
fn render_fixture_download(out: &mut Output, object: &str, bytes: &[u8], prefix: &str) {
    out.start_download(object, 0);
    draw_checkpoint(out, &format!("{prefix}-unknown"));
    let total = u64::try_from(bytes.len()).expect("fixture byte count fits u64");
    out.tick_progress(total / 2);
    draw_checkpoint(out, &format!("{prefix}-received"));
    out.set_progress_total(total);
    draw_checkpoint(out, &format!("{prefix}-active"));
    out.tick_progress(total - total / 2);
    out.finish(
        "Downloaded",
        &format!("{object} ({})", progress::byte_count(total)),
        Duration::ZERO,
    );
    checkpoint(&format!("{prefix}-done"));
}

/// Exercise setup output without loading a model, opening a store, or networking.
fn render_setup_fixture(mode: &str) {
    let mut out = Output::new(mode == "json", mode == "plain", false);
    let stderr_terminal = std::io::stderr().is_terminal();
    let streams = std::env::var("STM_SETUP_TEST_STREAMS").expect("fixture stream mode");
    assert_eq!(stderr_terminal, streams != "stderr-file");
    assert_eq!(std::io::stdout().is_terminal(), streams != "stdout-file");
    assert_eq!(out.supports_progress(), stderr_terminal && mode != "json");
    eprintln!();
    checkpoint("begin");
    render_fixture_download(&mut out, "card data", &[b'c'; 512], "cards");
    render_fixture_download(&mut out, "card tags", &[b't'; 256], "tags");
    out.status("Reusing", "cached card data");
    checkpoint("cache");
    out.start_task("Loading", "search model", 0);
    draw_checkpoint(&out, "model-active");
    out.finish("Loaded", "search model", Duration::ZERO);
    checkpoint("model-done");
    out.start_task("Building", "search index", 10);
    out.tick_progress(4);
    draw_checkpoint(&out, "index-active");
    out.warning("fixture warning");
    out.hint("retry setup");
    draw_checkpoint(&out, "diagnostics-active");
    out.finish("Built", "search index", Duration::ZERO);
    checkpoint("index-done");
    if std::env::var("STM_SETUP_TEST_WIDTH").expect("fixture terminal width") == "32" {
        out.start_download("large card data", 80 * 1024 * 1024);
        out.tick_progress(40 * 1024 * 1024);
        draw_checkpoint(&out, "large-active");
        out.clear_progress();
        checkpoint("large-cleared");
    }
    out.start_task("Loading", "retry model", 0);
    draw_checkpoint(&out, "error-active");
    out.error("fixture error");
    checkpoint("error");
    out.clear_progress();
    checkpoint("end");
}

/// Re-enter only this test in the PTY child process.
#[test]
fn setup_terminal_fixture() {
    if let Ok(mode) = std::env::var("STM_SETUP_TEST_MODE") {
        if mode == "failures" {
            render_setup_failures();
        } else {
            render_setup_fixture(&mode);
        }
    }
}

/// Fail the real setup encoder call without loading a model or producing vectors.
struct FailingEmbedder;

impl crate::embed::DocumentEmbedder for FailingEmbedder {
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        assert!(!texts.is_empty(), "the fixture must reach the encoder");
        anyhow::bail!("fixture encoder failure")
    }
}

/// Check production cleanup before an error printer can hide a leaked bar.
fn assert_failed_task_is_clear(out: &Output, name: &str) {
    assert!(out.progress.is_none(), "{name} left an active progress bar");
    assert!(out.task_output().is_some(), "{name} still owns task output");
    checkpoint(name);
}

/// Run real download and setup failure paths with only isolated local data.
fn render_setup_failures() {
    let directory = tempfile::tempdir().expect("failed setup fixture store");
    let paths = crate::paths::Paths::new(directory.path().to_path_buf());
    paths
        .ensure_dirs()
        .expect("failed setup fixture directories");
    let conn = crate::db::open(&paths.db()).expect("failed setup fixture database");
    conn.execute(
        "INSERT INTO cards (name, oracle_id) VALUES ('One', 'one'), ('Two', 'two')",
        [],
    )
    .expect("failed setup fixture cards");
    let mut out = Output::new(false, true, false);
    assert!(out.supports_progress(), "failures need a real terminal");
    eprintln!();
    checkpoint("begin");
    let download_error = crate::scryfall::download_to_named(
        "not a URL",
        &paths.bulk_file(),
        Some(512),
        &mut out,
        "card data",
    )
    .expect_err("invalid URL must fail without a request");
    assert_failed_task_is_clear(&out, "download-failed");
    assert!(!paths.bulk_file().exists());
    out.error(&download_error.to_string());
    checkpoint("download-reported");

    let model_error = crate::setup::embed_all_with_model(&paths, &mut out, &conn, || {
        anyhow::bail!("fixture model failure")
    })
    .expect_err("fixture model loader must fail");
    assert_failed_task_is_clear(&out, "model-failed");
    assert!(!paths.vectors_file().exists());
    out.error(&model_error.to_string());
    checkpoint("model-reported");

    let encoder_error = crate::setup::embed_all_with_model(&paths, &mut out, &conn, || {
        Ok(Box::new(FailingEmbedder))
    })
    .expect_err("fixture encoder must fail");
    assert_failed_task_is_clear(&out, "encoder-failed");
    assert!(!paths.vectors_file().exists());
    out.error(&encoder_error.to_string());
    checkpoint("end");
}

/// The completed transcript must retain task identities after transient bars clear.
fn completed_lines() -> Vec<String> {
    let styles = Styles::colorless();
    vec![
        styles.status("Downloaded", "card data (512 B)"),
        styles.status("Downloaded", "card tags (256 B)"),
        styles.status("Reusing", "cached card data"),
        styles.status("Loaded", "search model"),
        "warning: fixture warning".to_string(),
        "hint: retry setup".to_string(),
        styles.status("Built", "search index"),
        "error: fixture error".to_string(),
    ]
}

/// Check the label, object column, bounded bar, and separate narrow-screen counts.
fn assert_active_task(
    capture: &TerminalCapture,
    checkpoint: &str,
    verb: &str,
    object: &str,
    counts: Option<&str>,
) {
    let lines = capture.frame(checkpoint);
    let label = Styles::colorless().status(verb, object);
    let label_index = lines
        .iter()
        .position(|line| line == &label)
        .unwrap_or_else(|| panic!("missing {label:?} at {checkpoint}: {lines:?}"));
    assert_eq!(label.find(object), Some(13), "object column changed");
    let details = &lines[label_index + 1..];
    let expected_rows = if counts.is_some() && capture.width < 80 {
        2
    } else {
        1
    };
    assert_eq!(details.len(), expected_rows, "{checkpoint}: {lines:?}");
    let indent = " ".repeat(if capture.width < 48 { 2 } else { 13 });
    for line in details {
        assert!(line.starts_with(&indent), "{checkpoint}: {line:?}");
        assert!(
            console::measure_text_width(line) <= capture.width,
            "activity exceeds stderr width: {line:?}"
        );
    }
    if let Some(counts) = counts {
        assert_eq!(
            details[0].find('['),
            Some(indent.len()),
            "bar column changed"
        );
        assert!(
            details.join(" ").contains(counts),
            "{checkpoint}: {details:?}"
        );
        let bar = details[0]
            .split_once('[')
            .expect("bar start")
            .1
            .split_once(']')
            .expect("bar end")
            .0;
        assert_eq!(bar.len(), if capture.width < 80 { 10 } else { 20 });
        assert!(bar.chars().all(|ch| matches!(ch, '#' | '>' | '-')));
        if capture.width < 80 {
            assert!(details[0].contains('%'), "narrow bar needs a percentage");
            assert!(details[1].contains(counts), "counts need their own row");
            assert_eq!(details[1].find(counts), Some(indent.len()));
        }
    } else {
        assert!(details[0].contains("elapsed"), "{checkpoint}: {details:?}");
        assert!(!details[0].contains("0/0"), "unknown totals are not counts");
        assert!(
            !details[0].contains("0 B"),
            "unknown downloads need activity"
        );
    }
}

/// Every completion clears active rows and retains the completed object.
fn assert_terminal_transitions(capture: &TerminalCapture) {
    assert_active_task(capture, "cards-unknown", "Downloading", "card data", None);
    let received = capture.frame("cards-received");
    assert_eq!(received.len(), 2, "unknown downloads keep one activity row");
    assert_eq!(
        received[0],
        Styles::colorless().status("Downloading", "card data")
    );
    assert!(received[1].contains("256 B"), "received bytes must update");
    assert!(
        !received[1].contains("256 B/") && !received[1].contains("/0"),
        "unknown totals must stay unknown"
    );
    assert_active_task(
        capture,
        "cards-active",
        "Downloading",
        "card data",
        Some("256 B/512 B"),
    );
    assert_active_task(
        capture,
        "tags-active",
        "Downloading",
        "card tags",
        Some("128 B/256 B"),
    );
    assert_active_task(capture, "model-active", "Loading", "search model", None);
    for checkpoint in ["index-active", "diagnostics-active"] {
        assert_active_task(
            capture,
            checkpoint,
            "Building",
            "search index",
            Some("4/10"),
        );
    }
    let completed = completed_lines();
    for (checkpoint, expected) in [
        ("cards-done", &completed[..1]),
        ("tags-done", &completed[..2]),
        ("cache", &completed[..3]),
        ("model-done", &completed[..4]),
        ("index-done", &completed[..7]),
        ("error", completed.as_slice()),
        ("end", completed.as_slice()),
    ] {
        assert_eq!(capture.frame(checkpoint), expected, "{checkpoint}");
    }
    if capture.width == 32 {
        assert_active_task(
            capture,
            "large-active",
            "Downloading",
            "large card data",
            Some("40.00 MiB/80.00 MiB"),
        );
        assert_eq!(capture.frame("large-cleared"), &completed[..7]);
    }
}

/// Real PTYs preserve labels and completion lines at narrow and wide widths.
#[test]
fn setup_progress_preserves_named_tasks_in_real_terminals() {
    for width in [32, 48, 80, 120, 200] {
        for mode in ["plain", "color"] {
            let capture = TerminalCapture::run(width, mode, "terminal");
            assert_terminal_transitions(&capture);
            assert_eq!(capture.has_sgr(), mode == "color", "{width}, {mode}");
            if mode == "color" {
                assert!(
                    capture
                        .between("cards-unknown", "cards-active")
                        .contains("38;2;255;92;154m"),
                    "missing bar accent"
                );
                assert!(capture.has_sgr_parameter("32"), "missing green task verb");
            }
        }
    }
}

/// Stderr alone controls activity; JSON suppresses task text even on a terminal.
#[test]
fn setup_progress_handles_redirection_and_json() {
    let stdout_redirected = TerminalCapture::run(48, "color", "stdout-file");
    assert_terminal_transitions(&stdout_redirected);
    assert!(stdout_redirected.has_sgr(), "stderr keeps terminal colors");

    let stderr_redirected = TerminalCapture::run(48, "color", "stderr-file");
    assert!(
        !stderr_redirected.has_sgr(),
        "redirected diagnostics stay plain"
    );
    let plain = stderr_redirected.frame("end");
    let completed = completed_lines();
    let styles = Styles::colorless();
    let expected = [
        styles.status("Downloading", "card data"),
        completed[0].clone(),
        styles.status("Downloading", "card tags"),
        completed[1].clone(),
        completed[2].clone(),
        styles.status("Loading", "search model"),
        completed[3].clone(),
        styles.status("Building", "search index"),
        completed[4].clone(),
        completed[5].clone(),
        completed[6].clone(),
        styles.status("Loading", "retry model"),
        completed[7].clone(),
    ];
    assert_eq!(
        plain,
        expected.as_slice(),
        "plain logs need one start per task"
    );

    let json = TerminalCapture::run(48, "json", "terminal");
    assert!(!json.has_sgr());
    assert!(
        json.frame("index-active").is_empty(),
        "JSON leaked task output"
    );
    assert_eq!(
        json.frame("end"),
        ["hint: retry setup", "error: fixture error"]
    );
}

/// Failed production tasks clear progress before reporting errors or allowing retries.
#[test]
fn setup_progress_clears_real_failures_before_error_output() {
    let capture = TerminalCapture::run(80, "failures", "terminal");
    let download_error = "error: downloading card data";
    assert!(capture.frame("download-failed").is_empty());
    assert_eq!(capture.frame("download-reported"), [download_error]);
    assert_eq!(capture.frame("model-failed"), [download_error]);
    let model_errors = [download_error, "error: fixture model failure"];
    assert_eq!(capture.frame("model-reported"), model_errors);
    let encoder_failed = capture.frame("encoder-failed");
    assert_eq!(encoder_failed.len(), 3, "failed index left transient rows");
    assert_eq!(&encoder_failed[..2], model_errors);
    let loaded_prefix = Styles::colorless().status("Loaded", "search model in ");
    assert!(encoder_failed[2].starts_with(&loaded_prefix));
    assert!(encoder_failed[2].ends_with('s'));
    let finished = capture.frame("end");
    assert_eq!(finished.len(), 4);
    assert_eq!(&finished[..3], encoder_failed);
    assert_eq!(finished[3], "error: fixture encoder failure");
    assert!(
        !finished
            .iter()
            .any(|line| line.contains("Built") || line.contains("Downloaded"))
    );
    assert!(
        !capture.has_sgr(),
        "the failure fixture disables color only"
    );
}
