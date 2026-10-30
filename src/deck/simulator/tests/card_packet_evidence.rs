//! Callable packet evidence keeps selected sources, wider sources, and synthetic rules separate.

use crate::db::CardRow;
use crate::deck::simulator::card_test_support::{corpus, named_row, pinned_revision};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

use super::SourceAssertion;

/// A function pointer is the evidence; its checked Rust path is only a report label.
#[derive(Clone, Copy)]
pub(in crate::deck::simulator) enum Callback {
    Row(&'static str, SourceAssertion),
    Scenario(&'static str, fn()),
}

impl Callback {
    /// Compare functions across source bindings without counting repeated paths as new callbacks.
    pub(in crate::deck::simulator) fn same_function(self, other: Self) -> bool {
        match (self, other) {
            (Self::Row(_, first), Self::Row(_, second)) => std::ptr::fn_addr_eq(first, second),
            (Self::Scenario(_, first), Self::Scenario(_, second)) => {
                std::ptr::fn_addr_eq(first, second)
            }
            _ => false,
        }
    }

    /// Keep the compiled function path beside its evidence record.
    fn label(self) -> &'static str {
        match self {
            Self::Row(label, _) | Self::Scenario(label, _) => label,
        }
    }
}

/// One reviewed subset can have several callbacks without becoming several source identities.
pub(super) struct Packet {
    pub(super) id: &'static str,
    source: Option<&'static str>,
    clauses: &'static [&'static str],
    disposition: &'static str,
    scope: &'static str,
    approximation: &'static str,
    limit: &'static str,
    callbacks: &'static [Callback],
}

/// Bind labels and row callbacks to the same Rust paths at compile time.
macro_rules! row_callbacks {
    ($($path:path),+ $(,)?) => { &[$(Callback::Row(stringify!($path), $path)),+] };
}

/// Bind a complete no-argument scenario to a compiled Rust function.
macro_rules! scenario_callbacks {
    ($($path:path),+ $(,)?) => { &[$(Callback::Scenario(stringify!($path), $path)),+] };
}

/// Mana and cast payments use their own source and policy limits.
#[path = "card_payment_packet_evidence.rs"]
mod payment;
/// Events, tokens, and damage keep their modeled targets and bodies explicit.
#[path = "card_permanent_packet_evidence.rs"]
mod permanents;

/// Reviewed boundary reports add missing stages without replacing earlier source packets.
#[path = "card_boundary_packet_evidence.rs"]
mod boundaries;
/// Library movement and land rules retain their individual source revisions.
#[path = "card_zone_packet_evidence.rs"]
mod zones;

/// Iterate all reviewed packets without deriving support from compiler status.
pub(super) fn packets() -> impl Iterator<Item = &'static Packet> {
    payment::PACKETS
        .iter()
        .chain(zones::PACKETS)
        .chain(permanents::PACKETS)
        .chain(boundaries::PACKETS)
}

/// An obligation must name a real registered packet before it can use that packet's evidence.
pub(in crate::deck::simulator) fn has_packet(id: &str) -> bool {
    packets().any(|packet| packet.id == id)
}

/// Every packet must serve an explicit mechanic obligation rather than inflating a registry count.
pub(in crate::deck::simulator) fn ids() -> BTreeSet<&'static str> {
    packets().map(|packet| packet.id).collect()
}

/// Selected identities remain E, other genuine inputs remain W, and synthetic rules stay separate.
fn source_scope(source: Option<&CardRow>) -> &'static str {
    let Some(source) = source else {
        return "synthetic";
    };
    if corpus()["cards"]
        .as_array()
        .expect("corpus cards")
        .iter()
        .any(|card| {
            card["metadata"]["oracle_id"] == source.oracle_id
                && card["selection_tags"]
                    .as_array()
                    .expect("selection tags")
                    .iter()
                    .any(|tag| tag == "edhrec_top250")
        })
    {
        "E"
    } else {
        "W"
    }
}

/// A source's field-only cast evidence must not certify its unsupported Oracle abilities.
fn source_lines<'a>(packet: &Packet, source: &'a CardRow) -> Vec<&'a str> {
    packet
        .clauses
        .iter()
        .map(|clause| {
            let lines: Vec<_> = source
                .oracle_text
                .lines()
                .map(|line| line.strip_prefix("// ").unwrap_or(line))
                .filter(|line| line.contains(clause))
                .collect();
            let [line] = lines.as_slice() else {
                panic!("{}: one source line for {clause}", packet.id);
            };
            *line
        })
        .collect()
}

/// Preserve callable paths, immutable revisions, and limits without changing semantic review dispositions.
pub(in crate::deck::simulator) fn records(ids: &[&str]) -> Vec<Value> {
    ids.iter().map(|id| {
        let packet = packets().find(|packet| packet.id == *id).expect("bound packet exists");
        let source = packet.source.map(named_row);
        let scope = if packet.disposition == "multi_source_supported" { "E_and_W" } else { source_scope(source.as_ref()) };
        json!({"packet": id, "source": source.as_ref().map(|source| source.name.as_str()), "fixture_alias": packet.source, "source_scope": scope,
            "revision": source.as_ref().map(pinned_revision), "disposition": packet.disposition,
            "scope": packet.scope, "approximation": packet.approximation, "remaining_scope_limit": packet.limit,
            "printed_field_only": packet.source.is_some() && packet.clauses.is_empty(),
            "callbacks": packet.callbacks.iter().map(|callback| callback.label()).collect::<Vec<_>>(),
            "whole_card_execution": "not_established"})
    }).collect()
}

/// Link only the exact source clauses exercised by a packet, never all abilities on its card.
pub(in crate::deck::simulator) fn source_branches(source: &CardRow, wording: &str) -> Vec<Value> {
    packets()
        .filter(|packet| {
            packet.source.is_some_and(|name| {
                (name == source.name || source.name.split(" // ").next() == Some(name))
                    && named_row(name).oracle_id == source.oracle_id
            })
        })
        .flat_map(|packet| {
            source_lines(packet, source)
                .into_iter()
                .filter(|line| wording.contains(line))
                .map(|line| {
                    json!({"source_text": line, "packet": packet.id, "scope": packet.scope,
                "disposition": packet.disposition, "approximation": packet.approximation,
                "remaining_scope_limit": packet.limit, "whole_card_execution": "not_established"})
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Run each distinct source/function binding once, even when several obligations use the packet.
pub(in crate::deck::simulator) fn execute_all() -> (usize, Vec<Callback>) {
    execute_new_bindings(&[])
}

/// Reused source callbacks do not become new bindings or run twice in the combined catalog.
pub(in crate::deck::simulator) fn execute_new_bindings(
    previous: &[(Option<&'static str>, Callback)],
) -> (usize, Vec<Callback>) {
    let mut bindings = previous.to_vec();
    let mut callbacks = Vec::<Callback>::new();
    for packet in packets() {
        let source = packet.source.map(named_row);
        if let Some(source) = &source {
            pinned_revision(source);
        }
        for callback in packet.callbacks {
            if bindings
                .iter()
                .any(|(name, previous)| *name == packet.source && previous.same_function(*callback))
            {
                continue;
            }
            match (*callback, source.as_ref()) {
                (Callback::Row(_, function), Some(source)) => function(source),
                (Callback::Scenario(_, function), _) => function(),
                _ => panic!("{}: callback source kind must match its packet", packet.id),
            }
            bindings.push((packet.source, *callback));
            if !callbacks
                .iter()
                .any(|previous| previous.same_function(*callback))
            {
                callbacks.push(*callback);
            }
        }
    }
    (bindings.len() - previous.len(), callbacks)
}

/// Audit exact source inputs independently of the packet's execution certificate.
pub(in crate::deck::simulator) fn source_inventory() -> Value {
    let reviews = crate::deck::simulator::card_semantic_tests::reviewed_cards();
    let mut ids = BTreeSet::new();
    let mut selected = BTreeSet::new();
    let mut wider = BTreeSet::new();
    let mut groups = BTreeSet::new();
    let mut ambiguous_groups = BTreeMap::new();
    let mut lines = BTreeSet::new();
    let mut synthetic = 0;
    let mut multi_source = 0;
    let mut dispositions = BTreeMap::<&str, usize>::new();
    let mut source_dispositions = BTreeMap::<&str, BTreeSet<String>>::new();
    let mut bindings = Vec::<(Option<&str>, Callback)>::new();
    let mut callbacks = Vec::<Callback>::new();
    for packet in packets() {
        assert!(ids.insert(packet.id));
        assert!(!packet.scope.is_empty() && !packet.limit.is_empty());
        *dispositions.entry(packet.disposition).or_default() += 1;
        for callback in packet.callbacks {
            if !bindings.iter().any(|(source, previous)| {
                *source == packet.source && previous.same_function(*callback)
            }) {
                bindings.push((packet.source, *callback));
            }
            if !callbacks
                .iter()
                .any(|previous| previous.same_function(*callback))
            {
                callbacks.push(*callback);
            }
        }
        let Some(name) = packet.source else {
            if packet.disposition == "multi_source_supported" {
                multi_source += 1;
            } else {
                synthetic += 1;
            }
            continue;
        };
        let source = named_row(name);
        source_dispositions
            .entry(packet.disposition)
            .or_default()
            .insert(source.oracle_id.clone());
        if source_scope(Some(&source)) == "E" {
            selected.insert(source.oracle_id.clone());
        } else {
            wider.insert(source.oracle_id.clone());
        }
        for line in source_lines(packet, &source) {
            lines.insert((source.oracle_id.clone(), line.to_string()));
            let links = super::source_review_links(&source, line, &reviews);
            if let [review] = links.as_slice() {
                if let Some(group) = review["semantic_group_index"].as_u64() {
                    groups.insert((source.oracle_id.clone(), group));
                }
            } else {
                ambiguous_groups.insert(packet.id, links);
            }
        }
    }
    json!({"packets": ids.len(), "E_identities": selected.len(), "W_identities": wider.len(),
        "registered_source_function_bindings": bindings.len(), "registered_distinct_callbacks": callbacks.len(),
        "synthetic_packets": synthetic, "multi_source_packets": multi_source,
        "distinct_source_lines": lines.len(), "reviewed_groups": groups.len(),
        "ambiguous_review_groups_not_credited": ambiguous_groups,
        "genuine_identity_counts_by_disposition_non_disjoint": source_dispositions.into_iter().map(|(kind, ids)| (kind, ids.len())).collect::<BTreeMap<_, _>>(),
        "packet_dispositions_not_identity_counts": dispositions, "whole_card_claims": 0})
}

/// Source counts alone never certify execution or close a mechanic obligation.
#[test]
fn packet_sources_preserve_exact_revisions_faces_and_review_links() {
    println!(
        "packet source inventory, not execution proof: {}",
        source_inventory()
    );
}

/// Count actual function bindings only after all positive, negative, action, and report assertions pass.
#[test]
fn reviewed_packets_execute_unique_bindings_with_exact_source_scope() {
    let (bindings, callbacks) = execute_all();
    println!(
        "executed packet evidence: {bindings} distinct source/function bindings; {} distinct callbacks; {}",
        callbacks.len(),
        source_inventory()
    );
}
