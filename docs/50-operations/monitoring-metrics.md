# Monitoring metrics

Track decoder health and per-channel yield to catch problems early.

## Core metrics

**Input sample flow**:
- Samples received per second
- Sample drops (buffer overruns, USB issues)
- SDR connection status (for live inputs)

If you are seeing sample drops, reduce the sample rate, reduce the number of channels, or check USB power/cables.

**Demod frame counts**:
- Frames detected per channel per minute
- Frames with valid FCS/CRC vs total frames

Low frame counts can mean weak signal, wrong frequency, or gain too low/high. Compare against a known-good capture first.

**Decode success/failure**:
- ACARS messages parsed successfully
- Unknown payload types (e.g., ARINC 622 IMIs you don't recognize)
- Parse errors (should be rare if frame CRC is valid)

**Per-frequency yield**:
- Messages per channel per hour
- Compare channels to see if one is dead or over-active (interference)

## When to act on metrics

- **Sample drops**: reduce load (fewer channels, lower sample rate)
- **Low frame counts on all channels**: check antenna, gain, or frequency accuracy
- **Low frame counts on one channel**: that channel may not be active in your region, or you have a frequency error
- **High CRC failures**: increase gain (signal too weak) or decrease gain (overload/clipping)

## Logging

Log metrics to a file or time-series database (InfluxDB, Prometheus). Graph per-channel message rates over time to see trends (time of day, day of week).

Example quick-check script:
```bash
# Count messages per channel in the last hour (from JSON logs)
grep '"channel_hz": 136975000' decoder.log | wc -l
grep '"channel_hz": 136875000' decoder.log | wc -l
```

## Next reads

- `docs/60-troubleshooting/quick-triage.md` for diagnostic workflows
- `docs/10-modes/receiver-systems.md` for channel selection strategy
