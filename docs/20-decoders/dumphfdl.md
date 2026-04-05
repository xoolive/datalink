# dumphfdl reference

`dumphfdl` is the primary HFDL bearer reference for this workspace (though HFDL is not implemented as a frontend yet).

## Why dumphfdl

HFDL uses HF radio with different modulation (PSK variants), different framing, and different channel management than VHF bearers. `dumphfdl` is the operational standard for HFDL decoding.

## What we would compare (future)

If this workspace adds an HFDL frontend:

- **HFDL framing**: downlink/uplink frame structure, squitter handling
- **Multi-frequency handling**: ground station frequency selection, systable parsing
- **ACARS dispatch**: how HFDL payloads route to ACARS/app parsers

Higher-layer ACARS and application decode logic should be reusable from `crates/acars/src/decode/`.

## Current use

HFDL is documented in this workspace for context. See `docs/10-modes/hfdl.md` for bearer details.

## Repository

`../../github/dumphfdl`

## Next reads

- `docs/10-modes/hfdl.md` for HFDL bearer context
- `docs/20-decoders/reference-policy.md` for how references are used
