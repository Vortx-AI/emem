//! Memory consolidation knobs — TTL + consolidation thresholds read
//! from environment, with safe defaults. The actual background workers
//! live in `emem-api-rest` because they need the AppState + sled
//! handles, but the policy knobs are public so tests + ops can flip
//! them with short intervals.

#![forbid(unsafe_code)]

/// Read a positive unsigned env var with a default. `0` is treated as a
/// legitimate value (the TTL parsing layer turns 0 into "infinite").
pub fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

/// True when `EMEM_MEMORY_TTL_ENABLED=1` (or any non-empty value other
/// than `0` / `false`).
pub fn ttl_enabled() -> bool {
    truthy("EMEM_MEMORY_TTL_ENABLED")
}

/// True when `EMEM_MEMORY_CONSOLIDATION_ENABLED=1`.
pub fn consolidation_enabled() -> bool {
    truthy("EMEM_MEMORY_CONSOLIDATION_ENABLED")
}

/// TTL scan interval, default 3600 s. Tests can drop this to a few
/// seconds via `EMEM_MEMORY_TTL_INTERVAL_SECS`.
pub fn ttl_interval_secs() -> u64 {
    env_u64("EMEM_MEMORY_TTL_INTERVAL_SECS", 3600).max(1)
}

/// Consolidation scan interval, default 86400 s.
pub fn consolidation_interval_secs() -> u64 {
    env_u64("EMEM_MEMORY_CONSOLIDATION_INTERVAL_SECS", 86_400).max(1)
}

/// Minimum files in one directory to trigger consolidation, default 50.
pub fn consolidation_min_files() -> usize {
    env_u64("EMEM_MEMORY_CONSOLIDATION_MIN_FILES", 50) as usize
}

/// Minimum age (days) before a file is eligible for consolidation,
/// default 7.
pub fn consolidation_min_age_days() -> u64 {
    env_u64("EMEM_MEMORY_CONSOLIDATION_MIN_AGE_DAYS", 7)
}

/// Max concurrent SSE subscribers, default 256.
pub fn sse_max_subs() -> usize {
    env_u64("EMEM_SSE_MAX_SUBS", 256) as usize
}

fn truthy(key: &str) -> bool {
    match std::env::var(key) {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            !(t.is_empty() || t == "0" || t == "false" || t == "no")
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        // Don't assume the host's env is clean; just round-trip the
        // numeric defaults via a guaranteed-absent var name.
        assert_eq!(env_u64("EMEM_TEST_NOT_SET_xyzzy_42", 7), 7);
    }
}
