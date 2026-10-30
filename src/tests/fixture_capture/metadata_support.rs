//! Local response replay and byte checks for metadata capture tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::super::process::CaptureChild;
use super::support;

/// Replaces curl with one local response and records every argument.
const CURL_STUB: &str = r#"#!/bin/bash
set -euo pipefail
printf '%s\n' CALL "$@" >>"$METADATA_CAPTURE_CALL"
output=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output=$2; shift ;;
  esac
  shift
done
cp -- "$METADATA_CAPTURE_RESPONSE" "$output"
exit "$METADATA_CAPTURE_STATUS"
"#;

/// Holds all mutable test files outside the repository.
pub(super) struct MetadataCapture {
    _temp: TempDir,
    pub(super) work: PathBuf,
    bin: PathBuf,
    pinned: BTreeMap<PathBuf, String>,
}

impl MetadataCapture {
    /// Creates an isolated capture command that cannot use the network.
    pub(super) fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("stm-metadata-capture-")
            .tempdir()
            .expect("create metadata capture scratch directory");
        let work = temp
            .path()
            .canonicalize()
            .expect("resolve scratch directory without symlink ancestors");
        let bin = work.join("bin");
        fs::create_dir(&bin).expect("create command stub directory");
        support::write_stub(&bin.join("curl"), CURL_STUB);
        Self {
            _temp: temp,
            work,
            bin,
            pinned: pinned_hashes(),
        }
    }

    /// Saves input or response bytes without changing their formatting.
    pub(super) fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.work.join(name);
        fs::write(&path, bytes).expect("write metadata capture test input");
        path
    }

    /// Prepares JSON in Rust rather than in command stubs.
    pub(super) fn write_json(&self, name: &str, value: &Value) -> PathBuf {
        self.write(
            name,
            serde_json::to_vec_pretty(value).expect("encode metadata capture test input"),
        )
    }

    /// Builds a capture process with only child-local environment changes.
    pub(super) fn command(
        &self,
        names: &Path,
        destination: &Path,
        response: &Path,
        job: &str,
        curl_status: i32,
    ) -> Command {
        let mut command =
            support::script_command("capture-fixture-metadata.sh", &self.work, &self.bin);
        command
            .arg(names)
            .arg(destination)
            .env("METADATA_CAPTURE_RESPONSE", response)
            .env("METADATA_CAPTURE_CALL", self.call_path(job))
            .env("METADATA_CAPTURE_STATUS", curl_status.to_string());
        command
    }

    /// Runs a capture with a bounded wait and checks pinned source bytes.
    pub(super) fn run(&self, command: Command, job: &str) -> Output {
        let output = CaptureChild::spawn(command, &self.work, job).wait();
        self.assert_pinned_unchanged();
        output
    }

    /// Requires destination guards to fail before any curl call.
    pub(super) fn assert_refused(&self, destination: &Path) {
        let names = self.write_json("guard-names.json", &json!(["Guard input"]));
        let response = self.write_json("guard-response.json", &list_response(&["Guard input"]));
        let output = self.run(
            self.command(&names, destination, &response, "guard", 0),
            "guard",
        );
        assert_failed(&output);
        self.assert_call_count("guard", 0);
    }

    /// Finds the per-process curl argument record.
    pub(super) fn call_path(&self, job: &str) -> PathBuf {
        self.work.join(format!("curl-{job}.log"))
    }

    /// Counts curl invocations without losing repeated calls to the same job.
    pub(super) fn assert_call_count(&self, job: &str, expected: usize) {
        let path = self.call_path(job);
        if expected == 0 {
            assert!(!path.exists(), "job {job} must not call curl");
            return;
        }
        let log = fs::read_to_string(path).expect("read recorded curl calls");
        let actual = log.lines().filter(|line| *line == "CALL").count();
        assert_eq!(actual, expected, "unexpected curl call count for job {job}");
    }

    /// Adds only the exclusive-directory race gate to the command path.
    pub(super) fn install_mkdir_stub(&self, contents: &str) {
        support::write_stub(&self.bin.join("mkdir"), contents);
    }

    /// Detects edits to stored captures, referenced sources, or the capture script.
    pub(super) fn assert_pinned_unchanged(&self) {
        assert_eq!(self.pinned, pinned_hashes(), "pinned source bytes changed");
    }
}

/// Produces a collection response whose cards match the requested names.
pub(super) fn list_response(names: &[&str]) -> Value {
    let data: Vec<_> = names.iter().map(|name| json!({ "name": name })).collect();
    json!({ "object": "list", "data": data, "not_found": [] })
}

/// Includes command diagnostics when capture unexpectedly fails.
pub(super) fn assert_succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "capture failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Includes command diagnostics when capture unexpectedly succeeds.
pub(super) fn assert_failed(output: &Output) {
    assert!(
        !output.status.success(),
        "capture unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// Reads JSON for assertions without adding another command stub.
pub(super) fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read captured JSON"))
        .expect("parse captured JSON")
}

/// Checks request identity, raw response preservation, and manifest hashes.
pub(super) fn assert_bundle(destination: &Path, names: &[&str], raw_response: &[u8]) {
    let request = destination.join("request.json");
    let response = destination.join("response.json");
    let manifest = read_json(&destination.join("capture.json"));
    let identifiers: Vec<_> = names.iter().map(|name| json!({ "name": name })).collect();
    assert_eq!(read_json(&request), json!({ "identifiers": identifiers }));
    assert_eq!(
        fs::read(&response).expect("read raw response"),
        raw_response
    );
    assert_eq!(manifest["source"], "Scryfall");
    assert_eq!(
        manifest["endpoint"],
        "https://api.scryfall.com/cards/collection"
    );
    assert_eq!(manifest["method"], "POST");
    assert_eq!(manifest["operation"], "read-only card collection lookup");
    let captured_at = manifest["captured_at"]
        .as_str()
        .expect("capture timestamp is text");
    assert_eq!(captured_at.len(), 20);
    assert!(captured_at.ends_with('Z'));
    assert_eq!(manifest["request_sha256"], file_hash(&request));
    assert_eq!(manifest["response_sha256"], file_hash(&response));
    assert_eq!(
        directory_names(destination),
        ["capture.json", "request.json", "response.json"]
    );
}

/// Lists directory contents so guard tests also catch unexpected writes.
pub(super) fn directory_names(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .expect("read test directory")
        .map(|entry| {
            entry
                .expect("read test directory entry")
                .file_name()
                .into_string()
                .expect("test filename is UTF-8")
        })
        .collect();
    names.sort();
    names
}

/// Hashes file bytes without using a command that the test replaces.
fn file_hash(path: &Path) -> String {
    format!(
        "{:x}",
        Sha256::digest(fs::read(path).expect("read pinned file"))
    )
}

/// Snapshots the source set covered by the metadata maintenance checks.
fn pinned_hashes() -> BTreeMap<PathBuf, String> {
    let root = support::root();
    let supplement = root.join("src/deck/simulator/tests/fixture_metadata/supplement.json");
    let mut hashes = BTreeMap::new();
    hashes.insert(supplement.clone(), file_hash(&supplement));
    let script = root.join("scripts/capture-fixture-metadata.sh");
    hashes.insert(script.clone(), file_hash(&script));
    for source in read_json(&supplement)["sources"]
        .as_array()
        .expect("supplement sources are an array")
    {
        let path = root.join(source["path"].as_str().expect("source path is text"));
        hashes.insert(path.clone(), file_hash(&path));
    }
    for directory in [
        "src/deck/simulator/tests/fixture_metadata/sources",
        "src/deck/simulator/tests/fixture_identities/sources/2026-10-29",
    ] {
        add_json_hashes(&root.join(directory), &mut hashes);
    }
    hashes
}

/// Includes capture requests and manifests as well as response files.
fn add_json_hashes(directory: &Path, hashes: &mut BTreeMap<PathBuf, String>) {
    for entry in fs::read_dir(directory).expect("read pinned source directory") {
        let path = entry.expect("read pinned source entry").path();
        if path.is_dir() {
            add_json_hashes(&path, hashes);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            hashes.insert(path.clone(), file_hash(&path));
        }
    }
}
