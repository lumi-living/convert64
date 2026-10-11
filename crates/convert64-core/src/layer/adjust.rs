//! Per-layer colour adjustments (UX §6, group 2). Every field defaults to identity.

use serde::{Deserialize, Serialize};

/// A layer's colour adjustments. The passes themselves arrive with E4's adjustments
/// child; until then the only value is identity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Adjustments {}

impl Adjustments {
    /// Whether every adjustment is at its identity value.
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}
