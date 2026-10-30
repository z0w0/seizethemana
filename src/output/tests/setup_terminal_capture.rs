//! Bounded PTY capture and cursor-aware screen checks for setup output tests.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Stop both script and its fixture child if the capture fails or times out.
struct TerminalProcess {
    child: Child,
    fixture_pid: PathBuf,
}

impl TerminalProcess {
    /// Wait without allowing a broken terminal fixture to hang the test suite.
    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().expect("terminal child status") {
                return status;
            }
            assert!(Instant::now() < deadline, "setup PTY fixture timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for TerminalProcess {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        if let Ok(pid) = fs::read_to_string(&self.fixture_pid)
            && let Ok(pid) = pid.trim().parse::<u32>()
        {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Raw terminal bytes and the visible screen at each fixture checkpoint.
pub(super) struct TerminalCapture {
    raw: String,
    pub(super) width: usize,
    frames: BTreeMap<String, Vec<String>>,
}

impl TerminalCapture {
    /// Re-enter the fixture through the native script utility with a real size.
    pub(super) fn run(width: usize, mode: &str, streams: &str) -> Self {
        let directory = tempfile::tempdir().expect("setup terminal directory");
        let wrapper = directory.path().join("setup terminal wrapper.sh");
        fs::write(
            &wrapper,
            r#"#!/bin/bash
set -eu
stty -echo rows 60 cols "$STM_SETUP_TEST_WIDTH"
printf '%s' "$$" > "$STM_SETUP_TEST_PID"
case "$STM_SETUP_TEST_STREAMS" in
    stdout-file) exec > "$STM_SETUP_TEST_REDIRECTED" ;;
    stderr-file) exec 2> "$STM_SETUP_TEST_REDIRECTED" ;;
esac
exec "$STM_SETUP_TEST_BINARY" --exact "$STM_SETUP_TEST_NAME" --nocapture --test-threads=1 </dev/null
"#,
        )
        .expect("write setup terminal wrapper");
        let terminal_path = directory.path().join("terminal.txt");
        let diagnostic_path = directory.path().join("script-stderr.txt");
        let redirected_path = directory.path().join("redirected.txt");
        let pid_path = directory.path().join("fixture.pid");
        let mut command = script_command(&wrapper);
        command
            .env("STM_SETUP_TEST_WIDTH", width.to_string())
            .env("STM_SETUP_TEST_MODE", mode)
            .env("STM_SETUP_TEST_STREAMS", streams)
            .env("STM_SETUP_TEST_REDIRECTED", &redirected_path)
            .env("STM_SETUP_TEST_PID", &pid_path)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("COLORFGBG", "15;0")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("FORCE_COLOR")
            .stdin(Stdio::piped())
            .stdout(File::create(&terminal_path).expect("terminal capture file"))
            .stderr(File::create(&diagnostic_path).expect("script diagnostic file"));
        let mut process = TerminalProcess {
            child: command.spawn().expect("native script utility"),
            fixture_pid: pid_path,
        };
        let status = process.wait();
        let terminal = fs::read_to_string(&terminal_path).expect("terminal capture");
        let diagnostics = fs::read_to_string(&diagnostic_path).expect("script diagnostics");
        assert!(
            status.success(),
            "{mode}, {streams}, {width}: {terminal}\n{diagnostics}"
        );
        assert!(diagnostics.is_empty(), "script diagnostics: {diagnostics}");
        let raw = if streams == "stderr-file" {
            fs::read_to_string(redirected_path).expect("redirected stderr")
        } else {
            terminal
        };
        let begin = raw
            .find("\x1b]777;STM_SETUP:begin\x07")
            .expect("fixture start");
        let end_marker = "\x1b]777;STM_SETUP:end\x07";
        let end = raw.find(end_marker).expect("fixture end") + end_marker.len();
        let raw = raw[begin..end].to_string();
        let frames = Screen::checkpoints(&raw, width);
        assert!(
            frames.contains_key("end"),
            "fixture did not finish: {raw:?}"
        );
        Self { raw, width, frames }
    }

    /// Read the visible screen, not a concatenation of erased progress frames.
    pub(super) fn frame(&self, name: &str) -> &[String] {
        self.frames
            .get(name)
            .unwrap_or_else(|| panic!("missing checkpoint {name}"))
    }

    /// Inspect a redraw without counting styles from later hints or test output.
    pub(super) fn between(&self, from: &str, to: &str) -> &str {
        self.raw
            .split_once(&format!("\x1b]777;STM_SETUP:{from}\x07"))
            .expect("interval start")
            .1
            .split_once(&format!("\x1b]777;STM_SETUP:{to}\x07"))
            .expect("interval end")
            .0
    }

    /// Separate color codes from cursor controls, which remain in no-color mode.
    pub(super) fn has_sgr(&self) -> bool {
        self.sgr_parameters().next().is_some()
    }

    /// Test semantic colors without relying on the order of SGR parameters.
    pub(super) fn has_sgr_parameter(&self, expected: &str) -> bool {
        self.sgr_parameters()
            .any(|parameters| parameters.split(';').any(|value| value == expected))
    }

    /// Return only the parameters of complete terminal style commands.
    fn sgr_parameters(&self) -> impl Iterator<Item = &str> {
        self.raw.split("\x1b[").skip(1).filter_map(|part| {
            let end = part.find(|ch: char| ('@'..='~').contains(&ch))?;
            (part.as_bytes()[end] == b'm').then_some(&part[..end])
        })
    }
}

/// Use the native script arguments while keeping one fixture command on both OSes.
fn script_command(wrapper: &std::path::Path) -> Command {
    let mut command = Command::new("/usr/bin/script");
    if cfg!(target_os = "macos") {
        command.args(["-q", "/dev/null", "/bin/bash"]).arg(wrapper);
    } else {
        command.args([
            "-q",
            "-e",
            "-c",
            "exec /bin/bash \"$STM_SETUP_TEST_WRAPPER\"",
            "/dev/null",
        ]);
    }
    let module = module_path!()
        .split_once("::")
        .expect("crate test module")
        .1
        .rsplit_once("::")
        .expect("capture helper module")
        .0;
    command
        .env(
            "STM_SETUP_TEST_BINARY",
            std::env::current_exe().expect("test binary"),
        )
        .env(
            "STM_SETUP_TEST_NAME",
            format!("{module}::setup_terminal_fixture"),
        )
        .env("STM_SETUP_TEST_WRAPPER", wrapper);
    command
}

/// Apply the cursor controls used by console and indicatif to visible rows.
#[derive(Default)]
struct Screen {
    rows: Vec<Vec<char>>,
    row: usize,
    column: usize,
    width: usize,
}

impl Screen {
    /// Checkpoint controls are invisible and must not change the rendered screen.
    fn checkpoints(raw: &str, width: usize) -> BTreeMap<String, Vec<String>> {
        let mut screen = Self {
            width,
            ..Self::default()
        };
        let mut frames = BTreeMap::new();
        let mut remaining = raw;
        while let Some((text, rest)) = remaining.split_once("\x1b]777;STM_SETUP:") {
            screen.write(text);
            let (name, rest) = rest.split_once('\x07').expect("complete checkpoint");
            if name == "begin" {
                screen = Self {
                    width,
                    ..Self::default()
                };
            } else {
                frames.insert(name.to_string(), screen.lines());
            }
            remaining = rest;
            if name == "end" {
                break;
            }
        }
        frames
    }

    /// Keep leading columns while dropping cleared rows at the screen bottom.
    fn lines(&self) -> Vec<String> {
        let mut lines: Vec<_> = self
            .rows
            .iter()
            .map(|row| row.iter().collect::<String>().trim_end().to_string())
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    }

    /// Interpret only the controls emitted by the production terminal renderer.
    fn write(&mut self, text: &str) {
        let mut chars = text.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '\x1b' => {
                    assert_eq!(chars.next(), Some('['), "unsupported terminal escape");
                    let mut parameters = String::new();
                    loop {
                        let command = chars.next().expect("complete terminal command");
                        if ('@'..='~').contains(&command) {
                            self.control(command, &parameters);
                            break;
                        }
                        parameters.push(command);
                    }
                }
                '\r' => self.column = 0,
                '\n' => {
                    self.row += 1;
                    self.column = 0;
                }
                '\x08' => self.column = self.column.saturating_sub(1),
                ch => {
                    assert!(
                        ch.is_ascii() && !ch.is_control(),
                        "unexpected fixture glyph {ch:?}"
                    );
                    if self.column == self.width {
                        self.row += 1;
                        self.column = 0;
                    }
                    self.rows
                        .resize_with(self.rows.len().max(self.row + 1), Vec::new);
                    let row = &mut self.rows[self.row];
                    row.resize(row.len().max(self.column + 1), ' ');
                    row[self.column] = ch;
                    self.column += 1;
                }
            }
        }
    }

    /// Apply cursor movement, line erasure, style changes, and cursor visibility.
    fn control(&mut self, command: char, parameters: &str) {
        if command == 'm' || matches!((command, parameters), ('h' | 'l', "?25")) {
            return;
        }
        let amount = if parameters.is_empty() {
            1
        } else {
            parameters
                .parse::<usize>()
                .expect("numeric cursor distance")
                .max(1)
        };
        match command {
            'A' => self.row = self.row.saturating_sub(amount),
            'B' => self.row += amount,
            'C' => self.column += amount,
            'D' => self.column = self.column.saturating_sub(amount),
            'G' => self.column = amount - 1,
            'K' => {
                if let Some(row) = self.rows.get_mut(self.row) {
                    match parameters {
                        "2" => row.clear(),
                        "" | "0" => row.truncate(self.column),
                        _ => panic!("unsupported line erase {parameters:?}"),
                    }
                }
            }
            _ => panic!("unsupported cursor command {command:?}, {parameters:?}"),
        }
    }
}

/// Full-width padding wraps, while cursor erasure preserves completed rows above.
#[test]
fn screen_retains_completed_lines_when_progress_clears() {
    let mut screen = Screen {
        width: 32,
        ..Screen::default()
    };
    screen.write("  Downloaded card data (512 B)\r\n");
    screen.write(&format!(
        "{:32}{:32}{:32}",
        "    Building search index", "  [##>-------]", "  4/10"
    ));
    assert_eq!(screen.lines().len(), 4);
    screen.write("\x1b[2A\r\x1b[2K\x1b[1B\r\x1b[2K\x1b[1B\r\x1b[2K\x1b[2A\r");
    screen.write("       Built search index\r\n");
    assert_eq!(
        screen.lines(),
        [
            "  Downloaded card data (512 B)",
            "       Built search index"
        ]
    );
}
