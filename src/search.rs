/// Structured card filters shared by `query` and `collection query`.
///
/// Built from [`crate::cli::CardFilters`]; all criteria are AND-combined.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CardFilters {
    pub type_: Option<String>,
    pub color: Option<String>,
    pub color_identity: Option<String>,
    pub cmc: Option<NumOp>,
    pub power: Option<NumOp>,
    pub toughness: Option<NumOp>,
    pub rarity: Option<String>,
    pub set: Option<String>,
    pub keyword: Option<String>,
    pub oracle_text: Option<String>,
    pub format: Option<String>,
}

/// Numeric comparison: `<op> <value>`, e.g. `<=3`, `=2`, `>5`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumOp {
    pub op: Cmp,
    pub value: f64,
}

/// Comparison operators, in ascending token length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmp {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
}

impl Cmp {
    /// Build the comparison against a card statistic.
    ///
    /// `lhs` is the card's stat; the returned closure tests the filter
    /// value. Call as `op.check(card_stat)(filter_value)`.
    pub fn check(self, lhs: f64) -> impl Fn(f64) -> bool {
        move |rhs| match self {
            Self::Lt => lhs < rhs,
            Self::Le => lhs <= rhs,
            // Exact float equality: the filter values are plain user
            // decimals (cmc 3, power 2), not computed values, so an
            // epsilon adds nothing and would miss large values.
            Self::Eq => lhs == rhs,
            Self::Ge => lhs >= rhs,
            Self::Gt => lhs > rhs,
        }
    }
}

/// Parse a filter expression like `<=3`, `<3`, `=2`, `>=1.5`, `>4`.
///
/// Whitespace is tolerated; bare numbers mean equality.
///
/// # Errors
/// Fails with a human-readable message for missing value, unknown operator,
/// or unparseable number.
pub fn parse_num_op(s: &str) -> anyhow::Result<NumOp> {
    let s = s.trim();
    let (op, rest) = if let Some(rest) = s.strip_prefix("<=") {
        (Cmp::Le, rest)
    } else if let Some(rest) = s.strip_prefix(">=") {
        (Cmp::Ge, rest)
    } else if let Some(rest) = s.strip_prefix('<') {
        (Cmp::Lt, rest)
    } else if let Some(rest) = s.strip_prefix('>') {
        (Cmp::Gt, rest)
    } else if let Some(rest) = s.strip_prefix('=') {
        (Cmp::Eq, rest)
    } else {
        (Cmp::Eq, s)
    };
    let rest = rest.trim();
    anyhow::ensure!(
        !rest.is_empty(),
        "missing numeric value after operator in {s:?}"
    );
    let value: f64 = rest
        .parse()
        .map_err(|_| anyhow::anyhow!("{rest:?} is not a number (filter was {s:?})"))?;
    Ok(NumOp { op, value })
}

/// True when `haystack` contains `needle` (ASCII case-insensitive).
pub fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

impl CardFilters {
    /// Convert CLI filters into this filter struct, parsing numeric ops.
    ///
    /// # Errors
    /// Propagates [`parse_num_op`] failures with the flag name for context.
    pub fn from_cli(f: &crate::cli::CardFilters) -> anyhow::Result<Self> {
        Ok(Self {
            type_: f.type_.clone(),
            color: f.color.clone(),
            color_identity: f.color_identity.clone(),
            cmc: f
                .cmc
                .as_deref()
                .map(parse_num_op)
                .transpose()
                .map_err(|e| e.context("invalid --cmc filter"))?,
            power: f
                .power
                .as_deref()
                .map(parse_num_op)
                .transpose()
                .map_err(|e| e.context("invalid --power filter"))?,
            toughness: f
                .toughness
                .as_deref()
                .map(parse_num_op)
                .transpose()
                .map_err(|e| e.context("invalid --toughness filter"))?,
            rarity: f.rarity.clone(),
            set: f.set.clone(),
            keyword: f.keyword.clone(),
            oracle_text: f.oracle_text.clone(),
            format: f.format.clone(),
        })
    }

    /// True when no filter is set.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The fields [`CardFilters`] tests against; implemented by card rows and
/// collection-augmented rows alike.
pub trait Filterable {
    fn name(&self) -> &str;
    fn mana_cost(&self) -> &str;
    fn cmc(&self) -> f64;
    fn type_line(&self) -> &str;
    fn colors(&self) -> &str;
    fn color_identity(&self) -> &str;
    fn keywords(&self) -> &str;
    fn power(&self) -> Option<f64>;
    fn toughness(&self) -> Option<f64>;
    fn oracle_text(&self) -> &str;
    fn rarity(&self) -> &str;
    fn set_code(&self) -> &str;
    fn legalities(&self) -> &str;
}

impl CardFilters {
    /// AND-combined filter check.
    pub fn matches(&self, card: &dyn Filterable) -> bool {
        if let Some(t) = &self.type_
            && !contains_ci(card.type_line(), t)
        {
            return false;
        }
        // Colors: card colors must be a subset of the allowed set.
        if let Some(allowed) = &self.color
            && !subset_of(card.colors(), allowed)
        {
            return false;
        }
        if let Some(allowed) = &self.color_identity
            && !subset_of(card.color_identity(), allowed)
        {
            return false;
        }
        if let Some(op) = &self.cmc
            && !op.op.check(card.cmc())(op.value)
        {
            return false;
        }
        // Power/toughness: cards without the stat fail stat filters.
        if let Some(op) = &self.power {
            match card.power() {
                Some(v) if op.op.check(v)(op.value) => {}
                _ => return false,
            }
        }
        if let Some(op) = &self.toughness {
            match card.toughness() {
                Some(v) if op.op.check(v)(op.value) => {}
                _ => return false,
            }
        }
        if let Some(r) = &self.rarity
            && !card.rarity().eq_ignore_ascii_case(r)
        {
            return false;
        }
        if let Some(s) = &self.set
            && !card.set_code().eq_ignore_ascii_case(s)
        {
            return false;
        }
        if let Some(k) = &self.keyword
            && !contains_ci(card.keywords(), k)
        {
            return false;
        }
        if let Some(o) = &self.oracle_text
            && !contains_ci(card.oracle_text(), o)
        {
            return false;
        }
        if let Some(fmt) = &self.format
            && !legal_in(card.legalities(), fmt)
        {
            return false;
        }
        true
    }
}

/// True when every color in the JSON `colors` array is in `allowed` (subset).
fn subset_of(colors_json: &str, allowed: &str) -> bool {
    let allowed = normalize_colors(allowed);
    parse_colors(colors_json)
        .into_iter()
        .all(|c| allowed.contains(&c))
}

/// Parse a `WUBRG`-style flag value into an uppercase set.
fn normalize_colors(s: &str) -> Vec<char> {
    s.to_ascii_uppercase()
        .chars()
        .filter(|c| "WUBRG".contains(*c))
        .collect()
}

/// Parse a JSON color array like `["R","G"]` into chars.
fn parse_colors(colors_json: &str) -> Vec<char> {
    serde_json::from_str::<Vec<String>>(colors_json)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| {
            let mut chars = s.chars();
            let first = chars.next();
            first.filter(|_| chars.next().is_none())
        })
        .collect()
}

/// True when `legalities_json` marks `format` as playable: `legal` or
/// `restricted`. `banned` and `not_legal` fail, as does an unknown format.
pub fn legal_in(legalities_json: &str, format: &str) -> bool {
    let Ok(map) =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(legalities_json)
    else {
        return false;
    };
    let key = format.to_ascii_lowercase();
    matches!(
        map.get(&key).and_then(|v| v.as_str()),
        Some("legal") | Some("restricted")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::CardRow;

    fn row(overrides: impl Fn(&mut CardRow)) -> CardRow {
        let mut r = CardRow {
            name: "Test Card".into(),
            oracle_id: String::new(),
            mana_cost: "{1}{R}".into(),
            cmc: 2.0,
            type_line: "Creature — Human Warrior".into(),
            colors: r#"[ "R" ]"#.into(),
            color_identity: r#"["R"]"#.into(),
            keywords: r#"["Haste","First strike"]"#.into(),
            power: Some("2".into()),
            toughness: Some("3".into()),
            loyalty: None,
            oracle_text: "Haste. When this enters, deal 1 damage to any target.".into(),
            rarity: "uncommon".into(),
            edhrec_rank: Some(500),
            legalities: r#"{"modern":"legal","commander":"legal","legacy":"not_legal"}"#.into(),
            set_code: "TST".into(),
            collector_number: "1".into(),
            scryfall_id: String::new(),
            released_at: String::new(),
            game_changer: None,
        };
        overrides(&mut r);
        r
    }

    fn no_filters() -> CardFilters {
        CardFilters::default()
    }

    #[test]
    fn num_op_parses_all_operators() {
        let cases = [
            ("<=3", Cmp::Le, 3.0),
            ("<3", Cmp::Lt, 3.0),
            ("=2", Cmp::Eq, 2.0),
            (">=1.5", Cmp::Ge, 1.5),
            (">4", Cmp::Gt, 4.0),
            ("7", Cmp::Eq, 7.0),
            (" <= 3 ", Cmp::Le, 3.0),
        ];
        for (input, op, value) in cases {
            let parsed = parse_num_op(input).unwrap_or_else(|e| panic!("{input}: {e}"));
            assert_eq!(parsed.op, op, "{input}");
            assert_eq!(parsed.value, value, "{input}");
        }
    }

    #[test]
    fn num_op_rejects_garbage() {
        for bad in ["", "<", ">=", "abc", "<=three", "< 3x"] {
            assert!(parse_num_op(bad).is_err(), "{bad:?} should fail");
        }
    }

    #[test]
    fn num_op_evaluates() {
        let le3 = parse_num_op("<=3").unwrap();
        assert!(le3.op.check(2.5)(3.0));
        assert!(!le3.op.check(3.5)(3.0));
        let eq2 = parse_num_op("=2").unwrap();
        assert!(eq2.op.check(2.0)(2.0));
        assert!(!eq2.op.check(2.1)(2.0));
    }

    #[test]
    fn type_filter_substring_case_insensitive() {
        let f = CardFilters {
            type_: Some("creature".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        let f = CardFilters {
            type_: Some("Instant".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|_| ())));
    }

    #[test]
    fn color_filter_is_subset() {
        // Card is {R}; WUBRG superset passes, WU does not.
        let f = CardFilters {
            color: Some("WUBRG".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        let f = CardFilters {
            color: Some("WU".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|_| ())));
        let f = CardFilters {
            color: Some("R".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        // Multicolor card fully within the allowance passes.
        let f = CardFilters {
            color: Some("RG".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|r| r.colors = r#"["R","G"]"#.into())));
        // Multicolor card outside the allowance fails.
        let f = CardFilters {
            color: Some("RG".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|r| r.colors = r#"["R","G","U"]"#.into())));
        // Colorless card passes any color subset.
        let f = CardFilters {
            color: Some("W".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|r| r.colors = "[]".into())));
    }

    #[test]
    fn color_identity_filter_is_subset() {
        let f = CardFilters {
            color_identity: Some("R".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        let f = CardFilters {
            color_identity: Some("GU".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|_| ())));
    }

    #[test]
    fn stat_filters_compare_numerics() {
        let f = CardFilters {
            power: Some(parse_num_op(">=2").unwrap()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        let f = CardFilters {
            power: Some(parse_num_op(">2").unwrap()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|_| ())));
        let f = CardFilters {
            toughness: Some(parse_num_op("<=3").unwrap()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        // Missing stat fails the filter.
        let f = CardFilters {
            power: Some(parse_num_op(">=1").unwrap()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|r| r.power = None)));
    }

    #[test]
    fn stat_filters_handle_negative_and_star() {
        // -1 power parses and compares numerically.
        let f = CardFilters {
            power: Some(parse_num_op("<=-1").unwrap()),
            ..no_filters()
        };
        assert!(f.matches(&row(|r| r.power = Some("-1".into()))));
        // Non-numeric stat (*, 1+*) fails the filter rather than panicking.
        let f = CardFilters {
            power: Some(parse_num_op(">=1").unwrap()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|r| r.power = Some("*".into()))));
    }

    #[test]
    fn rarity_set_keyword_oracle_filters() {
        let f = CardFilters {
            rarity: Some("Uncommon".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        let f = CardFilters {
            rarity: Some("rare".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|_| ())));

        let f = CardFilters {
            set: Some("tst".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        let f = CardFilters {
            set: Some("MH3".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|_| ())));

        let f = CardFilters {
            keyword: Some("haste".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        let f = CardFilters {
            keyword: Some("flying".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|_| ())));

        let f = CardFilters {
            oracle_text: Some("DAMAGE".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        let f = CardFilters {
            oracle_text: Some("lifelink".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|_| ())));
    }

    #[test]
    fn format_filter_uses_legality() {
        let f = CardFilters {
            format: Some("modern".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        let f = CardFilters {
            format: Some("legacy".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|_| ())));
        let f = CardFilters {
            format: Some("standard".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|_| ())));
        let f = CardFilters {
            format: Some("pizza".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|r| r.legalities = r#"{"vintage":"restricted"}"#.into())));
        // Restricted still counts as playable.
        let f = CardFilters {
            format: Some("vintage".into()),
            ..no_filters()
        };
        assert!(f.matches(&row(|r| r.legalities = r#"{"vintage":"restricted"}"#.into())));
        let f = CardFilters {
            format: Some("modern".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|r| r.legalities = r#"{"modern":"banned"}"#.into())));
    }

    #[test]
    fn empty_filters_match_everything() {
        assert!(no_filters().matches(&row(|_| ())));
        assert!(no_filters().is_empty());
    }

    #[test]
    fn filters_combine_with_and() {
        let f = CardFilters {
            type_: Some("Creature".into()),
            color: Some("R".into()),
            cmc: Some(parse_num_op("=2").unwrap()),
            ..no_filters()
        };
        assert!(f.matches(&row(|_| ())));
        let f = CardFilters {
            type_: Some("Creature".into()),
            color: Some("G".into()),
            ..no_filters()
        };
        assert!(!f.matches(&row(|_| ())));
    }
}
