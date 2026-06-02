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
#[command(
    name = "datalink",
    about = "Decode demodulated ACARS/ARINC 622 payloads"
)]
struct Args {
    /// Include the full nested decoder output under raw_decode
    #[arg(long, global = true)]
    raw: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
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

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    match args.command {
        Command::Acars { hex, direction } => {
            let bytes = hex::decode(hex.trim())?;
            let dir = match direction {
                Direction::Unknown => MessageDirection::Unknown,
                Direction::Uplink => MessageDirection::GroundToAir,
                Direction::Downlink => MessageDirection::AirToGround,
            };
            let message = parse_acars_frame(&bytes, dir)?;
            let raw = serde_json::to_value(&message)?;
            let out = acars::decode::compact::compact_acars_value(raw, args.raw);
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Command::Avlc { hex } => {
            let bytes = hex::decode(hex.trim())?;
            let frame = parse_avlc_frame(&bytes)?;
            let mut obj = serde_json::to_value(&frame)?;
            if let serde_json::Value::Object(ref mut m) = obj {
                m.insert("frame".into(), bytes_to_hex(&bytes).into());
            }
            let out = acars::decode::compact::compact_avlc_value(obj, args.raw);
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Command::Adsc { payload } => {
            let adsc = parse_adsc_app_text(payload.trim())?;
            let raw = serde_json::to_value(&adsc)?;
            let mut out = serde_json::json!({
                "path": "acars",
                "protocol_stack": ["acars", "arinc622", "ads_c"],
                "message_class": "app_message",
                "summary": "ADS-C application payload",
                "app": { "protocol": "ads_c", "standard": "ARINC 622", "payload": raw.clone() },
            });
            if args.raw {
                out.as_object_mut()
                    .unwrap()
                    .insert("raw_decode".into(), raw);
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
        let _ = write!(&mut s, "{:02X}", b);
    }
    s
}
