//! Fee service orchestration — sponsored fees and transparency queries (#669, #670).

use serde::{Deserialize, Serialize};

/// Kind of on-chain fee event (mirrors fee_engine `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum FeeKind {
    Management = 1,
    Performance = 2,
    Combined = 3,
}

/// Off-chain view of a fee_engine `FeeRecord` for analytics integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeRecordView {
    pub seq: u64,
    pub vault: String,
    pub payer: String,
    pub gross: i128,
    pub discount: i128,
    pub net: i128,
    pub vault_share: i128,
    pub protocol_share: i128,
    pub timestamp: u64,
    pub kind: u32,
}

/// Request to pay fees via a sponsor (meta-transaction style).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SponsoredFeeRequest {
    pub vault: String,
    pub sponsor: String,
    pub nav: i128,
}

/// Validates a sponsored fee request before submitting the on-chain call.
pub fn validate_sponsored_request(req: &SponsoredFeeRequest) -> Result<(), &'static str> {
    if req.vault.is_empty() || req.sponsor.is_empty() {
        return Err("vault and sponsor are required");
    }
    if req.nav < 0 {
        return Err("nav must be non-negative");
    }
    Ok(())
}
