//! Cargo-style task labels with bounded progress bars on a separate line.

use super::{Output, Styles, VERB_WIDTH};

/// Units that determine whether progress shows item counts or transferred bytes.
#[derive(Debug, Clone, Copy)]
pub(super) enum ProgressUnits {
    /// A count of cards, games, or other task items.
    Items,
    /// Response bytes saved to disk, before any decompression.
    Bytes,
}

/// Use the same binary byte units in checkpoints, bars, and completion lines.
pub(crate) fn byte_count(bytes: u64) -> String {
    indicatif::HumanBytes(bytes).to_string()
}

/// Available stderr width, independent of redirected result output.
fn progress_width() -> usize {
    console::Term::stderr()
        .size_checked()
        .map_or(80, |(_, width)| usize::from(width))
        .max(1)
}

impl Output {
    /// Show one active task in a terminal, or one start line in a plain log.
    pub(crate) fn start_task(&mut self, verb: &str, msg: &str, total: u64) {
        if self.interactive {
            self.progress_bar(verb, msg, total);
        } else {
            self.status(verb, msg);
        }
    }

    /// Show a named download with byte units instead of raw item counts.
    pub(crate) fn start_download(&mut self, msg: &str, total: u64) {
        if self.interactive {
            self.begin_progress("Downloading", msg, total, ProgressUnits::Bytes);
        } else {
            self.status("Downloading", msg);
        }
    }

    /// Draw item progress on stderr; zero means the total is not yet known.
    pub fn progress_bar(&mut self, verb: &str, msg: &str, total: u64) {
        self.begin_progress(verb, msg, total, ProgressUnits::Items);
    }

    /// Apply the shared stream policy before creating a task's transient display.
    fn begin_progress(&mut self, verb: &str, msg: &str, total: u64, units: ProgressUnits) {
        if !self.interactive {
            return;
        }
        self.clear_progress();
        let width = progress_width();
        let styles = self.err_styles();
        let bar = indicatif::ProgressBar::new(total);
        bar.set_style(progress_style(total, units, width, &styles));
        bar.set_message(progress_message(verb, msg, width, &styles));
        bar.enable_steady_tick(std::time::Duration::from_millis(250));
        self.progress_units = units;
        self.progress = Some(bar);
    }

    /// Advance the task by completed items or received bytes.
    pub fn tick_progress(&self, delta: u64) {
        if let Some(bar) = &self.progress {
            bar.inc(delta);
        }
    }

    /// Set the number of completed items or received bytes.
    pub fn set_progress_position(&self, pos: u64) {
        if let Some(bar) = &self.progress {
            bar.set_position(pos);
        }
    }

    /// Replace an unknown total without changing the task's units.
    pub fn set_progress_total(&self, total: u64) {
        if let Some(bar) = &self.progress {
            bar.set_length(total);
            bar.set_style(progress_style(
                total,
                self.progress_units,
                progress_width(),
                &self.err_styles(),
            ));
        }
    }

    /// Clear transient activity; callers print the named result after successful work.
    pub fn clear_progress(&mut self) {
        if let Some(bar) = self.progress.take() {
            bar.finish_and_clear();
        }
    }
}

/// Keep wrapped task text in the same object column as Cargo-style status lines.
fn progress_message(verb: &str, msg: &str, width: usize, styles: &Styles) -> String {
    let object_width = width.saturating_sub(VERB_WIDTH + 1).max(1);
    Styles::wrap(msg, object_width)
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                styles.status(verb, &line)
            } else {
                format!("{:padding$}{line}", "", padding = VERB_WIDTH + 1)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build an ASCII bar that does not grow to fill a wide terminal.
pub(super) fn progress_style(
    total: u64,
    units: ProgressUnits,
    width: usize,
    styles: &Styles,
) -> indicatif::ProgressStyle {
    indicatif::ProgressStyle::with_template(&progress_template(total, units, width, styles))
        .expect("progress template is valid")
        .with_key("received", received_progress)
        .progress_chars("#>-")
        .tick_chars("|/-\\ ")
}

/// Show activity before the first byte, then actual received bytes without a fake total.
fn received_progress(state: &indicatif::ProgressState, text: &mut dyn std::fmt::Write) {
    let detail = if state.pos() == 0 {
        format!("{}s elapsed", state.elapsed().as_secs())
    } else {
        byte_count(state.pos())
    };
    text.write_str(&detail)
        .expect("progress details write to a string");
}

/// Put activity below the task label so labels stay aligned across transitions.
pub(super) fn progress_template(
    total: u64,
    units: ProgressUnits,
    width: usize,
    styles: &Styles,
) -> String {
    // Keep readable byte units on narrow screens without moving the verb column.
    let indent = " ".repeat(if width < 48 { 2 } else { VERB_WIDTH + 1 });
    if total == 0 {
        let detail = match units {
            ProgressUnits::Items => "{spinner} {elapsed} elapsed",
            ProgressUnits::Bytes => "{spinner} {received}",
        };
        return format!("{{msg}}\n{indent}{detail}");
    }
    let counts = match units {
        ProgressUnits::Items => "{pos}/{len}",
        ProgressUnits::Bytes => "{bytes}/{total_bytes}",
    };
    if width < 80 {
        let bar = styles.header("{bar:10}");
        format!("{{msg}}\n{indent}[{bar}] {{percent:>3}}%\n{indent}{counts}")
    } else {
        let bar = styles.header("{bar:20}");
        format!("{{msg}}\n{indent}[{bar}] {counts} ({{eta}})")
    }
}
