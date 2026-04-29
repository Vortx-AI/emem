//! Progressive-refinement scheduling for SSE responses. Spec §9.3.
//!
//! Recall and query_region streams MUST emit chunks from coarsest resolution
//! to finest. This module computes the coarse→fine schedule given a request's
//! target resolution.

/// Compute the resolution schedule for a target res. Always emits res-9, res-11,
/// then the target resolution (if different).
pub fn schedule(target_res: u8) -> Vec<u8> {
    let mut s = Vec::with_capacity(3);
    if target_res >= 9 {
        s.push(9);
    }
    if target_res >= 11 && target_res != 9 {
        s.push(11);
    }
    if !s.contains(&target_res) {
        s.push(target_res);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_for_res_13() {
        assert_eq!(schedule(13), vec![9, 11, 13]);
    }
    #[test]
    fn schedule_for_res_9() {
        assert_eq!(schedule(9), vec![9]);
    }
}
