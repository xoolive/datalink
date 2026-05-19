#![doc = include_str!("../readme.md")]

pub mod decode;
pub mod demod;

pub mod prelude {
    pub use deku::prelude::*;

    pub use crate::decode::acars::{AcarsMessage, AckType, MessageDirection, ReassemblyHint};
    pub use crate::decode::avlc::{
        parse_avlc_frame, AvlcAddr, AvlcFrame, AvlcLcf, AvlcPayload, SFunc,
    };
    pub use crate::decode::payload::{
        arinc622::{
            adsc::{parse_adsc_app_text, AdscMessage},
            Imi, Message as Arinc622Message, Payload as Arinc622Payload,
        },
        sq::{parse_squitter, SquitterLink, SquitterMessage},
        AcarsAppPayload,
    };
    pub use crate::decode::{DecodeError, DecodeResult};
}
