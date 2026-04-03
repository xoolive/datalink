#![doc = include_str!("../readme.md")]

pub mod decode;

pub mod prelude {
    pub use deku::prelude::*;

    pub use crate::decode::acars::{
        AcarsMessage, AcarsRawFrame, AckType, MessageDirection, ReassemblyHint,
    };
    pub use crate::decode::adsc::{parse_adsc_app_text, AdscMessage};
    pub use crate::decode::avlc::{
        parse_avlc_frame, AvlcAddr, AvlcFrame, AvlcLcf, AvlcPayload, SFunc,
    };
    pub use crate::decode::{DecodeError, DecodeResult};
}
