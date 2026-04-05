# dumpvdl2 reference

`dumpvdl2` is the primary VDL2 behavior reference for this workspace. It is a mature, widely-used VDL2 decoder with strong AVLC and X.25 handling.

## Why dumpvdl2

VDL2 has many edge cases: multi-block ACARS reassembly, X.25 fragmentation, XID parameter negotiation, frame direction inference. The specification exists but does not cover every ambiguity.

`dumpvdl2` has been validated by years of operational use and is considered the behavioral baseline for VDL2 AVLC and X.25 dispatch.

## What we compare

- **AVLC frame handling**: I/S/U frame parse, FCS validation, direction inference
- **XID behavior**: parameter extraction, station ID reporting
- **X.25 dispatch and reassembly**: packet boundary detection, multi-fragment reassembly
- **Application handoff**: when to route to ACARS parser vs X.25/CLNP parser

## What we don't compare directly

- JSON output schema (dumpvdl2 supports multiple output formats; we use a different schema)
- SDR interface and gain control (frontend-specific, not decode logic)
- Multi-output routing (dumpvdl2 can send to multiple sinks; we keep output simpler)

## libacars integration

`dumpvdl2` uses `libacars` for application-layer decoding (ACARS, ADS-C, CPDLC, MIAM, OHMA, Media Advisory). This workspace implements application parsers in Rust but uses `libacars` test vectors for validation.

See `crates/acars/tests/libacars_vectors.rs` for validation tests.

## Repository

`../../github/dumpvdl2`

Key files for reference:
- `src/avlc.c`: AVLC frame parsing
- `src/xid.c`: XID parameter decode
- `src/x25.c`: X.25 packet handling
- `src/acars.c`: ACARS dispatch and libacars integration

## Next reads

- `docs/10-modes/vdl2.md` for VDL2 bearer details
- `docs/20-decoders/reference-policy.md` for how references are used
