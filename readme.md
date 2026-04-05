# datalink

`datalink` is a Rust-first aviation datalink workspace.

Component layout:

- `acars`: core decoding library (`crates/acars`)
- `datalink`: payload decoder app for demodulated ACARS/ARINC 622 messages
- `vdl136`: VDL2 frontend (I/Q and SDR inputs)
- `acars131`: classic VHF ACARS frontend (initial implementation)

Design direction: demodulation logic is kept in-project and shared through Rust modules in
the `acars` crate as frontends mature.

Current status: VDL2 demod core is now shared from `crates/acars/src/demod/vdl2.rs` and used by `vdl136`.
Reusable DSP primitives are sourced from `../desperado` where appropriate (currently NCO and
Chebyshev filter helpers).

## Positioning

This project sits between two established reference points:

- **`dumpvdl2` + `libacars` parity target** for VDL2 bearer behavior and app decode conventions.
- **Rust-native implementation goal** for maintainability, portability, and tighter integration with the `jet1090` style ecosystem.

In practice:

- `dumpvdl2` is the primary behavior benchmark for AVLC/X.25 framing choices.
- `vdlm2dec` is an additional VDL2 operational reference from the same ecosystem.
- `libacars` is the app-layer semantics reference and fallback strategy model.
- `acarsdec`/`JAERO` are operational references (filters, routing, output shape).
- `dumphfdl` is the primary HFDL reference (HF bearer context and long-haul operations).

`acars` is not a direct port of libacars. It is a Rust-native implementation with
libacars-compatible behavior where implemented, and partial overlap at this stage.

## Current Scope

Current runtime pipeline focus:

- demodulate VDL Mode 2 over VHF from I/Q recordings (`vdl136 file`),
- decode AVLC and payload layers from recovered VDL2 frames,
- parse ACARS text and ARINC 622 app envelopes (including ADS-C and partial CPDLC paths).

Out of scope today:

- full parity-validated classic POA VHF ACARS demodulation (initial `acars131` implementation exists, validation pending).

- ACARS frame decoding (header/text/CRC)
- H1 sublabel/MFI extraction compatible with libacars behavior
- AVLC decode with payload dispatch (`Acars`, `X25`, `Xid`, `Unknown`)
- ADS-C app-layer decoding (downlink tags)
- Partial CPDLC extraction from COTP user data (currently heuristic/free-text oriented)

## What This Project Is (and Is Not)

- **Is:** a VDL2-first decoder stack with parity-driven evolution against known tools.
- **Is:** a codebase aiming to keep JSON outputs stable while deepening decode coverage.
- **Is not:** a one-to-one libacars port today.
- **Is not yet:** full app-layer parity for MIAM, OHMA, Media Advisory, and structured FANS-1/A CPDLC.
- **Is not yet:** complete X.25/COTP reassembly parity in all fragmented cases.

## Near-Term Priorities

1. Wire ARINC-622 app routing directly into normal `file`/`avlc` decode flow.
2. Add X.25 reassembly behavior matching `dumpvdl2` expectations.
3. Preserve non-XID U-frame and S-frame raw payload bytes instead of dropping them.
4. Expand X.25 inner protocol coverage (ESIS, uncompressed CLNP, SNDCF error report).
5. Complete structured CPDLC decode and add MIAM/OHMA/Media Advisory coverage.

## Frontend Roadmap

- Current frontend: VDL Mode 2 over VHF (`vdl136`).
- Current early frontend: classic POA VHF ACARS demodulation (`acars131`, needs dataset validation).

All frontends are intended to feed the same shared decode core (`acars`) and emit a
consistent output schema with bearer metadata.

For implementation details and comparison notes, see `plan.md`.

For high-level documentation, start with `docs/00-overview/README.md`.

The docs navigation index is at `docs/index.md`.

For protocol/layer orientation, see `docs/00-overview/architecture.md`.
