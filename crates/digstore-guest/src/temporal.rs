//! Temporal keys (§16). A request may carry a validity window; the guest checks
//! it against host_get_current_time. Outside the window -> the content path
//! returns a decoy (indistinguishable from a real miss).

use crate::request::ValidityWindow;

/// True iff `now` is within `[not_before, not_after]`, or no window is set.
pub fn within_window(window: &Option<ValidityWindow>, now: u64) -> bool {
    match window {
        None => true,
        Some(w) => now >= w.not_before && now <= w.not_after,
    }
}
