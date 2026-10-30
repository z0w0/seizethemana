use std::fs;
use std::path::Path;

use serde_json::{Value, json};

/// Replay support for the retained Commander maintenance command.
#[path = "commander_support.rs"]
mod commander_support;

use commander_support::{
    CommanderMode, CommanderWorkspace, commander_assert_absent, commander_read_json,
    commander_symlink, commander_write_json,
};

/// Keep exact response and request bytes, batch sizes, pacing, and manifest hashes.
#[test]
fn commander_replays_complete_bundle() {
    let workspace = CommanderWorkspace::new(CommanderMode::Popular);
    workspace.assert_success(&workspace.path("capture"));
    workspace.assert_requests(26);
}

/// Remove a partial capture after a transport failure.
#[test]
fn commander_rejects_transport_failure() {
    let workspace = CommanderWorkspace::new(CommanderMode::Popular);
    workspace.fail_transport();
    let output = workspace.assert_failure(&workspace.input(), &workspace.path("capture"), 1);
    assert_eq!(output.status.code(), Some(22));
}

/// Reject malformed source JSON before following pagination.
#[test]
fn commander_rejects_invalid_json() {
    let workspace = CommanderWorkspace::new(CommanderMode::Popular);
    workspace.replace_response("edhrec_page_00.json", b"not JSON\n");
    workspace.assert_failure(&workspace.input(), &workspace.path("capture"), 1);
}

/// Reject the wrong EDHREC reporting window.
#[test]
fn commander_rejects_wrong_window() {
    let workspace = CommanderWorkspace::new(CommanderMode::Popular);
    workspace.mutate_response("edhrec_page_00.json", |response| {
        response["header"] = json!("Top Cards (Past Week)");
    });
    workspace.assert_failure(&workspace.input(), &workspace.path("capture"), 1);
}

/// Reject a skipped continuation before requesting the wrong page.
#[test]
fn commander_rejects_wrong_continuation() {
    let workspace = CommanderWorkspace::new(CommanderMode::Popular);
    workspace.mutate_response("edhrec_page_00.json", |response| {
        response["container"]["json_dict"]["cardlists"][0]["more"] =
            json!("top/year-past2years-2.json");
    });
    workspace.assert_failure(&workspace.input(), &workspace.path("capture"), 1);
}

/// Reject duplicate popular card names before collection requests start.
#[test]
fn commander_rejects_duplicate_name() {
    let workspace = CommanderWorkspace::new(CommanderMode::Popular);
    workspace.mutate_response("edhrec_page_00.json", |response| {
        let cards = &mut response["container"]["json_dict"]["cardlists"][0]["cardviews"];
        cards[1]["name"] = cards[0]["name"].clone();
    });
    workspace.assert_failure(&workspace.input(), &workspace.path("capture"), 10);
}

/// Reject duplicate EDHREC printing IDs before collection requests start.
#[test]
fn commander_rejects_duplicate_id() {
    let workspace = CommanderWorkspace::new(CommanderMode::Popular);
    workspace.mutate_response("edhrec_page_00.json", |response| {
        let cards = &mut response["container"]["json_dict"]["cardlists"][0]["cardviews"];
        cards[1]["id"] = cards[0]["id"].clone();
    });
    workspace.assert_failure(&workspace.input(), &workspace.path("capture"), 10);
}

/// Reject a collection response with unresolved identifiers.
#[test]
fn commander_rejects_not_found() {
    commander_reject_collection(CommanderMode::Popular, 11, |response| {
        response["not_found"] = json!([{"name": "missing"}]);
    });
}

/// Require collection results to preserve the selected printing IDs.
#[test]
fn commander_rejects_wrong_print_id() {
    commander_reject_collection(CommanderMode::Popular, 11, |response| {
        response["data"][0]["id"] = json!("00000000-0000-0000-0000-000000000000");
    });
}

/// Require distinct Oracle identities in the complete Commander selection.
#[test]
fn commander_rejects_duplicate_oracle_id() {
    commander_reject_collection(CommanderMode::Popular, 26, |response| {
        response["data"][1]["oracle_id"] = response["data"][0]["oracle_id"].clone();
    });
}

/// Require metadata names to match the selected source names.
#[test]
fn commander_rejects_wrong_source_name() {
    commander_reject_collection(CommanderMode::Popular, 26, |response| {
        response["data"][0]["name"] = json!("Wrong source name");
    });
}

/// Reject a collection batch that omits a selected card.
#[test]
fn commander_rejects_missing_card() {
    commander_reject_collection(CommanderMode::Popular, 11, |response| {
        response["data"]
            .as_array_mut()
            .expect("collection cards")
            .remove(0);
    });
}

/// Remove all output when hashing the generated card metadata fails.
#[test]
fn commander_rejects_hash_failure() {
    commander_reject_hash(CommanderMode::Popular, 26);
}

/// Preserve existing paths and reject symlink ancestors before any request.
#[test]
fn commander_guards_existing_destinations_and_symlink_ancestors() {
    let workspace = CommanderWorkspace::new(CommanderMode::Popular);
    commander_check_existing_guards(&workspace);
    commander_check_symlink_ancestor_guards(&workspace);
}

/// Reject incomplete land cycles before creating output or making requests.
#[test]
fn commander_rejects_incomplete_land_cycle() {
    let workspace = CommanderWorkspace::new(CommanderMode::Popular);
    let mut lands = commander_read_json(&workspace.input());
    lands["groups"][0]["names"]
        .as_array_mut()
        .expect("land cycle names")
        .remove(0);
    let input = workspace.path("incomplete-lands.json");
    commander_write_json(&input, &lands);
    workspace.assert_failure(&input, &workspace.path("capture"), 0);
}

/// Support option-like destinations without weakening existing path guards.
#[test]
fn commander_supports_leading_hyphens_and_guards_destinations() {
    let mut workspace = CommanderWorkspace::new(CommanderMode::Popular);
    let repository = workspace.isolate_repository();
    workspace.assert_success(Path::new("-capture"));
    commander_symlink(
        &repository.join("-capture"),
        &repository.join("-existing-link"),
    );
    fs::create_dir(repository.join("physical-parent")).expect("create physical parent");
    commander_symlink(
        &repository.join("physical-parent"),
        &repository.join("-alias-parent"),
    );
    for destination in ["-capture", "-existing-link", "-alias-parent/capture"] {
        let output = workspace.capture(&workspace.input(), Path::new(destination));
        assert!(
            !output.status.success(),
            "accepted unsafe destination {destination}"
        );
    }
    workspace.assert_requests(26);
    assert_eq!(
        fs::read_link(repository.join("-existing-link")).expect("existing symlink"),
        repository.join("-capture")
    );
    assert_eq!(
        fs::read_link(repository.join("-alias-parent")).expect("parent symlink"),
        repository.join("physical-parent")
    );
    assert_eq!(
        fs::read_dir(repository.join("physical-parent"))
            .expect("physical parent")
            .count(),
        0
    );
    commander_assert_absent(&repository.join("-alias-parent/capture"));
    workspace.assert_bundle(&repository.join("-capture"));
}

/// Clean up option-like destinations after a late hash failure.
#[test]
fn commander_cleans_leading_hyphen_failure() {
    let mut workspace = CommanderWorkspace::new(CommanderMode::Popular);
    let repository = workspace.isolate_repository();
    workspace.fail_card_hash();
    let output = workspace.assert_failure(&workspace.input(), Path::new("-failed-capture"), 26);
    assert_eq!(output.status.code(), Some(73));
    assert_eq!(fs::read_dir(repository).expect("isolated root").count(), 0);
}

/// Preserve the complete Behold query, raw collection bytes, and manifest hashes.
#[test]
fn commander_behold_replays_complete_bundle() {
    let workspace = CommanderWorkspace::new(CommanderMode::Behold);
    workspace.assert_success(&workspace.path("capture"));
    workspace.assert_requests(2);
}

/// Remove a partial Behold capture after a transport failure.
#[test]
fn commander_behold_rejects_transport_failure() {
    let workspace = CommanderWorkspace::new(CommanderMode::Behold);
    workspace.fail_transport();
    let output = workspace.assert_failure(&workspace.input(), &workspace.path("capture"), 1);
    assert_eq!(output.status.code(), Some(22));
}

/// Reject malformed Behold search JSON before requesting the collection.
#[test]
fn commander_behold_rejects_invalid_json() {
    let workspace = CommanderWorkspace::new(CommanderMode::Behold);
    workspace.replace_response("behold_search_00.json", b"not JSON\n");
    workspace.assert_failure(&workspace.input(), &workspace.path("capture"), 1);
}

/// Reject unresolved Behold collection identifiers.
#[test]
fn commander_behold_rejects_not_found() {
    commander_reject_collection(CommanderMode::Behold, 2, |response| {
        response["not_found"] = json!([{"name": "missing"}]);
    });
}

/// Require Behold collection results to preserve the chosen printing IDs.
#[test]
fn commander_behold_rejects_wrong_print_id() {
    commander_reject_collection(CommanderMode::Behold, 2, |response| {
        response["data"][0]["id"] = json!("00000000-0000-0000-0000-000000000000");
    });
}

/// Require Behold collection Oracle identities to match the search results.
#[test]
fn commander_behold_rejects_duplicate_oracle_id() {
    commander_reject_collection(CommanderMode::Behold, 2, |response| {
        response["data"][1]["oracle_id"] = response["data"][0]["oracle_id"].clone();
    });
}

/// Require Behold metadata names to match the query results.
#[test]
fn commander_behold_rejects_wrong_source_name() {
    commander_reject_collection(CommanderMode::Behold, 2, |response| {
        response["data"][0]["name"] = json!("Wrong source name");
    });
}

/// Reject incomplete Behold collection batches.
#[test]
fn commander_behold_rejects_missing_card() {
    commander_reject_collection(CommanderMode::Behold, 2, |response| {
        response["data"]
            .as_array_mut()
            .expect("collection cards")
            .remove(0);
    });
}

/// Remove Behold output when hashing generated card metadata fails.
#[test]
fn commander_behold_rejects_hash_failure() {
    commander_reject_hash(CommanderMode::Behold, 2);
}

/// Require the Behold search total to match the returned cards.
#[test]
fn commander_behold_rejects_wrong_count() {
    commander_reject_search(1, |response| {
        response["total_cards"] =
            json!(response["total_cards"].as_u64().expect("search count") + 1);
    });
}

/// Reject a search identity that differs from the collection's Oracle identity.
#[test]
fn commander_behold_rejects_wrong_identity() {
    commander_reject_search(2, |response| {
        response["data"][0]["oracle_id"] = json!("00000000-0000-0000-0000-000000000000");
    });
}

/// Require the known Elven Passage identity in the Behold supplement.
#[test]
fn commander_behold_rejects_missing_elven_passage() {
    commander_reject_search(1, |response| {
        response["data"]
            .as_array_mut()
            .expect("search cards")
            .retain(|card| card["name"] != "Elven Passage");
        response["total_cards"] =
            json!(response["total_cards"].as_u64().expect("search count") - 1);
    });
}

/// Preserve existing files, directories, and live or dangling destination links.
fn commander_check_existing_guards(workspace: &CommanderWorkspace) {
    fs::write(workspace.path("existing-file"), "sentinel\n").expect("write file sentinel");
    fs::create_dir(workspace.path("existing-directory")).expect("create existing directory");
    fs::write(
        workspace.path("existing-directory/child"),
        "child sentinel\n",
    )
    .expect("write directory sentinel");
    commander_symlink(
        &workspace.path("existing-file"),
        &workspace.path("live-link"),
    );
    commander_symlink(
        &workspace.path("missing-target"),
        &workspace.path("dangling-link"),
    );
    for name in [
        "existing-file",
        "existing-directory",
        "live-link",
        "dangling-link",
    ] {
        let output = workspace.capture(&workspace.input(), &workspace.path(name));
        assert!(
            !output.status.success(),
            "accepted existing destination {name}"
        );
    }
    assert_eq!(
        fs::read(workspace.path("existing-file")).expect("file sentinel"),
        b"sentinel\n"
    );
    assert_eq!(
        fs::read(workspace.path("existing-directory/child")).expect("directory sentinel"),
        b"child sentinel\n"
    );
    assert_eq!(
        fs::read_link(workspace.path("live-link")).expect("live symlink"),
        workspace.path("existing-file")
    );
    assert_eq!(
        fs::read_link(workspace.path("dangling-link")).expect("dangling symlink"),
        workspace.path("missing-target")
    );
    workspace.assert_requests(0);
}

/// Reject both direct symlink parents and symlinks higher in the parent chain.
fn commander_check_symlink_ancestor_guards(workspace: &CommanderWorkspace) {
    fs::create_dir(workspace.path("physical-parent")).expect("create physical parent");
    commander_symlink(
        &workspace.path("physical-parent"),
        &workspace.path("alias-parent"),
    );
    workspace.assert_failure(
        &workspace.input(),
        &workspace.path("alias-parent/capture"),
        0,
    );
    assert_eq!(
        fs::read_dir(workspace.path("physical-parent"))
            .expect("physical parent")
            .count(),
        0
    );
    fs::create_dir(workspace.path("physical-parent/child")).expect("create nested parent");
    workspace.assert_failure(
        &workspace.input(),
        &workspace.path("alias-parent/child/capture"),
        0,
    );
    assert_eq!(
        fs::read_dir(workspace.path("physical-parent/child"))
            .expect("nested parent")
            .count(),
        0
    );
    assert_eq!(
        fs::read_link(workspace.path("alias-parent")).expect("parent symlink"),
        workspace.path("physical-parent")
    );
}

/// Apply a collection fault and verify refusal at its expected request boundary.
fn commander_reject_collection(
    mode: CommanderMode,
    requests: usize,
    mutate: impl FnOnce(&mut Value),
) {
    let workspace = CommanderWorkspace::new(mode);
    workspace.mutate_response("scryfall_response_00.json", mutate);
    workspace.assert_failure(&workspace.input(), &workspace.path("capture"), requests);
}

/// Apply a Behold search fault without changing the pinned query response.
fn commander_reject_search(requests: usize, mutate: impl FnOnce(&mut Value)) {
    let workspace = CommanderWorkspace::new(CommanderMode::Behold);
    workspace.mutate_response("behold_search_00.json", mutate);
    workspace.assert_failure(&workspace.input(), &workspace.path("capture"), requests);
}

/// Verify that a late hash error removes the complete temporary bundle.
fn commander_reject_hash(mode: CommanderMode, requests: usize) {
    let mut workspace = CommanderWorkspace::new(mode);
    workspace.fail_card_hash();
    let output = workspace.assert_failure(&workspace.input(), &workspace.path("capture"), requests);
    assert_eq!(output.status.code(), Some(73));
}
