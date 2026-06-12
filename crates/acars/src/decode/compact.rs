//! Compact, cross-protocol extraction helpers.
//!
//! The full protocol structs in [`crate::decode`] preserve bearer-specific
//! details. This module provides small shared summary types used by the CLI to
//! expose common information across ACARS, VDL2, HFDL, and application payloads.
//!
//! The main type is [`Kinematics`], a best-effort aircraft state summary
//! extracted from payloads such as ADS-C basic reports, CPDLC position reports,
//! AOC position messages, Airframes.io metadata, or HFDL MPDUs. Implementations
//! of [`ExtractKinematics`] never replace the original decode; they only add a
//! convenient normalized view for downstream JSON consumers.

use serde::{Deserialize, Serialize};

use crate::decode::acars::AcarsMessage;
use crate::decode::avlc::{AvlcFrame, AvlcPayload};
use crate::decode::hfdl::{HfdlMessage, HfdlPdu, LpduData, Mpdu};
use crate::decode::payload::aoc::label32::Label32Message;
use crate::decode::payload::aoc::oooi::{OooiOffDestination, OooiOffReport};
use crate::decode::payload::aoc::position::AocPositionMessage;
use crate::decode::payload::arinc620::squitter::SquitterMessage;
use crate::decode::payload::arinc622::adsc::{AdscBasicReport, AdscEarthAirReference, AdscTag};
use crate::decode::payload::arinc622::cpdlc::{
    CpdlcAltitude, CpdlcDegrees, CpdlcElementBody, CpdlcPosition, CpdlcPositionReport,
};
use crate::decode::payload::AcarsAppPayload;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct Position {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Kinematics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altitude_ft: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_speed_knots: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<String>,
}

impl Kinematics {
    pub fn merge(self, other: Self) -> Self {
        Self {
            position: self.position.or(other.position),
            altitude_ft: self.altitude_ft.or(other.altitude_ft),
            track: self.track.or(other.track),
            ground_speed_knots: self.ground_speed_knots.or(other.ground_speed_knots),
            derived_from: self.derived_from.or(other.derived_from),
        }
    }
}

pub trait ExtractKinematics {
    fn kinematics(&self) -> Option<Kinematics>;
}

impl From<&CpdlcPosition> for Option<Position> {
    fn from(pos: &CpdlcPosition) -> Self {
        match pos {
            CpdlcPosition::LatitudeLongitude {
                latitude,
                longitude,
            } => Some(Position {
                latitude: *latitude,
                longitude: *longitude,
            }),
            _ => None,
        }
    }
}

impl ExtractKinematics for CpdlcPosition {
    fn kinematics(&self) -> Option<Kinematics> {
        Option::<Position>::from(self).map(|position| Kinematics {
            position: Some(position),
            derived_from: Some("cpdlc".into()),
            ..Default::default()
        })
    }
}

impl ExtractKinematics for CpdlcAltitude {
    fn kinematics(&self) -> Option<Kinematics> {
        let alt = match self {
            CpdlcAltitude::FlightLevel(fl) => Some(*fl as i32 * 100),
            CpdlcAltitude::QnhFeet(ft) | CpdlcAltitude::QfeFeet(ft) => Some(*ft as i32),
            CpdlcAltitude::GnssFeet(ft) => Some(*ft as i32),
            CpdlcAltitude::FlightLevelMetric(fl) => Some((*fl as i32 * 100000) / 3048),
            CpdlcAltitude::QnhMeters(m) | CpdlcAltitude::QfeMeters(m) => {
                Some((*m as i32 * 10000) / 3048)
            }
            CpdlcAltitude::GnssMeters(m) => Some((*m as i32 * 10000) / 3048),
        };
        alt.map(|altitude_ft| Kinematics {
            altitude_ft: Some(altitude_ft),
            derived_from: Some("cpdlc".into()),
            ..Default::default()
        })
    }
}

impl ExtractKinematics for CpdlcPositionReport {
    fn kinematics(&self) -> Option<Kinematics> {
        let position = Option::<Position>::from(&self.current_position);
        let altitude_ft = self.altitude.kinematics().and_then(|k| k.altitude_ft);
        let track = self.true_heading.as_ref().map(|deg| match deg {
            CpdlcDegrees::True(v) | CpdlcDegrees::Magnetic(v) => *v as f64,
        });

        if position.is_some() || altitude_ft.is_some() {
            Some(Kinematics {
                position,
                altitude_ft,
                track,
                ground_speed_knots: self.ground_speed_knots,
                derived_from: Some("cpdlc_position_report".into()),
            })
        } else {
            None
        }
    }
}

impl ExtractKinematics for AdscBasicReport {
    fn kinematics(&self) -> Option<Kinematics> {
        Some(Kinematics {
            position: Some(Position {
                latitude: self.latitude,
                longitude: self.longitude,
            }),
            altitude_ft: Some(self.altitude_ft),
            derived_from: Some("adsc_basic".into()),
            ..Default::default()
        })
    }
}

impl ExtractKinematics for AdscEarthAirReference {
    fn kinematics(&self) -> Option<Kinematics> {
        Some(Kinematics {
            track: (!self.heading_invalid).then_some(self.heading_or_track_degrees),
            derived_from: Some("adsc_earth_air".into()),
            ..Default::default()
        })
    }
}

impl ExtractKinematics for CpdlcElementBody {
    fn kinematics(&self) -> Option<Kinematics> {
        match self {
            Self::PositionReport(pr) => pr.kinematics(),
            Self::Position(p) => p.kinematics(),
            Self::PositionAltitude { position, altitude }
            | Self::AltitudePosition { altitude, position } => {
                merge_cpdlc_kinematics(position, altitude)
            }
            Self::PositionDistanceOffsetDirection { position, .. }
            | Self::PositionIcaoUnitNameFrequency { position, .. }
            | Self::PositionTime { position, .. }
            | Self::PositionTimeTime { position, .. }
            | Self::PositionSpeedSpeed { position, .. } => position.kinematics(),
            Self::TimePositionAltitude {
                position, altitude, ..
            }
            | Self::PositionTimeAltitude {
                position, altitude, ..
            }
            | Self::TimePositionAltitudeSpeed {
                position, altitude, ..
            }
            | Self::PositionAltitudeSpeed {
                position, altitude, ..
            } => merge_cpdlc_kinematics(position, altitude),
            Self::PositionPosition { positions } => positions.first()?.kinematics(),
            _ => None,
        }
    }
}

fn merge_cpdlc_kinematics(
    position: &CpdlcPosition,
    altitude: &CpdlcAltitude,
) -> Option<Kinematics> {
    let k = position.kinematics().or_else(|| altitude.kinematics())?;
    Some(Kinematics {
        altitude_ft: altitude
            .kinematics()
            .and_then(|ak| ak.altitude_ft)
            .or(k.altitude_ft),
        derived_from: Some("cpdlc".into()),
        ..k
    })
}

impl ExtractKinematics for AdscTag {
    fn kinematics(&self) -> Option<Kinematics> {
        match self {
            Self::BasicReport(rep)
            | Self::EmergencyBasicReport(rep)
            | Self::LateralDeviationChangeEvent(rep)
            | Self::VerticalRateChangeEvent(rep)
            | Self::AltitudeRangeEvent(rep)
            | Self::WaypointChangeEvent(rep) => rep.kinematics(),
            Self::EarthReferenceData(erd) => erd.kinematics(),
            _ => None,
        }
    }
}

impl ExtractKinematics for AocPositionMessage {
    fn kinematics(&self) -> Option<Kinematics> {
        if let (Some(lat), Some(lon)) = (self.latitude, self.longitude) {
            Some(Kinematics {
                position: Some(Position {
                    latitude: lat,
                    longitude: lon,
                }),
                altitude_ft: self.altitude_ft,
                track: self.heading_deg.map(f64::from),
                derived_from: Some(self.format.clone()),
                ..Default::default()
            })
        } else {
            None
        }
    }
}

impl ExtractKinematics for Label32Message {
    fn kinematics(&self) -> Option<Kinematics> {
        if let (Some(lat), Some(lon)) = (self.latitude, self.longitude) {
            Some(Kinematics {
                position: Some(Position {
                    latitude: lat,
                    longitude: lon,
                }),
                altitude_ft: self.altitude_ft,
                track: self.heading_deg.map(f64::from),
                derived_from: Some("label32".into()),
                ..Default::default()
            })
        } else {
            None
        }
    }
}

impl ExtractKinematics for SquitterMessage {
    fn kinematics(&self) -> Option<Kinematics> {
        if let (Some(lat), Some(lon)) = (self.latitude, self.longitude) {
            Some(Kinematics {
                position: Some(Position {
                    latitude: lat,
                    longitude: lon,
                }),
                derived_from: Some("squitter".into()),
                ..Default::default()
            })
        } else {
            None
        }
    }
}

impl ExtractKinematics for OooiOffDestination {
    fn kinematics(&self) -> Option<Kinematics> {
        Some(Kinematics {
            derived_from: Some("oooi_qf".into()),
            ..Default::default()
        })
    }
}

impl ExtractKinematics for OooiOffReport {
    fn kinematics(&self) -> Option<Kinematics> {
        Some(Kinematics {
            derived_from: Some("oooi_qq".into()),
            ..Default::default()
        })
    }
}

impl ExtractKinematics for AcarsAppPayload {
    fn kinematics(&self) -> Option<Kinematics> {
        match self {
            Self::AocPosition(p) => p.kinematics(),
            Self::Label32(p) => p.kinematics(),
            Self::Squitter(p) => p.kinematics(),
            Self::OooiOffDestination(p) => p.kinematics(),
            Self::OooiOffReport(p) => p.kinematics(),
            Self::Arinc622(arinc) => match &arinc.payload {
                crate::decode::payload::arinc622::Payload::Adsc(adsc) => {
                    adsc.tags.iter().find_map(|t| t.kinematics())
                }
                crate::decode::payload::arinc622::Payload::Cpdlc(cpdlc) => {
                    let summaries = [cpdlc.uplink.as_ref(), cpdlc.downlink.as_ref()];
                    summaries
                        .into_iter()
                        .flatten()
                        .flat_map(|s| &s.elements)
                        .find_map(|e| e.body.as_ref().and_then(|b| b.kinematics()))
                }
                _ => None,
            },
            _ => None,
        }
    }
}

impl ExtractKinematics for AcarsMessage {
    fn kinematics(&self) -> Option<Kinematics> {
        self.app.kinematics()
    }
}

impl ExtractKinematics for AvlcFrame {
    fn kinematics(&self) -> Option<Kinematics> {
        if let Some(AvlcPayload::Acars(msg)) = &self.payload {
            msg.kinematics()
        } else {
            None
        }
    }
}

impl ExtractKinematics for HfdlMessage {
    fn kinematics(&self) -> Option<Kinematics> {
        if let HfdlPdu::Mpdu(Mpdu::Downlink(dl)) = &self.pdu {
            for lpdu in &dl.lpdus {
                if let LpduData::Hfnpdu { hfnpdu } = &lpdu.data {
                    match &hfnpdu.data {
                        crate::decode::hfdl::Hfnpdu::Performance { performance } => {
                            return Some(Kinematics {
                                position: Some(Position {
                                    latitude: performance.position.lat,
                                    longitude: performance.position.lon,
                                }),
                                derived_from: Some("hfdl_performance".into()),
                                ..Default::default()
                            });
                        }
                        crate::decode::hfdl::Hfnpdu::Frequency { frequency_data } => {
                            return Some(Kinematics {
                                position: Some(Position {
                                    latitude: frequency_data.position.lat,
                                    longitude: frequency_data.position.lon,
                                }),
                                derived_from: Some("hfdl_frequency".into()),
                                ..Default::default()
                            });
                        }
                        crate::decode::hfdl::Hfnpdu::Acars { acars } => {
                            if let Some(kin) = acars.kinematics() {
                                return Some(kin);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        None
    }
}
