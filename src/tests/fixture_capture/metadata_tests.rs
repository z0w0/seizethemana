//! Offline checks for safe metadata capture and raw response review.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use serde_json::json;

use super::support;

/// Owns local response replay and pinned source checks.
#[path = "metadata_support.rs"]
mod metadata_support;

use super::process::{CaptureChild, wait_ready};
use metadata_support::{
    MetadataCapture, assert_bundle, assert_failed, assert_succeeded, directory_names,
    list_response, read_json,
};

/// Pauses only the exclusive mkdir after the script has checked the destination.
const MKDIR_STUB: &str = r#"#!/bin/bash
set -euo pipefail
if [[ $# == 2 && "$1" == -- && "$2" == "$METADATA_CAPTURE_DEST" ]]; then
  : >"$METADATA_CAPTURE_GATE/ready-$METADATA_CAPTURE_JOB"
  for ((attempt=0; attempt<1000; attempt++)); do
    if [[ -e "$METADATA_CAPTURE_GATE/release-$METADATA_CAPTURE_JOB" ]]; then
      exec /bin/mkdir "$@"
    fi
    sleep 0.02
  done
  exit 76
fi
exec /bin/mkdir "$@"
"#;

/// Keeps an existing file intact and refuses it before calling curl.
#[test]
fn metadata_capture_preserves_existing_file() {
    let capture = MetadataCapture::new();
    let destination = capture.write("existing-file", b"existing output\n");
    capture.assert_refused(&destination);
    assert_eq!(
        fs::read(destination).expect("read existing file"),
        b"existing output\n"
    );
}

/// Keeps an existing directory and its children intact.
#[test]
fn metadata_capture_preserves_existing_directory() {
    let capture = MetadataCapture::new();
    let destination = capture.work.join("existing-directory");
    fs::create_dir(&destination).expect("create existing directory");
    fs::write(destination.join("marker"), b"existing child\n").expect("write existing child");
    capture.assert_refused(&destination);
    assert_eq!(directory_names(&destination), ["marker"]);
    assert_eq!(
        fs::read(destination.join("marker")).expect("read child"),
        b"existing child\n"
    );
}

/// Refuses live file links without changing the link or its target.
#[test]
fn metadata_capture_preserves_live_file_symlink() {
    let capture = MetadataCapture::new();
    let target = capture.write("target-file", b"file sentinel\n");
    let destination = capture.work.join("file-link");
    symlink(&target, &destination).expect("create live file symlink");
    capture.assert_refused(&destination);
    assert_eq!(
        fs::read_link(destination).expect("read file symlink"),
        target
    );
    assert_eq!(
        fs::read(target).expect("read symlink target"),
        b"file sentinel\n"
    );
}

/// Refuses live directory links without changing the link or target contents.
#[test]
fn metadata_capture_preserves_live_directory_symlink() {
    let capture = MetadataCapture::new();
    let target = capture.work.join("target-directory");
    fs::create_dir(&target).expect("create symlink target directory");
    fs::write(target.join("marker"), b"directory sentinel\n").expect("write target marker");
    let destination = capture.work.join("directory-link");
    symlink(&target, &destination).expect("create live directory symlink");
    capture.assert_refused(&destination);
    assert_eq!(
        fs::read_link(destination).expect("read directory symlink"),
        target
    );
    assert_eq!(directory_names(&target), ["marker"]);
    assert_eq!(
        fs::read(target.join("marker")).expect("read marker"),
        b"directory sentinel\n"
    );
}

/// Refuses a dangling link without replacing it or creating its target.
#[test]
fn metadata_capture_preserves_dangling_symlink() {
    let capture = MetadataCapture::new();
    let target = capture.work.join("missing-target");
    let destination = capture.work.join("dangling-link");
    symlink(&target, &destination).expect("create dangling symlink");
    capture.assert_refused(&destination);
    assert_eq!(
        fs::read_link(destination).expect("read dangling symlink"),
        target
    );
    assert!(fs::symlink_metadata(target).is_err());
}

/// Refuses symlink ancestors before creating directories or calling curl.
#[test]
fn metadata_capture_refuses_symlink_parent() {
    let capture = MetadataCapture::new();
    let physical = capture.work.join("physical-parent");
    fs::create_dir(&physical).expect("create physical parent");
    let alias = capture.work.join("alias-parent");
    symlink(&physical, &alias).expect("create parent symlink");
    capture.assert_refused(&alias.join("nested/new-capture"));
    assert_eq!(fs::read_link(alias).expect("read parent symlink"), physical);
    assert!(directory_names(&physical).is_empty());
}

/// A partial curl response releases the destination for a complete retry.
#[test]
fn metadata_capture_retries_after_partial_network_failure() {
    assert_retry_after_failure(b"partial response\n", 22);
}

/// Invalid JSON and invalid response fields release the destination for retry.
#[test]
fn metadata_capture_retries_after_malformed_response() {
    let responses: &[&[u8]] = &[
        b"not JSON\n",
        br#"{"object":"card","data":[],"not_found":[]}"#,
        br#"{"object":"list","data":{},"not_found":[]}"#,
        br#"{"object":"list","data":[],"not_found":{}}"#,
    ];
    for response in responses {
        assert_retry_after_failure(response, 0);
    }
}

/// Missing card names remain in a reviewable capture with exact response bytes.
#[test]
fn metadata_capture_keeps_not_found_response_for_review() {
    let capture = MetadataCapture::new();
    let names = capture.write_json("names.json", &json!(["Found card", "Missing card"]));
    let raw = b" \n{\"not_found\":[{\"name\":\"Missing card\"}],\"data\":[{\"name\":\"Found card\",\"review\":\"keep raw bytes\"}],\"object\":\"list\"}\n\n";
    let response = capture.write("response.json", raw);
    let destination = capture.work.join("review-capture");
    let output = capture.run(
        capture.command(&names, &destination, &response, "review", 0),
        "review",
    );
    assert_succeeded(&output);
    assert_bundle(&destination, &["Found card", "Missing card"], raw);
    capture.assert_call_count("review", 1);
    let summary: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse review summary");
    assert_eq!(
        summary,
        json!({ "returned": 1, "not_found": [{ "name": "Missing card" }] })
    );
    let arguments = fs::read_to_string(capture.call_path("review")).expect("read curl arguments");
    assert!(arguments.contains("--data-binary\n"));
    assert!(arguments.contains(&format!(
        "@{}\n",
        destination.join("request.json").display()
    )));
    assert!(arguments.ends_with("https://api.scryfall.com/cards/collection\n"));
}

/// Both contenders pass early guards, but only the mkdir winner may use curl.
#[test]
fn metadata_capture_race_has_one_winner_and_preserves_its_files() {
    for winner in ["a", "b"] {
        assert_capture_race(winner);
    }
}

/// Checks cleanup scope, retry success, and both captured file hashes.
fn assert_retry_after_failure(raw_response: &[u8], curl_status: i32) {
    let capture = MetadataCapture::new();
    let names = capture.write_json("names.json", &json!(["Retry input"]));
    let response = capture.write("failed-response.json", raw_response);
    let destination = capture.work.join("retry-capture");
    let sentinel = capture.write("unrelated-file", b"keep sibling\n");
    let failed = capture.run(
        capture.command(&names, &destination, &response, "failed", curl_status),
        "failed",
    );
    assert_failed(&failed);
    capture.assert_call_count("failed", 1);
    assert!(
        fs::symlink_metadata(&destination).is_err(),
        "partial capture remains"
    );
    assert_eq!(
        fs::read(&sentinel).expect("read sibling after failure"),
        b"keep sibling\n"
    );
    let response = capture.write_json("retry-response.json", &list_response(&["Retry input"]));
    let output = capture.run(
        capture.command(&names, &destination, &response, "retry", 0),
        "retry",
    );
    assert_succeeded(&output);
    assert_bundle(
        &destination,
        &["Retry input"],
        &fs::read(response).expect("read retry response"),
    );
    capture.assert_call_count("retry", 1);
    assert_eq!(
        fs::read(sentinel).expect("read sibling after retry"),
        b"keep sibling\n"
    );
}

/// Holds both captures at exclusive mkdir, then makes each job win in turn.
fn assert_capture_race(winner: &str) {
    let capture = MetadataCapture::new();
    capture.install_mkdir_stub(MKDIR_STUB);
    let gate = capture.work.join("gate");
    fs::create_dir(&gate).expect("create capture race gate");
    let destination = capture.work.join("shared-capture");
    let first = spawn_contender(&capture, &gate, &destination, "a", "Capture A");
    let second = spawn_contender(&capture, &gate, &destination, "b", "Capture B");
    wait_ready(&gate, &["a", "b"]);
    assert!(
        !destination.exists(),
        "both jobs must stop before exclusive mkdir"
    );
    let (winning_child, losing_child, winner_name, loser) = if winner == "a" {
        (first, second, "Capture A", "b")
    } else {
        (second, first, "Capture B", "a")
    };
    fs::write(gate.join(format!("release-{winner}")), []).expect("release winning capture");
    let winning_output = winning_child.wait();
    assert_succeeded(&winning_output);
    let before: Vec<_> = ["request.json", "response.json", "capture.json"]
        .iter()
        .map(|name| fs::read(destination.join(name)).expect("read winning capture"))
        .collect();
    fs::write(gate.join(format!("release-{loser}")), []).expect("release losing capture");
    assert_failed(&losing_child.wait());
    for (name, bytes) in ["request.json", "response.json", "capture.json"]
        .iter()
        .zip(&before)
    {
        assert_eq!(
            &fs::read(destination.join(name)).expect("read capture after loser"),
            bytes
        );
    }
    capture.assert_call_count(winner, 1);
    capture.assert_call_count(loser, 0);
    assert_eq!(
        read_json(&destination.join("response.json")),
        list_response(&[winner_name])
    );
    assert_bundle(&destination, &[winner_name], &before[1]);
    capture.assert_pinned_unchanged();
}

/// Starts one contender with its own input, response, logs, and race marker.
fn spawn_contender(
    capture: &MetadataCapture,
    gate: &Path,
    destination: &Path,
    job: &str,
    name: &str,
) -> CaptureChild {
    let names = capture.write_json(&format!("names-{job}.json"), &json!([name]));
    let response = capture.write_json(&format!("response-{job}.json"), &list_response(&[name]));
    let mut command = capture.command(&names, destination, &response, job, 0);
    command
        .env("METADATA_CAPTURE_GATE", gate)
        .env("METADATA_CAPTURE_DEST", destination)
        .env("METADATA_CAPTURE_JOB", job);
    CaptureChild::spawn(command, &capture.work, job)
}
