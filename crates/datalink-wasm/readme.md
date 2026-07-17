# datalink-wasm

WebAssembly bindings for the [`acars`](https://crates.io/crates/acars) decoder.

The initial API exposes two explicit entry points:

- `decode_arinc622(text, direction)` decodes an ARINC 622 envelope and dispatches its IMI to ADS-C or FANS-1/A CPDLC;
- `decode_acars(hex, direction)` decodes a hex-encoded binary ACARS frame and routes its application payload.

`direction` must be `"uplink"`, `"downlink"`, or `"unknown"`.

The package is intended for browser examples in the Aviation Data Handbook and returns ordinary JSON-compatible JavaScript objects.
