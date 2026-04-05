# UDP ingest notes

For multi-decoder or multi-station operations, UDP is a common output format. This page covers schema considerations when building UDP ingest pipelines.

## Why UDP

UDP is stateless and low-overhead. Decoders can send JSON messages to a UDP port without worrying about connection state or backpressure. The receiver can be on the same machine or remote.

Common pattern:
- Decoder sends JSON lines to `udp://127.0.0.1:5555`
- Ingest service listens on UDP port 5555, parses JSON, routes to database/aggregator

## Schema boundaries

Define clear boundaries per port or per source:

- **Format**: JSON lines, one message per UDP packet (or newline-delimited if you allow multi-message packets)
- **Station ID**: include a stable station identifier in each message so the receiver knows where it came from
- **Timestamp**: use UTC epoch seconds (or ISO 8601 strings). Be consistent across all sources.
- **Bearer/channel metadata**: include `bearer` (vdl2, acars-vhf, hfdl), `frequency_hz`, `channel_hz` so the receiver can deduplicate and route correctly

## Deduplication

If you run overlapping receivers (e.g., two stations both monitoring 136.975 MHz), you will see duplicate messages. Dedup strategies:

- **Hash the frame**: use raw frame hex or a hash of (timestamp, src, dst, payload) as a dedup key
- **Time window**: if two messages from different stations arrive within 2 seconds and have the same aircraft reg + label + text, assume duplicate
- **Defer to aggregator**: some aggregators (like airframes.io) handle dedup server-side

## Example JSON schema

```json
{
  "station_id": "mystation",
  "timestamp": 1712345678.123,
  "bearer": "vdl2",
  "frequency_hz": 136975000,
  "channel_hz": 136975000,
  "frame_hex": "...",
  "decoded": {
    "src": "20B677",
    "dst": "4854CA",
    "payload": {
      "Acars": {
        "mode": "2",
        "reg": "VT-ANB",
        "label": "B6",
        "txt": "..."
      }
    }
  }
}
```

## Next reads

- `docs/40-pipelines/normalization-schema.md` for cross-bearer schema design
- `docs/50-operations/station-naming-and-metadata.md` for station ID conventions
