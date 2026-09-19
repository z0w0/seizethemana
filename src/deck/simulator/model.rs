// Card data model for the goldfish simulator: costs, tap yields, station
// tiers, crew, and abilities. Pure data + small helpers; parsing lives in
// `parse`, execution in `game`.
//
// Everything the model cannot express is dropped at parse time; the
// documented limits ship in the output `assumptions` list (see `report`).

/// Colors tracked for mana modeling, WUBRG order.
pub const COLORS: [char; 5] = ['W', 'U', 'B', 'R', 'G'];

/// Default number of simulated games: ±0.5pp on percentages at 10k runs.
pub const DEFAULT_RUNS: u32 = 10_000;

/// Functional roles the simulator classifies each card into. One role per
/// card (first match wins) so role counts never double-count copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Role {
    /// Land (basic or nonbasic); one mana per turn when in play.
    Land,
    /// Nonland artifact that produces mana (Sol Ring, Arcane Signet).
    Rock,
    /// Creature that produces mana (Llanowar Elves, Birds of Paradise).
    Dork,
    /// Spell that makes mana or searches lands (rituals, ramp, fetches).
    RampSpell,
    /// One-shot or repeatable card-draw source.
    Draw,
    /// Targeted removal, counterspell, or board wipe.
    Removal,
    /// Static tax/restriction piece (Rishadan Port, Static Orb, Thalia):
    /// stax timing is the role-access question.
    Lock,
    /// Equipment, aura, or pump spell that suits up a threat (Voltron).
    Booster,
    /// Card the deck intends to win with: big threats and finishers.
    Wincon,
    /// Everything else.
    #[default]
    Other,
}

/// Mana a card asks for when casting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cost {
    /// Generic-mana value (the `{N}` parts; `{X}` counts as 1).
    pub generic: u32,
    /// Monocolor pips per WUBRG letter.
    pub pips: [u8; 5],
    /// Hybrid/phyrexian pips, payable from any of their colors.
    pub flex_pips: u32,
}

impl Cost {
    /// Total mana value (generic plus every pip).
    pub fn total(&self) -> u32 {
        self.generic + self.pips.iter().map(|p| u32::from(*p)).sum::<u32>() + self.flex_pips
    }
}

/// What one tap of a permanent yields.
///
/// Real cards fall into three shapes: an "or" choice (`Add {G} or {U}` or
/// "one mana of any color"), a fixed simultaneous set (`Add {W}{U}{B}{R}{G}`
/// on Jegantha), and colorless-only (`{C}`). Each is one tap = the listed
/// mana, never several independent sources.
#[derive(Debug, Clone, Default)]
pub struct TapYield {
    /// Fixed simultaneous pips produced on every tap (Jegantha).
    pub fixed: [u8; 5],
    /// Colors this source can choose from when tapped (choice sources).
    pub choice: [bool; 5],
    /// Any-color pips produced per tap (Command Tower 1, Gilded Lotus 3).
    pub any_pips: u32,
    /// True when the "any color" clause depends on an opponent's board
    /// ("any color a land an opponent controls could produce" — Fellwar
    /// Stone). Goldfish: yields like `any_pips` from turn 2, nothing on
    /// turn 1.
    pub opponent_any: bool,
    /// Conditional scaling: the tap's output grows with board state.
    pub scaling: Option<Scale>,
    /// Colorless-only production (`{C}`).
    pub colorless: u32,
    /// True when merged from several separate tap abilities: one tap
    /// yields one mana of any reachable color, never the sum
    /// (Plaza of Heroes, Verge lands).
    pub alternatives: bool,
    /// Spend restriction: the mana may only pay for certain casts
    /// ("spend this mana only to cast creature spells" — Secluded
    /// Courtyard; "…a legendary spell" — Plaza of Heroes).
    pub restriction: Option<Restriction>,
}

/// Spend-restriction classes the pool can honor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restriction {
    /// Creature spells only (Secluded Courtyard).
    Creature,
    /// Artifact spells only (Steelswarm Operator).
    Artifact,
    /// Legendary spells only (Plaza of Heroes).
    Legendary,
    /// Instant and sorcery spells only (Hydro-Channeler).
    InstantSorcery,
}

/// Board-state growth of a tap yield.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    /// One any-color pip per color among permanents you control
    /// (Faeburrow Elder, Bloom Tender, Plaza of Heroes' legendary mode).
    ColorsPresent,
    /// One any-color pip per charge counter on the source
    /// (Astral Cornucopia, Crystalline Crawler).
    PerChargeCounter,
}

impl TapYield {
    /// Total mana produced by one tap. Alternative-mode sources (several
    /// tap abilities on one permanent) yield one mana, never the sum.
    pub fn total(&self) -> u32 {
        if self.alternatives {
            let reachable = self.any_pips > 0 || self.choice.iter().any(|c| *c);
            return u32::from(reachable || self.colorless > 0);
        }
        self.fixed.iter().map(|p| u32::from(*p)).sum::<u32>()
            + self.any_pips
            + u32::from(self.choice.iter().any(|c| *c))
            + self.colorless
    }
}

/// What starts an ability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Trigger {
    /// Activated: the player pays `cost` to fire it (incl. `{T}` costs).
    Activated,
    /// Fired when the card enters the battlefield.
    #[default]
    OnEnter,
    /// Fired once per turn while the card is on the battlefield.
    OnUpkeep,
    /// Fired when the card attacks.
    OnAttack,
    /// Fired when the card deals combat damage to a player (best case:
    /// every attacker connects). Thrummingbird proliferate, draw, drain.
    OnCombatDamage,
    /// Fired when another spell is cast (Y'shtola, Vivi, Jhoira).
    OnCastSpell,
    /// Fired when the permanent dies or is sacrificed.
    OnDeath,
}

/// What an ability does when it resolves.
#[derive(Debug, Clone, Default)]
pub enum Effect {
    /// No modeled effect (ability exists but does nothing in the sim).
    #[default]
    None,
    /// Draw N cards.
    Draw(u32),
    /// Search for a card and put it into the hand (tutors).
    Tutor,
    /// Produce mana (activated mana abilities, planets at threshold).
    Mana(TapYield),
    /// Produce `counters × N` mana at upkeep, then clear the host's
    /// charge counters (banked-mana engines: Coalition Relic).
    ManaPerCounter(TapYield),
    /// Create token creatures (ETB tokens, crewed payoffs). The count is
    /// advisory: the sim tracks bodies, not individual tokens.
    Tokens(u32),
    /// Put N charge counters on this permanent (counter injection).
    Counters(u32),
    /// Extra land drop this turn (explore-style effects).
    ExtraLand,
    /// Put the top N cards of the library into the graveyard. Milled
    /// cards count as cards seen (velocity) and fill the graveyard log.
    Mill(u32),
    /// Return cards from the graveyard: to the hand (`to_hand`, draw
    /// credit) or the battlefield (a body, once per card).
    ReturnFromGraveyard {
        /// True: cards go to the hand (counts as cards seen).
        to_hand: bool,
        /// How many cards to return.
        count: u32,
    },
    /// Discard the hand into the graveyard, then draw that many (a
    /// wheel). Cards seen jump by the hand size; the graveyard log fills.
    Wheel,
    /// Draw N, then discard N (loot). Net velocity +N; the graveyard
    /// log fills with the discards.
    Loot(u32),
    /// Look at the top N cards (scry, surveil). Zero draw credit: the
    /// cards feed `library_awareness_by_turn` instead. Surveil also
    /// puts them in the graveyard (best case for graveyard decks).
    Scry(u32),
    /// Take an extra turn after this one (Time Sieve class). Queued
    /// and replayed by the turn loop; never chained mid-turn.
    ExtraTurn,
    /// Upkeep threshold engine: when the host holds `counters` charge
    /// counters it wins (Darksteel Reactor, Helix Pinnacle class).
    WinThreshold {
        /// Counters needed to win.
        counters: u32,
    },
    /// Lose N life (drain, burn at a player). Opponent-scoped phrasing
    /// ("target player", "each opponent") maps here; creature-target
    /// burn does not.
    Drain(u32),
}

/// One executable ability: what fires it, what it costs, what it does.
#[derive(Debug, Clone, Default)]
pub struct Ability {
    /// What fires the ability: an activation, a trigger event, or a tap.
    pub trigger: Trigger,
    /// Mana to pay when activating (0 when free / triggered).
    pub cost: Cost,
    /// Effect when the ability resolves.
    pub effect: Effect,
    /// Consumes the source's tap (mana abilities and tap-activated draws).
    pub taps: bool,
    /// Consumes one charge counter per fire (Pentad Prism's "remove a
    /// charge counter: add one mana of any color"). The source stays
    /// untapped and fires once per turn while counters last.
    pub uses_counters: bool,
    /// Bodies sacrificed as part of the cost (aristocrats outlets). The
    /// activation consumes an untapped body and fires its OnDeath
    /// triggers.
    pub sacrifice_bodies: u32,
    /// Loyalty spent to activate (planeswalker abilities). 0 = not a
    /// loyalty activation.
    pub loyalty_cost: u32,
}

/// A station tier: abilities unlocked at a charge-counter threshold.
///
/// Rule source: CR 702.184/721 — a station card's text box has one or two
/// striations, each led by an `{N+}` symbol meaning "as long as this
/// permanent has N or more charge counters, it has [abilities]" and, when a
/// P/T box is printed in the same striation, "…and is a creature with base
/// P/T". Planets never animate (no P/T box); a spacecraft animates only at
/// the tier whose striation carries the P/T box.
#[derive(Debug, Clone, Default)]
pub struct Tier {
    /// Charge-counter threshold (`12+` → 12).
    pub at: u32,
    /// True when this tier's striation also animates the card as a creature.
    pub animate: bool,
    /// Abilities unlocked at this tier (draw engines, mana taps).
    pub abilities: Vec<Ability>,
}

/// The card-level capabilities the simulator executes.
#[derive(Debug, Clone, Default)]
pub struct SimCard {
    /// Card name (matches the deck entry).
    pub name: String,
    /// Mana cost to cast (lands cost 0 and enter by the land rule).
    pub cost: Cost,
    /// Cheapest castable mana value after reductions (warp, affinity-lite).
    pub min_cost: Cost,
    /// Mana this card produces on tap; None when it cannot tap for mana.
    pub tap: Option<TapYield>,
    /// Station tiers, in threshold order (empty when not a station card).
    pub station_tiers: Vec<Tier>,
    /// Crew cost for Vehicles (None when not a Vehicle).
    pub crew: Option<u32>,
    /// Creature-ness for body counting and dork timing.
    pub is_creature: bool,
    /// True when the card is a Spacecraft or Planet (stationable type).
    pub is_station_card: bool,
    /// Charge counters gained on entering (Reckoner Bankbuster).
    pub enter_counters: u32,
    /// Functional role.
    pub role: Role,
    /// Enters the battlefield tapped (Restless Reef, planets, most duals).
    pub enters_tapped: bool,
    /// Basic types this land checks for (verge gates: "control a Swamp").
    pub gate_types: Vec<&'static str>,
    /// True when the card may begin the game on the battlefield (Leyline).
    pub opens_in_play: bool,
    /// True when the card is a saga (staged per-turn effects).
    pub is_saga: bool,
    /// One-shot charge-counter injection on cast (Drill Too Deep).
    pub counters_on_cast: u32,
    /// One-shot mana on cast (rituals), never joining the tap pool.
    pub mana_on_cast: Option<TapYield>,
    /// One-shot draw on cast (cantrips, Divination).
    pub draws_on_cast: u32,
    /// One-shot mill on cast or on entering ("mill N").
    pub mills_on_enter: u32,
    /// One-shot scry/surveil on cast. Awareness credit, not draw.
    pub scry_on_cast: u32,
    /// True when the one-shot look effect is surveil (the scry'd cards
    /// go to the graveyard — best case for graveyard decks).
    pub surveils: bool,
    /// Mill effects target opponents ("target player mills N") instead
    /// of the deck's own library (deck-out pressure direction).
    pub mills_opponent: bool,
    /// One-shot extra turn on cast ("take an extra turn").
    pub extra_turns_on_cast: bool,
    /// One-shot life loss at a player on cast (burn, drain).
    pub drain_on_cast: u32,
    /// Printed power (creatures); crew and station use it instead of the
    /// flat body power. Tokens and unknowns stay flat.
    pub printed_power: Option<u32>,
    /// Starting loyalty for planeswalkers; activations spend it.
    pub starting_loyalty: Option<u32>,
    /// Cost cuts scale with the artifact count on the board (improvise,
    /// affinity): the discount grows as artifacts enter, instead of the
    /// parse-time flat −2.
    pub board_discount: bool,
    /// Printed colors of the card (subset of WUBRG, by index). Powers
    /// "one mana per color among permanents you control" scaling.
    pub colors: [bool; 5],
    /// Treasure tokens created per token effect (Stark Industries
    /// Executive). Each treasure is one banked any-color pip, sacrificed
    /// to use; the bank lives on the game state, not the card.
    pub treasures_on_token: bool,
    /// Static mana grant while on the battlefield ("creatures you control
    /// have {T}: add one mana of any color" — Enduring Vitality;
    /// "lands you control have…" — Chromatic Lantern). Each matching
    /// permanent adds one flexible pip per turn, capped at 2.
    pub grant: Option<Grant>,
    /// Static creature buff while on the battlefield ("creatures you
    /// control get +2/+2"): (power, toughness). Power joins the attack
    /// sum.
    pub buff: Option<(i32, i32)>,
    /// Equipment stats: (equip cost, equipped-creature buff, draws when
    /// the equipped creature dies). None when not Equipment.
    pub equipment: Option<Equipment>,
    /// Attack power ×2 (double strike). Goldfish: no blockers, so the
    /// first-strike layer is pure damage multiplication.
    pub double_strike: bool,
    /// +1 power per noncreature spell cast this turn (prowess),
    /// credited in the combat phase of the same turn.
    pub prowess: bool,
    /// +1 power per land drop made after this permanent entered
    /// (landfall +1/+1 counter patterns).
    pub landfall: bool,
    /// Evasion census flag (trample, flying, menace): counted in the
    /// attack block, no math.
    pub evasion: bool,
    /// Castable at instant speed (Instant type or flash). Powers the
    /// interaction-readiness metric.
    pub is_instant_speed: bool,
    /// Interaction role (removal or counterspells): feeds readiness.
    pub is_interaction: bool,
}

/// One Equipment: the suit-up cost, the buff, and the Skullclamp-style
/// death-draw rider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Equipment {
    /// Equip cost (mana).
    pub cost: u32,
    /// Equipped-creature buff (power, toughness).
    pub buff: (i32, i32),
    /// Cards drawn when the equipped creature dies.
    pub death_draws: u32,
}

/// One static mana grant: the permanent class it empowers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// Lands you control each tap for one any-color pip.
    Lands,
    /// Creatures you control each tap for one any-color pip.
    Creatures,
}

impl SimCard {
    /// Abilities from the base card (tier 0) plus unlocked tiers.
    pub fn abilities(&self) -> impl Iterator<Item = &Ability> {
        self.station_tiers.iter().flat_map(|t| t.abilities.iter())
    }

    /// The station tier this card animates at, when any (spacecraft only).
    pub fn animate_at(&self) -> Option<u32> {
        self.station_tiers.iter().find(|t| t.animate).map(|t| t.at)
    }
}

/// Formats the simulator knows.
///
/// Pure library shape: how big the deck is and whether a command zone
/// exists. Per-format behavior (mulligan policy, turn defaults) comes
/// from the format rules table ([`super::format`]), not from branches on
/// this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Commander, brawl, oathbreaker: 100-card singleton with a commander.
    Commander,
    /// Any 60-card constructed format.
    Constructed,
}

/// A deck ready to simulate: the library plus the command-zone commander.
#[derive(Debug)]
pub struct SimDeck {
    /// Cards in the library (commander excluded for commander decks).
    pub cards: Vec<SimCard>,
    /// Commander card(s) starting in the command zone.
    pub commanders: Vec<SimCard>,
    /// Library shape (command-zone singleton vs constructed).
    pub format: Format,
    /// Per-format behavior: mulligan policy and turn defaults.
    pub rules: super::format::FormatRules,
}

/// Cards drawn when a draw spell resolves: numerals win ("draw two cards"),
/// otherwise 1.
pub fn draw_amount(text: &str) -> u32 {
    if !text.contains("draw ") && !text.contains("investigate") {
        return 0;
    }
    for word in ["five", "four", "three", "two", "5", "4", "3", "2"] {
        if text.contains(&format!("draw {word}")) || text.contains(&format!("draws {word}")) {
            return match word {
                "two" => 2,
                "three" => 3,
                "four" => 4,
                _ => 5,
            };
        }
    }
    1
}

/// Number-word and numeral amounts for generic clauses ("scry 2",
/// "look at the top three cards"). Returns 1 when present but uncounted.
pub fn amount_after(text: &str, needle: &str) -> u32 {
    let mut from = 0;
    while let Some(rel) = text[from..].find(needle) {
        let tail = &text[from + rel + needle.len()..];
        let digits: String = tail
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<u32>() {
            return n;
        }
        for (word, n) in [
            ("seven", 7u32),
            ("six", 6),
            ("five", 5),
            ("four", 4),
            ("three", 3),
            ("two", 2),
        ] {
            if tail.trim_start().starts_with(word) {
                return n;
            }
        }
        from += rel + needle.len();
    }
    1
}
