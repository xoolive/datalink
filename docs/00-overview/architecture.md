# datalink architecture

This workspace decodes messages across multiple protocol layers. The key is to separate bearer (how bits move), message transport (how messages are packaged), and applications (what messages mean).

## Component map

**Frontends** (bearer demod, per radio technology):
- `vdl136`: VDL2 on VHF (136 MHz band)
- `acars131`: classic ACARS on VHF (129-131 MHz)
- Future: HFDL frontend for HF bearer

**Shared decode library**:
- `acars` (`crates/acars`): demod modules, AVLC/X.25 parsing, ACARS parsing, application decoders (ADS-C, CPDLC)

**Payload decoder CLI**:
- `datalink`: decode AVLC, ACARS, or ADS-C payloads directly (for testing or when you already have hex/text inputs)

All frontends produce JSON output. The schema is consistent across bearers (same field names for frequency, timestamp, decoded payload structure).

## Layer model

```text
┌────────────────────────────────────────────────────────────┐
│ APPLICATIONS                                                │
│  ADS-C, CPDLC, AOC text, OOOI, weather, maintenance        │
└────────────────────────────┬───────────────────────────────┘
                             │
┌────────────────────────────▼───────────────────────────────┐
│ MESSAGE TRANSPORT: ACARS + ARINC 622                       │
│  ACARS framing (mode, reg, label, text, CRC)              │
│  ARINC 622 app envelope (/<ATSU>.<IMI>.<REG><data><crc>)  │
└────────────────────────────┬───────────────────────────────┘
                             │
         ┌───────────────────┼───────────────────┐
         │                   │                   │
┌────────▼────────┐  ┌───────▼────────┐  ┌──────▼──────────┐
│ VDL2 bearer     │  │ VHF ACARS (POA)│  │ HFDL (future)   │
│ (digital VHF)   │  │ (classic MSK)  │  │ (HF bands)      │
└────────┬────────┘  └────────────────┘  └─────────────────┘
         │
┌────────▼────────────────────────────────────────────────────┐
│ VDL2 LINK LAYER (AVLC frames)                              │
│  I-frame: data payload (ACARS or X.25)                     │
│  S-frame: supervisory (flow control)                       │
│  U-frame: control (XID exchange)                           │
│                                                             │
│  I-frame payload options:                                  │
│    - FF FF 01 marker -> ACARS message                      │
│    - X.25 -> CLNP/COTP -> application (e.g. CPDLC)        │
└─────────────────────────────────────────────────────────────┘
```

VDL2 is more complex than classic VHF ACARS. Classic ACARS is just ACARS frames over MSK modulation. VDL2 has a full link layer (AVLC) and can carry both ACARS and X.25 traffic.

## Bearer frequency reference

| Bearer | Example frequencies | Notes |
|--------|---------------------|-------|
| Classic VHF ACARS | 131.525, 131.725, 131.825 MHz | Initial `acars131` targets; regional usage varies |
| VDL Mode 2 | 136.725, 136.775, 136.875, 136.975 MHz | 136.975 is CSC (common signalling channel) |
| HFDL | HF bands, various ground station frequencies | See `dumphfdl` docs; not in this workspace yet |

These are orientation examples, not comprehensive channel lists. Operational channels vary by region and service provider (ARINC, SITA).

## Common decode paths

**Path A: Classic ACARS text**
```
RF (131 MHz MSK modulation)
  -> ACARS frame
  -> label + text
  -> output
```

**Path B: VDL2 carrying ACARS**
```
RF (136 MHz D8PSK burst)
  -> AVLC I-frame
  -> FF FF 01 marker
  -> ACARS frame
  -> ARINC 622 payload (/<ATSU>.ADS.<REG>...)
  -> ADS-C tags
  -> output
```

**Path C: VDL2 carrying X.25**
```
RF (136 MHz D8PSK burst)
  -> AVLC I-frame
  -> X.25 packet
  -> CLNP/COTP
  -> CPDLC application data
  -> output
```

The workspace handles all three paths. Path A is `acars131`. Paths B and C are `vdl136`.

## ARINC 622 application coverage

ARINC 622 envelopes use a three-letter IMI (application identifier) after the ATSU field. Example: `/<ATSU>.ADS.<REG><data>`. The IMI determines which application parser to use.

| IMI | Application | Status | Notes |
|-----|-------------|--------|-------|
| ADS | ADS-C position/surveillance | Partial | Native parser for 18 tag types; routing integration expanding |
| AT1, CC1, DR1 | CPDLC-related | Partial | Some extraction working; full structured decode pending |
| Others | MIAM, Media Advisory, OHMA | Planned | No native parsers yet; passed through as text |

Not all ACARS traffic uses ARINC 622. Many messages are plain AOC text (gate info, fuel requests, maintenance notes). The workspace preserves unknown payloads as text rather than discarding them.

## Decode implementation coverage

This table maps protocol features to workspace implementation and reference counterparts.

| Feature | Status | Workspace path | Reference |
|---------|--------|----------------|-----------|
| ACARS frame parse (header, text, CRC) | Implemented | `crates/acars/src/decode/acars.rs` | libacars `acars.c`, dumpvdl2 `acars.c` |
| H1 sublabel/MFI extraction | Implemented | `acars.rs` (`extract_sublabel_and_mfi`) | libacars, JAERO `arincparse.cpp` |
| ARINC 622 ADS envelope | Implemented | `crates/acars/src/decode/adsc.rs` | libacars `adsc.c`, JAERO |
| ADS-C tags | Implemented (18 tags) | `adsc.rs` (`AdscTag`) | libacars `adsc.c`, dumpvdl2 `icao.c` |
| AVLC frame parse | Implemented | `crates/acars/src/decode/avlc.rs` | dumpvdl2 `avlc.c` |
| X.25/CLNP/COTP parse | Partial | `crates/acars/src/decode/x25.rs` | dumpvdl2, libacars |
| CPDLC (X.25 path) | Partial (text extraction) | `x25.rs` (`parse_cpdlc_user_data`) | libacars `cpdlc.c`, dumpvdl2 |
| MIAM | Planned | Not yet implemented | libacars `miam.c` |
| Media Advisory | Planned | Not yet implemented | libacars `media-adv.c` |
| OHMA | Planned | Not yet implemented | libacars `ohma.c` |

References:
- `dumpvdl2` uses libacars for ACARS/app decoding (`../../github/dumpvdl2/src/acars.c`)
- `acarsdec` and JAERO also use libacars for app coverage

See `tests/libacars_vectors.rs` for decode validation against libacars test vectors.

## Code organization

**Frontends**:
- `crates/vdl136/src/main.rs`: VDL2 frontend
- `crates/acars131/src/main.rs`: classic ACARS frontend
- `crates/datalink/src/main.rs`: payload decoder CLI

**Shared library** (`crates/acars`):
- `src/demod/vdl2.rs`: VDL2 demod (D8PSK, burst detection, frame sync)
- `src/demod/acars131.rs`: classic ACARS demod (MSK, frame sync)
- `src/decode/avlc.rs`: VDL2 AVLC frame parsing
- `src/decode/xid.rs`: VDL2 XID control frame parsing
- `src/decode/x25.rs`: X.25/CLNP/COTP parsing
- `src/decode/acars.rs`: ACARS message parsing
- `src/decode/adsc.rs`: ADS-C application parsing

Reference implementations:
- `dumpvdl2`: VDL2 bearer and link-layer reference
- `libacars`: application-layer reference
- `acarsdec`: classic ACARS operations reference

This is a Rust reimplementation with compatibility goals, not a direct port. See `docs/20-decoders/reference-policy.md` for how references are used.
