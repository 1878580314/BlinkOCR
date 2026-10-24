//! Xiaomi-OCR-only supplemental text matching. Never rewrites the model's table.
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

static TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"</?(?:fcel|ecel|lcel|xcel|ucel|nl)>").unwrap());
static NUMBERS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[+-]?(?:\d+(?:\.\d+)?|\.\d+)(?:[%‰])?").unwrap());
static SEGMENTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\s,，、;；：:。!?！？]+").unwrap());
const NEGATIONS: &str = "不未无非勿否禁";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Supplement {
    pub text: String,
    pub source: String,
    pub conflict: bool,
}

pub fn normalize(text: &str) -> String {
    let text = text
        .nfkc()
        .collect::<String>()
        .replace('−', "-")
        .replace("<nl>", "\n");
    TAGS.replace_all(&text, "\t")
        .chars()
        .filter_map(|ch| {
            if ch == '\n' {
                Some(ch)
            } else if ch.is_whitespace() || ch == '|' {
                Some('\t')
            } else if ch.is_alphanumeric() || "+-.%‰/≤≥=±".contains(ch) {
                Some(ch)
            } else {
                None
            }
        })
        .collect()
}

fn compact(text: &str) -> String {
    text.replace('\t', "")
}
fn numbers(text: &str) -> Vec<&str> {
    NUMBERS.find_iter(text).map(|m| m.as_str()).collect()
}
fn negations(text: &str) -> String {
    text.chars().filter(|ch| NEGATIONS.contains(*ch)).collect()
}

fn mapped(text: &str) -> (String, Vec<usize>) {
    let mut flat = String::new();
    let mut positions = Vec::new();
    for (offset, ch) in text.char_indices().filter(|(_, ch)| *ch != '\t') {
        flat.push(ch);
        positions.extend(offset..offset + ch.len_utf8());
    }
    (flat, positions)
}

fn valid_span(norm: &str, row: &str, start: usize, end: usize) -> bool {
    let before = row[..start].chars().next_back();
    let after = row[end..].chars().next();
    let first = norm.chars().next().unwrap_or(' ');
    let last = norm.chars().next_back().unwrap_or(' ');
    let negative_prefix = before.is_some_and(|c| NEGATIONS.contains(c));
    let partial_number = (first.is_ascii_digit()
        && before.is_some_and(|c| c.is_ascii_digit() || "+-.".contains(c)))
        || (last.is_ascii_digit()
            && after.is_some_and(|c| c.is_ascii_digit() || ".%‰".contains(c)));
    !negative_prefix && !partial_number && numbers(norm) == numbers(&row[start..end])
}

fn exact_ranges(norm: &str, row: &str) -> Vec<(usize, usize)> {
    let needle = compact(norm);
    if needle.is_empty() {
        return Vec::new();
    }
    let (flat, map) = mapped(row);
    flat.match_indices(&needle)
        .filter_map(|(pos, matched)| {
            let end = pos + matched.len();
            valid_span(norm, row, map[pos], map[end - 1] + 1).then_some((pos, end))
        })
        .collect()
}

pub fn covered(norm: &str, base: &str) -> bool {
    base.lines()
        .any(|row| !exact_ranges(norm, row).is_empty() || echo(norm, row))
}

fn subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle
        .chars()
        .all(|ch| chars.by_ref().any(|candidate| candidate == ch))
}

fn echo(norm: &str, row: &str) -> bool {
    let needle = compact(norm);
    let size = needle.chars().count();
    if size < 6 || !needle.chars().any(|ch| ('一'..='鿿').contains(&ch)) {
        return false;
    }
    let head: String = needle.chars().take(2).collect();
    let tail: String = needle.chars().skip(size - 2).collect();
    let (flat, map) = mapped(row);
    for (start, _) in flat.match_indices(&head) {
        let offsets: Vec<_> = flat[start..]
            .char_indices()
            .map(|(i, _)| start + i)
            .chain(std::iter::once(flat.len()))
            .collect();
        for extra in 1..=3 {
            let Some(&end) = offsets.get(size + extra) else {
                continue;
            };
            let window = &flat[start..end];
            if window.ends_with(&tail)
                && negations(window) == negations(&needle)
                && valid_span(norm, row, map[start], map[end - 1] + 1)
                && subsequence(&needle, window)
            {
                return true;
            }
        }
    }
    false
}

fn conflict(norm: &str, row: &str) -> bool {
    let skeleton = |text: &str| {
        NUMBERS
            .replace_all(text, "#")
            .chars()
            .filter(|ch| *ch != '\t' && !NEGATIONS.contains(*ch))
            .collect::<String>()
    };
    let (a, b) = (skeleton(norm), skeleton(row));
    !a.is_empty()
        && (a == b
            || (a.contains('#') && b.contains(&a))
            || (negations(norm) != negations(row)
                && a.replace('#', "").chars().count() >= 4
                && subsequence(&a.replace('#', ""), &b.replace('#', ""))))
}

fn in_order(segments: &[(String, String)], row: &str) -> bool {
    let mut end = 0;
    for (_, norm) in segments {
        let Some(pair) = exact_ranges(norm, row)
            .into_iter()
            .find(|(start, _)| *start >= end)
        else {
            return false;
        };
        end = pair.1;
    }
    true
}

fn complete_rows_covered(norm: &str, rows: &[String]) -> bool {
    let needle = compact(norm);
    for start in 0..rows.len() {
        let mut joined = String::new();
        for (offset, row) in rows[start..].iter().enumerate() {
            joined.push('\t');
            joined.push_str(row);
            let flat = compact(&joined);
            if flat.len() > needle.len() {
                break;
            }
            if offset > 0 && flat == needle && numbers(norm) == numbers(&joined) {
                return true;
            }
        }
    }
    false
}

pub fn uncovered(extra: &str, base: &str, union: &str, relaxed: bool) -> Vec<Supplement> {
    let mut rows: Vec<String> = base
        .lines()
        .filter(|row| !compact(row).is_empty())
        .map(str::to_owned)
        .collect();
    let mut found = Vec::new();
    for raw in extra.lines() {
        let line = raw.trim().trim_matches('|').trim();
        if line.is_empty() || line.chars().all(|ch| "-|: ".contains(ch)) {
            continue;
        }
        let line = if line.contains('|') {
            line.split('|')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            line.into()
        };
        let norm = normalize(&line);
        if compact(&norm).is_empty() {
            continue;
        }
        let count: usize = rows.iter().map(|row| exact_ranges(&norm, row).len()).sum();
        if count > 0 {
            let union_count: usize = union
                .lines()
                .map(|row| exact_ranges(&norm, row).len())
                .sum();
            if compact(&norm).chars().count() <= 20 && count < union_count {
                found.push(Supplement {
                    text: line,
                    ..Default::default()
                });
                rows.push(norm);
            }
            continue;
        }
        if covered(&norm, &rows.join("\n")) || complete_rows_covered(&norm, &rows) {
            continue;
        }
        let segments: Vec<_> = SEGMENTS
            .split(&line)
            .map(|text| (text.to_string(), normalize(text)))
            .filter(|(_, norm)| !compact(norm).is_empty())
            .collect();
        let total: usize = segments.iter().map(|(_, norm)| norm.chars().count()).sum();
        if total < 2 {
            continue;
        }
        let mut best = (String::new(), vec![false; segments.len()], 0);
        for row in &rows {
            let flags: Vec<_> = segments
                .iter()
                .map(|(_, norm)| covered(norm, row))
                .collect();
            let score: usize = segments
                .iter()
                .zip(&flags)
                .filter(|(_, flag)| **flag)
                .map(|((_, norm), _)| norm.chars().count())
                .sum();
            if score > best.2 {
                best = (row.clone(), flags, score);
            }
        }
        let (row, flags, score) = best;
        let mut is_conflict = false;
        let selected = if score == 0 {
            is_conflict = rows.iter().any(|row| conflict(&norm, row));
            vec![line]
        } else if score == total {
            if in_order(&segments, &row) {
                continue;
            }
            is_conflict = true;
            vec![line]
        } else if score * 2 < total || (relaxed && (total - score) * 4 >= total) {
            is_conflict = conflict(&norm, &row);
            vec![line]
        } else {
            let first = flags.iter().take_while(|flag| !**flag).count();
            let last = flags.iter().rev().take_while(|flag| !**flag).count();
            let known = segments
                .iter()
                .zip(&flags)
                .filter(|(_, flag)| **flag)
                .map(|((_, norm), _)| norm.as_str())
                .collect::<Vec<_>>()
                .join("\t");
            if first + last != flags.iter().filter(|flag| !**flag).count() || !covered(&row, &known)
            {
                is_conflict = true;
                vec![line]
            } else {
                [&segments[..first], &segments[segments.len() - last..]]
                    .into_iter()
                    .filter(|slice| !slice.is_empty())
                    .map(|slice| {
                        slice
                            .iter()
                            .map(|(text, _)| text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect()
            }
        };
        for text in selected {
            let candidate = normalize(&text);
            let existing_row = rows.iter().any(|row| {
                compact(row) == compact(&candidate)
                    && row.split('\t').filter(|cell| !cell.is_empty()).count() >= 2
            });
            if (existing_row || complete_rows_covered(&candidate, &rows)) && !is_conflict {
                let count: usize = rows
                    .iter()
                    .map(|row| exact_ranges(&candidate, row).len())
                    .sum();
                let union_count: usize = union
                    .lines()
                    .map(|row| exact_ranges(&candidate, row).len())
                    .sum();
                if count >= union_count {
                    continue;
                }
            }
            rows.push(candidate);
            found.push(Supplement {
                text,
                source: String::new(),
                conflict: is_conflict,
            });
        }
    }
    found
}

pub fn combined(main: &str, supplements: &[Supplement]) -> String {
    let mut result = main.to_string();
    for source in ["上部", "中部", "下部"] {
        for is_conflict in [false, true] {
            let lines: Vec<_> = supplements
                .iter()
                .filter(|item| item.source == source && item.conflict == is_conflict)
                .map(|item| item.text.as_str())
                .collect();
            if !lines.is_empty() {
                result.push_str(&format!(
                    "\n\n【{}·{source}】\n{}",
                    if is_conflict {
                        "差异待核对"
                    } else {
                        "补充识别"
                    },
                    lines.join("\n")
                ));
            }
        }
    }
    result
}
