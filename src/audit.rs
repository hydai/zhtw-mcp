// Audit tracing: per-invocation reproducibility metadata.
//
// Each tool call generates a Trace with a unique trace_id (SHA-256 of
// timestamp + PID + counter + urandom seed), hashes of input/output text,
// and the ruleset hash. This enables deterministic replay.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Monotonic counter to guarantee uniqueness even within the same nanosecond.
static TRACE_SEQ: AtomicU64 = AtomicU64::new(0);

/// 16-byte random seed read once from /dev/urandom at first use.
/// Prevents trace_id collisions between processes with identical PID and clock
/// (common in containers where PID 1 restarts frequently).
static URANDOM_SEED: OnceLock<[u8; 16]> = OnceLock::new();

fn urandom_seed() -> &'static [u8; 16] {
    URANDOM_SEED.get_or_init(|| {
        let mut seed = [0u8; 16];
        // Best-effort: if /dev/urandom is unavailable (e.g., sandboxed), fall
        // back to all-zeros (still unique within a process via TRACE_SEQ).
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            let _ = f.read_exact(&mut seed);
        }
        seed
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Trace {
    pub trace_id: String,
    pub ruleset_hash: String,
    pub input_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_hash: Option<String>,
    pub timestamp: String,
    pub tool: String,
    pub issue_count: usize,
}

impl Trace {
    pub fn new(tool: &str, ruleset_hash: &str, input: &str) -> Self {
        Self {
            trace_id: generate_trace_id(),
            ruleset_hash: ruleset_hash.to_owned(),
            input_hash: sha256_hex(input.as_bytes()),
            output_hash: None,
            timestamp: now_iso8601(),
            tool: tool.to_owned(),
            issue_count: 0,
        }
    }

    pub fn with_issue_count(mut self, count: usize) -> Self {
        self.issue_count = count;
        self
    }

    pub fn with_output(mut self, output: &str) -> Self {
        self.output_hash = Some(sha256_hex(output.as_bytes()));
        self
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

/// Generate a unique trace ID from timestamp + PID + atomic counter, hashed
/// with SHA-256. Returns the first 32 hex characters (128 bits of entropy,
/// matching UUIDv4 uniqueness). No external RNG crate needed.
fn generate_trace_id() -> String {
    use std::process;
    use std::time::SystemTime;

    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = TRACE_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = process::id();

    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(pid.to_le_bytes());
    hasher.update(seq.to_le_bytes());
    hasher.update(urandom_seed());
    let digest = hasher.finalize();
    // Take first 16 bytes (128 bits) → 32 hex chars.
    let mut hex = String::with_capacity(32);
    for b in &digest[..16] {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

fn now_iso8601() -> String {
    // Use UNIX_EPOCH + SystemTime for a dependency-free ISO 8601 timestamp.
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let millis = dur.subsec_millis();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Convert days since epoch to Y-M-D (simplified Gregorian).
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Algorithm from Howard Hinnant's chrono-compatible date library.
    days += 719_468;
    let era = days / 146_097;
    let doe = days % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_deterministic() {
        let a = sha256_hex(b"hello");
        let b = sha256_hex(b"hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn sha256_known_value() {
        let h = sha256_hex(b"hello");
        assert_eq!(
            h,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn trace_unique_ids() {
        let t1 = Trace::new("zhtw", "abc", "text");
        let t2 = Trace::new("zhtw", "abc", "text");
        assert_ne!(t1.trace_id, t2.trace_id);
        // Same input → same input_hash
        assert_eq!(t1.input_hash, t2.input_hash);
    }

    #[test]
    fn trace_output_hash() {
        let t = Trace::new("zhtw", "abc", "input")
            .with_output("output")
            .with_issue_count(3);
        assert!(t.output_hash.is_some());
        assert_eq!(t.issue_count, 3);
    }

    #[test]
    fn timestamp_format() {
        let ts = now_iso8601();
        // Basic format check: YYYY-MM-DDTHH:MM:SS.mmmZ
        assert_eq!(ts.len(), 24);
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[19..20], ".");
    }
}
