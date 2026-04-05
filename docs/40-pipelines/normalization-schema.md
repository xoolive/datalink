# Normalization schema goals

This workspace outputs JSON. The schema should be consistent across bearers so downstream tools do not need bearer-specific logic.

## Cross-bearer schema design

**Fixed top-level fields** (all bearers):
- `timestamp`: message receive time (UTC epoch seconds or ISO 8601)
- `bearer`: bearer type (`vdl2`, `acars-vhf`, `hfdl`, `satcom-lband`)
- `frequency_hz`: tuned frequency
- `channel_hz`: channel center frequency (same as `frequency_hz` for single-channel, different for multi-channel demod)
- `station_id` (optional but recommended): stable station identifier

**Frame metadata** (when applicable):
- `frame_hex`: raw frame bytes (hex string)
- `fcs_ok` or `crc_ok`: frame/message integrity check result

**Decoded payload**:
- Nested object with bearer-specific and protocol-specific fields
- For VDL2: `src`, `dst`, `lcf` (link control field), `payload` (ACARS or X.25)
- For classic ACARS: `payload` (ACARS message directly)

## Example: VDL2 AVLC I-frame with ACARS payload

```json
{
  "timestamp": 1712345678.123,
  "bearer": "vdl2",
  "frequency_hz": 136975000,
  "channel_hz": 136975000,
  "station_id": "mystation",
  "frame_hex": "03A1...",
  "fcs_ok": true,
  "src": "20B677",
  "dst": "4854CA",
  "lcf": {"type": "I", "ns": 3, "nr": 1},
  "payload": {
    "Acars": {
      "mode": "2",
      "reg": "VT-ANB",
      "ack": "!",
      "label": "B6",
      "block_id": "C",
      "txt": "/BOMASAI.ADS.VT-ANB...",
      "crc_ok": true
    }
  }
}
```

## Example: Classic VHF ACARS

```json
{
  "timestamp": 1712345680.456,
  "bearer": "acars-vhf",
  "frequency_hz": 131725000,
  "channel_hz": 131725000,
  "station_id": "mystation",
  "payload": {
    "Acars": {
      "mode": "2",
      "reg": "N12345",
      "ack": ".",
      "label": "H1",
      "block_id": "1",
      "txt": "Flight plan uplink...",
      "crc_ok": true
    }
  }
}
```

## Design rule

Frontend-specific DSP (demod parameters, gain settings, timing recovery) should not leak into the output schema. The schema represents decoded protocol data, not demod internals.

## Next reads

- `docs/40-pipelines/udp-ingest.md` for UDP output patterns
- `docs/00-overview/architecture.md` for layer separation
