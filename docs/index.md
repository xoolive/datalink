# datalink documentation

## What this project is

`datalink` is the workspace for this Rust-native aviation datalink project.

Naming map used in this documentation:

- `acars`: core decoding library (`crates/acars`)
- `datalink`: decoder app for already demodulated ACARS/ARINC 622 payload messages
- `vdl136`: VDL2 frontend for I/Q/SDR inputs
- `acars131`: classic VHF ACARS frontend (initial implementation)

- It currently demodulates and decodes **VDL Mode 2 (VHF)** traffic through `vdl136`.
- It then decodes higher-layer payloads through `acars` (AVLC, X.25, ACARS, ARINC 622 app envelopes).
- It provides structured JSON outputs intended for analysis, parity testing, and downstream tooling.

This project is **not** a one-to-one port of libacars. It is a Rust implementation with
compatibility goals where behavior is already implemented.

## Why datalink exists

- Build an in-house, Rust-first decoder stack.
- Keep behavior parity with established references where it matters.
- Preserve stable output conventions while increasing decode depth over time.
- Support multi-bearer evolution with a shared decode core.

## Current scope

- In scope now:
  - VDL2/VHF demod path (`vdl136 file`)
  - AVLC parsing and payload dispatch
  - ACARS frame parsing + H1 sublabel/MFI extraction
  - ADS-C decoding
  - partial CPDLC extraction from X.25/COTP path

- Out of scope for now:
  - parity-validated classic POA VHF ACARS demodulation (`acars131` is implemented but not yet validated on reference IQ datasets)

## Reference position

- `dumpvdl2`: primary bearer-level parity reference for VDL2/AVLC/X.25 behavior.
- `libacars`: primary app-layer semantics reference (ADS-C, CPDLC, MIAM, OHMA, Media Advisory).
- `acarsdec` and JAERO: operational and workflow references.

External tools are references for validation and fixtures. Demod frontends remain in-house.

## Document map

- `docs/background.md` — protocol and operational context (non-code technical background).
- `docs/architecture.md` — protocol layers, glossary, decode paths, coverage matrix.
- `docs/decoding-pipeline.md` — end-to-end data flow from I/Q input to JSON output.
- `plan.md` — implementation roadmap, parity gaps, and phased delivery plan.

## Short glossary

- **VDL2**: VHF Data Link Mode 2 bearer.
- **AVLC**: VDL2 link-control framing.
- **ACARS**: message transport/envelope used across multiple bearers.
- **ARINC 622**: app envelope conventions carried over ACARS for ATS apps.
- **CPDLC / ADS-C**: application protocols carried above ACARS/ARINC paths.
