//! Privacy classes — spec §13.
//!
//! Implementations MUST enforce these at every conformance level. Bands with
//! `AggregateOnly` or `L2OnlyWithModelCid` MUST be snapped or refused per the
//! request's resolution and the responder's conformance level.
//!
//! Wire form is internally tagged JSON/CBOR with discriminator `"class"`:
//!
//! ```json
//! {"class": "public"}
//! {"class": "aggregate_only", "min_res": 11}
//! {"class": "l2_only_with_model_cid"}
//! {"class": "prohibited"}
//! ```

use serde::{Deserialize, Serialize};

/// Privacy class declared per band. Enforced before serving facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "class", rename_all = "snake_case")]
pub enum PrivacyClass {
    /// Unrestricted at any resolution.
    Public,

    /// Implementations MUST NOT serve at resolution finer than `min_res`.
    /// Queries at finer res return aggregated values from the parent at
    /// `min_res`, with `privacy_snapped: true` in the receipt.
    AggregateOnly {
        /// Coarsest (lowest-numbered) resolution at which serving is permitted.
        /// Picked at city-block scale (~24m at res 11) to avoid identifying
        /// individual buildings.
        min_res: u8,
    },

    /// Admissible only at conformance level L2; requires `Source.cid` of the
    /// model checkpoint that produced the band value.
    L2OnlyWithModelCid,

    /// Reserved. Conforming implementations MUST refuse to serve.
    Prohibited,
}

impl PrivacyClass {
    /// Returns true if a request at `requested_res` should be allowed without
    /// snapping. False means the implementation must snap up to a coarser
    /// resolution or refuse outright.
    pub fn permits_resolution(self, requested_res: u8, conformance_l2: bool) -> bool {
        match self {
            PrivacyClass::Public => true,
            PrivacyClass::AggregateOnly { min_res } => requested_res <= min_res,
            PrivacyClass::L2OnlyWithModelCid => conformance_l2,
            PrivacyClass::Prohibited => false,
        }
    }
}
