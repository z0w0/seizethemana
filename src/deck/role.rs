//! Card roles for `deck suggest`: classification and oracle-text /
//! tagger-label legs. Split from suggest.rs.

/// A deckbuilding job a card can do ("draw", "ramp", "removal"), used to
/// rank role-keyword scans and structured `--role` suggestions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Draws cards.
    Draw,
    /// Answers opposing permanents.
    Removal,
    /// Accelerates mana.
    Ramp,
    /// Closes out the game.
    Wincon,
    /// Counters spells.
    Counterspell,
    /// Fixes or provides mana.
    Land,
    /// Sweeps the board.
    BoardWipe,
    /// Searches the library.
    Tutor,
    /// Provides sacrifice outlets.
    Sacrifice,
    /// Returns cards from the graveyard to the battlefield.
    Reanimate,
    /// Recurs cards from the graveyard.
    Recursion,
    /// Fuels or punishes discarding.
    Discard,
    /// Mills cards.
    Mill,
    /// Gains or drains life.
    Lifegain,
    /// Deals direct damage.
    Burn,
    /// Creates tokens.
    Token,
    /// Boosts the whole team.
    Anthem,
    /// Equips creatures.
    Equipment,
    /// Evades blockers.
    Evasion,
    /// Tricks in combat.
    CombatTrick,
    /// Steals opposing permanents.
    Theft,
    /// Protects permanents.
    Protection,
    /// Taxes or locks opponents.
    Stax,
    /// Attacks graveyards.
    GraveyardHate,
    /// Forms a combo.
    Combo,
    /// Builds a storm count.
    Storm,
    /// Takes extra turns.
    ExtraTurn,
    /// Blinks permanents for enter-the-battlefield value.
    Blink,
    /// Rewards landfall.
    Landfall,
    /// Synergizes with artifacts.
    Artifact,
    /// Synergizes with enchantments.
    Enchantment,
    /// Synergizes with planeswalkers.
    Planeswalker,
    /// Synergizes with counters.
    Counters,
    /// Uses energy counters.
    Energy,
    /// Synergizes with Vehicles.
    Vehicles,
    /// Helps every player.
    GroupHug,
    /// Rewards politics.
    Politics,
    /// Builds one large commander.
    Voltron,
    /// Rewards casting instants and sorceries.
    Spellslinger,
    /// Rewards a creature type.
    Typal,
    /// Spends spare mana.
    ManaSink,
    /// Improves card quality.
    CardSelection,
}

impl Role {
    /// Parse a role word or alias ("draw", "card-draw", "card draw");
    /// `None` when the word names no role.
    #[must_use]
    pub fn parse(s: &str) -> Option<Role> {
        let norm = |w: &str| w.replace(['-', '_', ' '], "");
        let q = norm(s).to_ascii_lowercase();
        Some(match q.as_str() {
            // Card advantage
            "draw" | "carddraw" | "drawcards" | "drawengine" | "cardadvantage" | "cardadv"
            | "wheel" | "wheels" => Role::Draw,
            "cantrip" | "cantrips" | "scry" | "scrying" | "surveil" | "cardselection"
            | "filtering" | "selection" => Role::CardSelection,
            "loot" | "looting" | "rummage" | "rummaging" | "impulse" | "discard"
            | "discardoutlet" | "discardsynergy" => Role::Discard,
            "mill" | "milling" | "millself" => Role::Mill,
            // Mana
            "ramp" | "mana" | "manaacceleration" | "manaramp" => Role::Ramp,
            "landramp" | "manarock" | "rocks" | "rock" | "manadork" | "dorks" | "dork" => {
                Role::Ramp
            }
            "manafixing" | "fixing" | "colors" | "land" | "manabase" => Role::Land,
            "manasink" | "sink" => Role::ManaSink,
            // Interaction
            "removal" | "interaction" | "killspell" | "kill" | "spotremoval" | "bounce"
            | "bouncer" | "tapper" | "tapdown" | "freeze" => Role::Removal,
            "boardwipe" | "wipe" | "wrath" | "wratheffect" | "sweeper" | "boardclear"
            | "massremoval" => Role::BoardWipe,
            "counterspell" | "counters" | "counter" | "countermagic" | "interactioncounters" => {
                Role::Counterspell
            }
            "theft" | "steal" | "stealing" | "gaimcontrol" | "gaincontrol" => Role::Theft,
            "protection" | "protect" | "hexproof" | "ward" | "indestructible" | "prevention"
            | "damageprevention" | "pillowfort" => Role::Protection,
            "hate" | "graveyardhate" | "gyhate" | "antigraveyard" | "restinpeace" | "hatepiece" => {
                Role::GraveyardHate
            }
            "stax" | "tax" | "taxes" | "slug" | "groupslug" | "lock" | "lockpiece" => Role::Stax,
            // Sacrifice / graveyard
            "sacrifice" | "sac" | "sacoutlet" | "sacrificeoutlet" | "aristocrats"
            | "aristocrat" => Role::Sacrifice,
            "reanimate" | "reanimator" | "reanimation" => Role::Reanimate,
            "recursion" | "graveyardrecursion" | "gyrecursion" | "regrowth" | "graveyard"
            | "graveyardmatters" | "gymatters" | "gy" => Role::Recursion,
            // Board presence
            "token" | "tokens" | "tokengenerator" | "gowide" | "widestrate" => Role::Token,
            "anthem" | "anthems" | "pumpall" | "buffall" | "teamboost" => Role::Anthem,
            "equipment" | "equip" | "equips" | "weapons" => Role::Equipment,
            "aura" | "auras" | "enchantments" | "enchantment" | "enchantmentsynergy" => {
                Role::Enchantment
            }
            "evasion" | "flying" | "unblockable" | "menace" | "intimidate" | "fear"
            | "cantbeblocked" => Role::Evasion,
            "combattrick" | "trick" | "tricks" | "pump" | "giantgrowth" | "buff" => {
                Role::CombatTrick
            }
            "burn" | "pinger" | "ping" | "directdamage" | "dmg" | "bolt" => Role::Burn,
            "lifegain" | "life" | "gainlife" | "heal" | "healing" | "lifedrain" | "drain" => {
                Role::Lifegain
            }
            // Tutors and finishers
            "tutor" | "tutors" | "toolbox" | "search" => Role::Tutor,
            "wincon" | "wincons" | "finisher" | "finishers" | "wincondition" | "winconcard"
            | "payoff" | "payoffs" => Role::Wincon,
            "combo" | "combos" | "combopiece" | "combopieces" | "twocardcombo" => Role::Combo,
            "storm" | "stormcount" | "ritual" | "rituals" => Role::Storm,
            "extraturn" | "extraturns" | "timewalk" | "timewalks" => Role::ExtraTurn,
            // Themes
            "blink" | "flicker" | "flickering" | "etb" | "enters" => Role::Blink,
            "landfall" | "lands" | "landsmatter" | "landsythe" => Role::Landfall,
            "artifact" | "artifacts" | "artifactsynergy" | "affinity" => Role::Artifact,
            "planeswalker" | "planeswalkers" | "walkers" | "pw" => Role::Planeswalker,
            "countersmatter" | "countersynergy" | "plusoneplusone" | "experience"
            | "proliferate" => Role::Counters,
            "energy" => Role::Energy,
            "vehicle" | "vehicles" | "crew" => Role::Vehicles,
            "grouphug" | "hug" | "symmetrical" => Role::GroupHug,
            "politics" | "political" | "diplomacy" | "bargaining" => Role::Politics,
            "voltron" | "commanderdamage" | "generaldamage" => Role::Voltron,
            "spellslinger" | "instants" | "sorceries" | "castspells" | "prowess" => {
                Role::Spellslinger
            }
            "typal" | "tribal" | "kindred" | "theme" | "themes" | "lords" => Role::Typal,
            _ => return None,
        })
    }

    /// All names that resolve to this role, for the help line.
    #[must_use]
    pub fn known_names() -> &'static [&'static str] {
        &[
            "draw",
            "cantrip",
            "scry",
            "card-selection",
            "wheel",
            "discard",
            "loot",
            "mill",
            "ramp",
            "mana-rock",
            "mana-dork",
            "mana-sink",
            "land",
            "mana-fixing",
            "removal",
            "board-wipe",
            "wrath",
            "counterspell",
            "bounce",
            "tapper",
            "theft",
            "protection",
            "pillow-fort",
            "hate",
            "graveyard-hate",
            "stax",
            "tax",
            "sacrifice",
            "sac-outlet",
            "aristocrats",
            "reanimate",
            "recursion",
            "graveyard",
            "token",
            "go-wide",
            "anthem",
            "equipment",
            "aura",
            "enchantment",
            "evasion",
            "combat-trick",
            "pump",
            "burn",
            "pinger",
            "lifegain",
            "drain",
            "tutor",
            "toolbox",
            "wincon",
            "finisher",
            "combo",
            "storm",
            "extra-turn",
            "blink",
            "flicker",
            "landfall",
            "lands-matter",
            "artifact",
            "planeswalker",
            "counters-matter",
            "energy",
            "vehicles",
            "group-hug",
            "politics",
            "voltron",
            "spellslinger",
            "typal",
            "tribal",
            "interaction",
        ]
    }

    /// Substrings that mark the role in oracle text (keyword leg).
    #[must_use]
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Role::Draw => &["draw ", "investigate", "surveil"],
            Role::CardSelection => &["scry ", "surveil", "look at the top"],
            Role::Removal => &["destroy target", "exile target", "destroy all creatures"],
            Role::Ramp => &["search your library for a", "add one mana", "add {"],
            Role::Wincon => &["win the game", "each creature you control gets"],
            Role::Counterspell => &["counter target"],
            Role::Land => &["enters tapped", "Add {", "search for a"],
            Role::BoardWipe => &[
                "destroy all",
                "each creature gets",
                "each creature gets -",
                "exile all",
            ],
            Role::Tutor => &["search your library for a", "search your library"],
            Role::Sacrifice => &["sacrifice", "dies", "whenever you sacrifice"],
            Role::Reanimate => &[
                "return target card from your graveyard",
                "put onto the battlefield from your graveyard",
            ],
            Role::Recursion => &[
                "from your graveyard",
                "from a graveyard",
                "return target card",
            ],
            Role::Discard => &["discard", "loot", "rummage"],
            Role::Mill => &["mill", "put the top"],
            Role::Lifegain => &["gain ", "you gain life", "lifelink"],
            Role::Burn => &["deals damage", "damage to any target"],
            Role::Token => &["create", "tokens"],
            Role::Anthem => &["creatures you control get", "get +1/+1"],
            Role::Equipment => &["equip", "equipment"],
            Role::Evasion => &["flying", "menace", "can't be blocked", "unblockable"],
            Role::CombatTrick => &["gets +", "target creature gets"],
            Role::Theft => &["gain control of"],
            Role::Protection => &["hexproof", "indestructible", "prevent", "protection from"],
            Role::Stax => &["players can't", "can't untap", "spells cost", "pay "],
            Role::GraveyardHate => &[
                "exile all",
                "exile target player's graveyard",
                "from a graveyard",
            ],
            Role::Combo => &["win the game"],
            Role::Storm => &["copy", "whenever you cast"],
            Role::Blink => &["exile", "return", "leaves the battlefield"],
            Role::Landfall => &["landfall", "enters the battlefield tapped"],
            Role::Artifact => &["artifact"],
            Role::Enchantment => &["enchantment", "enchant"],
            Role::Planeswalker => &["loyalty"],
            Role::Counters => &["+1/+1 counter", "counter on"],
            Role::Energy => &["energy"],
            Role::Vehicles => &["crew"],
            Role::GroupHug => &["each player draws", "each player may"],
            Role::Politics => &["target opponent", "choose an opponent"],
            Role::Voltron => &["equipped creature gets", "gets +X/+0"],
            Role::Spellslinger => &["whenever you cast", "instant or sorcery"],
            Role::Typal => &["creatures you control of the chosen type"],
            Role::ManaSink => &["add {", "mana of any"],
            Role::ExtraTurn => &["extra turn", "take an additional turn"],
        }
    }

    /// Tagger labels that mark this role (tag leg). Labels are matched as
    /// whole words against the real Tagger vocabulary.
    #[must_use]
    pub fn tag_labels(self) -> &'static [&'static str] {
        match self {
            Role::Draw => &["draw", "wheel", "burst", "engine"],
            Role::CardSelection => &["scry", "surveil"],
            Role::Removal => &["removal", "destroy", "exile"],
            Role::Ramp => &["ramp", "acceleration", "rock", "dork", "fixing"],
            Role::Wincon => &["win", "overrun", "finisher"],
            Role::Counterspell => &["counterspell", "counter"],
            Role::Land => &["fixing", "land", "tapped"],
            Role::BoardWipe => &["sweeper", "destroy"],
            Role::Tutor => &["tutor"],
            Role::Sacrifice => &["sacrifice", "outlet", "death", "martyr"],
            Role::Reanimate => &["reanimate"],
            Role::Recursion => &["recursion", "graveyard", "regrowth"],
            Role::Discard => &["discard", "loot", "impulse"],
            Role::Mill => &["mill"],
            Role::Lifegain => &["lifegain", "drain", "life"],
            Role::Burn => &["burn", "pinger"],
            Role::Token => &["tokens", "token"],
            Role::Anthem => &["anthem", "boost"],
            Role::Equipment => &["equipment", "equip"],
            Role::Evasion => &["evasion", "flying", "unblockable", "menace"],
            Role::CombatTrick => &["trick", "enlarge", "growth"],
            Role::Theft => &["theft"],
            Role::Protection => &["protection", "hexproof", "ward", "prevention", "protects"],
            Role::Stax => &["toll", "slug", "hate"],
            Role::GraveyardHate => &["hate", "exile"],
            Role::Combo => &["win", "combo"],
            Role::Storm | Role::Spellslinger => &["copy", "cast"],
            Role::Blink => &["bounce", "rescue"],
            Role::Landfall => &["landfall", "land"],
            Role::Artifact => &["artifact", "synergy"],
            Role::Enchantment => &["enchantment", "aura"],
            Role::Planeswalker => &["planeswalker"],
            Role::Counters => &["counters", "pp"],
            Role::Energy => &["energy", "counters"],
            Role::Vehicles => &["crew", "vehicle"],
            Role::GroupHug => &["hug", "symmetrical"],
            Role::Politics => &["per-player", "player"],
            Role::Voltron => &["equipment", "power"],
            Role::Typal => &["typal", "count"],
            Role::ManaSink => &["sink"],
            Role::ExtraTurn => &["turn", "extra"],
        }
    }
}

#[cfg(test)]
#[path = "tests/role_tests.rs"]
mod role_tests;
