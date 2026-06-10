mod airframes;
mod hfdl;
mod iq_pipeline;
mod merged;
mod source;
mod util;
mod vdl2;
mod vhf;

use acars::decode::acars::{parse_acars_frame, MessageDirection};
use acars::decode::avlc::parse_avlc_frame;
use acars::decode::payload::arinc622::adsc::parse_adsc_app_text;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Direction {
    Unknown,
    Uplink,
    Downlink,
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
    /// Decode an ADS-C application-layer text payload.
    Adsc {
        #[arg(help = "ADS-C app text payload")]
        payload: String,
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
            let dir = match direction {
                Direction::Unknown => MessageDirection::Unknown,
                Direction::Uplink => MessageDirection::GroundToAir,
                Direction::Downlink => MessageDirection::AirToGround,
            };
            let message = parse_acars_frame(&bytes, dir)?;
            let pmsg = crate::merged::ProtocolMessage::Acars(Box::new(message));

            let event = crate::merged::DecodedEvent {
                event: "message",
                timestamp: None,
                bearer: crate::merged::Bearer::Vhf,
                source: crate::merged::SourceMetadata {
                    id: "decode_cli".into(),
                    name: "decode_cli".into(),
                    class: crate::merged::SourceClass::Frames,
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

            let pmsg = crate::merged::ProtocolMessage::Avlc(Box::new(frame));

            let event = crate::merged::DecodedEvent {
                event: "message",
                timestamp: None,
                bearer: crate::merged::Bearer::Vdl2,
                source: crate::merged::SourceMetadata {
                    id: "decode_cli".into(),
                    name: "decode_cli".into(),
                    class: crate::merged::SourceClass::Frames,
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
        DecodeCommand::Adsc { payload } => {
            let adsc = parse_adsc_app_text(payload.trim())?;

            let acars_app = acars::decode::payload::AcarsAppPayload::Arinc622(
                acars::decode::payload::arinc622::Message {
                    atsu_address: adsc.atsu_address.clone(),
                    imi: acars::decode::payload::arinc622::Imi::Ads,
                    registration: adsc.registration.clone(),
                    payload: acars::decode::payload::arinc622::Payload::Adsc(adsc.clone()),
                },
            );

            let pmsg = crate::merged::ProtocolMessage::App(Box::new(acars_app));

            let event = crate::merged::DecodedEvent {
                event: "message",
                timestamp: None,
                bearer: crate::merged::Bearer::Decoded,
                source: crate::merged::SourceMetadata {
                    id: "decode_cli".into(),
                    name: "decode_cli".into(),
                    class: crate::merged::SourceClass::Frames,
                    format: None,
                },
                receiver: None,
                aircraft: crate::merged::aircraft_summary(&pmsg),
                kinematics: pmsg.kinematics(),
                raw_frame_hex: None,
                message: pmsg,
            };
            println!("{}", serde_json::to_string_pretty(&event)?);
        }
    }

    Ok(())
}
