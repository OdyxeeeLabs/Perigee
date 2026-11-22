//! Fee collector helpers — maps on-chain fee events into persistence rows.

use super::service::FeeRecordView;

/// Parse a fee_engine `fee_chg` event payload into a [`FeeRecordView`].
///
/// Expected topic: (`fee_chg`, vault, kind)
/// Expected data: (seq, payer, gross, discount, net, vault_share, protocol_share, timestamp)
pub fn from_event_parts(
    vault: String,
    kind: u32,
    seq: u64,
    payer: String,
    gross: i128,
    discount: i128,
    net: i128,
    vault_share: i128,
    protocol_share: i128,
    timestamp: u64,
) -> FeeRecordView {
    FeeRecordView {
        seq,
        vault,
        payer,
        gross,
        discount,
        net,
        vault_share,
        protocol_share,
        timestamp,
        kind,
    }
}
