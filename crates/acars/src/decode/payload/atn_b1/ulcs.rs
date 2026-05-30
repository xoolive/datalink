//! ATN B1 Upper Layer Communications Service (ULCS) types.
//!
//! Provides the minimal set of ASN.1 UPER types needed to decode
//! ATN CPDLC COTP user data:
//!   - [`FullyEncodedData`]: SEQUENCE { spare NULL OPTIONAL, data PDV-list }
//!   - [`PdvList`]: PDV-list with context identifier and presentation data values

use rasn::prelude::*;

/// Fully-encoded-data (ATN B1 ULCS, atn-b1_ulcs.asn1)
/// SEQUENCE { spare NULL OPTIONAL, data PDV-list }
#[derive(AsnType, Debug, Clone, Decode, Encode, PartialEq, Eq, Hash)]
pub struct FullyEncodedData {
    pub spare: Option<()>,
    pub data: PdvList,
}

/// PDV-list
#[derive(AsnType, Debug, Clone, Decode, Encode, PartialEq, Eq, Hash)]
pub struct PdvList {
    pub transfer_syntax_name: Option<ObjectIdentifier>,
    pub presentation_context_identifier: PresentationContextIdentifier,
    pub presentation_data_values: PdvDataValues,
}

/// Presentation-context-identifier (1..127 extensible)
#[derive(AsnType, Debug, Clone, Decode, Encode, PartialEq, Eq, Hash)]
#[rasn(delegate, value("1..=127", extensible))]
pub struct PresentationContextIdentifier(pub Integer);

/// PDV-list presentation-data-values CHOICE
#[derive(AsnType, Debug, Clone, Decode, Encode, PartialEq, Eq, Hash)]
#[rasn(choice)]
pub enum PdvDataValues {
    #[rasn(tag(context, 0))]
    SingleAsn1Type(Any),
    #[rasn(tag(context, 1))]
    OctetAligned(OctetString),
    #[rasn(tag(context, 2))]
    Arbitrary(BitString),
}
