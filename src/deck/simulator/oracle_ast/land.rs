//! Typed conditions that replace a land's tapped entry.

/// A complete land-entry condition before it is lowered to runtime data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleLandEntryCondition {
    /// Count only lands already controlled before entry.
    OtherLandsAtMost(u32),
    /// Require the stated number of existing lands.
    OtherLandsAtLeast(u32),
    /// Require the stated number of active opponents.
    OpponentsAtLeast(u32),
    /// Control a land with any listed basic land type, in WUBRG order.
    ControlsLandTypes([bool; 5]),
}
