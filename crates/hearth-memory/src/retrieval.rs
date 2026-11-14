//! The cheap retrieval primitives.

use std::collections::HashSet;

use crate::query::Term;
use crate::store::VectorRow;

/// The lowest cosine similarity that counts as a semantic match.
const FLOOR: f32 = 0.40;

/// The most terms one index query carries. A longer query is scaffolding.
const MAX_TERMS: usize = 8;

/// Escape one term as an FTS5 string literal.
fn literal(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// An expression requiring every strong term.
///
/// The strongest few terms are used rather than all of them: a requirement that
/// every word of a long message appears makes the query return nothing, and an
/// empty strict query is worse than a broad one.
pub fn match_all(terms: &[Term]) -> Option<String> {
    let mut strongest: Vec<&Term> = terms.iter().filter(|term| term.weight >= 1.0).collect();
    strongest.sort_by(|left, right| {
        right
            .weight
            .total_cmp(&left.weight)
            .then_with(|| left.text.cmp(&right.text))
    });
    let mut seen = HashSet::new();
    let parts: Vec<String> = strongest
        .into_iter()
        .filter(|term| seen.insert(term.text.clone()))
        .take(MAX_TERMS.min(4))
        .map(|term| literal(&term.text))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" AND "))
}

/// An expression matching any term, with prefix matching on the last few.
///
/// Prefix matching is what lets "deploy" reach "deployment" without relying only
/// on the stemmer, and it is confined to the broad query so it never narrows a
/// precise one.
pub fn match_any(terms: &[Term], expanded: &[Term], entities: &[String]) -> Option<String> {
    let mut seen = HashSet::new();
    let mut parts: Vec<String> = Vec::new();
    for entity in entities {
        if entity.split(' ').count() > 1 && seen.insert(entity.clone()) {
            parts.push(literal(entity));
        }
    }
    for term in terms.iter().chain(expanded) {
        if !seen.insert(term.text.clone()) {
            continue;
        }
        if term.weight >= 1.0 {
            parts.push(format!("{}*", literal(&term.text)));
        } else {
            parts.push(literal(&term.text));
        }
        if parts.len() >= MAX_TERMS * 2 {
            break;
        }
    }
    (!parts.is_empty()).then(|| parts.join(" OR "))
}

/// The dot product of a quantised row and a quantised query.
///
/// Both sides are unit length and scaled to 127, so the result divided by
/// `127 * 127` is the cosine similarity.
pub fn dot(row: &[i8], query: &[i8]) -> i32 {
    row.iter()
        .zip(query)
        .map(|(left, right)| i32::from(*left) * i32::from(*right))
        .sum()
}

/// Quantise a query vector the same way a stored vector was quantised.
pub fn quantise(values: &[f32]) -> Vec<i8> {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    let denominator = if norm > 0.0 { norm } else { 1.0 };
    values
        .iter()
        .map(|value| (value / denominator * 127.0).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

/// The best current rows for a query vector.
///
/// A linear scan is the right structure here: a personal store holds thousands of
/// rows, not millions, and a scan of INT8 vectors is a few milliseconds where an
/// approximate index would add a build step, a tuning parameter, and a recall
/// loss.
pub fn top_vectors(rows: &[VectorRow], query: &[i8], limit: usize) -> Vec<String> {
    // Cosine floor. Both sides are unit length and scaled to 127, so a dot
    // product below this is a weak association that would outrank a real
    // keyword hit purely by being first in its own channel.
    let floor = (FLOOR * 127.0 * 127.0) as i32;
    let mut best: Vec<(i32, &str)> = Vec::with_capacity(limit + 1);
    for row in rows {
        if !row.current || row.quantised.len() != query.len() {
            continue;
        }
        let score = dot(&row.quantised, query);
        if score < floor {
            continue;
        }
        if best.len() < limit {
            best.push((score, row.memory_id.as_str()));
            if best.len() == limit {
                best.sort_by_key(|entry| std::cmp::Reverse(entry.0));
            }
            continue;
        }
        if score <= best[limit - 1].0 {
            continue;
        }
        best[limit - 1] = (score, row.memory_id.as_str());
        let mut index = limit - 1;
        while index > 0 && best[index].0 > best[index - 1].0 {
            best.swap(index, index - 1);
            index -= 1;
        }
    }
    best.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    best.into_iter().map(|(_, id)| id.to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(text: &str, weight: f32) -> Term {
        Term {
            text: text.to_owned(),
            weight,
        }
    }

    #[test]
    fn the_strict_query_requires_the_strongest_terms() {
        let terms = vec![term("alice", 1.6), term("boss", 1.0), term("we", 0.4)];
        let expression = match_all(&terms).expect("expression");
        assert!(expression.contains("\"alice\""), "{expression}");
        assert!(expression.contains(" AND "), "{expression}");
        assert!(!expression.contains("we"), "{expression}");
    }

    #[test]
    fn the_broad_query_prefixes_written_terms_and_quotes_expansions() {
        let terms = vec![term("deploy", 1.0)];
        let expanded = vec![term("release", 0.4)];
        let expression = match_any(&terms, &expanded, &[]).expect("expression");
        assert!(expression.contains("\"deploy\"*"), "{expression}");
        assert!(expression.contains("\"release\""), "{expression}");
        assert!(!expression.contains("\"release\"*"), "{expression}");
    }

    #[test]
    fn a_name_is_searched_as_a_phrase() {
        let expression = match_any(&[], &[], &["or board".to_owned()]).expect("expression");
        assert_eq!(expression, "\"or board\"");
    }

    #[test]
    fn the_vector_scan_keeps_only_current_rows() {
        let rows = vec![
            VectorRow {
                memory_id: "old".to_owned(),
                current: false,
                quantised: vec![127, 0],
            },
            VectorRow {
                memory_id: "live".to_owned(),
                current: true,
                quantised: vec![127, 0],
            },
        ];
        assert_eq!(top_vectors(&rows, &[127, 0], 4), vec!["live".to_owned()]);
    }

    #[test]
    fn the_scan_orders_by_similarity() {
        let rows = vec![
            VectorRow {
                memory_id: "far".to_owned(),
                current: true,
                quantised: vec![90, 90],
            },
            VectorRow {
                memory_id: "near".to_owned(),
                current: true,
                quantised: vec![127, 20],
            },
        ];
        assert_eq!(
            top_vectors(&rows, &quantise(&[1.0, 0.1]), 2),
            vec!["near".to_owned(), "far".to_owned()]
        );
    }

    #[test]
    fn a_weak_vector_match_is_not_a_candidate() {
        let rows = vec![VectorRow {
            memory_id: "unrelated".to_owned(),
            current: true,
            quantised: vec![0, 127],
        }];
        assert!(top_vectors(&rows, &quantise(&[1.0, 0.0]), 4).is_empty());
    }
}
