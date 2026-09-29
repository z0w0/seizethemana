//! Runtime keywords and counters: what a permanent can hold and grant.
//!
//! The simulator's own keyword set, separate from the Oracle keyword
//! list: it covers what a counter, a static grant, or a temporary effect
//! can hand out, and what combat reads.

/// The keywords a permanent can hold at runtime.
///
/// This is the simulator's own keyword set, separate from the Oracle
/// keyword list: it covers what a counter, a static grant, or a temporary
/// effect can hand out, and what combat reads. Add a variant here and a
/// row in `KEYWORD_TABLE` to support one more keyword everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Keyword {
    /// Flying.
    Flying,
    /// First strike.
    FirstStrike,
    /// Double strike.
    DoubleStrike,
    /// Deathtouch.
    Deathtouch,
    /// Lifelink.
    Lifelink,
    /// Menace.
    Menace,
    /// Reach.
    Reach,
    /// Trample.
    Trample,
    /// Vigilance.
    Vigilance,
    /// Haste.
    Haste,
    /// Prowess.
    Prowess,
    /// Hexproof.
    Hexproof,
    /// Indestructible.
    Indestructible,
}

/// One row per runtime keyword: the variant and its Oracle spelling.
///
/// The table is the single source of truth for naming; parsing and the
/// grant/counter paths read it, so a new keyword is one row.
pub const KEYWORD_TABLE: &[(&str, Keyword)] = &[
    ("flying", Keyword::Flying),
    ("first strike", Keyword::FirstStrike),
    ("double strike", Keyword::DoubleStrike),
    ("deathtouch", Keyword::Deathtouch),
    ("lifelink", Keyword::Lifelink),
    ("menace", Keyword::Menace),
    ("reach", Keyword::Reach),
    ("trample", Keyword::Trample),
    ("vigilance", Keyword::Vigilance),
    ("haste", Keyword::Haste),
    ("prowess", Keyword::Prowess),
    ("hexproof", Keyword::Hexproof),
    ("indestructible", Keyword::Indestructible),
];

impl Keyword {
    /// Bit for this keyword in a [`KeywordSet`].
    pub const fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

/// A set of runtime keywords (a bitmask over [`Keyword`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeywordSet(u32);

impl KeywordSet {
    /// An empty set.
    pub const fn new() -> Self {
        Self(0)
    }

    /// Add one keyword to the set.
    pub fn insert(&mut self, keyword: Keyword) {
        self.0 |= keyword.bit();
    }

    /// True when the set holds the keyword.
    pub const fn contains(self, keyword: Keyword) -> bool {
        self.0 & keyword.bit() != 0
    }
}

/// Every permanent's counters, split by kind.
///
/// A single number cannot represent a card that enters with charge
/// counters and later gains +1/+1 counters; each mechanic reads its own
/// bucket (station tiers and banked mana read charge, body power reads
/// +1/+1, removal reads -1/-1, activations read loyalty, combat reads
/// keyword counters).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// +1/+1 counters: join body power, feed undying's return check.
    pub plus1: u32,
    /// -1/-1 counters: kill a body once they reach its toughness.
    pub minus1: u32,
    /// Charge counters: station tiers, banked mana, win thresholds.
    pub charge: u32,
    /// Planeswalker loyalty: activations spend it.
    pub loyalty: u32,
    /// Burden counters (The One Ring class): feed the draw scaling and
    /// the upkeep life loss.
    pub burden: u32,
    /// Keyword counters (Ikoria class): each grants its keyword.
    pub keyword: KeywordSet,
}

/// Counters the player holds, once per game (not per permanent).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerCounters {
    /// Energy counters: produced by energy cards, spent by energy sinks.
    pub energy: u32,
}

/// The counters a card enters with, from its Oracle text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EnterCounters {
    /// No enter counters.
    #[default]
    None,
    /// Enters with N charge counters.
    Charge(u32),
    /// Enters with N +1/+1 counters.
    Plus1(u32),
    /// "Enters with X charge counters": the cast leftover converts.
    XCharge,
    /// "Enters with X +1/+1 counters": the cast leftover converts.
    XPlus1,
}

impl EnterCounters {
    /// Counters the card enters with on a normal cast. The X forms
    /// convert the cast's leftover pool at the cast site instead.
    pub fn fixed(self) -> Counters {
        match self {
            Self::Charge(n) => Counters {
                charge: n,
                ..Counters::default()
            },
            Self::Plus1(n) => Counters {
                plus1: n,
                ..Counters::default()
            },
            Self::None | Self::XCharge | Self::XPlus1 => Counters::default(),
        }
    }

    /// Fixed enter counters plus `bonus` +1/+1 counters, for effects
    /// that put the card in with an extra counter (Birthing Pod class).
    pub fn fixed_with_plus1(self, bonus: u32) -> Counters {
        let mut counters = self.fixed();
        counters.plus1 += bonus;
        counters
    }

    /// Enter counters from a paid X, for the card's X kind.
    pub fn with_paid_x(self, x: u32) -> Counters {
        match self {
            Self::XCharge => Counters {
                charge: x,
                ..Counters::default()
            },
            Self::XPlus1 => Counters {
                plus1: x,
                ..Counters::default()
            },
            Self::None | Self::Charge(_) | Self::Plus1(_) => Counters::default(),
        }
    }

    /// True when the card's X form converts the leftover pool.
    pub fn is_x(self) -> bool {
        matches!(self, Self::XCharge | Self::XPlus1)
    }
}
