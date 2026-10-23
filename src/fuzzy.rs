//! The one fuzzy matcher, shared by Tab completion, Ctrl-R history search and `/switch`.
//!
//! A candidate matches when the query's characters appear in it in order (case-insensitive).
//! The score rewards a prefix match most, then matches at word starts and runs of adjacent
//! characters, and penalizes gaps, so `sw` ranks `switch` above `show-window`.

/// A match: higher scores are better; `positions` are the matched char indices (for
/// highlighting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub score: i64,
    pub positions: Vec<usize>,
}

const PREFIX: i64 = 1000;
const START: i64 = 30;
const ADJACENT: i64 = 20;
const BOUNDARY: i64 = 12;
const GAP: i64 = 1;

fn boundary(prev: Option<char>, c: char) -> bool {
    match prev {
        None => true,
        Some(p) => !p.is_alphanumeric() || (p.is_lowercase() && c.is_uppercase()),
    }
}

/// Score `candidate` against `query` (an empty query matches everything with score 0).
pub fn score(query: &str, candidate: &str) -> Option<Match> {
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if q.is_empty() {
        return Some(Match {
            score: 0,
            positions: vec![],
        });
    }
    let chars: Vec<char> = candidate.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    // Prefix match: the best kind.
    if lower.len() >= q.len() && lower[..q.len()] == q[..] {
        return Some(Match {
            score: PREFIX + (q.len() as i64) * ADJACENT - (lower.len() - q.len()) as i64,
            positions: (0..q.len()).collect(),
        });
    }
    // Greedy left-to-right, but prefer a word-boundary occurrence of each character when
    // one exists before the next required character can no longer fit.
    let mut positions = Vec::with_capacity(q.len());
    let mut from = 0;
    for (qi, qc) in q.iter().enumerate() {
        let left = q.len() - qi - 1;
        let last_ok = lower.len().checked_sub(left + 1)?;
        let cands: Vec<usize> = (from..=last_ok.max(from))
            .filter(|&i| i < lower.len() && lower[i] == *qc)
            .collect();
        let first = *cands.first()?;
        let adjacent = positions.last().map(|&p: &usize| p + 1);
        let pick = if Some(first) == adjacent {
            first
        } else {
            cands
                .iter()
                .copied()
                .find(|&i| boundary(i.checked_sub(1).map(|j| chars[j]), chars[i]))
                .unwrap_or(first)
        };
        positions.push(pick);
        from = pick + 1;
    }
    let mut s = 0i64;
    for (k, &p) in positions.iter().enumerate() {
        if p == 0 {
            s += START;
        }
        if boundary(p.checked_sub(1).map(|j| chars[j]), chars[p]) {
            s += BOUNDARY;
        }
        if k > 0 {
            let prev = positions[k - 1];
            if p == prev + 1 {
                s += ADJACENT;
            } else {
                s -= GAP * (p - prev - 1) as i64;
            }
        }
    }
    s -= (chars.len() as i64) / 8;
    Some(Match {
        score: s,
        positions,
    })
}

/// Indices of `candidates` matching `query`, best first; ties keep the input order (so
/// callers that pass recent-first history get recent first on ties).
pub fn rank<S: AsRef<str>>(query: &str, candidates: &[S]) -> Vec<(usize, Match)> {
    let mut v: Vec<(usize, Match)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| score(query, c.as_ref()).map(|m| (i, m)))
        .collect();
    v.sort_by(|a, b| b.1.score.cmp(&a.1.score).then(a.0.cmp(&b.0)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_case_insensitive_and_positions() {
        let m = score("acp", "ApplyChangePatch").unwrap();
        assert_eq!(m.positions, vec![0, 5, 11]);
        assert!(score("xyz", "switch").is_none());
        assert!(score("hcti", "switch").is_none(), "order matters");
        assert_eq!(score("", "x").unwrap().score, 0);
    }

    #[test]
    fn prefix_beats_word_start_beats_scattered() {
        let names = ["xsxw", "show-window", "switch", "rest-switch", "sweep"];
        let order: Vec<&str> = rank("sw", &names).iter().map(|(i, _)| names[*i]).collect();
        // Prefixes first (shorter first), then word-start matches, the scattered one last.
        assert_eq!(
            order,
            ["sweep", "switch", "show-window", "rest-switch", "xsxw"]
        );
        let t = ["one", "tokyo-night", "default"];
        assert_eq!(rank("tn", &t)[0].0, 1);
        assert_eq!(rank("d", &t)[0].0, 2);
    }

    #[test]
    fn ties_keep_input_order() {
        let h = ["git push", "git push", "git pull"];
        let r = rank("push", &h);
        assert_eq!(r.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1]);
    }

    #[test]
    fn word_boundaries_are_preferred_for_highlighting() {
        // "tn" should light up the T and N of tokyo-night, not the 'n' in "tokyo".
        let m = score("tn", "tokyo-night").unwrap();
        assert_eq!(m.positions, vec![0, 6]);
    }
}
