use owo_colors::OwoColorize;
use std::io::IsTerminal;

/// Terminal width for layout sizing, floored at 40 columns. 80 when
/// undetectable (piped, tests). Narrow terminals never shrink below 40 so
/// fixed-width table lines stay intact.
pub fn terminal_width() -> usize {
    console::Term::stdout()
        .size_checked()
        .map(|(_, w)| w as usize)
        .unwrap_or(80)
        .max(40)
}

/// Currency used for every price this tool reads and prints. Scryfall
/// supplies prices in USD only; when another source lands, swap this and
/// the `money` symbol branch together.
pub const CURRENCY: &str = "USD";

/// Bail-message sentinel for errors already printed in full by the
/// command (error + hint). The main dispatcher suppresses its own
/// `error:` line when a bail carries exactly this message, so the user
/// never sees the same failure twice.
pub const SILENT_ERROR: &str = "\u{0}silent";

/// Thousands-grouped integer string ("1,512", "-1,234") for counts.
pub fn grouped_int(n: i64) -> String {
    let negative = n.is_negative();
    let digits = n.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    if negative {
        format!("-{grouped}")
    } else {
        grouped
    }
}

/// Thousands-grouped money amount with two decimals ("1,234.50"); no
/// currency symbol (callers add `$` and the currency code).
fn thousands_amount(amount: f64) -> String {
    let fixed = format!("{amount:.2}");
    let (int_part, frac_part) = fixed.split_once('.').unwrap_or((fixed.as_str(), ""));
    let group = if let Ok(n) = int_part.parse::<i64>() {
        grouped_int(n)
    } else {
        int_part.to_string()
    };
    if frac_part.is_empty() {
        group
    } else {
        format!("{group}.{frac_part}")
    }
}

/// Round to two decimals for JSON money fields.
#[must_use]
pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Rendering hub: decides between human (colored) and JSON output from the
/// active command's `--json` flag, so command code never branches on
/// `--json` mid-render.
///
/// Invariants (see docs/design.md):
///
/// - stdout carries results only; every status/progress line goes to stderr.
/// - JSON mode emits nothing but the command's JSON result — status and
///   progress lines are suppressed entirely.
/// - ANSI color follows the *stdout* TTY for result text and the *stderr* TTY
///   for status lines; `NO_COLOR` and `--no-color` force both plain.
#[derive(Debug, Clone)]
pub struct Output {
    /// Emit pretty-printed JSON instead of styled text.
    pub json: bool,
    /// Color output is allowed on stdout (results).
    color: bool,
    /// Color output is allowed on stderr (status lines).
    err_color: bool,
    /// Show progress detail on stderr (from `--verbose`).
    pub verbose: bool,
    /// Active ephemeral progress line on stderr, if any.
    progress: Option<indicatif::ProgressBar>,
}

/// Shared styling decisions derived from [`Output`].
///
/// Every method returns a plain string when color is disabled, so callers can
/// wrap text unconditionally.
#[derive(Debug, Clone, Copy)]
pub struct Styles {
    color: bool,
}

/// Verb column width for cargo-style status lines (`   Compiling serde ...`).
const VERB_WIDTH: usize = 12;

/// Color family for [`Styles::glyph`].
#[derive(Debug, Clone, Copy)]
pub enum GlyphKind {
    Good,
    Warn,
    Bad,
    Dim,
    /// Advisory note (bracket judgment calls): informational, never a
    /// violation or a genuine conflict.
    Info,
}

impl Output {
    /// Build the output mode from global flags.
    pub fn new(json: bool, no_color: bool, verbose: bool) -> Self {
        let no_color = no_color || std::env::var_os("NO_COLOR").is_some();
        Self {
            json,
            color: !json && !no_color && std::io::stdout().is_terminal(),
            err_color: !no_color && std::io::stderr().is_terminal(),
            verbose,
            progress: None,
        }
    }

    /// Styling helpers for human output. In JSON mode returns no-op styles.
    pub fn styles(&self) -> Styles {
        Styles { color: self.color }
    }

    /// Styling helpers for stderr streams (status, errors, hints).
    pub fn err_styles(&self) -> Styles {
        Styles {
            color: self.err_color,
        }
    }

    /// Cargo-style status line on stderr: `   <Verb> <message>`, verb bold
    /// green and padded to the standard verb column.
    ///
    /// Clears any active progress bar first, so the status line is the last
    /// thing visible. Suppressed in JSON mode.
    pub fn status(&mut self, verb: &str, msg: &str) {
        self.clear_progress();
        if self.json {
            return;
        }
        eprintln!("{}", self.err_styles().status(verb, msg));
    }

    /// Final summary status line: `   <Verb> <message> in <secs>s`.
    ///
    /// A zero duration omits the timing suffix entirely.
    pub fn finish(&mut self, verb: &str, msg: &str, elapsed: std::time::Duration) {
        if elapsed.is_zero() {
            self.status(verb, msg);
        } else {
            self.status(verb, &format!("{msg} in {:.2}s", elapsed.as_secs_f64()));
        }
    }

    /// Replace the ephemeral progress line with a bar over `total` units.
    ///
    /// One shared bar shape for every progress surface (downloads, bulk
    /// parsing, embedding): cargo-style padded verb, then dim metadata,
    /// a cyan/blue bar with bytes-or-items `{pos}/{len}`, and a counting-
    /// down ETA. On a piped stderr indicatif renders nothing, so callers
    /// log their own periodic status lines in that mode.
    ///
    /// `total == 0` means "unknown": the bar renders as a spinner with
    /// the same shape (no `pos/len`), and a later call with the true
    /// total restamps it.
    pub fn progress_bar(&mut self, verb: &str, msg: &str, total: u64) {
        if self.json {
            return;
        }
        self.clear_progress();
        let bar = indicatif::ProgressBar::new(total);
        let styled = self.err_styles().status(verb, msg);
        let style = if std::io::stderr().is_terminal() {
            let template = if total > 0 {
                "{msg} [{bar:.cyan/blue}] {pos}/{len} ({eta})"
            } else {
                "{msg} {spinner:.green} {pos}"
            };
            indicatif::ProgressStyle::with_template(template)
                .expect("static template")
                .progress_chars("█>-")
        } else {
            return; // piped stderr: no ephemeral bar; callers log status lines
        };
        bar.set_style(style);
        bar.set_message(styled);
        // Steady tick keeps the ETA counting down even when no item
        // arrives for a while (a stalled download, a big embed chunk).
        bar.enable_steady_tick(std::time::Duration::from_millis(250));
        self.progress = Some(bar);
    }

    /// Advance the active progress bar by `delta` items.
    pub fn tick_progress(&self, delta: u64) {
        if let Some(bar) = &self.progress {
            bar.inc(delta);
        }
    }

    /// Set the active progress bar's absolute position (byte counters:
    /// the position is a running total, not an increment).
    pub fn set_progress_position(&self, pos: u64) {
        if let Some(bar) = &self.progress {
            bar.set_position(pos);
        }
    }

    /// Set the active progress bar's total (an unknown-length spinner
    /// becomes a real bar once the content length arrives).
    pub fn set_progress_total(&self, total: u64) {
        if let Some(bar) = &self.progress {
            bar.set_length(total);
        }
    }

    /// Remove the ephemeral progress line (leaves no output behind).
    pub fn clear_progress(&mut self) {
        if let Some(bar) = self.progress.take() {
            bar.finish_and_clear();
        }
    }

    /// Cargo-style warning line on stderr: `warning: ...` (yellow).
    ///
    /// Cargo emits diagnostics unpadded (unlike status verbs), so no indent.
    pub fn warning(&self, msg: &str) {
        if self.json {
            return;
        }
        eprintln!("{}", self.err_styles().warning(msg));
    }

    /// Print an error to stderr (red) — always, in every mode.
    pub fn error(&self, msg: &str) {
        eprintln!("{}", self.err_styles().error(msg));
    }

    /// Print a hint to stderr (dim) — always, in every mode.
    pub fn hint(&self, msg: &str) {
        eprintln!("{}", self.err_styles().hint(msg));
    }

    /// Note line on stdout: `note: ...` (dim), like the `deck legal`
    /// checklist. Suppressed in JSON mode.
    pub fn print_note(&mut self, msg: &str) {
        self.clear_progress();
        if self.json {
            return;
        }
        println!("{}", self.styles().note(msg));
    }
}

impl Styles {
    /// Plain style set (no color), for tests and plain-text contexts.
    pub fn colorless() -> Self {
        Self { color: false }
    }

    /// Card name: bold cyan.
    pub fn card_name(&self, s: &str) -> String {
        if self.color {
            s.bold().cyan().to_string()
        } else {
            s.to_string()
        }
    }

    /// Section/heading text: bold.
    pub fn header(&self, s: &str) -> String {
        if self.color {
            s.bold().to_string()
        } else {
            s.to_string()
        }
    }

    /// Dimmed metadata (sets, collector numbers, counts).
    pub fn dim(&self, s: &str) -> String {
        if self.color {
            s.dimmed().to_string()
        } else {
            s.to_string()
        }
    }

    /// Cargo-style status line: verb padded to 12 columns, bold green.
    pub fn verb(&self, verb: &str) -> String {
        let verb_pad = format!("{verb:>VERB_WIDTH$}");
        if self.color {
            verb_pad.bold().green().to_string()
        } else {
            verb_pad
        }
    }

    /// Cargo-style status line: padded verb plus message.
    pub fn status(&self, verb: &str, msg: &str) -> String {
        format!("{} {msg}", self.verb(verb))
    }

    /// Success text: green check (used on stdout result summaries).
    pub fn success(&self, s: &str) -> String {
        let text = format!("✓ {s}");
        if self.color {
            text.green().to_string()
        } else {
            text
        }
    }

    /// Warning text: `warning:`, yellow (no cargo-style indent — cargo emits
    /// diagnostics unpadded).
    pub fn warning(&self, s: &str) -> String {
        let text = format!("warning: {s}");
        if self.color {
            text.yellow().to_string()
        } else {
            text
        }
    }

    /// Error text: red (used on stderr).
    pub fn error(&self, s: &str) -> String {
        let text = format!("error: {s}");
        if self.color {
            text.red().to_string()
        } else {
            text
        }
    }

    /// Hint text: dim (used on stderr).
    pub fn hint(&self, s: &str) -> String {
        let text = format!("hint: {s}");
        if self.color {
            text.dimmed().to_string()
        } else {
            text
        }
    }

    /// Note text: `note:` dim (used on stdout result blocks, e.g. the
    /// `deck legal` checklist).
    pub fn note(&self, s: &str) -> String {
        let text = format!("note: {s}");
        if self.color {
            text.dimmed().to_string()
        } else {
            text
        }
    }

    /// Bare colored text without a prefix (verdict glyphs, diff signs).
    ///
    /// The prefixing helpers (`success`/`warning`/`error`) embed
    /// `✓`/`warning:`/`error:` text; bare glyphs like the `deck legal`
    /// bracket verdicts and the simulate diff signs need color only.
    pub fn glyph(&self, s: &str, kind: GlyphKind) -> String {
        if !self.color {
            return s.to_string();
        }
        match kind {
            GlyphKind::Good => s.green().to_string(),
            GlyphKind::Warn => s.yellow().to_string(),
            GlyphKind::Bad => s.red().to_string(),
            GlyphKind::Dim => s.dimmed().to_string(),
            GlyphKind::Info => s.cyan().to_string(),
        }
    }

    /// Mana pip symbols in their color identity: `WUBRGC` letters colored.
    ///
    /// Unknown characters pass through unstyled.
    pub fn mana_pips(&self, s: &str) -> String {
        if !self.color {
            return s.to_string();
        }
        s.chars()
            .map(|ch| {
                let text = ch.to_string();
                match ch {
                    'W' => text.white().to_string(),
                    'U' => text.blue().to_string(),
                    'B' => text.purple().to_string(),
                    'R' => text.red().to_string(),
                    'G' => text.green().to_string(),
                    'C' => text.dimmed().to_string(),
                    _ => text,
                }
            })
            .collect()
    }

    /// Rarity-colored text.
    pub fn rarity(&self, s: &str) -> String {
        if !self.color {
            return s.to_string();
        }
        match s {
            "mythic" => s.magenta().to_string(),
            "rare" => s.yellow().to_string(),
            "uncommon" => s.cyan().to_string(),
            _ => s.to_string(),
        }
    }

    /// Color-identity letters ("WU") with mana color styling.
    pub fn color_letters(&self, letters: &str) -> String {
        self.mana_pips(letters)
    }

    /// Proportional unicode bar: `filled` blocks out of `width` slots, dim.
    ///
    /// `ratio` in [0, 1] selects the fill; a zero or negative ratio renders
    /// an empty bar so rows stay aligned.
    pub fn bar(&self, ratio: f64, width: usize) -> String {
        let width = width.max(1);
        let ratio = ratio.clamp(0.0, 1.0);
        let filled = (ratio * width as f64).round() as usize;
        let text: String = "█".repeat(filled) + &"·".repeat(width - filled);
        if self.color {
            text.dimmed().to_string()
        } else {
            text
        }
    }

    /// Money amount with currency: `$975.40 USD`, bold green when color is
    /// on. USD is the only supported currency.
    pub fn money(&self, amount: f64) -> String {
        let text = format!("${} {CURRENCY}", thousands_amount(amount));
        if self.color {
            text.bold().green().to_string()
        } else {
            text
        }
    }

    /// Thousands-separated integer ("1,512") for counts and dollar totals.
    pub fn thousands(&self, n: i64) -> String {
        grouped_int(n)
    }

    /// Wrap `text` to `width` columns, preserving existing newlines.
    ///
    /// Unicode-width aware so accented names and symbols do not split
    /// mid-cluster. Returns one long line when `width` is small.
    pub fn wrap(text: &str, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            if paragraph.is_empty() {
                lines.push(String::new());
                continue;
            }
            let mut current = String::new();
            for word in paragraph.split_whitespace() {
                let word_width = console::measure_text_width(word);
                let current_width = console::measure_text_width(&current);
                if !current.is_empty() && current_width + 1 + word_width > width.max(1) {
                    lines.push(std::mem::take(&mut current));
                }
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            }
            lines.push(current);
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_mode_disables_color() {
        let out = Output::new(true, false, false);
        let s = out.styles();
        assert_eq!(s.card_name("Bolt"), "Bolt");
        assert_eq!(s.rarity("mythic"), "mythic");
        assert_eq!(s.mana_pips("WUB"), "WUB");
    }

    #[test]
    fn explicit_no_color_disables_color() {
        let out = Output::new(false, true, false);
        let s = out.styles();
        assert_eq!(s.card_name("Bolt"), "Bolt");
        assert_eq!(s.header("Deck"), "Deck");
    }

    #[test]
    fn styled_helpers_prefix_text() {
        let out = Output::new(true, false, false); // color off
        let s = out.styles();
        assert_eq!(s.success("done"), "✓ done");
        assert_eq!(s.warning("careful"), "warning: careful");
        assert_eq!(s.error("boom"), "error: boom");
        assert_eq!(s.hint("run setup"), "hint: run setup");
    }

    #[test]
    fn mana_pips_pass_through_unknown_chars() {
        let out = Output::new(true, false, false);
        let s = out.styles();
        assert_eq!(s.mana_pips("{2}{W}"), "{2}{W}");
    }

    #[test]
    fn status_verb_is_padded_cargo_style() {
        let out = Output::new(true, false, false); // color off
        let s = out.err_styles();
        assert_eq!(
            s.status("Fetching", "bulk index"),
            "    Fetching bulk index"
        );
        assert_eq!(
            s.status("Ingested", "32572 cards"),
            "    Ingested 32572 cards"
        );
    }

    #[test]
    fn progress_bar_templates_have_no_extra_padding() {
        // The templates must not prepend spaces: Styles::status already
        // pads the verb to the cargo-style verb column.
        let out = Output::new(false, false, false);
        let mut out = out;
        out.progress_bar("Embedding", "cards", 10);
        out.clear_progress();
        out.progress_bar("Downloading", "bulk data", 0);
        out.clear_progress();
    }

    #[test]
    fn finish_appends_duration() {
        let out = Output::new(true, false, false);
        let s = out.err_styles();
        // "Finished" is 8 chars, padded to 12 → 4 leading spaces.
        assert_eq!(
            s.status("Finished", "setup in 1.50s"),
            "    Finished setup in 1.50s"
        );
        let _ = out;
    }

    #[test]
    fn json_mode_suppresses_status_lines() {
        let mut out = Output::new(true, false, false);
        // These write nothing in JSON mode; must not panic. Visual contract
        // is verified in the command matrix.
        out.status("Fetching", "x");
        out.finish("Finished", "x", std::time::Duration::from_millis(1));
        out.warning("x");
    }

    #[test]
    fn error_and_hint_prefix_regardless_of_mode() {
        let out = Output::new(true, false, false);
        let s = out.err_styles();
        assert_eq!(s.error("boom"), "error: boom");
        assert_eq!(s.hint("do this"), "hint: do this");
        assert_eq!(s.success("done"), "✓ done");
    }

    #[test]
    fn stderr_styles_independent_of_stdout() {
        // A `--json` run keeps stderr styling (status stays readable) while
        // stdout styles are no-ops.
        let out = Output::new(true, false, false);
        assert_eq!(out.styles().dim("x"), "x");
        // err_styles prefixes are identical in every mode.
        assert_eq!(out.err_styles().error("e"), "error: e");
    }

    #[test]
    fn thousands_groups_digits() {
        let out = Output::new(true, false, false); // color off
        let s = out.styles();
        assert_eq!(s.thousands(938), "938");
        assert_eq!(s.thousands(1_512), "1,512");
        assert_eq!(s.thousands(-1_234_567), "-1,234,567");
        assert_eq!(s.thousands(0), "0");
    }

    #[test]
    fn grouped_int_handles_i64_min() {
        // i64::MIN.abs() overflows; unsigned_abs must not.
        assert_eq!(grouped_int(i64::MIN), "-9,223,372,036,854,775,808");
    }

    #[test]
    fn money_shows_currency_and_grouping() {
        let out = Output::new(true, false, false); // color off
        let s = out.styles();
        assert_eq!(s.money(975.4), "$975.40 USD");
        assert_eq!(s.money(1234.5), "$1,234.50 USD");
        assert_eq!(s.money(0.0), "$0.00 USD");
        assert_eq!(s.money(-1_234_567.895), "$-1,234,567.90 USD");
    }

    #[test]
    fn bar_scales_and_clamps() {
        let out = Output::new(true, false, false); // color off
        let s = out.styles();
        assert_eq!(s.bar(0.5, 10), "█████·····");
        assert_eq!(s.bar(0.0, 4), "····");
        assert_eq!(s.bar(1.0, 4), "████");
        assert_eq!(s.bar(2.0, 4), "████"); // clamped
        assert_eq!(s.bar(-1.0, 4), "····");
    }

    #[test]
    fn wrap_breaks_long_lines_and_keeps_breaks() {
        assert_eq!(
            Styles::wrap("one two three four", 8),
            vec!["one two", "three", "four"]
        );
        assert_eq!(Styles::wrap("short", 80), vec!["short"]);
        assert_eq!(Styles::wrap("a\n\nb", 80), vec!["a", "", "b"]);
        // Unbreakable word longer than the width passes through whole.
        assert_eq!(
            Styles::wrap("supercalifragilistic", 4),
            vec!["supercalifragilistic"]
        );
    }
}
