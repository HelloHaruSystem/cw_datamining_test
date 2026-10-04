//! Word-level text diffs, so a reworded description shows exactly which
//! words changed instead of two near-identical paragraphs.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Same,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Segment {
    pub op: Op,
    pub text: String,
}

/// Above this many token pairs the LCS table gets too big; fall back to
/// "everything removed, everything added".
const MAX_CELLS: usize = 1_000_000;

/// Diff `old` against `new` by words. Whitespace and punctuation are their
/// own tokens, so `items/skills` vs `items` marks just `/skills`.
/// Adjacent segments with the same op are merged.
pub fn diff_words(old: &str, new: &str) -> Vec<Segment> {
    let a = tokenize(old);
    let b = tokenize(new);
    let mut out = Vec::new();

    // Common prefix and suffix need no table.
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);

    push_all(&mut out, Op::Same, &a[..prefix]);
    if am.len().saturating_mul(bm.len()) > MAX_CELLS {
        push_all(&mut out, Op::Removed, am);
        push_all(&mut out, Op::Added, bm);
    } else {
        lcs(&mut out, am, bm);
    }
    push_all(&mut out, Op::Same, &a[a.len() - suffix..]);
    out
}

/// Compact one-line form for terminals: `same [-removed-]{+added+} same`.
pub fn inline(old: &str, new: &str) -> String {
    diff_words(old, new)
        .into_iter()
        .map(|s| match s.op {
            Op::Same => s.text,
            Op::Removed => format!("[-{}-]", s.text),
            Op::Added => format!("{{+{}+}}", s.text),
        })
        .collect()
}

/// Share of `new` that is unchanged, from 0.0 to 1.0. Low values mean the
/// text was rewritten rather than edited.
pub fn similarity(segments: &[Segment]) -> f64 {
    let len = |op: Op| -> usize {
        segments
            .iter()
            .filter(|s| s.op == op)
            .map(|s| s.text.len())
            .sum()
    };
    let same = len(Op::Same);
    let total = same + len(Op::Added).max(len(Op::Removed));
    if total == 0 {
        1.0
    } else {
        same as f64 / total as f64
    }
}

fn tokenize(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<bool> = None; // Some(true) = inside a word
    for (i, c) in s.char_indices() {
        let word = c.is_alphanumeric() || c == '\'' || c == '%';
        if prev != Some(word) || !word {
            if i > start {
                out.push(&s[start..i]);
            }
            start = i;
        }
        prev = Some(word);
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

fn push(out: &mut Vec<Segment>, op: Op, text: &str) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(last) if last.op == op => last.text.push_str(text),
        _ => out.push(Segment {
            op,
            text: text.to_owned(),
        }),
    }
}

fn push_all(out: &mut Vec<Segment>, op: Op, tokens: &[&str]) {
    for t in tokens {
        push(out, op, t);
    }
}

/// Classic longest-common-subsequence diff over tokens.
fn lcs(out: &mut Vec<Segment>, a: &[&str], b: &[&str]) {
    let (n, m) = (a.len(), b.len());
    // len[i][j] = LCS length of a[i..] and b[j..].
    let mut len = vec![0u32; (n + 1) * (m + 1)];
    let idx = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            len[idx(i, j)] = if a[i] == b[j] {
                len[idx(i + 1, j + 1)] + 1
            } else {
                len[idx(i + 1, j)].max(len[idx(i, j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            push(out, Op::Same, a[i]);
            i += 1;
            j += 1;
        } else if len[idx(i + 1, j)] >= len[idx(i, j + 1)] {
            push(out, Op::Removed, a[i]);
            i += 1;
        } else {
            push(out, Op::Added, b[j]);
            j += 1;
        }
    }
    push_all(out, Op::Removed, &a[i..]);
    push_all(out, Op::Added, &b[j..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_only_removed_words() {
        let old = "HP recovery from items and skills is boosted.";
        let new = "HP recovery from items is boosted.";
        assert_eq!(
            inline(old, new),
            "HP recovery from items [-and skills -]is boosted."
        );
    }

    #[test]
    fn marks_replacements() {
        assert_eq!(
            inline("Weapon Def. +10%", "Weapon Def. +5%"),
            "Weapon Def. +[-10%-]{+5%+}"
        );
        assert_eq!(inline("items/skills", "items"), "items[-/skills-]");
    }

    #[test]
    fn identical_and_empty() {
        assert_eq!(inline("same", "same"), "same");
        assert_eq!(inline("", "new"), "{+new+}");
        assert_eq!(similarity(&diff_words("a b", "a b")), 1.0);
    }

    #[test]
    fn similarity_detects_rewrites() {
        let edit = diff_words(
            "HP recovery from items and skills is boosted.",
            "HP recovery from items is boosted.",
        );
        let rewrite = diff_words("Totally different text here", "Nothing in common at all");
        assert!(similarity(&edit) > 0.7);
        assert!(similarity(&rewrite) < 0.3);
    }
}
