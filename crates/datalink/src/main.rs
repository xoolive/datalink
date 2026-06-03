mod hfdl;
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
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// VDL Mode 2 frontend for I/Q and SDR inputs.
    Vdl2(vdl2::Options),
    /// Classic VHF ACARS frontend.
    #[command(alias = "acars-vhf")]
    Vhf(vhf::Options),
    /// Airframes.io websocket feed.
    #[command(name = "airframes.io", alias = "airframes")]
    AirframesIo(AirframesOptions),
    /// HF Data Link frontend.
    #[command(alias = "hf")]
    Hfdl(hfdl::Options),
    /// Decode standalone payloads or frames.
    Decode {
        #[command(subcommand)]
        command: DecodeCommand,
    },
}

#[derive(Debug, Parser)]
struct AirframesOptions {
    /// Airframes source URL; defaults to airframes://
    source: Option<String>,
    /// Dump a copy of decoded websocket rows as JSONL
    #[arg(short, long)]
    output: Option<String>,
    /// Print counters to stderr at end
    #[arg(long)]
    stats: bool,
    /// Include the original websocket payload under raw
    #[arg(long)]
    raw: bool,
}

#[derive(Debug, Subcommand)]
enum DecodeCommand {
    /// Decode a hex ACARS frame.
    Acars {
        #[arg(help = "Hex-encoded ACARS frame bytes")]
        hex: String,
        #[arg(short, long, value_enum, default_value_t = Direction::Unknown)]
        direction: Direction,
        /// Include the full nested decoder output under raw_decode
        #[arg(long)]
        raw: bool,
    },
    /// Decode a hex AVLC frame (including 2-byte FCS).
    Avlc {
        #[arg(help = "Hex-encoded AVLC frame bytes (with FCS)")]
        hex: String,
        /// Include the full nested decoder output under raw_decode
        #[arg(long)]
        raw: bool,
    },
    /// Decode an ADS-C application-layer text payload.
    Adsc {
        #[arg(help = "ADS-C app text payload")]
        payload: String,
        /// Include the full nested decoder output under raw_decode
        #[arg(long)]
        raw: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    match args.command {
        Command::Vdl2(options) => vdl2::run(options).await,
        Command::Vhf(options) => vhf::run(options).await,
        Command::AirframesIo(options) => {
            vdl2::run_airframes_simple(options.source, options.output, options.stats, options.raw)
                .await
        }
        Command::Hfdl(options) => hfdl::run(options),
        Command::Decode { command } => run_decode(command),
    }
}

fn run_decode(command: DecodeCommand) -> anyhow::Result<()> {
    match command {
        DecodeCommand::Acars {
            hex,
            direction,
            raw,
        } => {
            let bytes = hex::decode(hex.trim())?;
            let dir = match direction {
                Direction::Unknown => MessageDirection::Unknown,
                Direction::Uplink => MessageDirection::GroundToAir,
                Direction::Downlink => MessageDirection::AirToGround,
            };
            let message = parse_acars_frame(&bytes, dir)?;
            let raw_value = serde_json::to_value(&message)?;
            let out = acars::decode::compact::compact_acars_value(raw_value, raw);
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        DecodeCommand::Avlc { hex, raw } => {
            let bytes = hex::decode(hex.trim())?;
            let frame = parse_avlc_frame(&bytes)?;
            let mut obj = serde_json::to_value(&frame)?;
            if let serde_json::Value::Object(ref mut m) = obj {
                m.insert("frame".into(), bytes_to_hex(&bytes).into());
            }
            let out = acars::decode::compact::compact_avlc_value(obj, raw);
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        DecodeCommand::Adsc { payload, raw } => {
            let adsc = parse_adsc_app_text(payload.trim())?;
            let raw_value = serde_json::to_value(&adsc)?;
            let mut out = serde_json::json!({
                "path": "acars",
                "protocol_stack": ["acars", "arinc622", "ads_c"],
                "message_class": "app_message",
                "summary": "ADS-C application payload",
                "app": { "protocol": "ads_c", "standard": "ARINC 622", "payload": raw_value.clone() },
            });
            if raw {
                out.as_object_mut()
                    .unwrap()
                    .insert("raw_decode".into(), raw_value);
            }
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
    }

    Ok(())
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02X}");
    }
    s
}
