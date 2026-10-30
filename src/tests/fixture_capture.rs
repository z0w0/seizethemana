//! Offline process tests for the opt-in fixture capture commands.
//!
//! The retained commands use Unix tools. Tests control each child environment
//! and keep all generated files outside the repository.

#![cfg(unix)]

/// Source replay, identity checks, and destination safety for Commander captures.
#[path = "fixture_capture/commander_tests.rs"]
mod commander_tests;
/// Metadata capture failures, exclusive creation, and retry behavior.
#[path = "fixture_capture/metadata_tests.rs"]
mod metadata_tests;
/// Bounded process waits and cleanup for both capture tools.
#[path = "fixture_capture/process.rs"]
mod process;
/// Temporary command shims and process setup shared by both capture tools.
#[path = "fixture_capture/support.rs"]
mod support;
