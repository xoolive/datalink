# acars

Core decoding crate for ACARS, VDL2, ADS-C, and CPDLC payloads.

It is shared by:

- `vdl136` (VDL2 demod frontend),
- `datalink` (payload decoder CLI),
- future frontends such as `acars131`.

Current module split includes:

- `src/demod/vdl2.rs` for reusable VDL2 demodulation,
- `src/decode/*` for protocol decoding layers.

Design intent:

- keep parser behavior close to `dumpvdl2`/`libacars` where practical,
- expose strongly typed Rust structures for downstream tooling,
- preserve stable JSON-facing conventions while expanding decode depth.

Initial implementation includes:

- ACARS frame parsing from octets (post-SOH, with trailing DEL)
- CRC-16-CCITT verification
- ACARS header and text extraction
- H1 sublabel/MFI extraction
- VDL payload wrapper that can dispatch to ACARS
- ADS-C app-layer parsing (`/ATSU.ADS.<reg><payload><crc>`) with deku bitfield
  structs for all downlink tag types present in OpenSky `adsc_decoded.txt`
