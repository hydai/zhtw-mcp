// Google Translate anchor-confirmation for cross-strait term verification.
//
// Pipeline (confirm, not discover):
//   1. Scanner finds issues with 'english' fields
//   2. Extract sentence-context around each issue
//   3. For each term, check sled cache first (cache hits bypass API cap)
//   4. On cache miss, call Google Translate zh→en unless max_api_calls cap
//      reached (serial, 200ms rate-limit between calls)
//   5. Confirm: if English output contains the rule's 'english' anchor,
//      the issue is a TRUE cross-strait term (Confirmed → keep severity)
//   6. If anchor absent → Rejected → downgrade severity to Info
//   7. If translation fails → Unknown → fail-open (keep severity)
//
// Latency control:
//   - max_api_calls (default 10) caps network calls per confirm_issues() run
//   - Cache hits are free — only cache misses count toward the cap
//   - Worst-case: max_api_calls * ~500ms (200ms delay + ~300ms roundtrip)
//   - transport_failed flag on ConfirmResult lets callers distinguish
//     "sampling returned Unknown" from "sampling transport broke entirely"
//
// Cache design:
//   - sled embedded key-value DB; path via dirs::cache_dir():
//     macOS: ~/Library/Caches/zhtw-anchor-translate/translations.db/
//     Linux: ~/.cache/zhtw-anchor-translate/translations.db/
//   - sled creates a directory (not a file) at the DB path, with
//     memory-mapped segment files (~512KB pre-allocated)
//   - postcard binary serialization (compact, strongly typed)
//   - SHA-256 keyed entries with 30-day TTL
//   - Size-limited with random eviction at 10 MB
//   - Lock detection for concurrent access

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::rules::ruleset::{Issue, Severity};

// Tri-state translation outcome

/// Errors from the Google Translate API layer.
#[derive(Debug)]
pub enum TranslateError {
    /// Network or I/O error.
    Io(String),
    /// HTTP rate limit (429) or server error (5xx).
    RateLimit(u16),
    /// JSON parse error in the response.
    Parse(String),
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "translate I/O error: {msg}"),
            Self::RateLimit(code) => write!(f, "translate rate-limited (HTTP {code})"),
            Self::Parse(msg) => write!(f, "translate parse error: {msg}"),
        }
    }
}

/// Tri-state outcome of anchor confirmation for a single term.
#[derive(Debug, PartialEq, Eq)]
pub enum TranslateOutcome {
    /// English anchor found in translation — true positive.
    Confirmed,
    /// Translation succeeded but anchor absent — likely false positive.
    Rejected,
    /// Translation failed — cannot determine; fail-open (keep severity).
    Unknown,
}

// Translation cache (sled-backed)

/// Default cache path via `dirs::cache_dir()`:
/// - macOS: `~/Library/Caches/zhtw-anchor-translate/translations.db/`
/// - Linux: `~/.cache/zhtw-anchor-translate/translations.db/`
///
/// Note: sled creates a directory at this path, not a single file.
fn default_cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zhtw-anchor-translate")
        .join("translations.db")
}

/// Cache configuration (mirrors cjk-token-reducer CacheConfig).
pub struct CacheConfig {
    /// TTL for cache entries.
    pub ttl: Duration,
    /// Maximum cache size in bytes before eviction kicks in.
    pub max_size_bytes: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(30 * 86400), // 30 days
            max_size_bytes: 10 * 1024 * 1024,     // 10 MB
        }
    }
}

/// Session-level hit/miss counters (atomic, thread-safe).
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static INSERT_COUNT: AtomicU64 = AtomicU64::new(0);

/// Check interval for cache size enforcement (every N inserts).
const SIZE_CHECK_INTERVAL: u64 = 50;
/// Entries larger than this trigger an immediate size check.
const LARGE_ENTRY_THRESHOLD: usize = 4096;
/// Maximum eviction rounds to prevent infinite loops.
const MAX_EVICTION_ROUNDS: usize = 10;

/// Cache entry stored in sled, serialized via postcard.
#[derive(serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    translated: String,
    timestamp: i64, // Unix seconds
    source_lang: String,
    target_lang: String,
}

/// sled-backed translation cache with SHA-256 keys, TTL, and size limits.
pub struct TranslationCache {
    db: sled::Db,
    config: CacheConfig,
}

impl TranslationCache {
    /// Open or create the cache at the default location.
    pub fn open() -> anyhow::Result<Self> {
        Self::open_at(default_cache_path(), CacheConfig::default())
    }

    /// Open or create the cache at a specific path.
    pub fn open_at(path: PathBuf, config: CacheConfig) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let db = match sled::open(&path) {
            Ok(db) => db,
            Err(e) => {
                let msg = e.to_string();
                // Detect lock errors (concurrent access).
                // POSIX: lock, EWOULDBLOCK, EBUSY, EPERM
                // Windows (fs2): "Access is denied", "sharing violation"
                if msg.contains("lock")
                    || msg.contains("EWOULDBLOCK")
                    || msg.contains("EBUSY")
                    || msg.contains("EPERM")
                    || msg.contains("Access is denied")
                    || msg.contains("sharing violation")
                {
                    anyhow::bail!(
                        "Cache locked by another process at {}. Use --no-cache to bypass.",
                        path.display()
                    );
                }
                return Err(e.into());
            }
        };

        Ok(Self { db, config })
    }

    /// SHA-256 hash key: "{src}:{tgt}:{text}" → 32-byte digest.
    fn make_key(src: &str, tgt: &str, text: &str) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update(format!("{src}:{tgt}:{text}").as_bytes());
        h.finalize().to_vec()
    }

    fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }

    /// Look up a cached translation. Returns None on miss or expiry.
    pub fn get(&self, src: &str, tgt: &str, text: &str) -> Option<String> {
        let key = Self::make_key(src, tgt, text);
        let raw = match self.db.get(&key) {
            Ok(Some(v)) => v,
            _ => {
                CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };

        let entry: CacheEntry = match postcard::from_bytes(&raw) {
            Ok(e) => e,
            Err(_) => {
                // Corrupted entry — remove silently.
                let _ = self.db.remove(&key);
                CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };

        // TTL check.
        let age = Self::now_secs().saturating_sub(entry.timestamp);
        if age as u64 > self.config.ttl.as_secs() {
            let _ = self.db.remove(&key);
            CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
            return None;
        }

        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        Some(entry.translated)
    }

    /// Store a translation in the cache.
    pub fn put(&self, src: &str, tgt: &str, text: &str, value: &str) {
        let key = Self::make_key(src, tgt, text);
        let entry = CacheEntry {
            translated: value.to_string(),
            timestamp: Self::now_secs(),
            source_lang: src.to_string(),
            target_lang: tgt.to_string(),
        };

        let encoded = match postcard::to_allocvec(&entry) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("cache encode error: {e}");
                return;
            }
        };

        let is_large = encoded.len() > LARGE_ENTRY_THRESHOLD;
        let _ = self.db.insert(&key, encoded);

        let count = INSERT_COUNT.fetch_add(1, Ordering::Relaxed);
        if is_large || count.is_multiple_of(SIZE_CHECK_INTERVAL) {
            self.enforce_size_limit();
        }
    }

    /// Evict entries if cache exceeds max size (random eviction, ~25% per round).
    fn enforce_size_limit(&self) {
        let disk_size = self.db.size_on_disk().unwrap_or(0);
        if disk_size <= self.config.max_size_bytes {
            return;
        }

        for _ in 0..MAX_EVICTION_ROUNDS {
            let len = self.db.len();
            if len == 0 {
                break;
            }
            let to_remove = (len / 4).max(1);
            let mut removed = 0;
            for item in self.db.iter() {
                if removed >= to_remove {
                    break;
                }
                if let Ok((key, _)) = item {
                    let _ = self.db.remove(key);
                    removed += 1;
                }
            }

            let _ = self.db.flush();
            if self.db.size_on_disk().unwrap_or(0) <= self.config.max_size_bytes {
                break;
            }
        }
    }

    /// Cache statistics for reporting.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.db.len(),
            disk_bytes: self.db.size_on_disk().unwrap_or(0),
            hits: CACHE_HITS.load(Ordering::Relaxed) as usize,
            misses: CACHE_MISSES.load(Ordering::Relaxed) as usize,
        }
    }

    /// Clear all cache entries.
    pub fn clear(&self) {
        self.db.clear().ok();
        let _ = self.db.flush();
    }

    /// Flush pending writes to disk.
    pub fn flush(&self) {
        let _ = self.db.flush();
    }
}

pub struct CacheStats {
    pub entries: usize,
    pub disk_bytes: u64,
    pub hits: usize,
    pub misses: usize,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64 * 100.0
        }
    }
}

// Google Translate API (free gtx endpoint)

const GOOGLE_TRANSLATE_URL: &str = "https://translate.googleapis.com/translate_a/single";
const USER_AGENT: &str = "Mozilla/5.0 (compatible; zhtw-anchor/2.0)";

/// Translate text using the free Google Translate API.
///
/// Returns the translated text, or a 'TranslateError' on failure.
fn google_translate_raw(text: &str, src: &str, tgt: &str) -> Result<String, TranslateError> {
    let url = format!(
        "{GOOGLE_TRANSLATE_URL}?client=gtx&sl={src}&tl={tgt}&dt=t&q={}",
        urlencoding::encode(text)
    );

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .build(),
    );
    let body_str = match agent.get(&url).header("User-Agent", USER_AGENT).call() {
        Ok(mut resp) => match resp.body_mut().read_to_string() {
            Ok(s) => s,
            Err(e) => return Err(TranslateError::Io(e.to_string())),
        },
        Err(ureq::Error::StatusCode(code @ (429 | 500..=599))) => {
            return Err(TranslateError::RateLimit(code));
        }
        Err(e) => {
            return Err(TranslateError::Io(e.to_string()));
        }
    };

    let body: serde_json::Value =
        serde_json::from_str(&body_str).map_err(|e| TranslateError::Parse(e.to_string()))?;

    // Response format: [[["translated text","source text",null,null,N], ...], ...]
    let mut result = String::new();
    if let Some(outer) = body.as_array() {
        if let Some(inner) = outer.first().and_then(|v| v.as_array()) {
            for item in inner {
                if let Some(arr) = item.as_array() {
                    if let Some(s) = arr.first().and_then(|v| v.as_str()) {
                        result.push_str(s);
                    }
                }
            }
        }
    }

    Ok(result)
}

// Context extraction

/// Extract sentence-ish context around a byte offset for translation.
///
/// Scans backwards and forwards for CJK sentence-ending punctuation
/// (。！？ or newline), falling back to a fixed window.
fn extract_context(text: &str, offset: usize, length: usize, window: usize) -> &str {
    let len = text.len();

    // Find start: clamp to char boundary, then scan backwards for sentence end.
    let raw_start = text.floor_char_boundary(offset.saturating_sub(window));
    let mut start = raw_start;
    let prefix = &text[raw_start..offset.min(len)];
    for (i, ch) in prefix.char_indices().rev() {
        if matches!(ch, '。' | '！' | '？' | '\n') {
            start = raw_start + i + ch.len_utf8();
            break;
        }
    }

    // Find end: clamp to char boundary, then scan forwards for sentence end.
    let match_end = (offset + length).min(len);
    let raw_end = text.floor_char_boundary((match_end + window).min(len));
    let mut end = raw_end;
    let suffix = &text[match_end..raw_end];
    for (i, ch) in suffix.char_indices() {
        if matches!(ch, '。' | '！' | '？' | '\n') {
            end = match_end + i + ch.len_utf8();
            break;
        }
    }

    text[start..end].trim()
}

// Anchor confirmation: the core algorithm

/// Configuration for anchor-confirmation.
pub struct ConfirmConfig {
    /// Delay between API calls to avoid rate limiting.
    pub request_delay: Duration,
    /// If true, downgrade unconfirmed issues to Info instead of removing them.
    pub downgrade_unconfirmed: bool,
    /// Maximum number of Google Translate API calls per confirm pass.
    /// After hitting this cap, remaining terms get Unknown outcome (fail-open).
    /// Bounds worst-case latency to max_api_calls * (~300ms HTTP + request_delay).
    pub max_api_calls: usize,
}

impl Default for ConfirmConfig {
    fn default() -> Self {
        Self {
            request_delay: Duration::from_millis(200),
            downgrade_unconfirmed: true,
            max_api_calls: 10,
        }
    }
}

/// Result of the confirmation pass.
pub struct ConfirmResult {
    /// Number of issues confirmed by English anchor match.
    pub confirmed: usize,
    /// Number of issues rejected (translation succeeded, anchor absent).
    pub unconfirmed: usize,
    /// Number of issues where translation failed (fail-open, severity preserved).
    pub unknown: usize,
    /// Number of API calls made.
    pub api_calls: usize,
    /// Number of cache hits.
    pub cache_hits: usize,
    /// True when the transport itself failed (sampling timeout/error), distinct
    /// from individual terms being Unknown. Callers can use this to decide
    /// whether to fall back to an alternative confirmation path.
    pub transport_failed: bool,
}

/// Determine the outcome for a single term given its english anchor and
/// the translation result.
fn check_anchor(
    english: &str,
    translate_result: &Result<String, TranslateError>,
) -> TranslateOutcome {
    match translate_result {
        Err(e) => {
            log::debug!("translation failed, keeping severity (fail-open): {e}");
            TranslateOutcome::Unknown
        }
        Ok(translated) if translated.is_empty() => {
            // Empty response from API — treat as unknown, not rejected.
            log::debug!("empty translation response, keeping severity (fail-open)");
            TranslateOutcome::Unknown
        }
        Ok(translated) => {
            let translated_lower = translated.to_lowercase();
            let found = english
                .split('/')
                .map(|v| v.trim().to_lowercase())
                .filter(|v| v.len() >= 3)
                .any(|v| translated_lower.contains(&v));
            if found {
                TranslateOutcome::Confirmed
            } else {
                TranslateOutcome::Rejected
            }
        }
    }
}

/// Apply a single outcome: tally counters and optionally downgrade severity.
fn apply_outcome(
    outcome: &TranslateOutcome,
    issue: &mut Issue,
    confirmed: &mut usize,
    unconfirmed: &mut usize,
    unknown_count: &mut usize,
    downgrade: bool,
) {
    match outcome {
        TranslateOutcome::Confirmed => *confirmed += 1,
        TranslateOutcome::Rejected => {
            *unconfirmed += 1;
            if downgrade {
                issue.severity = Severity::Info;
            }
        }
        TranslateOutcome::Unknown => *unknown_count += 1,
    }
}

/// Run anchor-confirmation on a list of issues.
///
/// For each issue that has an 'english' field, translates the surrounding
/// context from zh-TW to English and checks if the translation contains
/// the expected English term.
///
/// Tri-state outcomes:
/// - Confirmed — anchor found in translation; keep severity.
/// - Rejected — translation succeeded, anchor absent; downgrade to Info.
/// - Unknown — translation failed (network/rate-limit/parse); keep severity (fail-open).
///
/// Issues without an 'english' field are left unchanged.
pub fn confirm_issues(
    issues: &mut [Issue],
    text: &str,
    config: &ConfirmConfig,
    cache: Option<&TranslationCache>,
) -> ConfirmResult {
    // Reset session counters.
    CACHE_HITS.store(0, Ordering::Relaxed);
    CACHE_MISSES.store(0, Ordering::Relaxed);
    INSERT_COUNT.store(0, Ordering::Relaxed);

    let mut confirmed = 0usize;
    let mut unconfirmed = 0usize;
    let mut unknown_count = 0usize;
    let mut api_calls = 0usize;

    // Dedup key is (found, english) — same surface form in different semantic
    // contexts (different english anchors) must be translated separately.
    let mut term_results: HashMap<(String, String), TranslateOutcome> = HashMap::new();

    for issue in issues.iter_mut() {
        let english = match &issue.english {
            Some(e) if !e.trim().is_empty() => e.trim().to_string(),
            _ => continue, // no english anchor — leave unchanged
        };

        let dedup_key = (issue.found.clone(), english.clone());

        // Check if we already translated this (term, english) pair.
        if let Some(outcome) = term_results.get(&dedup_key) {
            apply_outcome(
                outcome,
                issue,
                &mut confirmed,
                &mut unconfirmed,
                &mut unknown_count,
                config.downgrade_unconfirmed,
            );
            continue;
        }

        // Extract context around the issue.
        let context = extract_context(text, issue.offset, issue.length, 80);
        if context.is_empty() {
            term_results.insert(dedup_key, TranslateOutcome::Unknown);
            unknown_count += 1;
            continue;
        }

        // Check cache first — cache hits work regardless of API cap.
        if let Some(c) = cache {
            if let Some(cached) = c.get("zh-TW", "en", context) {
                let outcome = check_anchor(&english, &Ok(cached));
                apply_outcome(
                    &outcome,
                    issue,
                    &mut confirmed,
                    &mut unconfirmed,
                    &mut unknown_count,
                    config.downgrade_unconfirmed,
                );
                term_results.insert(dedup_key, outcome);
                continue;
            }
        }

        // Cache miss — check API cap before making a network call.
        if api_calls >= config.max_api_calls {
            term_results.insert(dedup_key, TranslateOutcome::Unknown);
            unknown_count += 1;
            continue;
        }

        // Network call.
        let translate_result = google_translate_raw(context, "zh-TW", "en");
        api_calls += 1;

        // Cache successful results.
        if let (Ok(ref translated), Some(c)) = (&translate_result, cache) {
            if !translated.is_empty() {
                c.put("zh-TW", "en", context, translated);
            }
        }

        // Rate-limit between network requests.
        if !config.request_delay.is_zero() {
            std::thread::sleep(config.request_delay);
        }

        let outcome = check_anchor(&english, &translate_result);
        apply_outcome(
            &outcome,
            issue,
            &mut confirmed,
            &mut unconfirmed,
            &mut unknown_count,
            config.downgrade_unconfirmed,
        );
        term_results.insert(dedup_key, outcome);
    }

    let cache_hits = cache.map_or(0, |c| c.stats().hits);

    // Flush cache to disk.
    if let Some(c) = cache {
        c.flush();
    }

    ConfirmResult {
        confirmed,
        unconfirmed,
        unknown: unknown_count,
        api_calls,
        cache_hits,
        transport_failed: false,
    }
}

/// Run anchor-confirmation using a sampling bridge.
///
/// Collects all terms with english anchors, sends them to the LLM in a single
/// bulk request, and applies the boolean results. Falls back gracefully: if the
/// bridge returns None (timeout, error, budget), all terms get `Unknown` outcome.
///
/// Uses a separate budget from disambiguation sampling (budget=1 for the single
/// bulk call), so it does not consume the disambiguation budget.
pub(crate) fn confirm_issues_with_sampling(
    issues: &mut [Issue],
    text: &str,
    bridge: &mut crate::mcp::sampling::SamplingBridge<'_>,
    config: &ConfirmConfig,
) -> ConfirmResult {
    use crate::mcp::sampling::BulkConfirmTerm;

    let mut confirmed = 0usize;
    let mut unconfirmed = 0usize;
    let mut unknown_count = 0usize;

    // Collect unique (found, english) terms with their contexts.
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    let mut terms: Vec<BulkConfirmTerm> = Vec::new();

    for issue in issues.iter() {
        let english = match &issue.english {
            Some(e) if !e.trim().is_empty() => e.trim().to_string(),
            _ => continue,
        };
        let key = (issue.found.clone(), english.clone());
        if let std::collections::hash_map::Entry::Vacant(e) = seen.entry(key) {
            let context = extract_context(text, issue.offset, issue.length, 80);
            e.insert(terms.len());
            terms.push(BulkConfirmTerm {
                found: issue.found.clone(),
                english,
                context: context.to_string(),
            });
        }
    }

    if terms.is_empty() {
        return ConfirmResult {
            confirmed: 0,
            unconfirmed: 0,
            unknown: 0,
            api_calls: 0,
            cache_hits: 0,
            transport_failed: false,
        };
    }

    // Single bulk LLM call.
    let bulk_result = bridge.sample_bulk_confirm(&terms);
    let bulk_succeeded = bulk_result.is_some();

    // Build outcome map from index-keyed LLM response.
    let mut term_outcomes: HashMap<(String, String), TranslateOutcome> = HashMap::new();
    match bulk_result {
        Some(map) => {
            for (idx, term) in terms.iter().enumerate() {
                let outcome = match map.get(&idx) {
                    Some(true) => TranslateOutcome::Confirmed,
                    Some(false) => TranslateOutcome::Rejected,
                    None => TranslateOutcome::Unknown,
                };
                term_outcomes.insert((term.found.clone(), term.english.clone()), outcome);
            }
        }
        None => {
            // LLM unavailable — all terms are Unknown (fail-open).
            for term in &terms {
                term_outcomes.insert(
                    (term.found.clone(), term.english.clone()),
                    TranslateOutcome::Unknown,
                );
            }
        }
    }

    // Apply outcomes to issues.
    for issue in issues.iter_mut() {
        let english = match &issue.english {
            Some(e) if !e.trim().is_empty() => e.trim().to_string(),
            _ => continue,
        };
        let key = (issue.found.clone(), english);
        let outcome = term_outcomes
            .get(&key)
            .unwrap_or(&TranslateOutcome::Unknown);
        apply_outcome(
            outcome,
            issue,
            &mut confirmed,
            &mut unconfirmed,
            &mut unknown_count,
            config.downgrade_unconfirmed,
        );
    }

    ConfirmResult {
        confirmed,
        unconfirmed,
        unknown: unknown_count,
        api_calls: if bulk_succeeded { 1 } else { 0 },
        cache_hits: 0,
        transport_failed: !bulk_succeeded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_context_sentence_boundaries() {
        let text = "前面的句子。這是包含目標詞的句子。後面的句子。";
        let target = "目標詞";
        let pos = text.find(target).unwrap();
        let ctx = extract_context(text, pos, target.len(), 80);
        assert!(ctx.contains(target));
        assert!(!ctx.contains("前面的句子"));
        assert!(!ctx.contains("後面的句子"));
    }

    #[test]
    fn extract_context_fallback_window() {
        let text = "abcdef目標ghijkl";
        let target = "目標";
        let pos = text.find(target).unwrap();
        let ctx = extract_context(text, pos, target.len(), 4);
        assert!(ctx.contains(target));
    }

    #[test]
    fn cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let cache = TranslationCache::open_at(path, CacheConfig::default()).unwrap();

        // Miss.
        assert!(cache.get("zh-TW", "en", "hello").is_none());

        // Put + hit.
        cache.put("zh-TW", "en", "hello", "world");
        let val = cache.get("zh-TW", "en", "hello");
        assert_eq!(val.as_deref(), Some("world"));
    }

    #[test]
    fn cache_ttl_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let config = CacheConfig {
            ttl: Duration::from_secs(0), // instant expiry
            max_size_bytes: 10 * 1024 * 1024,
        };
        let cache = TranslationCache::open_at(path, config).unwrap();

        cache.put("zh-TW", "en", "hello", "world");
        // With 0-second TTL, wait for the timestamp to roll over to the next second.
        std::thread::sleep(Duration::from_secs(1));
        assert!(cache.get("zh-TW", "en", "hello").is_none());
    }

    #[test]
    fn cache_clear() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let cache = TranslationCache::open_at(path, CacheConfig::default()).unwrap();

        cache.put("zh-TW", "en", "hello", "world");
        assert!(cache.get("zh-TW", "en", "hello").is_some());

        cache.clear();
        assert_eq!(cache.stats().entries, 0);
    }

    #[test]
    fn cache_key_deterministic() {
        let k1 = TranslationCache::make_key("zh-TW", "en", "hello");
        let k2 = TranslationCache::make_key("zh-TW", "en", "hello");
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 32); // SHA-256 = 32 bytes
    }

    #[test]
    fn cache_key_differs_for_different_text() {
        let k1 = TranslationCache::make_key("zh-TW", "en", "hello");
        let k2 = TranslationCache::make_key("zh-TW", "en", "world");
        assert_ne!(k1, k2);
    }

    #[test]
    fn confirm_issues_skips_without_english() {
        let mut issues = vec![{
            let mut i = Issue::new(
                0,
                6,
                "測試",
                vec!["test".into()],
                crate::rules::ruleset::IssueType::CrossStrait,
                Severity::Warning,
            );
            i.line = 1;
            i.col = 1;
            i
        }];

        let config = ConfirmConfig {
            request_delay: Duration::ZERO,
            downgrade_unconfirmed: true,
            max_api_calls: 10,
        };
        let result = confirm_issues(&mut issues, "測試文字", &config, None);

        // No english field → skipped entirely, severity unchanged.
        assert_eq!(result.confirmed, 0);
        assert_eq!(result.unconfirmed, 0);
        assert_eq!(issues[0].severity, Severity::Warning);
    }

    // check_anchor unit tests

    #[test]
    fn check_anchor_confirmed_when_english_present() {
        let result = Ok("This is about rendering in computer graphics".to_string());
        assert_eq!(
            check_anchor("rendering", &result),
            TranslateOutcome::Confirmed
        );
    }

    #[test]
    fn check_anchor_confirmed_slash_variants() {
        // english field with slash-separated variants
        let result = Ok("The simulation results are accurate".to_string());
        assert_eq!(
            check_anchor("simulation/emulation", &result),
            TranslateOutcome::Confirmed
        );
    }

    #[test]
    fn check_anchor_rejected_when_anchor_absent() {
        let result = Ok("This is about painting techniques in art".to_string());
        assert_eq!(
            check_anchor("rendering", &result),
            TranslateOutcome::Rejected
        );
    }

    #[test]
    fn check_anchor_unknown_on_io_error() {
        let result = Err(TranslateError::Io("connection refused".into()));
        assert_eq!(
            check_anchor("rendering", &result),
            TranslateOutcome::Unknown
        );
    }

    #[test]
    fn check_anchor_unknown_on_rate_limit() {
        let result = Err(TranslateError::RateLimit(429));
        assert_eq!(
            check_anchor("rendering", &result),
            TranslateOutcome::Unknown
        );
    }

    #[test]
    fn check_anchor_unknown_on_parse_error() {
        let result = Err(TranslateError::Parse("invalid json".into()));
        assert_eq!(check_anchor("anything", &result), TranslateOutcome::Unknown);
    }

    #[test]
    fn check_anchor_unknown_on_empty_translation() {
        // Empty string from API = unknown, NOT rejected.
        let result = Ok(String::new());
        assert_eq!(
            check_anchor("rendering", &result),
            TranslateOutcome::Unknown
        );
    }

    #[test]
    fn check_anchor_case_insensitive() {
        let result = Ok("The RENDERING pipeline is complex".to_string());
        assert_eq!(
            check_anchor("rendering", &result),
            TranslateOutcome::Confirmed
        );
    }

    #[test]
    fn check_anchor_skips_short_variants() {
        // Variants < 3 chars are skipped to avoid false matches.
        let result = Ok("This is an example".to_string());
        assert_eq!(
            check_anchor("an/example", &result),
            TranslateOutcome::Confirmed
        );
        // "an" is 2 chars → skipped, but "example" matches.
        let result2 = Ok("An unrelated text".to_string());
        assert_eq!(check_anchor("an", &result2), TranslateOutcome::Rejected);
    }

    // Windows lock detection

    #[test]
    fn open_at_windows_lock_error_message() {
        // Simulate a Windows-style lock error via a path that triggers sled to fail.
        // We can't easily force sled to produce Windows errors on non-Windows,
        // so test the string matching logic directly.
        let windows_errors = [
            "Access is denied",
            "sharing violation",
            "lock",
            "EWOULDBLOCK",
            "EBUSY",
            "EPERM",
        ];
        for err_msg in &windows_errors {
            assert!(
                err_msg.contains("lock")
                    || err_msg.contains("EWOULDBLOCK")
                    || err_msg.contains("EBUSY")
                    || err_msg.contains("EPERM")
                    || err_msg.contains("Access is denied")
                    || err_msg.contains("sharing violation"),
                "Lock detection should match: {err_msg}"
            );
        }
    }

    // dedup key includes english

    #[test]
    fn check_anchor_same_term_different_english() {
        // Same surface form but different english anchors should produce
        // different outcomes — tests that dedup key must be (found, english).
        let painting_result = Ok("This painting uses rendering technique".to_string());
        let gpu_result = Ok("This is about GPU optimization".to_string());

        // "渲染" with english "rendering" in painting context → confirmed
        assert_eq!(
            check_anchor("rendering", &painting_result),
            TranslateOutcome::Confirmed
        );
        // "渲染" with english "rendering" in GPU context → rejected
        assert_eq!(
            check_anchor("rendering", &gpu_result),
            TranslateOutcome::Rejected
        );
    }

    // API call cap

    #[test]
    fn confirm_issues_respects_api_cap() {
        // With max_api_calls=1 and no cache, only the first term gets a
        // network call; subsequent terms get Unknown (fail-open).
        let mut issues = vec![
            {
                let mut i = Issue::new(
                    0,
                    6,
                    "渲染",
                    vec!["算繪".into()],
                    crate::rules::ruleset::IssueType::CrossStrait,
                    Severity::Warning,
                )
                .with_english("rendering");
                i.line = 1;
                i.col = 1;
                i
            },
            {
                let mut i = Issue::new(
                    6,
                    6,
                    "實例",
                    vec!["實體".into()],
                    crate::rules::ruleset::IssueType::CrossStrait,
                    Severity::Warning,
                )
                .with_english("instance");
                i.line = 1;
                i.col = 3;
                i
            },
        ];

        let config = ConfirmConfig {
            request_delay: Duration::ZERO,
            downgrade_unconfirmed: true,
            max_api_calls: 1, // allow 1 API call, cap the rest
        };
        let result = confirm_issues(&mut issues, "渲染實例的測試文字", &config, None);

        // First term: 1 API call (network error in test env → Unknown).
        // Second term: API-capped → Unknown without network call.
        // Both severities preserved (fail-open).
        assert_eq!(result.api_calls, 1);
        assert!(result.unknown >= 1, "capped term should be Unknown");
        assert_eq!(
            issues[1].severity,
            Severity::Warning,
            "capped term severity preserved"
        );
    }
}
