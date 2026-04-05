# Decoding pipeline

This page describes how data flows through the `datalink` workspace, from radio samples to decoded JSON.

Runtime roles:

- `vdl136`: VDL2 demod frontend (I/Q and SDR inputs).
- `datalink`: payload decoder app for demodulated ACARS/ARINC 622 messages.
- `acars`: shared decoding library used by both apps.

## End-to-end flow

```text
I/Q recording (.rtl/.cu8)
        |
        v
VDL2 demod (per configured channel)
        |
        v
AVLC frame candidate extraction + FCS check
        |
        v
AVLC payload dispatch
   - I-frame + FF FF 01 -> ACARS
   - I-frame + other     -> X.25
   - U-frame + XID       -> XID
   - other               -> unknown/none (current behavior depends on frame type)
        |
        v
Higher-layer decoding
   - ACARS frame parse (header/text/CRC)
   - ADS-C parse for supported app payload shapes
   - X.25 -> CLNP/COTP partial parse + partial CPDLC extraction
        |
        v
JSON output lines
```

## Typical CLI paths

- Full VDL2 pipeline from I/Q file:

```bash
vdl136 file --file sample.cu8 --center-freq 136850000 --sample-rate 1050000 --channel 136875000 136975000
```

- Decode one AVLC frame from hex:

```bash
datalink avlc "<hex_with_fcs>"
```

- Decode one ACARS frame from hex:

```bash
datalink acars "<hex>" --direction downlink
```

- Decode one ADS-C app payload text:

```bash
datalink adsc "/BOMASAI.ADS.VT-ANB072501A070A988CA73248F0E5DC10200000F5EE1ABC000102B885E0A19F5"
```

## One concrete example

```text
Input: I/Q samples around 136.975 MHz
  -> demod finds a VDL2 burst
  -> AVLC parser validates frame CRC
  -> payload starts with FF FF 01
  -> ACARS parser extracts label/text
  -> if text is ARINC 622 ADS payload, ADS-C decoder returns tag list
  -> output emitted as one JSON object (with frame metadata + decoded payload)
```

## Where ADS-C enters the flow

Today there are two practical entry points:

1. Direct ADS-C decode path:

```text
ADS-C app text -> `datalink adsc` -> `parse_adsc_app_text()` -> ADS-C tags JSON
```

2. VDL2 bearer path (expected app routing path):

```text
I/Q -> VDL2 demod -> AVLC I-frame -> ACARS text
    -> ARINC 622 payload identified as `.ADS.`
    -> ADS-C decode
```

Notes:

- The direct `adsc` subcommand is the explicit/guaranteed ADS-C path today.
- Automatic app routing in the full bearer pipeline is being expanded; this is why ADS-C may appear as parsed ACARS text in some current outputs.

## Example JSON (trimmed)

```json
{
  "timestamp": 1712345678.123,
  "channel_hz": 136975000,
  "raw_frame_hex": "03A1...F0B8",
  "fcs_ok": true,
  "src": "20B677",
  "dst": "4854CA",
  "lcf": { "type": "I" },
  "payload": {
    "Acars": {
      "label": "B6",
      "reg": "VT-ANB",
      "txt": "/BOMASAI.ADS.VT-ANB072501A070A988CA73248F0E5DC10200000F5EE1ABC000102B885E0A19F5",
      "crc_ok": true
    }
  }
}
```

Notes:

- This is an illustrative trimmed shape; exact fields depend on subcommand and decode path.
- ADS-C details appear when app payload decoding is invoked, with tags under the decoded ADS-C object.

## Current limitations to keep in mind

- App routing is still being expanded; full ARINC 622 IMI dispatch is not complete yet.
- CPDLC decode on X.25/COTP path is still partial (heuristic/free-text oriented).
- X.25/COTP reassembly parity is not complete yet.
- `acars131` classic POA VHF ACARS frontend is implemented in an initial form; validation against reference IQ datasets is still pending.

For protocol placement and coverage tables, see `docs/architecture.md`.
For roadmap and implementation tasks, see `plan.md`.
