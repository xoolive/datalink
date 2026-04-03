# vdl136

Rust workspace for VDL2 / ACARS decoding, following the split used in `jet1090` and `ship162`:

- `crates/acars`: core decode library
- `crates/vdl136`: thin CLI for local decoding and inspection

Current decode scope:

- pure Rust ACARS frame decoding (header/text/CRC)
- H1 sublabel/MFI extraction compatible with libacars behavior
- ADS-C app-layer decoding (all downlink tag types)
- VDL payload wrapper that can dispatch to ACARS decoding
