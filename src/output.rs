//! Result text on stdout and diagnostics on stderr, with optional terminal styles.

use owo_colors::OwoColorize;
use std::io::IsTerminal;

/// Shared terminal layouts for task activity, item counts, and download bytes.
mod progress;
use progress::ProgressUnits;
pub(crate) use progress::byte_count;

#[cfg(test)]
use progress::{progress_style, progress_template};

/// Available stdout columns, or 80 when the terminal size is unknown.
pub fn terminal_width() -> usize {
    console::Term::stdout()
        .size_checked()
        .map(|(_, w)| w as usize)
        .unwrap_or(80)
        .max(1)
}

/// Print human text at the available stdout width without clipping long words.
pub fn print_wrapped(text: &str) {
    for line in Styles::wrap(text, terminal_width()) {
        println!("{line}");
    }
}

/// Currency for prices supplied by Scryfall.
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

/// Stream and style policy for human text and JSON results.
///
/// Invariants (see docs/design.md):
///
/// - stdout carries results only; every status/progress line goes to stderr.
/// - JSON mode suppresses status and progress. Errors remain plain on stderr.
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
    /// RGB accent for a known background, or terminal-palette magenta.
    accent: Option<(u8, u8, u8)>,
    /// Stderr supports cursor-based progress.
    interactive: bool,
    /// Show progress detail on stderr (from `--verbose`).
    pub verbose: bool,
    /// Active ephemeral progress line on stderr, if any.
    progress: Option<indicatif::ProgressBar>,
    /// Unit used when an unknown progress total becomes known.
    progress_units: ProgressUnits,
}

/// Shared styling decisions derived from [`Output`].
///
/// Every method returns a plain string when color is disabled, so callers can
/// wrap text unconditionally.
#[derive(Debug, Clone, Copy)]
pub struct Styles {
    color: bool,
    accent: Option<(u8, u8, u8)>,
}

/// Select the shared output/help accent from declared terminal capabilities.
pub(crate) fn accent_rgb() -> Option<(u8, u8, u8)> {
    terminal_accent_rgb(
        std::env::var("COLORTERM").ok().as_deref(),
        std::env::var("COLORFGBG").ok().as_deref(),
    )
}

/// Use RGB only for known standard backgrounds; leave other palettes in control.
fn terminal_accent_rgb(colorterm: Option<&str>, colorfgbg: Option<&str>) -> Option<(u8, u8, u8)> {
    if !matches!(colorterm, Some("truecolor" | "24bit")) {
        return None;
    }
    let (prefix, background) = colorfgbg?.rsplit_once(';')?;
    if !prefix
        .split(';')
        .all(|field| field == "default" || field.parse::<u8>().is_ok())
    {
        return None;
    }
    match background {
        "0" => Some((255, 92, 154)),
        "7" | "15" => Some((151, 23, 77)),
        _ => None,
    }
}

/// Color and cursor policy, kept separate from environment access for tests.
fn stream_policy(json: bool, no_color: bool, terminal: bool, dumb: bool) -> (bool, bool) {
    let interactive = !json && terminal && !dumb;
    (interactive && !no_color, interactive)
}

/// Verb column width for cargo-style status lines (`   Compiling serde ...`).
const VERB_WIDTH: usize = 12;

/// Color family for [`Styles::glyph`].
#[derive(Debug, Clone, Copy)]
pub enum GlyphKind {
    /// Positive result.
    Good,
    /// Caution.
    Warn,
    /// Negative result.
    Bad,
    /// De-emphasized text.
    Dim,
    /// Advisory note (bracket judgment calls): informational, never a
    /// violation or a genuine conflict.
    Info,
}

impl Output {
    /// Build the output mode from global flags.
    pub fn new(json: bool, no_color: bool, verbose: bool) -> Self {
        let no_color = no_color || std::env::var_os("NO_COLOR").is_some();
        let dumb = std::env::var_os("TERM").is_some_and(|term| term == "dumb");
        let accent = accent_rgb();
        let (color, _) = stream_policy(json, no_color, std::io::stdout().is_terminal(), dumb);
        let (err_color, interactive) =
            stream_policy(json, no_color, std::io::stderr().is_terminal(), dumb);
        Self {
            json,
            color,
            err_color,
            accent,
            interactive,
            verbose,
            progress: None,
            progress_units: ProgressUnits::Items,
        }
    }

    /// Styling helpers for human output. In JSON mode returns no-op styles.
    pub fn styles(&self) -> Styles {
        Styles {
            color: self.color,
            accent: self.accent,
        }
    }

    /// Styling helpers for stderr streams (status, errors, hints).
    pub fn err_styles(&self) -> Styles {
        Styles {
            color: self.err_color,
            accent: self.accent,
        }
    }

    /// Print wrapped human results without clipping names, paths, or values.
    pub fn print_wrapped(&self, text: &str) {
        if !self.json {
            print_wrapped(text);
        }
    }

    /// Allow a prompt only when all required streams support terminal input.
    pub fn can_prompt(&self) -> bool {
        self.interactive && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }

    /// Whether stderr can draw cursor-based progress in the active output mode.
    pub(crate) fn supports_progress(&self) -> bool {
        self.interactive
    }

    /// Copy task output only when it cannot stop or share another active bar.
    pub(crate) fn task_output(&self) -> Option<Self> {
        if self.progress.is_some() {
            None
        } else {
            Some(self.clone())
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

    /// Cargo-style warning line on stderr: `warning: ...` (yellow).
    ///
    /// Cargo emits diagnostics unpadded (unlike status verbs), so no indent.
    pub fn warning(&self, msg: &str) {
        if self.json {
            return;
        }
        self.print_diagnostic(&self.err_styles().warning(msg));
    }

    /// Stop active progress and print an error. JSON errors stay plain.
    pub fn error(&self, msg: &str) {
        if let Some(bar) = &self.progress {
            bar.finish_and_clear();
        }
        eprintln!("{}", self.err_styles().error(msg));
    }

    /// Print recovery instructions to stderr in every mode.
    pub fn hint(&self, msg: &str) {
        self.print_diagnostic(&self.err_styles().hint(msg));
    }

    /// Keep diagnostics separate from the active progress line.
    fn print_diagnostic(&self, text: &str) {
        if let Some(bar) = &self.progress {
            bar.suspend(|| eprintln!("{text}"));
        } else {
            eprintln!("{text}");
        }
    }

    /// Note line on stdout: `note: ...` (dim), like the `deck legal`
    /// checklist. Suppressed in JSON mode.
    pub fn print_note(&mut self, msg: &str) {
        self.clear_progress();
        if self.json {
            return;
        }
        self.print_wrapped(&self.styles().note(msg));
    }
}

impl Styles {
    /// Plain style set (no color), for tests and plain-text contexts.
    pub fn colorless() -> Self {
        Self {
            color: false,
            accent: None,
        }
    }

    /// Use background-aware pink or terminal-palette magenta.
    fn accent(&self, text: &str) -> String {
        if !self.color {
            text.to_string()
        } else if let Some((red, green, blue)) = self.accent {
            text.truecolor(red, green, blue).bold().to_string()
        } else {
            text.magenta().bold().to_string()
        }
    }

    /// Card name in the shared pink accent.
    pub fn card_name(&self, s: &str) -> String {
        self.accent(s)
    }

    /// Section heading in the shared pink accent.
    pub fn header(&self, s: &str) -> String {
        self.accent(s)
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

    /// Recovery instructions on stderr, with an accented label only.
    pub fn hint(&self, s: &str) -> String {
        format!("{} {s}", self.accent("hint:"))
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
    /// Measure display columns, not bytes. Keep unbroken names and paths whole,
    /// even when one word exceeds the width. Preserve lines that already fit.
    pub fn wrap(text: &str, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            if console::measure_text_width(paragraph) <= width.max(1) {
                lines.push(paragraph.to_string());
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

/// Offline checks for progress layout and diagnostic cleanup.
#[cfg(test)]
#[path = "output/tests/progress_tests.rs"]
mod progress_tests;

/// Real-terminal coverage for setup transitions, color, and redirected streams.
#[cfg(test)]
#[path = "output/tests/setup_terminal_tests.rs"]
mod setup_terminal_tests;

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
    fn progress_bar_is_suppressed_in_json_mode() {
        // JSON mode emits only the command's JSON result, so a progress
        // bar must not be created or drawn.
        let mut out = Output::new(true, false, false);
        out.progress_bar("Embedding", "cards", 10);
        assert!(out.progress.is_none(), "no bar in JSON mode");
    }

    #[test]
    fn progress_templates_keep_the_label_and_use_the_stream_color_policy() {
        for total in [0, 10] {
            for color in [false, true] {
                let styles = Styles {
                    color,
                    accent: None,
                };
                let template = progress_template(total, ProgressUnits::Items, 80, &styles);
                assert!(template.starts_with("{msg}"), "{template}");
                assert_eq!(template.contains('\u{1b}'), color && total > 0);
                assert!(template.contains('\n'), "activity is below the task label");
                indicatif::ProgressStyle::with_template(&template).expect("valid progress shape");
            }
        }
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
        // JSON errors stay plain even when stderr is a terminal.
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

    /// Every stream uses its own terminal state, with shared mode overrides.
    #[test]
    fn stream_policy_covers_redirection_and_color_overrides() {
        for stdout in [false, true] {
            for stderr in [false, true] {
                assert_eq!(stream_policy(false, false, stdout, false), (stdout, stdout));
                assert_eq!(stream_policy(false, false, stderr, false), (stderr, stderr));
                for terminal in [stdout, stderr] {
                    assert_eq!(stream_policy(true, false, terminal, false), (false, false));
                    assert_eq!(stream_policy(false, false, terminal, true), (false, false));
                    assert_eq!(
                        stream_policy(false, true, terminal, false),
                        (false, terminal)
                    );
                }
            }
        }
    }

    /// Accent color follows declared capability without changing semantic colors.
    #[test]
    fn accent_has_truecolor_and_ansi_magenta_forms() {
        let basic = Styles {
            color: true,
            accent: None,
        };
        let full = Styles {
            color: true,
            accent: Some((255, 92, 154)),
        };
        assert!(basic.header("Deck").contains("35m"));
        assert!(full.header("Deck").contains("38;2;255;92;154m"));
        assert_eq!(console::strip_ansi_codes(&full.card_name("漢字")), "漢字");
        assert_eq!(basic.error("failed"), full.error("failed"));
        assert_eq!(basic.mana_pips("WUBRGC"), full.mana_pips("WUBRGC"));
        let light = Styles {
            color: true,
            accent: Some((151, 23, 77)),
        };
        assert!(light.header("Deck").contains("38;2;151;23;77m"));
        assert_eq!(light.error("failed"), full.error("failed"));
        assert_eq!(light.mana_pips("WUBRGC"), full.mana_pips("WUBRGC"));
        assert_eq!(
            light.status("Testing", "cards"),
            full.status("Testing", "cards")
        );
        assert_eq!(light.warning("check"), full.warning("check"));
    }

    /// RGB requires both truecolor support and a known standard background.
    #[test]
    fn accent_uses_only_known_standard_backgrounds() {
        for capability in ["truecolor", "24bit"] {
            assert_eq!(
                terminal_accent_rgb(Some(capability), Some("15;0")),
                Some((255, 92, 154))
            );
            for background in ["0;7", "0;15", "0;default;15"] {
                assert_eq!(
                    terminal_accent_rgb(Some(capability), Some(background)),
                    Some((151, 23, 77))
                );
            }
            for background in [
                None,
                Some(""),
                Some("0"),
                Some("0;8"),
                Some("0;16"),
                Some("0;unknown"),
                Some("garbage;0"),
                Some("0;;15"),
            ] {
                assert_eq!(terminal_accent_rgb(Some(capability), background), None);
            }
        }
        for capability in [None, Some(""), Some("256color")] {
            for background in ["15;0", "0;7", "0;15"] {
                assert_eq!(terminal_accent_rgb(capability, Some(background)), None);
            }
        }
    }

    /// Compute WCAG relative luminance for the fixed RGB contrast checks.
    fn luminance((red, green, blue): (u8, u8, u8)) -> f64 {
        let linear = |value| {
            let channel = f64::from(value) / 255.0;
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
    }

    /// The light accent meets normal-text contrast on standard white and silver.
    #[test]
    fn known_background_accents_meet_normal_text_contrast() {
        let light = terminal_accent_rgb(Some("truecolor"), Some("0;15")).expect("light RGB");
        let dark = terminal_accent_rgb(Some("truecolor"), Some("15;0")).expect("dark RGB");
        for background in [(255, 255, 255), (192, 192, 192)] {
            let contrast = (luminance(background) + 0.05) / (luminance(light) + 0.05);
            assert!(contrast >= 4.5, "light contrast {contrast}");
        }
        for background in [(0, 0, 0), (33, 24, 32)] {
            let contrast = (luminance(dark) + 0.05) / (luminance(background) + 0.05);
            assert!(contrast >= 4.5, "dark contrast {contrast}");
        }
    }

    /// Wrapping preserves Unicode clusters and leaves long paths available to copy.
    #[test]
    fn wrapping_covers_narrow_widths_unicode_and_paths() {
        let text = "Jötun 漢字 e\u{301} Name with a second face and a price $1,234.50 USD";
        let path = "/a/very/long/unbroken/path/to/a/decklist/that/must/not/be/clipped.txt";
        for width in [32, 40, 60, 80, 120] {
            let lines = Styles::wrap(text, width);
            assert!(
                lines
                    .iter()
                    .all(|line| console::measure_text_width(line) <= width)
            );
            assert_eq!(lines.join(" "), text);
            assert_eq!(Styles::wrap(path, width).join(""), path);
            assert_eq!(
                Styles::wrap("  Short    aligned row", width),
                ["  Short    aligned row"]
            );
        }
    }

    /// Exercise the real progress renderer without downloads or model loading.
    #[test]
    fn progress_bar_uses_terminal_capabilities() {
        let mut out = Output::new(false, false, false);
        let interactive = out.interactive;
        out.progress_bar("Testing", "synthetic cards", 10);
        assert_eq!(out.progress.is_some(), interactive);
        out.tick_progress(2);
        out.set_progress_position(5);
        out.set_progress_total(12);
        if let Some(bar) = &out.progress {
            bar.force_draw();
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        out.clear_progress();
        assert!(out.progress.is_none());
        out.finish("Tested", "12 synthetic cards", std::time::Duration::ZERO);
    }
}
