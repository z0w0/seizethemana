// Card roles for `deck suggest`: classification and oracle-text /
// tagger-label legs. Split from suggest.rs.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Draw,
    Removal,
    Ramp,
    Wincon,
    Counterspell,
    Land,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s.to_ascii_lowercase().as_str() {
            "draw" | "card-draw" => Some(Role::Draw),
            "removal" | "interaction" => Some(Role::Removal),
            "ramp" | "mana" => Some(Role::Ramp),
            "wincon" | "wincons" | "finisher" | "finishers" => Some(Role::Wincon),
            "counterspell" | "counters" | "counter" => Some(Role::Counterspell),
            "land" | "lands" | "manabase" => Some(Role::Land),
            _ => None,
        }
    }

    /// Substrings that mark the role in oracle text (keyword leg).
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Role::Draw => &["draw ", "investigate", "surveil"],
            Role::Removal => &["destroy target", "exile target", "destroy all creatures"],
            Role::Ramp => &["search your library for a", "add one mana", "add {"],
            Role::Wincon => &["win the game", "each creature you control gets"],
            Role::Counterspell => &["counter target"],
            Role::Land => &["Dual Land", "enters tapped", "Add {"],
        }
    }

    /// Tagger labels that mark this role (tag leg).
    pub fn tag_labels(self) -> &'static [&'static str] {
        match self {
            Role::Draw => &["card draw", "wheel"],
            Role::Removal => &["removal", "board wipe", "destroy"],
            Role::Ramp => &["ramp", "mana acceleration", "mana rock", "mana fixing"],
            Role::Wincon => &["win the game", "overrun", "finisher"],
            Role::Counterspell => &["counterspell", "counter"],
            Role::Land => &["mana fixing", "dual land", "land enters tapped"],
        }
    }
}
