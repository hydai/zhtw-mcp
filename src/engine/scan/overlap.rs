// Overlap resolution for detected issues.
//
// Priority-based greedy algorithm: longest match wins; on tie, higher
// severity wins.  Avoids the ghost-suppression flaw in forward greedy scans.

use crate::rules::ruleset::Issue;

/// Remove overlapping issues from a sorted (by offset) issue list.
///
/// Priority: longer match wins; on tie, higher severity wins.
///
/// Uses a priority-based greedy algorithm: issues are processed longest-first,
/// accepted only when non-overlapping with all already-accepted matches.
/// This avoids the ghost-suppression flaw in a forward greedy scan, where A
/// can be wrongly discarded because B beats A and then C beats B — even though
/// A and C would not have overlapped.
pub(crate) fn resolve_overlaps(issues: &mut Vec<Issue>) {
    if issues.len() <= 1 {
        return;
    }

    // Process in priority order: longest first; on tie, highest severity;
    // on further tie, earliest offset (deterministic).
    let n = issues.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        issues[b]
            .length
            .cmp(&issues[a].length)
            .then_with(|| issues[b].severity.cmp(&issues[a].severity))
            .then_with(|| issues[a].offset.cmp(&issues[b].offset))
    });

    let mut keep = vec![false; n];
    // Accepted byte intervals (start, end), kept sorted by start offset
    // for O(log n) overlap checks via binary search.
    let mut accepted: Vec<(usize, usize)> = Vec::new();

    for &i in &order {
        let start = issues[i].offset;
        let end = start + issues[i].length;

        // Binary search for the insertion point, then check neighbors.
        // Two intervals [s1,e1) and [s2,e2) overlap iff s1 < e2 && e1 > s2.
        let pos = accepted.partition_point(|&(s, _)| s < start);
        let overlaps =
            // Check the interval just before (if it extends past our start).
            (pos > 0 && accepted[pos - 1].1 > start)
            // Check the interval at pos (if our end extends past its start).
            || (pos < accepted.len() && end > accepted[pos].0);

        if !overlaps {
            keep[i] = true;
            accepted.insert(pos, (start, end));
        }
    }

    // Retain in original (offset-sorted) order.
    let mut i = 0;
    issues.retain(|_| {
        let k = keep[i];
        i += 1;
        k
    });
}
