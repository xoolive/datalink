# datalink

`datalink` is an open-source **receiver and decoder for aviation datalink traffic**: the messages aircraft exchange with airline systems, air traffic service units, and ground-station networks.

!!! tip "Start here"

    If you are new to ACARS, VDL2, HFDL, ADS-C, or CPDLC, read [Getting Started](getting-started.md) first.

You may be familiar with ADS-B/Mode S tooling such as [`jet1090`](https://mode-s.org/jet1090):

- ADS-B tells you where an aircraft is (**surveillance**);
- Datalink helps explain what the aircraft and ground systems are communicating (**communication** with some surveillance): clearances, acknowledgements, surveillance contracts, route intent, ATIS/weather requests, OOOI events, and airline operational messages.

!!! warning "Datalink or `datalink`"

    We refer here to datalink in plain characters for the concept, and to `datalink` for the software.

## What `datalink` does

- **Demodulate** VHF ACARS, VDL Mode 2, and HFDL from files and supported SDR sources.
- **Decode** ACARS frames, AVLC/VDL2 frames, HFDL messages, ADS-C payloads, CPDLC paths, and selected airline operational payloads.
- **Ingest and structure** [airframes.io](https://app.airframes.io/) websocket rows when you want worldwide live data.
- **Normalize** decoded traffic into a common JSONL event envelope with source, receiver, aircraft, kinematics, raw-frame, and protocol-body fields.
- **Publishes** to stdout/files and optional Redis topics for integration with research tools, dashboards, or [tangram](https://mode-s.org/tangram).

!!! tip "airframes.io"

    [airframes.io](https://app.airframes.io/) has a good infrastructure. Consider contributing to their feed.

## Where to go next

1. [Getting Started](getting-started.md) — the datalink concepts and how `datalink` maps to them.
2. [Installation](install.md) — current build/install notes.
3. [Sources](sources.md) — files, stdin, SDRs, and Airframes.io event sources.
4. [Output & Schema](output.md) — the normalized JSON event shape.
5. [Configuration](config.md) — merged receiver TOML examples.
