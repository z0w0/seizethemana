use std::collections::HashSet;

use seizethemana::{db, embed, tags};

use super::model::Candidate;

/// Card-document layouts that can be compared without changing card facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DocumentFormat {
    /// Production layout from `embed::doc_for_row`.
    Production,
    /// One explicit label per field, with line breaks between fields.
    Fielded,
    /// The same labeled fields on one compact line.
    Compact,
    /// Put rules text before secondary fields.
    RulesFirst,
    /// Keep name and type immediately before rules text.
    NameTypeRulesFirst,
    /// Put role tags before other card facts.
    TagsFirst,
    /// Put tags last after the rules text.
    TagsLast,
    /// Place card identity next to rules text in short prose.
    Contextual,
}

impl DocumentFormat {
    /// Stable cache and report key for this layout.
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Fielded => "fielded",
            Self::Compact => "compact",
            Self::RulesFirst => "rules-first",
            Self::NameTypeRulesFirst => "name-type-rules-first",
            Self::TagsFirst => "tags-first",
            Self::TagsLast => "tags-last",
            Self::Contextual => "contextual",
        }
    }
}

/// Select document layouts in input order, defaulting to the production layout.
pub(super) fn select_document_formats(only: Option<&str>) -> anyhow::Result<Vec<DocumentFormat>> {
    let Some(formats) = only else {
        return Ok(vec![DocumentFormat::Production]);
    };
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for key in formats
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        anyhow::ensure!(
            seen.insert(key),
            "document format {key:?} was selected twice"
        );
        let format = match key {
            "production" => DocumentFormat::Production,
            "fielded" => DocumentFormat::Fielded,
            "compact" => DocumentFormat::Compact,
            "rules-first" => DocumentFormat::RulesFirst,
            "name-type-rules-first" => DocumentFormat::NameTypeRulesFirst,
            "tags-first" => DocumentFormat::TagsFirst,
            "tags-last" => DocumentFormat::TagsLast,
            "contextual" => DocumentFormat::Contextual,
            _ => anyhow::bail!(
                "unknown document format {key:?}; expected production, fielded, compact, rules-first, name-type-rules-first, tags-first, tags-last, or contextual"
            ),
        };
        selected.push(format);
    }
    anyhow::ensure!(!selected.is_empty(), "--format needs at least one layout");
    Ok(selected)
}

/// Render card facts using one stable document layout.
pub(super) fn render_document(
    format: DocumentFormat,
    card: &db::CardRow,
    tag_index: &tags::TagIndex,
) -> String {
    if format == DocumentFormat::Production {
        return embed::doc_for_row(card, tag_index);
    }
    let keywords: Vec<String> = serde_json::from_str(&card.keywords).unwrap_or_default();
    let keywords = if keywords.is_empty() {
        "none".to_string()
    } else {
        keywords.join(", ")
    };
    let colors: Vec<String> = serde_json::from_str(&card.colors).unwrap_or_default();
    let colors = colors
        .iter()
        .filter_map(|color| color.chars().next())
        .collect::<String>();
    let colors = if colors.is_empty() {
        "colorless".to_string()
    } else {
        colors
    };
    let tags = tag_index
        .doc_tags_line(&card.oracle_id)
        .strip_prefix("Tags: ")
        .unwrap_or("none")
        .to_string();
    let stats = match (card.power.as_deref(), card.toughness.as_deref()) {
        (Some(power), Some(toughness)) => Some(format!("P/T: {power}/{toughness}")),
        _ => card
            .loyalty
            .as_deref()
            .map(|loyalty| format!("Loyalty: {loyalty}")),
    };
    let mut fields = vec![
        format!("Name: {}", card.name),
        format!("Mana cost: {}", card.mana_cost),
        format!("Type: {}", card.type_line),
        format!("Keywords: {keywords}"),
        format!("Colors: {colors}"),
        format!("Tags: {tags}"),
    ];
    if let Some(stats) = stats {
        fields.push(stats);
    }
    fields.push(format!("Rules text: {}", card.oracle_text));
    let rules_index = fields
        .iter()
        .position(|field| field.starts_with("Rules text:"))
        .expect("the rendered card fields include rules text");
    match format {
        DocumentFormat::Production => embed::doc_for_row(card, tag_index),
        DocumentFormat::Fielded => fields.join("\n"),
        DocumentFormat::Compact => fields.join(" | "),
        DocumentFormat::RulesFirst => {
            let mut ordered = vec![fields[rules_index].clone(), fields[0].clone()];
            ordered.extend(
                fields
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        *index != 0 && *index != 2 && *index != rules_index && *index != 5
                    })
                    .map(|(_, field)| field.clone()),
            );
            ordered.push(fields[2].clone());
            ordered.push(fields[5].clone());
            ordered.join("\n")
        }
        DocumentFormat::NameTypeRulesFirst => {
            let mut ordered = vec![
                fields[0].clone(),
                fields[2].clone(),
                fields[rules_index].clone(),
            ];
            ordered.extend(
                fields
                    .iter()
                    .enumerate()
                    .filter(|(index, field)| {
                        *index != 0
                            && *index != 2
                            && *index != rules_index
                            && !field.starts_with("Tags:")
                    })
                    .map(|(_, field)| field.clone()),
            );
            ordered.push(fields[5].clone());
            ordered.join("\n")
        }
        DocumentFormat::TagsFirst => {
            let mut ordered = vec![fields[5].clone()];
            ordered.extend(fields[..5].iter().cloned());
            ordered.extend(fields[6..].iter().cloned());
            ordered.join("\n")
        }
        DocumentFormat::TagsLast => {
            let mut ordered = fields[..5].to_vec();
            ordered.extend(fields[6..].iter().cloned());
            ordered.push(fields[5].clone());
            ordered.join("\n")
        }
        DocumentFormat::Contextual => {
            let name = card.name.as_str();
            format!(
                "{name} · {}\n{}\n{}",
                card.type_line,
                card.oracle_text,
                fields[1..rules_index].join("\n")
            )
        }
    }
}

/// Apply model-specific document instructions after rendering card fields.
pub(super) fn candidate_document(
    candidate: &Candidate,
    format: DocumentFormat,
    card: &db::CardRow,
    tag_index: &tags::TagIndex,
) -> String {
    let document = render_document(format, card, tag_index);
    format!("{}{document}", candidate.document_prompt)
}
