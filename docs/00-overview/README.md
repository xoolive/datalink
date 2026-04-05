# Overview

`datalink` is a Rust workspace for decoding aviation datalink protocols. It focuses on software, not hardware platforms or GUIs.

## What's in this workspace

**Frontends** (demod + frame extract):
- `vdl136`: VDL2 on VHF (136 MHz, D8PSK bursts)
- `acars131`: classic VHF ACARS (129-131 MHz, MSK continuous)

**Shared library** (`acars`):
- Demod modules for VDL2 and classic ACARS
- Protocol parsers: AVLC, X.25, ACARS, ADS-C, partial CPDLC
- All frontends use this library

**Payload decoder** (`datalink`):
- CLI for decoding hex payloads directly (AVLC, ACARS, ADS-C)
- Useful for testing or when you already have frame data

## Implementation status

**Working and tested**:
- VDL2 demod (validated against `dumpvdl2`)
- AVLC frame parsing
- ACARS message parsing
- ADS-C decode (18 tag types)
- Classic ACARS demod (initial validation vs `acarsdec`, 7/7 messages matched on test file)

**Partial**:
- CPDLC (text extraction works, structured decode in progress)
- ARINC 622 app routing (ADS works, full IMI dispatch expanding)
- X.25/COTP reassembly (basic parse works, multi-fragment partial)

**Not started**:
- MIAM, OHMA, Media Advisory native parsers (passed through as text for now)
- HFDL frontend (HF bearer, separate track)

## Reference baseline

This workspace does not port external tools directly. It reimplements protocols in Rust and uses external tools for behavioral validation:

- `dumpvdl2`: VDL2 bearer and link-layer reference
- `vdlm2dec`: secondary VDL2 reference
- `libacars`: application-layer reference (test vectors, not runtime dependency)
- `acarsdec`: classic ACARS reference
- `dumphfdl`: HFDL reference (for future use)
- `JAERO`: satcom workflow reference

See `docs/20-decoders/` for how these are used.

## Next reads

- `docs/00-overview/getting-started.md`: run your first decode
- `docs/00-overview/background.md`: protocol context (bearers, layers, applications)
- `docs/00-overview/architecture.md`: layer diagrams and coverage tables
- `docs/10-modes/`: bearer-specific details (VDL2, classic ACARS, HFDL context)
- `docs/40-pipelines/decoding-pipeline.md`: end-to-end dataflow
