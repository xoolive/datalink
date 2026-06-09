# Merged Receiver CLI and Source Model

Status: proposal / design notes for review.

This document sketches a CLI and configuration model for running `datalink` as a heterogeneous receiver. The goal is to support simple one-protocol commands while also allowing one or more physical/logical sources to feed multiple datalink decoders.

## Goals

- Keep simple use simple:
  - `datalink vhf rtlsdr://`
  - `datalink vdl2 airspy://`
  - `datalink hfdl capture.wav`
- Use top-level `datalink` with no bearer subcommand for complex merged operation.
- Separate **source type** from **bearer/protocol**.
- Allow one physical source to feed several bearer receivers, e.g. one HackRF covering both VHF ACARS and VDL2.
- Treat Airframes.io as a network/event source, not as part of the VDL2 frontend.
- Keep `raw` as an output option, not a receiver option.
- Avoid `--stats` in the merged CLI; operational counters can be logs/metrics later.

## Non-goals for the first version

- A complete deduplication/track-fusion design.
- SDR auto-planning across incompatible center frequencies.
- A stable external JSON schema guarantee.
- A full metrics system.

## Current problem

Today, `datalink` has protocol-specific frontends:

```text
datalink vhf ...
datalink vdl2 ...
datalink hfdl ...
datalink airframes.io ...
```

That is good for focused decoding, but it becomes awkward when a station has several devices, recordings, websocket feeds, and bearers active at once. The problem is harder because:

- different devices expose different sample formats and bandwidths;
- one device may cover several bearers at once;
- some inputs are I/Q, some are frames, and some are already partially decoded events;
- payloads differ, but should still share a common output envelope.

## Vocabulary

### Source

A source is where data comes from.

Examples:

- `file://capture.cs16`
- `rtlsdr://0`
- `airspy://serial=...`
- `hackrf://0`
- `soapy://driver=rtlsdr`
- `websocket = "airframes://"`

A source is not necessarily a protocol. `hackrf://0` says how to obtain samples; it does not say whether those samples contain VHF ACARS, VDL2, HFDL, or something else.

### Receiver

A receiver is a decoder pipeline attached to a source.

Examples:

- VHF ACARS receiver on 131.525 MHz and 131.725 MHz.
- VDL2 receiver on 136.875 MHz.
- HFDL receiver on 11387 kHz.

A single I/Q source may have several receivers.

### Bearer

A bearer is the datalink family/protocol carried by the source or decoded by the receiver.

Initial bearer names:

```text
vhf
vdl2
hfdl
decoded
```

Airframes.io is not itself a bearer. It is a source format that may contain VHF, VDL2, HFDL, or unknown events depending on what the feed provides.

### Output

Output controls how normalized events are emitted.

Examples:

- JSONL file or stdout.
- Include protocol-specific raw/full decode.
- Later: Redis, websocket, HTTP, metrics.

`raw = true` belongs here, not under a receiver.

Redis pub/sub is part of the first merged-mode output target set.

## Proposed CLI

### Simple mode: explicit bearer subcommands

These remain the friendly one-liners:

```bash
datalink vhf rtlsdr://
datalink vdl2 airspy://
datalink hfdl ~/captures/hfdl.wav
```

These commands should behave like focused decoders. They can keep bearer-specific defaults and concise options.

### Complex mode: no bearer subcommand

Top-level `datalink` with no bearer subcommand enters merged receiver mode:

```bash
datalink --config datalink.toml
```

Merged mode is config-file only in the first version. We intentionally do **not** add `--source` / `--receiver` one-liners because that syntax becomes hard to read as soon as one physical source feeds several receivers.

The config file is the interface for merged stations.

## Proposed config model

### One HackRF source feeding VHF and VDL2

```toml
[output]
jsonl = "-"
raw = true
redis_url = "redis://localhost:6379"
redis_topic = "datalink"

[[sources]]
id = "hackrf-vhf-wide"
name = "HackRF VHF wide capture"
type = "iq"
hackrf = { device = 0 }
center_freq = 134_000_000
sample_rate = 8_000_000
lna_gain = 32
vga_gain = 20

  [[sources.receivers]]
  bearer = "vhf"
  # Optional. If omitted, use the VHF default channels that fit the source bandwidth.
  channels = [131_525_000, 131_725_000, 131_825_000]


  [[sources.receivers]]
  bearer = "vdl2"
  # Optional. If omitted, use the VDL2 default channels that fit the source bandwidth.
  channels = [136_875_000, 136_975_000]
```

The HackRF is opened once. Its I/Q stream is fanned out to two receiver pipelines.

### HFDL recording

```toml
[[sources]]
id = "hfdl-sdruno"
name = "SDRuno HFDL sample"
file = "./captures/hfdl/SDRuno_20200904_143937Z_11404kHz.wav"

  [[sources.receivers]]
  bearer = "hfdl"
  channels = [11_387_000]
```

File metadata may provide defaults where possible, e.g. WAV sample rate and SDRuno-style center frequency from filename.

WAV frequency and sample rate should also work directly on `datalink vhf`, `datalink vdl2`, and `datalink hfdl` where the recording format or filename provides enough metadata.

### Airframes.io event source

Airframes.io should be modeled as an event source, not as `vdl2`.

```toml
[[sources]]
id = "airframes"
name = "Airframes.io websocket"
type = "events"
websocket = "airframes://"
format = "airframes.io"
```

No RF fields. No channels. No demod receivers.

The normalization layer maps each Airframes.io row into the common output envelope and sets `bearer` when the row contains enough information to infer it.

## Source classes

### I/Q sources

I/Q sources produce complex samples and require one or more receiver pipelines.

```text
I/Q source -> receiver/channelizer/demod -> frame decode -> normalized event
```

Examples:

- file recordings
- RTL-SDR
- Airspy
- HackRF
- SoapySDR

Required or defaultable fields:

```toml
type = "iq"
center_freq = 134_000_000
sample_rate = 8_000_000
format = "cu8" # for file sources when not inferable
```

### Frame sources

Frame sources produce protocol frames but not raw I/Q. This is not necessarily needed immediately, but the model should leave room for it.

```text
frame source -> frame decode -> normalized event
```

Examples:

- future AVLC frame socket
- raw ACARS frame JSONL
- replayed HFDL MPDU JSONL

Possible config shape:

```toml
[[sources]]
id = "vdl2-frame-feed"
type = "frames"
tcp = "localhost:5555"
format = "avlc-jsonl"
bearer = "vdl2"
```

### Event sources

Event sources produce already decoded or partially decoded events.

```text
event source -> normalize -> normalized event
```

Examples:

- Airframes.io websocket
- future external APIs

Possible config shape:

```toml
[[sources]]
id = "airframes"
type = "events"
websocket = "airframes://"
format = "airframes.io"
```

## Output envelope

Merged output should use a shared outer envelope and put bearer-specific data inside `message` and optionally `raw`.

Example:

```json
{
  "event": "message",
  "timestamp": 1599228877.12,
  "bearer": "hfdl",
  "source": {
    "id": "hfdl-sdruno",
    "name": "SDRuno HFDL sample"
  },
  "receiver": {
    "bearer": "hfdl",
    "channel_hz": 11387000
  },
  "aircraft": {
    "aircraft_id": 250,
    "icao24": null
  },
  "message": {
    "kind": "hfdl.performance",
    "ground_station_id": 4,
    "position": {
      "lat": 42.1120493165,
      "lon": -89.0422993513
    },
    "time_utc": "14:14:24"
  },
  "raw": {
    "frame_hex": "0784FA000400...",
    "decode": {
      "...": "full protocol-specific decode when output.raw=true"
    }
  }
}
```

Notes:

- `source` describes where the data came from.
- `receiver` describes the decoder path and RF channel.
- `bearer` is the inferred/decoded datalink bearer.
- `message` contains normalized useful fields.
- `raw` is only present when `[output].raw = true`.

## Airframes.io normalization

Airframes.io should produce the same envelope, but with `type = "events"` source metadata.

Example:

```json
{
  "event": "message",
  "timestamp": 1710000000.0,
  "bearer": "vdl2",
  "source": {
    "id": "airframes",
    "name": "Airframes.io websocket",
    "type": "events",
    "format": "airframes.io"
  },
  "receiver": null,
  "aircraft": {
    "icao24": "A1B2C3"
  },
  "message": {
    "kind": "acars",
    "label": "SA",
    "text": "..."
  },
  "raw": {
    "airframes": {
      "...": "original row when output.raw=true"
    }
  }
}
```

If Airframes.io does not provide enough information to infer the bearer, use:

```json
"bearer": "unknown"
```

not:

```json
"bearer": "airframes.io"
```

because Airframes.io is the source format, not the datalink bearer.

## Validation rules

Suggested validation rules:

- `type = "iq"` sources may contain `[[sources.receivers]]`.
- `type = "events"` sources should not contain `[[sources.receivers]]`.
- Source class is inferred from fields in the first implementation:
  - SDR or I/Q file fields imply an I/Q source.
  - `websocket = "airframes://..."` plus `format = "airframes.io"` implies an event source.
  - Frame sources are reserved for later and are not required in v1.
- `raw` is only valid under `[output]`.
- `channels` belong under receivers, not the physical source, except in simple protocol-specific commands where the command itself implies a single receiver.
- One source ID must be unique.
- One receiver must have a supported `bearer`.
- Receiver channel frequencies must fit inside the source bandwidth when the source is I/Q and has known `center_freq`/`sample_rate`.

## Open questions

- Complex mode is `datalink --config file.toml` only.
- Redis is part of first merged mode.
- Source class is inferred from fields like `hackrf`, `rtlsdr`, `file`, and `websocket`; explicit `type` remains allowed for clarity.
- Should source files support multiple time bases and replay speed controls in this same config?
- How much normalization should happen before a stable schema is declared?
- Should Airframes.io be a top-level simple command too, or only a complex-mode source?

## Tentative implementation direction

Later, implementation could be organized around these internal types:

```text
ReceiverConfig
SourceConfig
OutputConfig
DecodedEvent
SourceMetadata
ReceiverMetadata
```

Runtime pipeline:

```text
load config
validate sources and receivers
start one task per source
fan out I/Q sources to attached receivers
normalize each decoded message into DecodedEvent
write JSONL output
```

The existing protocol-specific commands can remain thin wrappers that construct an equivalent single-source/single-receiver config internally.

# example files for testing

write a proper configuration for all

./captures/acars/SDRuno_20200908_152020Z_129535kHz.wav
./captures/hfdl/SDRuno_20200904_143937Z_11404kHz.wav
./captures/vdl2/rtlsdr_136850000_1050000_dumpvdl_6min.rtl
./captures/vdl2/gqrx_20260518_114025_136500000_1800000_fc.raw
./captures/vhf/gqrx_20260518_114201_131500000_1800000_fc.raw
