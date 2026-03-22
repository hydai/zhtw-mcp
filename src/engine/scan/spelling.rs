// Spelling rule scan using Aho-Corasick.
//
// Uses daachorse's CharwiseDoubleArrayAhoCorasick when available (charwise
// transitions reduce state count ~3x for CJK patterns).  Falls back to
// BurntSushi's bytewise Aho-Corasick otherwise.
//
// Context-clue checking uses a windowed approach: a separate bytewise AC
// automaton (built from all unique context_clue and negative_context_clue
// strings) is run only over the bounded context slice for matches that
// actually need clue resolution.  This avoids the old full-document pre-scan
// and its document-sized allocation on the first clue-gated match.
//
// The clue AC uses MatchKind::Standard with overlapping iteration so that
// substring clues (e.g. "下拉" inside "下拉菜單") are all captured.

use std::collections::HashMap;

use crate::engine::excluded::{is_excluded, ByteRange};
use crate::engine::zhtype::ChineseType;
use crate::rules::ruleset::{Issue, IssueType, ProfileConfig, RuleType};

use super::{
    already_correct_form, clamp_at_excluded, PositionalClue, Scanner, CONTEXT_WINDOW_CHARS,
    MIN_SCAN_CLUE_MATCHES, POSITIONAL_WINDOW_CHARS,
};

impl Scanner {
    /// Spelling rule scan using Aho-Corasick.
    ///
    /// Uses charwise double-array AC (daachorse) when available for ~3x fewer
    /// state transitions on CJK text.  Falls back to bytewise AC (BurntSushi).
    ///
    /// Before emitting an issue, checks whether the surrounding text already
    /// contains a correct form that is a superstring of the wrong term (e.g.
    /// "演算法" contains "算法").  This prevents false positives that would
    /// otherwise cause apply_fixes to produce gibberish like "演演算法".
    pub(crate) fn scan_spelling(
        &self,
        text: &str,
        excluded: &[ByteRange],
        zh_type: ChineseType,
        issues: &mut Vec<Issue>,
        cfg: &ProfileConfig,
    ) {
        // Exclusion cursor: both AC iterators yield matches in increasing
        // start order, so we advance a cursor through the sorted excluded
        // ranges for amortized O(1) exclusion checks instead of O(log E)
        // binary search per match.
        let mut excl_cursor: usize = 0;

        // Word-boundary straddle cache: memoizes word_straddles_boundary()
        // results per byte offset.  Multiple AC matches can share boundary
        // positions, and MMSEG trie traversal is the heaviest per-match check.
        let mut boundary_cache: HashMap<usize, bool> = HashMap::new();

        // Dispatch to charwise AC when available; fall back to bytewise.
        if let Some(ref cw_ac) = self.spelling_ac_charwise {
            for mat in cw_ac.leftmost_find_iter(text) {
                self.process_spelling_match(
                    text,
                    excluded,
                    &mut excl_cursor,
                    zh_type,
                    issues,
                    cfg,
                    &mut boundary_cache,
                    mat.start(),
                    mat.end(),
                    mat.value(),
                );
            }
        } else if let Some(ref bw_ac) = self.spelling_ac_bytewise {
            for mat in bw_ac.find_iter(text) {
                self.process_spelling_match(
                    text,
                    excluded,
                    &mut excl_cursor,
                    zh_type,
                    issues,
                    cfg,
                    &mut boundary_cache,
                    mat.start(),
                    mat.end(),
                    mat.pattern().as_usize(),
                );
            }
        }
    }

    /// Process a single spelling AC match.  Shared between charwise and
    /// bytewise code paths.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn process_spelling_match(
        &self,
        text: &str,
        excluded: &[ByteRange],
        excl_cursor: &mut usize,
        zh_type: ChineseType,
        issues: &mut Vec<Issue>,
        cfg: &ProfileConfig,
        boundary_cache: &mut HashMap<usize, bool>,
        start: usize,
        end: usize,
        rule_idx: usize,
    ) {
        // Absorption sentinel: exception/superstring patterns injected into
        // the AC to shadow shorter `from` matches via LeftmostLongest.
        if rule_idx >= self.spelling_rules.len() {
            return;
        }

        let rule = &self.spelling_rules[rule_idx];

        // Variant rules are character-form corrections (裏→裡, 着→著) that
        // only apply in Traditional Chinese context.  Skip when the profile
        // disables variant_normalization or when text is Simplified.
        if rule.rule_type == RuleType::Variant
            && (!cfg.variant_normalization || zh_type == ChineseType::Simplified)
        {
            return;
        }

        // AI filler rules are profile-gated: only fire when ai_filler_detection
        // is enabled (de_ai / strict_moe profiles).
        if rule.rule_type == RuleType::AiFiller && !cfg.ai_filler_detection {
            return;
        }

        // Political stance filtering: suppress political_coloring rules
        // based on the active stance sub-profile.
        if rule.rule_type == RuleType::PoliticalColoring
            && !cfg.political_stance.allows_rule(&rule.from)
        {
            return;
        }

        // Exclusion check with advancing cursor.  AC matches arrive in
        // increasing start order, so we advance past ranges that end
        // before the current match for amortized O(1) instead of
        // O(log E) binary search.
        while *excl_cursor < excluded.len() && excluded[*excl_cursor].end <= start {
            *excl_cursor += 1;
        }
        if *excl_cursor < excluded.len()
            && excluded[*excl_cursor].start < end
            && start < excluded[*excl_cursor].end
        {
            return;
        }

        // Skip if surrounding text already contains a correct form.
        // Short-circuit: most rules have no superstring relationship between
        // `from` and any `to` entry, so the check is guaranteed false.
        if self.spelling_has_superstring[rule_idx] && already_correct_form(text, start, rule) {
            return;
        }

        // Word-boundary check: skip if a known dictionary word straddles
        // either edge of the AC match.  This catches false positives where
        // the matched pattern spans two distinct words — e.g. "積分" found
        // inside "累積分佈" (累積 + 分佈), "程序" inside "排程序列"
        // (排程 + 序列), "導出" inside "引導出" (引導 + 出).
        //
        // Results are memoized per byte offset: boundaries are text-fixed,
        // not rule-dependent, and clustered/overlapping AC matches repeat
        // lookups at the same positions.
        let straddles_start = *boundary_cache
            .entry(start)
            .or_insert_with(|| self.segmenter.word_straddles_boundary(text, start));
        let straddles_end = *boundary_cache
            .entry(end)
            .or_insert_with(|| self.segmenter.word_straddles_boundary(text, end));
        if straddles_start || straddles_end {
            return;
        }

        // Exception check: skip if the match falls inside an exception
        // phrase.  Applies to all rule types — variant, cross_strait,
        // typo, confusable, etc.  (e.g. chess term 下著 keeps 着; 分類
        // keeps 類 from firing as an OOP-class warning).
        if let Some(ref exceptions) = rule.exceptions {
            let in_exception = exceptions.iter().any(|exc| {
                for (pos, _) in exc.match_indices(&rule.from) {
                    if let Some(exc_start) = start.checked_sub(pos) {
                        let exc_end = exc_start + exc.len();
                        if text.get(exc_start..exc_end) == Some(exc.as_str()) {
                            return true;
                        }
                    }
                }
                false
            });
            if in_exception {
                return;
            }
        }

        // Context-clue gate via a windowed clue AC scan.
        //
        // Only rules with context clues pay this cost, and the automaton runs
        // over the bounded local window rather than the full document.
        let has_pos = self.rule_pos_clue_ids[rule_idx].is_some();
        let has_neg = self.rule_neg_clue_ids[rule_idx].is_some();

        if has_pos || has_neg {
            // Compute byte-offset window matching surrounding_window_bounded
            // semantics: +-CONTEXT_WINDOW_CHARS characters, clamped at excluded
            // range boundaries.
            let (win_start, win_end) = context_byte_window(text, start, end, excluded);
            let (pos_matches, any_neg) = scan_clues_in_window(
                self.clue_ac.as_ref(),
                &text[win_start..win_end],
                self.rule_pos_clue_ids[rule_idx].as_deref(),
                self.rule_neg_clue_ids[rule_idx].as_deref(),
            );

            if has_pos && pos_matches < MIN_SCAN_CLUE_MATCHES {
                return;
            }

            if has_neg && any_neg {
                return;
            }
        }

        // Positional clue gate: directional constraints on where context
        // terms must appear relative to the match.  Checked after the flat
        // context-clue gate (which confirms co-occurrence in +-40-char window).
        // When both context_clues and positional_clues are present on a rule,
        // both must pass (AND semantics).
        if let Some(ref pos_clues) = self.rule_positional_clues[rule_idx] {
            if !check_positional_clues(text, start, end, excluded, pos_clues) {
                return;
            }
        }

        // AiFiller deletion rules: extend span to consume trailing fullwidth
        // punctuation (，：) so that a single base rule handles all variants
        // without leaving dangling punctuation after fix application.
        // Guard: do not extend into an excluded range (code block, URL).
        let end = if rule.is_deletion_rule() {
            match text[end..].chars().next() {
                Some(c @ ('\u{FF0C}' | '\u{FF1A}'))
                    if !is_excluded(end, end + c.len_utf8(), excluded) =>
                {
                    end + c.len_utf8()
                }
                _ => end,
            }
        } else {
            end
        };

        let mut issue = Issue::new(
            start,
            end - start,
            &text[start..end],
            self.spelling_suggestions[rule_idx].clone(),
            IssueType::from(rule.rule_type),
            rule.rule_type.default_severity(),
        );
        issue.context.clone_from(&rule.context);
        issue.english.clone_from(&rule.english);
        issue.context_clues.clone_from(&rule.context_clues);
        issues.push(issue);
    }
}

/// Compute the byte-offset window for context-clue proximity checks.
///
/// Walks ±CONTEXT_WINDOW_CHARS characters from the match boundaries (same
/// as surrounding_window), then clamps at excluded-range boundaries (same
/// as surrounding_window_bounded).  Returns (win_start, win_end) in byte
/// offsets suitable for direct comparison against clue_hits positions.
fn context_byte_window(
    text: &str,
    match_start: usize,
    match_end: usize,
    excluded: &[ByteRange],
) -> (usize, usize) {
    let bytes = text.as_bytes();

    // Find paragraph boundaries (\n\n or \r\n\r\n) around the match.
    // Context clues from a different paragraph are semantically irrelevant
    // and can cause false triggers/suppressions.
    //
    // Clamp the search range to CONTEXT_WINDOW_CHARS * 4 bytes (max possible
    // window extent for CJK text) to avoid O(N) scans per match.
    let max_search = CONTEXT_WINDOW_CHARS * 4;
    let para_start = {
        let search_start = match_start.saturating_sub(max_search);
        let search = &bytes[search_start..match_start];
        find_last_paragraph_break(search).map_or(0, |pos| search_start + pos + 1)
    };
    let para_end = {
        let search_end = (match_end + max_search).min(text.len());
        let search = &bytes[match_end..search_end];
        find_first_paragraph_break(search).map_or(text.len(), |pos| match_end + pos)
    };

    // Walk backward CONTEXT_WINDOW_CHARS characters, clamped at paragraph start.
    let mut byte_start = match_start;
    for _ in 0..CONTEXT_WINDOW_CHARS {
        if byte_start <= para_start {
            byte_start = para_start;
            break;
        }
        byte_start = text.floor_char_boundary(byte_start - 1);
    }
    byte_start = byte_start.max(para_start);

    // Walk forward CONTEXT_WINDOW_CHARS characters, clamped at paragraph end.
    let mut byte_end = match_end;
    for _ in 0..CONTEXT_WINDOW_CHARS {
        if byte_end >= para_end {
            byte_end = para_end;
            break;
        }
        byte_end = text.ceil_char_boundary(byte_end + 1);
    }
    byte_end = byte_end.min(para_end);

    if excluded.is_empty() {
        return (byte_start, byte_end);
    }

    clamp_at_excluded(text, byte_start, byte_end, match_start, match_end, excluded)
}

/// Find the byte offset of the last paragraph break (`\n\n`) in `bytes`.
/// Returns the offset of the second `\n` (i.e. the byte just before the
/// new paragraph starts).  Handles `\r\n\r\n` as well.
fn find_last_paragraph_break(bytes: &[u8]) -> Option<usize> {
    // Scan backward for \n\n.
    let len = bytes.len();
    if len < 2 {
        return None;
    }
    let mut i = len - 1;
    while i > 0 {
        if bytes[i] == b'\n' && bytes[i - 1] == b'\n' {
            return Some(i);
        }
        // Handle \r\n\r\n: bytes[i]=\n, bytes[i-1]=\r, bytes[i-2]=\n
        if i >= 2 && bytes[i] == b'\n' && bytes[i - 1] == b'\r' && bytes[i - 2] == b'\n' {
            return Some(i);
        }
        i -= 1;
    }
    None
}

/// Find the byte offset of the first paragraph break (`\n\n`) in `bytes`.
/// Returns the offset of the first `\n` in the pair.
fn find_first_paragraph_break(bytes: &[u8]) -> Option<usize> {
    let len = bytes.len();
    if len < 2 {
        return None;
    }
    for i in 0..len - 1 {
        if bytes[i] == b'\n' && bytes[i + 1] == b'\n' {
            return Some(i);
        }
        // \n\r\n also counts.
        if i + 2 < len && bytes[i] == b'\n' && bytes[i + 1] == b'\r' && bytes[i + 2] == b'\n' {
            return Some(i);
        }
    }
    None
}

/// Scan a bounded text window for positive and negative clue IDs in one pass.
///
/// Returns `(positive_matches, any_negative_match)`, counting each positive
/// clue ID at most once even if it appears multiple times in the window.
fn scan_clues_in_window(
    clue_ac: Option<&aho_corasick::AhoCorasick>,
    window: &str,
    pos_ids: Option<&[u16]>,
    neg_ids: Option<&[u16]>,
) -> (usize, bool) {
    let Some(clue_ac) = clue_ac else {
        return (0, false);
    };
    if window.is_empty() || (pos_ids.is_none() && neg_ids.is_none()) {
        return (0, false);
    }

    // Rule-local clue lists are small, so fixed bitsets beat HashSet
    // allocation in the per-match hot path.
    let mut pos_seen = [false; 32];
    let mut pos_found = 0usize;
    let mut any_neg = false;

    for mat in clue_ac.find_overlapping_iter(window) {
        let clue_id = mat.pattern().as_usize() as u16;

        if let Some(pos_ids) = pos_ids {
            if let Some(pos) = pos_ids.iter().position(|&id| id == clue_id) {
                if !pos_seen[pos] {
                    pos_seen[pos] = true;
                    pos_found += 1;
                }
            }
        }

        if let Some(neg_ids) = neg_ids {
            if neg_ids.contains(&clue_id) {
                any_neg = true;
            }
        }

        // Early exit: a negative hit irrevocably vetoes the rule, so
        // stop immediately — no point counting more positives.
        // Otherwise, break once all distinct positive IDs are found and
        // there are no negative clues left to discover.
        if any_neg {
            break;
        }
        let pos_done = match pos_ids {
            Some(ids) => pos_found >= ids.len(),
            None => true,
        };
        if pos_done && neg_ids.is_none() {
            break;
        }
    }

    (pos_found, any_neg)
}

/// Check all positional clues for a match at [start, end) in `text`.
///
/// All positive clues (Before, After, Adjacent) must match (AND).
/// Any negative clue (NotBefore, NotAfter) vetoes (short-circuit false).
/// Returns true when all conditions are satisfied.
///
/// Window computation respects paragraph breaks and excluded ranges (code
/// spans, URLs, inline suppressions) — same discipline as context_byte_window.
fn check_positional_clues(
    text: &str,
    start: usize,
    end: usize,
    excluded: &[ByteRange],
    clues: &[PositionalClue],
) -> bool {
    // Compute bounded windows once per direction, lazily.
    // Positional windows are ~80 bytes max; the cost is trivial but
    // caching avoids redundant paragraph-break + excluded-range scans
    // when multiple clues share the same direction.
    let mut after_win: Option<(usize, usize)> = None;
    let mut before_win: Option<(usize, usize)> = None;

    for clue in clues {
        match clue {
            PositionalClue::Before(term) => {
                let (ws, we) =
                    *after_win.get_or_insert_with(|| positional_bounds_after(text, end, excluded));
                if !text[ws..we].contains(term.as_str()) {
                    return false;
                }
            }
            PositionalClue::After(term) => {
                let (ws, we) = *before_win
                    .get_or_insert_with(|| positional_bounds_before(text, start, excluded));
                if !text[ws..we].contains(term.as_str()) {
                    return false;
                }
            }
            PositionalClue::Adjacent(term) => {
                // Immediately before: term ends right at match start.
                let before_ok = start >= term.len()
                    && text.get(start - term.len()..start) == Some(term.as_str())
                    && !is_excluded(start - term.len(), start, excluded);
                // Immediately after: term starts right at match end.
                let after_ok = text.get(end..end + term.len()) == Some(term.as_str())
                    && !is_excluded(end, end + term.len(), excluded);
                if !before_ok && !after_ok {
                    return false;
                }
            }
            PositionalClue::NotBefore(term) => {
                let (ws, we) =
                    *after_win.get_or_insert_with(|| positional_bounds_after(text, end, excluded));
                if text[ws..we].contains(term.as_str()) {
                    return false;
                }
            }
            PositionalClue::NotAfter(term) => {
                let (ws, we) = *before_win
                    .get_or_insert_with(|| positional_bounds_before(text, start, excluded));
                if text[ws..we].contains(term.as_str()) {
                    return false;
                }
            }
        }
    }
    true
}

/// Compute byte-offset bounds for the positional window AFTER the match
/// ending at `match_end`.  Returns `(win_start, win_end)` spanning up to
/// POSITIONAL_WINDOW_CHARS characters forward, clamped at paragraph breaks
/// and excluded-range boundaries — same discipline as context_byte_window.
fn positional_bounds_after(text: &str, match_end: usize, excluded: &[ByteRange]) -> (usize, usize) {
    if match_end >= text.len() {
        return (text.len(), text.len());
    }
    let bytes = text.as_bytes();

    // Paragraph boundary: stop at first \n\n after match_end.
    let max_search = POSITIONAL_WINDOW_CHARS * 4;
    let search_end = (match_end + max_search).min(text.len());
    let para_end = {
        let search = &bytes[match_end..search_end];
        find_first_paragraph_break(search).map_or(text.len(), |pos| match_end + pos)
    };

    // Walk forward POSITIONAL_WINDOW_CHARS chars, clamped at paragraph end.
    let mut byte_end = match_end;
    for _ in 0..POSITIONAL_WINDOW_CHARS {
        if byte_end >= para_end {
            byte_end = para_end;
            break;
        }
        byte_end = text.ceil_char_boundary(byte_end + 1);
    }
    byte_end = byte_end.min(para_end);

    // Clamp at the nearest excluded range that starts after match_end.
    if !excluded.is_empty() {
        let right_idx = excluded.partition_point(|r| r.start < match_end);
        for excl in &excluded[right_idx..] {
            if excl.start >= byte_end {
                break;
            }
            if excl.start >= match_end && excl.start < byte_end {
                byte_end = excl.start;
            }
        }
    }

    let byte_end = text.floor_char_boundary(byte_end.min(text.len()));
    if match_end > byte_end {
        return (match_end, match_end);
    }
    (match_end, byte_end)
}

/// Compute byte-offset bounds for the positional window BEFORE the match
/// starting at `match_start`.  Returns `(win_start, win_end)` spanning up
/// to POSITIONAL_WINDOW_CHARS characters backward, clamped at paragraph
/// breaks and excluded-range boundaries.
fn positional_bounds_before(
    text: &str,
    match_start: usize,
    excluded: &[ByteRange],
) -> (usize, usize) {
    if match_start == 0 {
        return (0, 0);
    }
    let bytes = text.as_bytes();

    // Paragraph boundary: stop at last \n\n before match_start.
    let max_search = POSITIONAL_WINDOW_CHARS * 4;
    let search_start = match_start.saturating_sub(max_search);
    let para_start = {
        let search = &bytes[search_start..match_start];
        find_last_paragraph_break(search).map_or(0, |pos| search_start + pos + 1)
    };

    // Walk backward POSITIONAL_WINDOW_CHARS chars, clamped at paragraph start.
    let mut byte_start = match_start;
    for _ in 0..POSITIONAL_WINDOW_CHARS {
        if byte_start <= para_start {
            byte_start = para_start;
            break;
        }
        byte_start = text.floor_char_boundary(byte_start - 1);
    }
    byte_start = byte_start.max(para_start);

    // Clamp at the nearest excluded range that ends before match_start.
    if !excluded.is_empty() {
        let left_idx = excluded.partition_point(|r| r.start < match_start);
        for excl in excluded[..left_idx].iter().rev() {
            if excl.end <= byte_start {
                break;
            }
            if excl.end <= match_start && excl.end > byte_start {
                byte_start = excl.end;
            }
        }
    }

    let byte_start = text.ceil_char_boundary(byte_start);
    if byte_start > match_start {
        return (match_start, match_start);
    }
    (byte_start, match_start)
}
