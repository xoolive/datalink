pub mod acars;
pub mod avlc;
pub mod helpers;
pub mod payload;
pub mod x25;
pub mod xid;

use crate::decode::payload::PayloadError;
use thiserror::Error;

pub type DecodeResult<T> = Result<T, DecodeError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("frame too short: {0} bytes")]
    FrameTooShort(usize),
    #[error("missing trailing DEL byte")]
    MissingDel,
    #[error("missing ETX/ETB terminator")]
    MissingTextTerminator,
    #[error("missing STX after ACARS preamble")]
    MissingStx,
    #[error("missing downlink text fields")]
    MissingDownlinkFields,
    #[error("invalid direction for requested operation")]
    InvalidDirection,
    #[error("ACARS CRC check failed")]
    CrcFail,
    #[error("invalid payload: {0}")]
    InvalidPayload(#[from] PayloadError),
    #[error("deku parse error: {0}")]
    Deku(String),
    #[error("invalid VDL frame")]
    InvalidVdlFrame,
}
