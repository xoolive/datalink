# Station naming and metadata

When running multiple decoders or contributing to aggregators (like airframes.io), station metadata helps with debugging, deduplication, and coverage analysis.

## Minimum recommended metadata

Include in each message (or in a station config file):

- **Station ID**: stable identifier (e.g., `mystation-vdl2-west`, `n6abc-acars`). Do not use dynamic IDs that change on restart.
- **Location**: city/region tag or lat/lon (not required in every message, but useful for coverage maps)
- **Bearer and decoder type**: `vdl2` + `vdl136`, `acars-vhf` + `acars131`, etc.
- **Sample source**: SDR model, sample rate, format (e.g., `RTL-SDR v3, 1.05 Msps, cu8`)

## Why station ID matters

If you run two instances of `vdl136` on different machines monitoring the same channel, the station ID lets downstream tools know they are seeing two independent feeds. Without station IDs, deduplication and yield analysis get messy.

## Station ID conventions

Examples:
- `callsign-bearer-location`: `n6abc-vdl2-norcal`
- `hostname-bearer`: `pi4-acars-131`
- `uuid-bearer`: `a3f2-vdl2` (if you want anonymity)

Pick a convention and stick with it. Changing station IDs breaks historical analysis.

## Location tagging

If you want your data to show up on coverage maps (airframes.io, etc.), include location in your station config. You can:
- Hard-code lat/lon in the decoder config
- Use a separate metadata file that the aggregator reads
- Include a `location` field in each JSON message (less common, more overhead)

## Next reads

- `docs/40-pipelines/udp-ingest.md` for schema design
- `docs/50-operations/monitoring-metrics.md` for yield tracking
