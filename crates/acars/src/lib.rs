#![doc = include_str!("../readme.md")]

pub mod decode;
#[cfg(feature = "demod")]
pub mod demod;

pub mod prelude {
    pub use crate::decode::acars::{AcarsMessage, AckType, MessageDirection, ReassemblyHint};
    pub use crate::decode::avlc::{
        parse_avlc_frame, AvlcAddr, AvlcFrame, AvlcLcf, AvlcPayload, SFunc,
    };
    pub use crate::decode::payload::{
        arinc620::squitter::{parse_squitter, SquitterLink, SquitterMessage},
        arinc622::{
            adsc::{parse_adsc_app_text, AdscMessage},
            Imi, Message as Arinc622Message, Payload as Arinc622Payload,
        },
        AcarsAppPayload,
    };
    pub use crate::decode::{DecodeError, DecodeResult};
}
