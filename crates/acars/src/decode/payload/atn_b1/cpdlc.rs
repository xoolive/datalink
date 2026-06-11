//! ATN B1 CPC (CPDLC) application decoder.
//!
//! Decodes `ProtectedGroundPdus` / `ProtectedAircraftPDUs` from a ULCS
//! Fully-encoded-data arbitrary BIT STRING and converts the result to
//! [`CpdlcPduSummary`] / [`CpdlcElement`], identical in shape to the
//! FANS-1/A decoder output.

use rasn_atn_cpdlc::{
    AtcDownlinkMessage, AtcDownlinkMsgElementId, AtcUplinkMessage, AtcUplinkMsgElementId, Facility,
    FacilityDesignation, FacilityFunction, Frequency, ProtectedAircraftPDUs, ProtectedGroundPdus,
    UnitName,
};

use crate::decode::payload::arinc622::cpdlc::{
    AtcMessageHeader as FansHeader, CpdlcElement, CpdlcElementBody, CpdlcFrequency,
    CpdlcPduSummary, CpdlcTimestamp, IcaoFacilityFunction, IcaoFacilityIdentification,
    IcaoUnitName, PduKind,
};

/// Attempt to decode ATN B1 CPDLC from the inner bytes of a ULCS PDV-list
/// arbitrary BIT STRING (after Fully-encoded-data has been stripped).
pub(super) fn decode_inner(inner: &[u8]) -> Option<(CpdlcPduSummary, PduKind)> {
    // Try uplink (ground → aircraft)
    if let Ok(ProtectedGroundPdus::Send(pum)) = rasn::uper::decode::<ProtectedGroundPdus>(inner) {
        if let Some(msg_bits) = pum.protected_message {
            if let Ok(msg) = rasn::uper::decode::<AtcUplinkMessage>(msg_bits.0.as_raw_slice()) {
                return Some((uplink_to_summary(msg), PduKind::Uplink));
            }
        }
    }
    // Try downlink (aircraft → ground)
    if let Ok(ProtectedAircraftPDUs::Send(pdm)) = rasn::uper::decode::<ProtectedAircraftPDUs>(inner)
    {
        if let Some(msg_bits) = pdm.protected_message {
            if let Ok(msg) = rasn::uper::decode::<AtcDownlinkMessage>(msg_bits.0.as_raw_slice()) {
                return Some((downlink_to_summary(msg), PduKind::Downlink));
            }
        }
    }
    None
}

fn uplink_to_summary(msg: AtcUplinkMessage) -> CpdlcPduSummary {
    CpdlcPduSummary {
        header: convert_header(&msg.header),
        elements: msg
            .message_data
            .element_ids
            .iter()
            .map(convert_uplink_element)
            .collect(),
        remaining_bits_after_element: 0,
    }
}

fn downlink_to_summary(msg: AtcDownlinkMessage) -> CpdlcPduSummary {
    CpdlcPduSummary {
        header: convert_header(&msg.header),
        elements: msg
            .message_data
            .element_ids
            .iter()
            .map(convert_downlink_element)
            .collect(),
        remaining_bits_after_element: 0,
    }
}

fn convert_header(h: &rasn_atn_cpdlc::AtcMessageHeader) -> FansHeader {
    FansHeader {
        msg_id: h.message_id_number.0,
        msg_ref: h.message_ref_number.as_ref().map(|r| r.0),
        timestamp: Some(CpdlcTimestamp {
            hour: h.date_time.timehhmmss.hoursminutes.hours.0,
            minute: h.date_time.timehhmmss.hoursminutes.minutes.0,
            second: h.date_time.timehhmmss.seconds.0,
        }),
    }
}

fn make_element(id: u16, _kind: PduKind, body: Option<CpdlcElementBody>) -> CpdlcElement {
    CpdlcElement {
        id,
        body,
        is_additional: false,
    }
}

fn convert_uplink_element(e: &AtcUplinkMsgElementId) -> CpdlcElement {
    use AtcUplinkMsgElementId::*;
    let (id, body) = match e {
        UM0Null(()) => (0, Some(CpdlcElementBody::Null)),
        UM1Null(()) => (1, Some(CpdlcElementBody::Null)),
        UM2Null(()) => (2, Some(CpdlcElementBody::Null)),
        UM3Null(()) => (3, Some(CpdlcElementBody::Null)),
        UM4Null(()) => (4, Some(CpdlcElementBody::Null)),
        UM5Null(()) => (5, Some(CpdlcElementBody::Null)),
        UM107Null(()) => (107, Some(CpdlcElementBody::Null)),
        UM116Null(()) => (116, Some(CpdlcElementBody::Null)),
        UM127Null(()) => (127, Some(CpdlcElementBody::Null)),
        UM132Null(()) => (132, Some(CpdlcElementBody::Null)),
        UM137Null(()) => (137, Some(CpdlcElementBody::Null)),
        UM141Null(()) => (141, Some(CpdlcElementBody::Null)),
        UM144Null(()) => (144, Some(CpdlcElementBody::Null)),
        UM147Null(()) => (147, Some(CpdlcElementBody::Null)),
        UM154Null(()) => (154, Some(CpdlcElementBody::Null)),
        UM161Null(()) => (161, Some(CpdlcElementBody::Null)),
        UM167Null(()) => (167, Some(CpdlcElementBody::Null)),
        UM191Null(()) => (191, Some(CpdlcElementBody::Null)),
        UM193Null(()) => (193, Some(CpdlcElementBody::Null)),
        UM200Null(()) => (200, Some(CpdlcElementBody::Null)),
        UM227Null(()) => (227, Some(CpdlcElementBody::Null)),
        UM134SpeedTypeSpeedTypeSpeedType(_) => (134, Some(CpdlcElementBody::Null)),
        UM166TrafficType(_) => (166, Some(CpdlcElementBody::Null)),
        UM183FreeText(ft) => (183, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        UM187FreeText(ft) => (187, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        UM194FreeText(ft) => (194, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        UM195FreeText(ft) => (195, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        UM196FreeText(ft) => (196, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        UM197FreeText(ft) => (197, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        UM198FreeText(ft) => (198, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        UM199FreeText(ft) => (199, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        UM160Facility(f) => (160, Some(facility_body(f))),
        UM163FacilityDesignation(fd) => (
            163,
            Some(CpdlcElementBody::IcaoFacilityDesignation(fd_str(fd))),
        ),
        UM117UnitNameFrequency(unf) => (117, Some(unit_freq(&unf.unit_name, &unf.frequency))),
        UM118PositionUnitNameFrequency(_) => (118, None),
        UM119TimeUnitNameFrequency(_) => (119, None),
        UM120UnitNameFrequency(unf) => (120, Some(unit_freq(&unf.unit_name, &unf.frequency))),
        UM121PositionUnitNameFrequency(_) => (121, None),
        UM122TimeUnitNameFrequency(_) => (122, None),
        UM123Code(code) => {
            let digits: String = code.0.iter().map(|d| (b'0' + d.0) as char).collect();
            (123, Some(CpdlcElementBody::BeaconCode(digits)))
        }
        other => (id_from_debug(&format!("{other:?}"), "UM"), None),
    };
    make_element(id, PduKind::Uplink, body)
}

fn convert_downlink_element(e: &AtcDownlinkMsgElementId) -> CpdlcElement {
    use AtcDownlinkMsgElementId::*;
    let (id, body) = match e {
        DM0Null(()) => (0, Some(CpdlcElementBody::Null)),
        DM1Null(()) => (1, Some(CpdlcElementBody::Null)),
        DM2Null(()) => (2, Some(CpdlcElementBody::Null)),
        DM3Null(()) => (3, Some(CpdlcElementBody::Null)),
        DM4Null(()) => (4, Some(CpdlcElementBody::Null)),
        DM5Null(()) => (5, Some(CpdlcElementBody::Null)),
        DM63Null(()) => (63, Some(CpdlcElementBody::Null)),
        DM65Null(()) => (65, Some(CpdlcElementBody::Null)),
        DM66Null(()) => (66, Some(CpdlcElementBody::Null)),
        DM99Null(()) => (99, Some(CpdlcElementBody::Null)),
        DM100Null(()) => (100, Some(CpdlcElementBody::Null)),
        DM67FreeText(ft) => (67, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        DM68FreeText(ft) => (68, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        DM90FreeText(ft) => (90, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        DM91FreeText(ft) => (91, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        DM92FreeText(ft) => (92, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        DM93FreeText(ft) => (93, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        DM94FreeText(ft) => (94, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        DM95FreeText(ft) => (95, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        DM96FreeText(ft) => (96, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        DM97FreeText(ft) => (97, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        DM98FreeText(ft) => (98, Some(CpdlcElementBody::FreeText(ia5_str(&ft.0)))),
        DM64FacilityDesignation(fd) => (
            64,
            Some(CpdlcElementBody::IcaoFacilityDesignation(fd_str(fd))),
        ),
        other => (id_from_debug(&format!("{other:?}"), "DM"), None),
    };
    make_element(id, PduKind::Downlink, body)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn ia5_str(s: &rasn::types::Ia5String) -> String {
    s.to_string().trim().to_string()
}

fn fd_str(fd: &FacilityDesignation) -> String {
    ia5_str(&fd.0)
}

fn facility_body(f: &Facility) -> CpdlcElementBody {
    match f {
        Facility::FacilityDesignation(fd) => CpdlcElementBody::IcaoFacilityDesignation(fd_str(fd)),
        Facility::NoFacility(()) => CpdlcElementBody::Null,
    }
}

fn unit_freq(unit: &UnitName, freq: &Frequency) -> CpdlcElementBody {
    let facility = IcaoFacilityIdentification::Designation(fd_str(&unit.facility_designation));
    let function = match unit.facility_function {
        FacilityFunction::Center | FacilityFunction::Control => IcaoFacilityFunction::Center,
        FacilityFunction::Approach => IcaoFacilityFunction::Approach,
        FacilityFunction::Tower => IcaoFacilityFunction::Tower,
        FacilityFunction::Departure => IcaoFacilityFunction::Departure,
        _ => IcaoFacilityFunction::Center,
    };
    let frequency = match freq {
        // FrequencyVhf: 1 unit = 5 kHz
        Frequency::FrequencyVhf(v) => CpdlcFrequency::VhfKhz(v.0 as u32 * 5),
        Frequency::FrequencyHf(h) => CpdlcFrequency::HfKhz(h.0 as u32),
        Frequency::FrequencyUhf(u) => CpdlcFrequency::UhfKhz(u.0 as u32 * 25),
        Frequency::FrequencySatChannel(_) => CpdlcFrequency::VhfKhz(0),
    };
    CpdlcElementBody::IcaoUnitNameFrequency {
        unit: IcaoUnitName { facility, function },
        frequency,
    }
}

fn id_from_debug(s: &str, prefix: &str) -> u16 {
    s.strip_prefix(prefix)
        .and_then(|r| {
            r.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::super::ulcs::{FullyEncodedData, PdvDataValues};
    use super::*;

    fn decode_fixture(hex: &str) -> (CpdlcPduSummary, PduKind) {
        let bytes = hex::decode(hex).unwrap();
        assert_eq!(bytes[0], 0x00);
        let fed = rasn::uper::decode::<FullyEncodedData>(&bytes).unwrap();
        let PdvDataValues::Arbitrary(bits) = &fed.data.presentation_data_values else {
            panic!("not arbitrary");
        };
        decode_inner(bits.as_raw_slice()).unwrap()
    }

    #[test]
    fn logical_acknowledgment() {
        let (s, kind) = decode_fixture("00a7332790001e2f39c6c1c6404fe7ce32");
        assert!(matches!(kind, PduKind::Uplink));
        assert_eq!(s.header.msg_id, 0);
        assert_eq!(s.elements[0].id, 227);
        assert!(matches!(
            s.elements[0].body.as_ref(),
            Some(CpdlcElementBody::Null)
        ));
    }

    #[test]
    fn contact_with_freetext() {
        let (s, _) = decode_fixture("00a82693304548878bce74009d622c7a950e64f9d127ce03222b7369d16c54414e2c3a93e920874224c868274fa8824ce41569c54156754933104e9f524c693162205a82ad38a82b4f930e2903d51eb7c8");
        assert_eq!(s.header.msg_id, 4);
        assert_eq!(s.elements[0].id, 117);
        // verify facility EGTT
        if let Some(CpdlcElementBody::IcaoUnitNameFrequency { unit, frequency }) =
            &s.elements[0].body
        {
            if let IcaoFacilityIdentification::Designation(d) = &unit.facility {
                assert_eq!(d, "EGTT");
            } else {
                panic!("expected Designation");
            }
            assert_eq!(*frequency, CpdlcFrequency::VhfKhz(129605));
        } else {
            panic!("expected IcaoUnitNameFrequency");
        }
        // second element is free text
        if let Some(CpdlcElementBody::FreeText(text)) = &s.elements[1].body {
            assert!(text.contains("NEXT SECTOR"));
        } else {
            panic!("expected FreeText");
        }
    }

    #[test]
    fn next_data_authority() {
        let (s, _) = decode_fixture("00a808e32ae8678bce7c2028226468b14808a578d140");
        assert_eq!(s.elements[0].id, 160);
        if let Some(CpdlcElementBody::IcaoFacilityDesignation(f)) = &s.elements[0].body {
            assert_eq!(f, "LFEE");
        } else {
            panic!("expected IcaoFacilityDesignation");
        }
    }

    #[test]
    fn free_text_current_atc() {
        let (s, _) = decode_fixture("00a818e33029e0478bce7c205b9343ab4a9459d51041a90d0559d26a208b12cd959360c1a7529498722a2c87167548b48815a8052700");
        if let Some(CpdlcElementBody::FreeText(text)) = &s.elements[0].body {
            assert!(text.contains("EDYY") && text.contains("MAASTRICHT"));
        } else {
            panic!("expected FreeText");
        }
    }
}
