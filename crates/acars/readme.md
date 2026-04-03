# acars

Core decoding crate for ACARS, VDL2, ADS-C, and CPDLC payloads.

Initial implementation includes:

- ACARS frame parsing from octets (post-SOH, with trailing DEL)
- CRC-16-CCITT verification
- ACARS header and text extraction
- H1 sublabel/MFI extraction
- VDL payload wrapper that can dispatch to ACARS
- ADS-C app-layer parsing (`/ATSU.ADS.<reg><payload><crc>`) with deku bitfield
  structs for all downlink tag types present in OpenSky `adsc_decoded.txt`
