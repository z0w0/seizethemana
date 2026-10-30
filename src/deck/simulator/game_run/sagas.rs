//! Resolve Saga chapter abilities and final chapter sacrifices.

use super::super::game::{GameState, card_of};
use super::super::model::SimDeck;
use super::super::resolution::{begin_resolution, end_resolution, queue_saga_chapter};

/// Resolve the next chapter of each eligible Saga, then sacrifice completed Sagas.
pub(in crate::deck::simulator) fn run_sagas(deck: &SimDeck, st: &mut GameState, turn: usize) {
    if st.completion.is_some() {
        return;
    }
    let saga_firings: Vec<(u32, usize)> = st
        .battlefield
        .iter()
        .filter_map(|perm| {
            let card = card_of(deck, perm);
            if !card.is_saga
                || perm.entered_turn == 0
                || turn <= perm.entered_turn
                || perm.saga_step as usize >= card.chapter_count()
            {
                return None;
            }
            let step = perm.saga_step as usize;
            Some((perm.uid, step))
        })
        .collect();
    begin_resolution(st);
    for (uid, chapter) in saga_firings {
        let Some(perm) = st.battlefield.iter_mut().find(|perm| perm.uid == uid) else {
            continue;
        };
        perm.saga_step += 1;
        let source = perm.clone();
        queue_saga_chapter(st, &source, chapter, turn as u32);
    }
    end_resolution(deck, st, None);
}
