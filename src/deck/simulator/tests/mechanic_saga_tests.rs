use super::*;

#[test]
fn saga_combined_numerals_parse_chapter_effect() {
    // "I, II, III — Create a 1/1 token" gives every chapter the token
    // effect (Sauron class), not the fallback draw.
    let row = card(
        "Sauron Saga",
        "{3}{B}",
        "Enchantment — Saga",
        "I, II, III — Create a 1/1 Orc creature token.",
    );
    let sim = parse_sim_card(&row);
    let chapters: &[Vec<SimEffect>] = &sim.saga.chapters;
    assert_eq!(chapters.len(), 3, "combined numerals fill all chapters");
    assert!(
        chapters.iter().all(|e| e
            .iter()
            .any(|effect| matches!(effect, SimEffect::Tokens(_)))),
        "every chapter carries the token effect, got {chapters:?}"
    );
}

#[test]
fn saga_iv_chapter_fires_and_saga_leaves_board() {
    // A 4-chapter saga: the fourth chapter fires and the permanent
    // leaves the battlefield after the final chapter.
    let row = card(
        "Long Saga",
        "{2}{G}",
        "Enchantment — Saga",
        "I — Draw a card.\nII — Draw a card.\nIII — Draw a card.\nIV — Create two 1/1 Elf creature tokens.",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(sim.chapter_count(), 4, "chapter IV parses");
    assert!(
        sim.saga.chapters[3]
            .iter()
            .any(|effect| matches!(effect, SimEffect::Tokens(2))),
        "IV is tokens"
    );
    let deck = row_deck(30, &[row], Format::Constructed);
    let stats = run_avg(&deck, 9, 300, 42);
    assert!(
        stats.creatures_by_turn[8] > 0.3,
        "the chapter IV tokens join the board, got {:.1}",
        stats.creatures_by_turn[8]
    );
}

#[test]
fn saga_with_extra_trigger_counts_chapters_only() {
    // A saga whose oracle text also carries an ETB trigger: the extra
    // trigger must not extend the chapter count (no phantom fourth
    // chapter, no extra turn on the board).
    let row = card(
        "Rider Saga",
        "{2}{U}",
        "Enchantment — Saga",
        "When this Saga enters, draw a card.\nI — Create a Treasure token.\nII — Create a Treasure token.\nIII — Draw two cards.",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(sim.chapter_count(), 3, "only the chapters count");
    // The ETB draw still parses as a trigger.
    assert!(
        sim.unlocked_abilities(0)
            .any(|a| a.trigger == SimTrigger::Enters),
        "the enter trigger stays a trigger"
    );
    let chapters: &[Vec<SimEffect>] = &sim.saga.chapters;
    assert_eq!(chapters.len(), 3, "three real chapters");
}
