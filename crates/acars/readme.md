# acars

Core decoding crate for aviation datalink protocols.

This crate is shared by the `datalink` CLI and downstream Rust users that want to parse or
inspect datalink frames without depending on the CLI.

## Modules

- `decode::acars` — ACARS frame parsing, CRC verification, headers, text, labels, and
  direction handling.
- `decode::avlc` — VDL2 AVLC frame parsing, FCS status, addresses, control fields, and
  payload dispatch.
- `decode::hfdl` — HFDL PDU/MPDU/LPDU parsing.
- `decode::payload` — application payload decoders, including ARINC 622 ADS-C and partial
  CPDLC-related paths.
- `decode::compact` — compact JSON shaping used by the CLI.
- `demod` — optional demodulators for VDL2, VHF ACARS, HFDL, and resampling helpers.

## Features

The default build is decode-only:

```toml
acars = { path = "crates/acars" }
```

Enable demodulation support when I/Q sample processing is needed:

```toml
acars = { path = "crates/acars", features = ["demod"] }
```

The `demod` feature enables DSP dependencies such as `desperado` and `num-complex`. Keeping
it optional lets pure parser users avoid SDR/DSP dependency builds.

## Current decode coverage

- ACARS frame parsing from octets
- CRC-16-CCITT verification
- ACARS header/text extraction
- H1 sublabel/MFI extraction
- AVLC parsing with payload dispatch (`Acars`, `X25`, `Xid`, `Unknown`)
- HFDL PDU parsing
- ADS-C application-layer parsing for implemented downlink tags
- Partial CPDLC and related ARINC 622 payload extraction

The implementation is Rust-native and uses `dumpvdl2`, `libacars`, `acarsdec`, `JAERO`, and
`dumphfdl` as behavioral references where applicable.
