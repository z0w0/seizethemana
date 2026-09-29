//! Named mana colors and basic land types shared by parsing and execution.

/// A color in the Magic color pie, in WUBRG order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManaColor {
    /// White.
    White,
    /// Blue.
    Blue,
    /// Black.
    Black,
    /// Red.
    Red,
    /// Green.
    Green,
}

impl ManaColor {
    /// All colors in the order used by mana arrays.
    pub const ALL: [Self; 5] = [Self::White, Self::Blue, Self::Black, Self::Red, Self::Green];

    /// The WUBRG index used by mana arrays and card color masks.
    pub const fn index(self) -> usize {
        match self {
            Self::White => 0,
            Self::Blue => 1,
            Self::Black => 2,
            Self::Red => 3,
            Self::Green => 4,
        }
    }

    /// The conventional mana symbol letter.
    pub const fn symbol(self) -> char {
        match self {
            Self::White => 'W',
            Self::Blue => 'U',
            Self::Black => 'B',
            Self::Red => 'R',
            Self::Green => 'G',
        }
    }

    /// Parse a conventional mana symbol letter.
    pub fn from_symbol(symbol: char) -> Option<Self> {
        match symbol.to_ascii_uppercase() {
            'W' => Some(Self::White),
            'U' => Some(Self::Blue),
            'B' => Some(Self::Black),
            'R' => Some(Self::Red),
            'G' => Some(Self::Green),
            _ => None,
        }
    }

    /// The lowercase color name used in human-facing text.
    pub const fn name(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Blue => "blue",
            Self::Black => "black",
            Self::Red => "red",
            Self::Green => "green",
        }
    }
}

/// A basic land type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BasicLandType {
    /// Plains.
    Plains,
    /// Island.
    Island,
    /// Swamp.
    Swamp,
    /// Mountain.
    Mountain,
    /// Forest.
    Forest,
}

impl BasicLandType {
    /// All basic land types in WUBRG order.
    pub const ALL: [Self; 5] = [
        Self::Plains,
        Self::Island,
        Self::Swamp,
        Self::Mountain,
        Self::Forest,
    ];

    /// The WUBRG index used by card type masks.
    pub const fn index(self) -> usize {
        match self {
            Self::Plains => 0,
            Self::Island => 1,
            Self::Swamp => 2,
            Self::Mountain => 3,
            Self::Forest => 4,
        }
    }

    /// The basic land type name used in Oracle text and type lines.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Plains => "Plains",
            Self::Island => "Island",
            Self::Swamp => "Swamp",
            Self::Mountain => "Mountain",
            Self::Forest => "Forest",
        }
    }
}

/// Colors tracked for mana modeling, WUBRG order.
pub const COLORS: [ManaColor; 5] = ManaColor::ALL;
