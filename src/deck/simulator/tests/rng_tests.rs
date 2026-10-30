//! Fixed stream and sampling checks for the simulator's versioned RNG contract.

use super::*;

/// Game IDs and domains identify different streams, regardless of consumption.
#[test]
fn streams_do_not_share_state() {
    let mut opening = game_rng(42, 0, RngDomain::Opening);
    let first = opening.next_u64();
    for _ in 0..100 {
        opening.next_u64();
    }
    assert_eq!(first, game_rng(42, 0, RngDomain::Opening).next_u64());
    for domain in [RngDomain::Library, RngDomain::Pilot, RngDomain::Opponent] {
        assert_ne!(first, game_rng(42, 0, domain).next_u64());
    }
    assert_ne!(first, game_rng(42, 1, RngDomain::Opening).next_u64());
    assert_ne!(first, game_rng(43, 0, RngDomain::Opening).next_u64());
}

/// Fixed inputs pin seed encoding, domain numbering, and the ChaCha8 stream.
#[test]
fn version_one_stream_is_stable() {
    assert_eq!(RNG_VERSION, 1);
    let mut rng = game_rng(42, 7, RngDomain::Opening);
    let mut actual = [0u8; 32];
    rng.fill_bytes(&mut actual);
    let expected = [
        203, 0, 151, 7, 167, 133, 110, 12, 195, 33, 213, 100, 74, 150, 199, 98, 72, 43, 242, 144,
        116, 163, 206, 6, 172, 31, 84, 95, 232, 144, 228, 212,
    ];
    assert_eq!(actual, expected);
}

/// Pin shuffle order so a sampling change requires a deliberate version change.
#[test]
fn version_one_shuffle_is_stable() {
    let mut cards: Vec<u32> = (0..10).collect();
    shuffle(&mut cards, &mut game_rng(42, 0, RngDomain::Opening));
    assert_eq!(cards, [8, 7, 2, 1, 0, 9, 4, 5, 3, 6]);
}

/// Empty and singleton inputs consume no randomness; larger shuffles retain cards.
#[test]
fn shuffle_preserves_cards_and_small_inputs() {
    let mut rng = game_rng(42, 0, RngDomain::Opening);
    shuffle::<u8>(&mut [], &mut rng);
    shuffle(&mut [7u8], &mut rng);
    assert_eq!(
        rng.next_u64(),
        game_rng(42, 0, RngDomain::Opening).next_u64()
    );
    let mut left: Vec<u32> = (0..100).collect();
    let mut right = left.clone();
    shuffle(&mut left, &mut game_rng(42, 0, RngDomain::Opening));
    shuffle(&mut right, &mut game_rng(42, 0, RngDomain::Opening));
    assert_eq!(left, right);
    assert_ne!(left, (0..100).collect::<Vec<_>>());
    left.sort_unstable();
    assert_eq!(left, (0..100).collect::<Vec<_>>());
}
