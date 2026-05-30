//! ATN B1 (ICAO Doc 9705 Suite B) application-layer decoder for VDL2.
//!
//! ATN B1 protocols are carried over the VDL2 X.25/COTP path.
//! After COTP DT reassembly, the user data is a null-session/presentation
//! header (`0x00`) followed by a UPER-encoded `Fully-encoded-data` PDU.
//!
//! ## Application types
//!
//! | App | Module | Status |
//! |-----|--------|--------|
//! | CPC (CPDLC) | [`cpdlc`] | ✅ implemented |
//! | CMA (Context Management) | cm | future |
//! | ADS-C v2 | adsc | future |

pub mod adsc;
pub mod cm;
pub mod cpdlc;
pub mod ulcs;

use crate::decode::payload::arinc622::cpdlc::CpdlcPduSummary;
use ulcs::{FullyEncodedData, PdvDataValues};

/// Attempt to decode a COTP DT `user_data` field as an ATN B1 application PDU.
///
/// Currently only CPC (CPDLC) is decoded; other application types are
/// returned as `None` until their decoders are implemented.
///
/// Returns `None` if the data is not ATN-encoded or decode fails.
pub fn decode_cotp_user_data(user_data: &[u8]) -> Option<AtnB1Pdu> {
    if user_data.first() != Some(&0x00) {
        return None;
    }
    let fed = rasn::uper::decode::<FullyEncodedData>(user_data).ok()?;
    let PdvDataValues::Arbitrary(bits) = &fed.data.presentation_data_values else {
        return None;
    };
    let inner = bits.as_raw_slice();
    // Try CPC (CPDLC)
    if let Some((summary, _kind)) = cpdlc::decode_inner(inner) {
        return Some(AtnB1Pdu::Cpdlc(summary));
    }
    None
}

/// Decoded ATN B1 application PDU.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "app", rename_all = "snake_case")]
pub enum AtnB1Pdu {
    Cpdlc(CpdlcPduSummary),
}
