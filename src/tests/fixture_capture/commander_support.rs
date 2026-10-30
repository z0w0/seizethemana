use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::super::process::CaptureChild;
use super::super::support;

const FIXTURES: &str = "src/deck/simulator/tests/commander_card_fixtures";

const CURL_STUB: &str = r#"#!/bin/bash
set -euo pipefail
work=$COMMANDER_FIXTURE_WORK
index=$(cat "$work/next")
printf '%s\n' "$((index + 1))" >"$work/next"
printf -v number '%02d' "$index"
printf '%s\0' "$@" >"$work/calls/$number.args"
printf 'request\n' >>"$work/events"
output= request=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output=$2; shift ;;
    --data-binary) request=${2#@}; shift ;;
  esac
  shift
done
[[ -n "$output" ]]
if [[ -n "$request" ]]; then cp -- "$request" "$work/calls/$number.request"; fi
cp -- "$work/plan/$number.body" "$output"
exit "$(cat "$work/plan/$number.status")"
"#;

const SLEEP_STUB: &str = r#"#!/bin/bash
set -euo pipefail
printf '%s\n' "$@" >>"$COMMANDER_FIXTURE_WORK/sleeps"
printf 'sleep\n' >>"$COMMANDER_FIXTURE_WORK/events"
"#;

const SHASUM_STUB: &str = r#"#!/bin/bash
set -euo pipefail
for last in "$@"; do :; done
if [[ "$COMMANDER_FIXTURE_HASH_FAILURE" == 1 && "$last" == */cards.json ]]; then
  exit 73
fi
exec "$COMMANDER_FIXTURE_SHASUM" "$@"
"#;

const GIT_STUB: &str = r#"#!/bin/bash
set -euo pipefail
printf '%s\n' "$COMMANDER_FIXTURE_ROOT"
"#;

/// Select the pinned source bundle for a capture mode.
#[derive(Clone, Copy)]
pub(super) enum CommanderMode {
    Popular,
    Behold,
}

impl CommanderMode {
    /// Locate the read-only sources for this capture mode.
    fn sources(self) -> PathBuf {
        let directory = match self {
            Self::Popular => "2026-10-30",
            Self::Behold => "2026-10-30-behold",
        };
        support::root()
            .join(FIXTURES)
            .join("sources")
            .join(directory)
    }
}

/// Keep replay files and command faults separate from other test processes.
pub(super) struct CommanderWorkspace {
    _temporary: TempDir,
    work: PathBuf,
    mode: CommanderMode,
    sources: PathBuf,
    records: Vec<Value>,
    source_hashes: BTreeMap<PathBuf, String>,
    shasum: PathBuf,
    hash_failure: bool,
    repository: Option<PathBuf>,
}

impl CommanderWorkspace {
    /// Prepare local responses without writing to the pinned source bundle.
    pub(super) fn new(mode: CommanderMode) -> Self {
        let source_hashes = commander_source_hashes();
        let temporary = tempfile::tempdir().expect("create commander replay directory");
        let work = temporary
            .path()
            .canonicalize()
            .expect("resolve replay path");
        for directory in ["bin", "plan", "calls"] {
            fs::create_dir(work.join(directory)).expect("create replay subdirectory");
        }
        for file in ["events", "sleeps"] {
            fs::write(work.join(file), "").expect("create replay log");
        }
        fs::write(work.join("next"), "0\n").expect("create replay counter");
        support::write_stub(&work.join("bin/curl"), CURL_STUB);
        support::write_stub(&work.join("bin/sleep"), SLEEP_STUB);
        support::write_stub(&work.join("bin/shasum"), SHASUM_STUB);
        let sources = mode.sources();
        let manifest = commander_read_json(&sources.join("capture.json"));
        let records = manifest["sources"]
            .as_array()
            .expect("source records")
            .clone();
        for (index, record) in records.iter().enumerate() {
            let name = record["path"].as_str().expect("source response path");
            fs::copy(
                sources.join(name),
                work.join(format!("plan/{index:02}.body")),
            )
            .expect("copy response into replay plan");
            fs::write(work.join(format!("plan/{index:02}.status")), "0\n")
                .expect("write replay status");
        }
        Self {
            _temporary: temporary,
            work,
            mode,
            sources,
            records,
            source_hashes,
            shasum: commander_find_program("shasum"),
            hash_failure: false,
            repository: None,
        }
    }

    /// Locate a file that belongs only to this replay.
    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.work.join(name)
    }

    /// Select the maintenance command's mode argument.
    pub(super) fn input(&self) -> PathBuf {
        match self.mode {
            CommanderMode::Popular => support::root().join(FIXTURES).join("land_cycles.json"),
            CommanderMode::Behold => PathBuf::from("--behold"),
        }
    }

    /// Replace a raw response in the test-owned replay plan.
    pub(super) fn replace_response(&self, name: &str, bytes: &[u8]) {
        fs::write(self.response_path(name), bytes).expect("replace local replay response");
    }

    /// Apply a JSON fault to a local copy rather than to a pinned response.
    pub(super) fn mutate_response(&self, name: &str, mutate: impl FnOnce(&mut Value)) {
        let path = self.response_path(name);
        let mut response = commander_read_json(&path);
        mutate(&mut response);
        commander_write_json(&path, &response);
    }

    /// Find the local response for a named source record.
    fn response_path(&self, name: &str) -> PathBuf {
        let index = self
            .records
            .iter()
            .position(|record| record["path"] == name)
            .expect("response is present in the replay plan");
        self.path(&format!("plan/{index:02}.body"))
    }

    /// Return a partial first response with curl's transport failure status.
    pub(super) fn fail_transport(&self) {
        fs::write(self.path("plan/00.body"), "partial response\n").expect("write partial response");
        fs::write(self.path("plan/00.status"), "22\n").expect("write transport failure status");
    }

    /// Fail only the generated cards file hash after all requests finish.
    pub(super) fn fail_card_hash(&mut self) {
        self.hash_failure = true;
    }

    /// Put relative destination tests in a repository root owned by the test.
    pub(super) fn isolate_repository(&mut self) -> PathBuf {
        let repository = self.path("repository");
        fs::create_dir(&repository).expect("create isolated repository root");
        support::write_stub(&self.path("bin/git"), GIT_STUB);
        self.repository = Some(repository.clone());
        repository
    }

    /// Run the retained maintenance script with faults scoped to this command.
    pub(super) fn capture(&self, input: &Path, destination: &Path) -> Output {
        let mut command = support::script_command(
            "capture-commander-fixtures.sh",
            &self.work,
            &self.path("bin"),
        );
        command
            .arg(input)
            .arg(destination)
            .env("COMMANDER_FIXTURE_WORK", &self.work)
            .env("COMMANDER_FIXTURE_SHASUM", &self.shasum)
            .env(
                "COMMANDER_FIXTURE_HASH_FAILURE",
                if self.hash_failure { "1" } else { "0" },
            );
        if let Some(repository) = &self.repository {
            command.env("COMMANDER_FIXTURE_ROOT", repository);
        }
        let output = CaptureChild::spawn(command, &self.work, "capture").wait();
        assert_eq!(
            self.source_hashes,
            commander_source_hashes(),
            "pinned source bytes changed"
        );
        output
    }

    /// Require a complete capture with exact source bytes and valid manifests.
    pub(super) fn assert_success(&self, destination: &Path) {
        let output = self.capture(&self.input(), destination);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.assert_requests(self.records.len());
        self.assert_bundle(&self.resolve_destination(destination));
    }

    /// Require refusal, complete cleanup, and the expected request boundary.
    pub(super) fn assert_failure(
        &self,
        input: &Path,
        destination: &Path,
        requests: usize,
    ) -> Output {
        let output = self.capture(input, destination);
        assert!(!output.status.success(), "capture unexpectedly succeeded");
        commander_assert_absent(&self.resolve_destination(destination));
        self.assert_requests(requests);
        output
    }

    /// Resolve relative paths as the maintenance script does after its root change.
    fn resolve_destination(&self, destination: &Path) -> PathBuf {
        if destination.is_absolute() {
            return destination.to_path_buf();
        }
        let root = self
            .repository
            .as_deref()
            .unwrap_or_else(|| support::root());
        root.join(destination)
    }

    /// Check request order, exact request bodies, batch limits, and pacing.
    pub(super) fn assert_requests(&self, count: usize) {
        let next = fs::read_to_string(self.path("next")).expect("read replay counter");
        assert_eq!(
            next.trim()
                .parse::<usize>()
                .expect("numeric replay counter"),
            count
        );
        assert!(count <= self.records.len(), "unexpected extra request");
        let sleeps = fs::read_to_string(self.path("sleeps")).expect("read pacing log");
        assert_eq!(sleeps, "0.15\n".repeat(count));
        let events = fs::read_to_string(self.path("events")).expect("read event log");
        assert_eq!(events, "sleep\nrequest\n".repeat(count));
        for (index, record) in self.records.iter().take(count).enumerate() {
            self.assert_request(index, record);
        }
    }

    /// Check curl's full argument list and preserve each submitted request body.
    fn assert_request(&self, index: usize, record: &Value) {
        let bytes = fs::read(self.path(&format!("calls/{index:02}.args")))
            .expect("read recorded curl arguments");
        let arguments: Vec<&str> = bytes
            .split(|byte| *byte == 0)
            .filter(|argument| !argument.is_empty())
            .map(|argument| std::str::from_utf8(argument).expect("UTF-8 curl argument"))
            .collect();
        let mut expected = vec![
            "--fail-with-body",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "30",
            "--max-time",
            "120",
            "--user-agent",
            "seizethemana-fixture-maintenance/0.1",
            "--header",
            "Accept: application/json",
        ];
        if let Some(request) = record["request_path"].as_str() {
            let request_argument = commander_argument(&arguments, "--data-binary");
            let request_path = request_argument
                .strip_prefix('@')
                .expect("request file argument");
            assert_eq!(
                Path::new(request_path)
                    .file_name()
                    .expect("request file name"),
                request
            );
            expected.extend([
                "--header",
                "Content-Type: application/json",
                "--data-binary",
                request_argument,
            ]);
            let request_bytes = fs::read(self.path(&format!("calls/{index:02}.request")))
                .expect("read submitted request bytes");
            assert_eq!(
                request_bytes,
                fs::read(self.sources.join(request)).expect("read pinned request")
            );
            let body: Value = serde_json::from_slice(&request_bytes).expect("valid request JSON");
            let identifiers = body["identifiers"].as_array().expect("request identifiers");
            assert!(
                (1..=75).contains(&identifiers.len()),
                "invalid collection batch size"
            );
        }
        let output = commander_argument(&arguments, "--output");
        assert_eq!(
            Path::new(output).file_name().expect("response file name"),
            record["path"].as_str().expect("source path")
        );
        expected.extend([
            "--output",
            output,
            record["url"].as_str().expect("source URL"),
        ]);
        assert_eq!(arguments, expected, "curl arguments for request {index}");
    }

    /// Compare generated files and every manifest hash with the pinned bundle.
    pub(super) fn assert_bundle(&self, directory: &Path) {
        let expected_files = commander_file_names(&self.sources);
        assert_eq!(commander_file_names(directory), expected_files);
        for name in expected_files
            .iter()
            .filter(|name| name.as_str() != "capture.json")
        {
            assert_eq!(
                fs::read(directory.join(name)).expect("read captured file"),
                fs::read(self.sources.join(name)).expect("read pinned file"),
                "captured bytes differ for {name}"
            );
        }
        let manifest = commander_read_json(&directory.join("capture.json"));
        for field in ["files", "sources"] {
            for record in manifest[field].as_array().expect("manifest records") {
                let path = record["path"].as_str().expect("manifest file path");
                assert_eq!(
                    Value::String(commander_hash(&directory.join(path))),
                    record["raw_bytes_sha256"]
                );
                if let Some(request) = record["request_path"].as_str() {
                    assert_eq!(
                        Value::String(commander_hash(&directory.join(request))),
                        record["request_sha256"]
                    );
                }
            }
        }
        let mut expected = commander_read_json(&self.sources.join("capture.json"));
        let mut actual = manifest;
        commander_remove_timestamps(&mut expected);
        commander_remove_timestamps(&mut actual);
        assert_eq!(actual, expected, "capture manifest metadata");
    }
}

/// Read a JSON fixture or a test-owned override.
pub(super) fn commander_read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read JSON file")).expect("parse JSON file")
}

/// Write JSON only to a path owned by the current test.
pub(super) fn commander_write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec(value).expect("encode JSON override"),
    )
    .expect("write JSON override");
}

/// Require cleanup to remove both files and dangling symlinks.
pub(super) fn commander_assert_absent(path: &Path) {
    let error = fs::symlink_metadata(path).expect_err("failed capture still exists");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

/// Read a required option from the recorded curl argument list.
fn commander_argument<'a>(arguments: &[&'a str], option: &str) -> &'a str {
    arguments
        .windows(2)
        .find(|pair| pair[0] == option)
        .expect("required curl option")[1]
}

/// Find the real hash command before the test prepends its shim directory.
fn commander_find_program(name: &str) -> PathBuf {
    let path = env::var_os("PATH").expect("PATH is available");
    env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|path| {
            fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
        .expect("shasum must be installed for fixture capture tests")
}

/// Hash raw bytes without relying on the command being tested.
fn commander_hash(path: &Path) -> String {
    format!(
        "{:x}",
        Sha256::digest(fs::read(path).expect("read file for byte hash"))
    )
}

/// Include both source bundles and the land selection in the immutability check.
fn commander_source_hashes() -> BTreeMap<PathBuf, String> {
    let mut hashes = BTreeMap::new();
    for mode in [CommanderMode::Popular, CommanderMode::Behold] {
        commander_hash_tree(&mode.sources(), &mut hashes);
    }
    let lands = support::root().join(FIXTURES).join("land_cycles.json");
    hashes.insert(lands.clone(), commander_hash(&lands));
    hashes
}

/// Include nested files so added or removed pinned files cannot go unnoticed.
fn commander_hash_tree(directory: &Path, hashes: &mut BTreeMap<PathBuf, String>) {
    for entry in fs::read_dir(directory).expect("read pinned source directory") {
        let entry = entry.expect("read pinned source entry");
        let path = entry.path();
        if entry.file_type().expect("read pinned source type").is_dir() {
            commander_hash_tree(&path, hashes);
        } else {
            hashes.insert(path.clone(), commander_hash(&path));
        }
    }
}

/// List bundle files to catch leftovers from incomplete capture work.
fn commander_file_names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("read capture directory")
        .map(|entry| {
            entry
                .expect("capture directory entry")
                .file_name()
                .into_string()
                .expect("UTF-8 capture file name")
        })
        .collect();
    names.sort();
    names
}

/// Ignore only the capture times when comparing complete manifest content.
fn commander_remove_timestamps(value: &mut Value) {
    let object = value.as_object_mut().expect("capture manifest object");
    for field in ["started_at", "finished_at"] {
        let timestamp = object.remove(field).expect("capture timestamp");
        assert!(!timestamp.as_str().expect("timestamp string").is_empty());
    }
    for record in object
        .get_mut("sources")
        .expect("manifest sources")
        .as_array_mut()
        .expect("source records")
    {
        let timestamp = record
            .as_object_mut()
            .expect("source record")
            .remove("captured_at")
            .expect("source timestamp");
        assert!(!timestamp.as_str().expect("timestamp string").is_empty());
    }
}

/// Create a test-owned symlink without passing a path through a shell.
pub(super) fn commander_symlink(target: &Path, link: &Path) {
    symlink(target, link).expect("create test symlink");
}
