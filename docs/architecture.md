# datalink architecture

This project decodes messages across multiple protocol layers. The key to understanding it is to separate:

- bearer/link transport (how bits move),
- message transport/envelope (how aviation messages are packaged),
- application semantics (what the message means operationally).

## Layer map

Current project scope note:

- `vdl136` is the current VDL2 frontend for the workspace.
- `datalink` is the current payload-decoder app for demodulated frames/messages.
- `acars131` is the planned classic VHF ACARS frontend.
- `acars` is the shared decode library (`crates/acars`).

```text
┌────────────────────────────────────────────────────────────────────┐
│ APPLICATION LAYER                                                  │
│  CPDLC, ADS-C, AOC/ATIS/OOOI, airline/ops applications           │
└───────────────────────────────┬────────────────────────────────────┘
                                │ carried in
┌───────────────────────────────▼────────────────────────────────────┐
│ ACARS MESSAGE TRANSPORT + ARINC 622 APP ENVELOPE                  │
│  ACARS framing/labels/text + ARINC 622 app routing conventions    │
└───────────────────────────────┬────────────────────────────────────┘
                                │ carried over bearer
        ┌───────────────────────┼───────────────────────────┐
        │                       │                           │
┌───────▼────────┐      ┌───────▼────────┐         ┌────────▼───────┐
│ VDL Mode 2     │      │ POA VHF ACARS  │         │ Future bearer  │
│ (digital VHF)  │      │ (classic ACARS)│         │ frontends      │
└───────┬────────┘      └────────────────┘         └────────────────┘
        │
┌───────▼─────────────────────────────────────────────────────────────┐
│ VDL2 LINK/NETWORK INTERIOR (when bearer is VDL2)                   │
│  AVLC frames (I/S/U)                                                │
│   - I-frame payload: ACARS marker (FF FF 01) or X.25               │
│   - U-frame payload: control, including XID                         │
│  X.25 payload may carry CLNP/COTP and then upper-layer apps        │
└─────────────────────────────────────────────────────────────────────┘
```

## Quick glossary

- `VDL2`: VHF Data Link Mode 2 bearer/link technology.
- `AVLC`: VDL2 link control framing (I/S/U frame types).
- `X.25`: packet protocol found in many VDL2 I-frame payloads.
- `ACARS`: message protocol used across multiple bearers (VHF, VDL2, and others).
- `ARINC 622`: application envelope/routing conventions layered on ACARS for ATS apps.
- `CPDLC`: controller-pilot data link communications application.
- `ADS-C`: automatic dependent surveillance-contract application.

## Bearer frequencies (operational overview)

| Bearer | Typical frequencies | Notes |
|--------|----------------------|-------|
| Classic VHF ACARS (POA) | Initial `acars131` target channels: `131.525`, `131.725`, `131.825` MHz | Operational usage is region/provider dependent |
| VDL Mode 2 (VHF) | Common channels include `136.975` MHz (CSC), plus `136.725`, `136.775`, `136.875` MHz | Matches `dumpvdl2` operational guidance; regional usage still varies |

These values are a practical orientation aid, not a complete regulatory channel list.

## Common decode paths

```text
Path A: classic ACARS text
RF bearer -> ACARS frame -> label/text -> output

Path B: VDL2 carrying ACARS payload
RF -> AVLC I-frame -> FF FF 01 -> ACARS -> ARINC 622 app -> ADS-C/CPDLC/etc

Path C: VDL2 carrying X.25 payload
RF -> AVLC I-frame -> X.25 -> CLNP/COTP -> CPDLC/ATN payload
```

## ARINC 622 IMI support table

The ARINC 622 envelope typically carries an IMI (application identifier). In practice,
support is tracked per IMI family.

| IMI | Typical application family | Implementation status |
|-----|-----------------------------|---------------|
| `ADS` | ADS-C | partial (native ADS-C parser exists; routing/integration still being expanded) |
| `AT1` | ATS/CPDLC-related flows | partial (some CPDLC extraction; full structured decode pending) |
| `CC1` | ATS/CPDLC-related flows | partial (same as above) |
| `DR1` | ATS/CPDLC-related flows | partial (same as above) |
| other app IMIs | MIAM, Media Advisory, OHMA, others seen by libacars | planned (no full native parser coverage yet) |

Notes:

- Not all ACARS traffic is ARINC 622; many messages are plain ACARS/AOC text.
- Unknown or unsupported IMIs should remain safely represented as raw/unknown payloads.

## ACARS + ARINC 622 decode coverage matrix

| Variant | Workspace status | Rust implementation | Reference counterparts |
|---------|---------------|---------------------|------------------------|
| ACARS frame parse (mode/reg/ack/label/block id/text) + CRC + ETX/ETB hint | implemented | `crates/acars/src/decode/acars.rs` | `../../github/libacars/libacars/acars.c`, `../../github/dumpvdl2/src/acars.c` |
| H1 sublabel/MFI extraction | implemented | `crates/acars/src/decode/acars.rs` (`extract_sublabel_and_mfi`) | `../../github/libacars/libacars/acars.c` (`la_acars_extract_sublabel_and_mfi`), `../../github/JAERO/JAERO/arincparse.cpp` |
| ACARS app dispatch entrypoint | partial (native routing still expanding) | currently split across `decode/adsc.rs` and X.25/COTP path in `decode/x25.rs` | `../../github/libacars/libacars/acars.c` (`la_acars_decode_apps`), `../../github/JAERO/JAERO/arincparse.cpp`, `../../github/acarsdec/README.md` |
| ARINC 622 envelope parse for ADS (`/<ATSU>.ADS.<REG><payload><crc>`) | implemented | `crates/acars/src/decode/adsc.rs` (`parse_adsc_app_text`) | `../../github/libacars/libacars/adsc.c`, `../../github/JAERO/JAERO/arincparse.cpp` |
| ADS-C tag set | implemented for tags `3,4,5,6,7,9,10,12,13,14,15,16,17,18,19,20,22,23` | `crates/acars/src/decode/adsc.rs` (`AdscTag`) | `../../github/libacars/libacars/adsc.c`, `../../github/dumpvdl2/src/icao.c`, `../../github/JAERO/JAERO/arincparse.cpp` |
| CPDLC decode from ATN path (X.25/CLNP/COTP) | partial (heuristic free text only) | `crates/acars/src/decode/x25.rs` (`parse_cpdlc_user_data`) | `../../github/libacars/libacars/cpdlc.c`, `../../github/dumpvdl2/src/icao.c` |
| MIAM | planned (not yet decoded natively) | not yet implemented | `../../github/libacars/libacars/miam.c` |
| Media Advisory | planned (not yet decoded natively) | not yet implemented | `../../github/libacars/libacars/media-adv.c` |
| OHMA | planned (not yet decoded natively) | not yet implemented | `../../github/libacars/libacars/ohma.c` |

Notes:

- `dumpvdl2` is the main VDL2 bearer reference and uses libacars for ACARS/app decoding (`../../github/dumpvdl2/src/acars.c`).
- `acarsdec` and JAERO both rely on libacars-backed app decoding for broad ARINC 622 app coverage.

## Where this lives in the workspace

- Shared VDL2 demod module: `crates/acars/src/demod/vdl2.rs`
- AVLC + VDL2 framing: `crates/acars/src/decode/avlc.rs`
- AVLC XID control: `crates/acars/src/decode/xid.rs`
- X.25/CLNP/COTP path: `crates/acars/src/decode/x25.rs`
- ACARS message parsing: `crates/acars/src/decode/acars.rs`
- ADS-C parsing: `crates/acars/src/decode/adsc.rs`
- VDL2 frontend CLI: `crates/vdl136/src/main.rs` (`vdl136`)
- Payload decoder CLI: `crates/datalink/src/main.rs` (`datalink`)
- Classic ACARS frontend scaffold: `crates/acars131/src/main.rs` (`acars131`)

## Positioning note

- `dumpvdl2` is the primary behavior reference for VDL2 AVLC/X.25 routing.
- `libacars` is the primary app-layer behavior reference (ADS-C, CPDLC, MIAM, OHMA, Media Advisory).
- `datalink` goal is to keep parity where needed while building a Rust-native, typed decoding stack.
- `acars` should be described as a Rust-native reimplementation with compatibility goals,
  not as a direct one-to-one libacars port.
