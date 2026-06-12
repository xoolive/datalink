# acars

`acars` is the reusable Rust decode library behind the `datalink` CLI. It parses
aviation datalink frames and application payloads without requiring SDR or DSP
dependencies by default.

The crate covers classic VHF ACARS, VDL Mode 2 AVLC/X.25 paths, HFDL PDUs, and
common ACARS application payloads such as ADS-C, CPDLC, ATIS, AOC, OOOI, MIAM,
OHMA, and squitter messages.

## Quick example

```rust,no_run
use acars::decode::acars::{parse_acars_frame, MessageDirection};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = hex::decode("<hex-encoded-acars-frame>")?;
    let message = parse_acars_frame(&bytes, MessageDirection::Unknown)?;

    println!("{}", serde_json::to_string_pretty(&message)?);
    Ok(())
}
```

For VDL2 frames use [`decode::avlc::parse_avlc_frame`]. For ADS-C application
text use [`decode::payload::arinc622::adsc::parse_adsc_app_text`].

## Features

The default build is decode-only:

```toml
acars = "0.1"
```

or from this workspace:

```toml
acars = { path = "crates/acars" }
```

Enable demodulation support only when processing I/Q samples:

```toml
acars = { version = "0.1", features = ["demod"] }
```

The `demod` feature enables DSP dependencies such as `desperado` and
`num-complex` and exposes the [`demod`] module. Parser-only users do not need to
build SDR or DSP support.

## Module overview

| Module | Purpose |
|--------|---------|
| [`decode::acars`] | ACARS frame parsing, CRC verification, header/text fields, labels, direction handling, and app-payload dispatch. |
| [`decode::avlc`] | VDL2 AVLC frames: addresses, I/S/U control fields, FCS status, ACARS/X.25/XID payload dispatch. |
| [`decode::x25`] | VDL2 X.25/CLNP/COTP structural decode and ATN B1 handoff. |
| [`decode::xid`] | VDL2 XID / GSIF control-frame parsing. |
| [`decode::hfdl`] | HFDL PDU parsing, including SPDU/MPDU/LPDU structures and embedded ACARS payloads. |
| [`decode::payload`] | Application payload namespace for ARINC 620/622/623, ATN B1, AOC, Boeing OHMA, MIAM, weather, squitter, and text fallbacks. |
| [`decode::compact`] | Cross-protocol helpers used to extract aircraft kinematics and human-facing compact output fields. |
| [`demod`] | Optional VDL2, VHF ACARS, and HFDL demodulators plus resampling helpers. Requires `features = ["demod"]`. |

## Payload families

The [`decode::payload`] tree is organised by standards or application family:

- `arinc620` — media advisory and ground-station squitter payloads.
- `arinc622` — FANS-1/A envelope dispatch, ADS-C, CPDLC, AFN, and oceanic clearance.
- `arinc623` — ATIS request and delivery payloads.
- `atn_b1` — ATN B1 applications carried over VDL2 X.25/COTP/ULCS, currently CPDLC.
- `aoc` — airline operations payloads such as OOOI reports, label 80, label 5Z,
  weather bundles, and telemetry classifiers.
- `boeing` — Boeing-specific OHMA health-monitoring payloads.
- `miam` — MIAM / ACMS maintenance payloads.

Unknown or unsupported application text is preserved rather than discarded.

## Prelude

[`prelude`] re-exports the most common decode entry points and types:

- ACARS frame types and [`decode::acars::parse_acars_frame`] dependencies.
- AVLC frame parsing and link-layer enums.
- ARINC 620 squitter and ARINC 622 ADS-C helpers.
- [`decode::DecodeError`] and [`decode::DecodeResult`].

Use it for small tools and examples:

```rust,no_run
use acars::prelude::*;
```

## Decode coverage

Implemented coverage includes:

- ACARS frame parsing from octets, including CRC-16-CCITT verification.
- ACARS header/text extraction and H1 sublabel/MFI handling.
- AVLC parsing with FCS status and payload dispatch (`Acars`, `X25`, `Xid`, `Unknown`).
- HFDL SPDU/MPDU/LPDU parsing.
- ADS-C downlink tags and uplink contract requests.
- FANS-1/A CPDLC and ATN B1 CPDLC summaries with shared `CpdlcElement` output.
- Common AOC, ATIS, AFN, OOOI, weather, squitter, MIAM, and OHMA payloads.

The implementation is Rust-native and aims to follow the relevant aviation datalink
standards and observed operational message formats.
