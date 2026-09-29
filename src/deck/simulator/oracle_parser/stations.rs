//! Parse station reminder text into typed striations.

use super::super::oracle_ast::{OracleStations, OracleStriation};

/// Parse station striations and the card's station type metadata.
pub(super) fn parse_stations(oracle_text: &str, type_line: &str) -> OracleStations {
    let mut stations = OracleStations {
        is_station_card: type_line.contains("Spacecraft") || type_line.contains("Planet"),
        has_station_ability: oracle_text.to_ascii_lowercase().contains("station"),
        striations: Vec::new(),
    };
    for line in oracle_text.lines() {
        let Some((head, rest)) = line.trim().split_once('|') else {
            continue;
        };
        let Some(at) = head.trim().trim_end_matches('+').trim().parse::<u32>().ok() else {
            continue;
        };
        let abilities = rest
            .split('\n')
            .filter_map(|part| super::activation::parse_oracle_activated_ability(part.trim()))
            .collect();
        stations.striations.push(OracleStriation {
            at,
            animate: false,
            abilities,
        });
    }
    if let Some(animate_at) = animate_threshold(oracle_text) {
        if let Some(striation) = stations
            .striations
            .iter_mut()
            .find(|striation| striation.at == animate_at)
        {
            striation.animate = true;
        } else {
            stations.striations.push(OracleStriation {
                at: animate_at,
                animate: true,
                abilities: Vec::new(),
            });
        }
    }
    stations.striations.sort_by_key(|striation| striation.at);
    stations
}

/// Find the spacecraft animation threshold in its reminder text.
fn animate_threshold(oracle_text: &str) -> Option<u32> {
    let lower = oracle_text.to_ascii_lowercase();
    let index = lower.find("artifact creature at ")?;
    let tail = &lower[index + "artifact creature at ".len()..];
    let number: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    number.parse::<u32>().ok()
}
