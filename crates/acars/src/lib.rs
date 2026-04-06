#![doc = include_str!("../readme.md")]

pub mod decode;
pub mod demod;

pub mod prelude {
    pub use deku::prelude::*;

    pub use crate::decode::acars::{
        AcarsMessage, AcarsRawFrame, AckType, MessageDirection, ReassemblyHint,
    };
    pub use crate::decode::adsc::{parse_adsc_app_text, AdscMessage};
    pub use crate::decode::arinc622::{
        parse_arinc622_envelope, dispatch_by_imi, Arinc622Envelope, AppPayload,
    };
    pub use crate::decode::avlc::{
        parse_avlc_frame, AvlcAddr, AvlcFrame, AvlcLcf, AvlcPayload, SFunc,
    };
    pub use crate::decode::{DecodeError, DecodeResult};
}
