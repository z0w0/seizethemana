//! Commander prompt index mapping: Skip and Other rows resolve correctly.

use super::*;

/// Skip is the second-to-last row, Other (typed name) the last row.
/// Skip and Esc both skip; Other marks a typed name for the caller's
/// input prompt.
#[test]
fn commander_prompt_indices_map_to_skip_and_other() {
    let items = prompt_items(&["A".into(), "B".into()]);
    assert_eq!(items.len(), 4);
    assert_eq!(
        map_pick(None, &items, "Other (type the name)"),
        CommanderPick::Skip
    );
    assert_eq!(
        map_pick(Some(0), &items, "Other (type the name)"),
        CommanderPick::Name("A".into())
    );
    assert_eq!(
        map_pick(Some(2), &items, "Other (type the name)"),
        CommanderPick::Skip
    );
    assert_eq!(
        map_pick(Some(3), &items, "Other (type the name)"),
        CommanderPick::Name("Other (type the name)".into())
    );
}
