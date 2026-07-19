//! Datalink ingestion and decoding CLI
mod airframes;
mod hfdl;
mod iq_pipeline;
mod merged;
mod source;
mod util;
mod vdl2;
mod vhf;

use datalink::event;

use crate::event::{Bearer, DecodedEvent, ProtocolMessage, SourceClass, SourceMetadata};
use acars::decode::acars::{parse_acars_frame, MessageDirection};
use acars::decode::avlc::parse_avlc_frame;
use acars::decode::payload::arinc622::{
    parse_with_direction as parse_arinc622_with_direction, Payload as Arinc622Payload,
};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Direction {
    Unknown,
    Uplink,
    Downlink,
}

impl Direction {
    fn as_message_direction(self) -> MessageDirection {
        match self {
            Direction::Unknown => MessageDirection::Unknown,
            Direction::Uplink => MessageDirection::GroundToAir,
            Direction::Downlink => MessageDirection::AirToGround,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "datalink", about = "Decode aviation datalink traffic")]
struct Args {
    /// Merged receiver configuration file. Used when no bearer subcommand is provided.
    #[arg(long)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// VDL Mode 2 frontend for I/Q and SDR inputs.
    Vdl2(vdl2::Cli),
    /// Classic VHF ACARS frontend.
    #[command(alias = "acars-vhf")]
    Vhf(vhf::Cli),
    /// Airframes.io websocket feed.
    #[command(name = "airframes.io", alias = "airframes")]
    AirframesIo(airframes::Options),
    /// HF Data Link frontend.
    #[command(alias = "hf")]
    Hfdl(hfdl::Options),
    /// Decode standalone payloads or frames.
    Decode {
        #[command(subcommand)]
        command: DecodeCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DecodeCommand {
    /// Decode a hex ACARS frame.
    Acars {
        #[arg(help = "Hex-encoded ACARS frame bytes")]
        hex: String,
        #[arg(short, long, value_enum, default_value_t = Direction::Unknown)]
        direction: Direction,
    },
    /// Decode a hex AVLC frame (including 2-byte FCS).
    Avlc {
        #[arg(help = "Hex-encoded AVLC frame bytes (with FCS)")]
        hex: String,
    },
    /// Decode an ARINC 622 envelope and dispatch ADS-C, CPDLC, DIS, or raw IMI payloads.
    Arinc622 {
        #[arg(help = "ARINC 622 application text envelope")]
        text: String,
        #[arg(short, long, value_enum, default_value_t = Direction::Unknown)]
        direction: Direction,
    },
    /// Decode an ADS-C ARINC 622 envelope, including ADS-C disconnect.
    Adsc {
        #[arg(help = "ADS-C ARINC 622 application text envelope")]
        text: String,
        #[arg(short, long, value_enum, default_value_t = Direction::Unknown)]
        direction: Direction,
    },
    /// Decode a CPDLC ARINC 622 envelope or control message.
    Cpdlc {
        #[arg(help = "CPDLC ARINC 622 application text envelope")]
        text: String,
        #[arg(short, long, value_enum, default_value_t = Direction::Unknown)]
        direction: Direction,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    match args.command {
        Some(Command::Vdl2(options)) => vdl2::run(options).await,
        Some(Command::Vhf(options)) => vhf::run(options).await,
        Some(Command::AirframesIo(options)) => airframes::run(options).await,
        Some(Command::Hfdl(options)) => hfdl::run(options).await,
        Some(Command::Decode { command }) => run_decode(command),
        None => merged::run(args.config).await,
    }
}

fn run_decode(command: DecodeCommand) -> anyhow::Result<()> {
    match command {
        DecodeCommand::Acars { hex, direction } => {
            let bytes = hex::decode(hex.trim())?;
            let message = parse_acars_frame(&bytes, direction.as_message_direction())?;
            let pmsg = ProtocolMessage::Acars(Box::new(message));

            let event = DecodedEvent {
                event: "message".to_string(),
                timestamp: None,
                bearer: Bearer::Vhf,
                source: SourceMetadata {
                    id: "decode_cli".into(),
                    name: "decode_cli".into(),
                    class: SourceClass::Frames,
                    format: None,
                },
                receiver: None,
                aircraft: crate::merged::aircraft_summary(&pmsg),
                kinematics: pmsg.kinematics(),
                raw_frame_hex: Some(hex.clone()),
                message: pmsg,
            };
            println!("{}", serde_json::to_string_pretty(&event)?);
        }
        DecodeCommand::Avlc { hex } => {
            let bytes = hex::decode(hex.trim())?;
            let frame = parse_avlc_frame(&bytes)?;

            let pmsg = ProtocolMessage::Avlc(Box::new(frame));

            let event = DecodedEvent {
                event: "message".to_string(),
                timestamp: None,
                bearer: Bearer::Vdl2,
                source: SourceMetadata {
                    id: "decode_cli".into(),
                    name: "decode_cli".into(),
                    class: SourceClass::Frames,
                    format: None,
                },
                receiver: None,
                aircraft: crate::merged::aircraft_summary(&pmsg),
                kinematics: pmsg.kinematics(),
                raw_frame_hex: Some(hex.clone()),
                message: pmsg,
            };
            println!("{}", serde_json::to_string_pretty(&event)?);
        }
        DecodeCommand::Arinc622 { text, direction } => {
            let message =
                parse_arinc622_with_direction(text.trim(), direction.as_message_direction())?;
            print_app_event(acars::decode::payload::AcarsAppPayload::Arinc622(message))?;
        }
        DecodeCommand::Adsc { text, direction } => {
            let message =
                parse_arinc622_with_direction(text.trim(), direction.as_message_direction())?;
            if !matches!(
                &message.payload,
                Arinc622Payload::Adsc(_) | Arinc622Payload::AdscDisconnect(_)
            ) {
                anyhow::bail!("ARINC 622 envelope did not contain an ADS-C payload");
            }
            print_app_event(acars::decode::payload::AcarsAppPayload::Arinc622(message))?;
        }
        DecodeCommand::Cpdlc { text, direction } => {
            let message =
                parse_arinc622_with_direction(text.trim(), direction.as_message_direction())?;
            if !matches!(&message.payload, Arinc622Payload::Cpdlc(_)) {
                anyhow::bail!("ARINC 622 envelope did not contain a CPDLC payload");
            }
            print_app_event(acars::decode::payload::AcarsAppPayload::Arinc622(message))?;
        }
    }

    Ok(())
}

fn print_app_event(app: acars::decode::payload::AcarsAppPayload) -> anyhow::Result<()> {
    let pmsg = ProtocolMessage::App(Box::new(app));

    let event = DecodedEvent {
        event: "message".to_string(),
        timestamp: None,
        bearer: Bearer::Decoded,
        source: SourceMetadata {
            id: "decode_cli".into(),
            name: "decode_cli".into(),
            class: SourceClass::Frames,
            format: None,
        },
        receiver: None,
        aircraft: crate::merged::aircraft_summary(&pmsg),
        kinematics: pmsg.kinematics(),
        raw_frame_hex: None,
        message: pmsg,
    };
    println!("{}", serde_json::to_string_pretty(&event)?);
    Ok(())
}
