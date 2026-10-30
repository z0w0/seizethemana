//! Offline checks for progress layout changes and diagnostic cleanup.

use super::*;
use std::sync::{Arc, Mutex};

/// Record progress text without requiring a real terminal or cursor handling.
#[derive(Debug, Clone, Default)]
struct RecordingTerm(Arc<Mutex<String>>);

impl indicatif::TermLike for RecordingTerm {
    fn width(&self) -> u16 {
        120
    }

    fn move_cursor_up(&self, _lines: usize) -> std::io::Result<()> {
        Ok(())
    }

    fn move_cursor_down(&self, _lines: usize) -> std::io::Result<()> {
        Ok(())
    }

    fn move_cursor_right(&self, _columns: usize) -> std::io::Result<()> {
        Ok(())
    }

    fn move_cursor_left(&self, _columns: usize) -> std::io::Result<()> {
        Ok(())
    }

    fn write_line(&self, text: &str) -> std::io::Result<()> {
        let mut output = self.0.lock().expect("progress recording lock");
        output.push_str(text);
        output.push('\n');
        Ok(())
    }

    fn write_str(&self, text: &str) -> std::io::Result<()> {
        self.0
            .lock()
            .expect("progress recording lock")
            .push_str(text);
        Ok(())
    }

    fn clear_line(&self) -> std::io::Result<()> {
        Ok(())
    }

    fn flush(&self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Create a spinner whose rendered text is available to offline assertions.
fn recording_output() -> (Output, RecordingTerm) {
    let terminal = RecordingTerm::default();
    let mut out = Output::new(false, true, false);
    let bar = indicatif::ProgressBar::with_draw_target(
        Some(0),
        indicatif::ProgressDrawTarget::term_like(Box::new(terminal.clone())),
    );
    bar.set_style(progress_style(
        0,
        ProgressUnits::Items,
        120,
        &out.err_styles(),
    ));
    bar.set_message("Testing cards");
    out.progress = Some(bar);
    (out, terminal)
}

/// A newly known total replaces the spinner with a count and character bar.
#[test]
fn known_total_changes_the_rendered_progress_layout() {
    let (mut out, terminal) = recording_output();
    out.set_progress_position(2);
    terminal.0.lock().expect("progress recording lock").clear();
    out.set_progress_total(10);
    out.progress.as_ref().expect("active progress").force_draw();
    let text = terminal.0.lock().expect("progress recording lock").clone();
    assert!(text.contains("2/10"), "{text}");
    assert!(text.contains('[') && text.contains(']'), "{text}");
    assert!(text.is_ascii(), "plain progress uses ASCII: {text}");
    out.clear_progress();
}

/// Errors stop progress, while advisory diagnostics let the task continue.
#[test]
fn errors_finish_progress_but_warnings_and_hints_keep_it_active() {
    let (mut out, _terminal) = recording_output();
    let bar = out.progress.as_ref().expect("active progress").clone();
    out.warning("synthetic warning");
    out.hint("synthetic next step");
    assert!(!bar.is_finished());
    out.error("synthetic error");
    assert!(bar.is_finished());
    out.clear_progress();
}

/// Cursor progress uses the shared stream policy, independent of color.
#[test]
fn progress_capability_matches_the_stream_policy() {
    let mut out = Output::new(false, true, false);
    for (json, terminal, dumb) in [
        (false, false, false),
        (false, true, false),
        (false, true, true),
        (true, true, false),
    ] {
        let (_, interactive) = stream_policy(json, true, terminal, dumb);
        out.interactive = interactive;
        assert_eq!(out.supports_progress(), !json && terminal && !dumb);
    }
}

/// A nested task leaves an existing bar alone and keeps JSON copies silent.
#[test]
fn task_output_never_shares_an_active_progress_bar() {
    let (mut out, _terminal) = recording_output();
    assert!(out.task_output().is_none());
    assert!(
        !out.progress
            .as_ref()
            .expect("active progress")
            .is_finished()
    );
    out.clear_progress();
    assert!(
        out.task_output()
            .expect("idle task output")
            .progress
            .is_none()
    );
    let json = Output::new(true, false, false);
    let task = json.task_output().expect("JSON task output");
    assert!(task.json);
    assert!(!task.supports_progress());
}
