# Output format

By default, `datalink` emits newline-delimited JSON on stdout.  
Every decoded row uses a common `DecodedEvent` envelope.

## JSONL format

`datalink` normalizes all supported input paths into a common event shape.

The output includes:

- `bearer`: `vhf`, `vdl2`, `hfdl`, `decoded`, or `unknown`;
- `source`: file, SDR, websocket, or standalone decode metadata;
- `receiver`: channel information for I/Q receivers;
- `aircraft`: best-effort ICAO24, aircraft id, or registration;
- `kinematics`: best-effort position, speed, altitude, or meteorological values;
- `raw_frame_hex`: raw frame bytes when available;
- `message`: the protocol-specific decoded body.

A simplified ADS-C-like event might look like this:

```json
{
  "event": "message",
  "timestamp": 1781356283.362,
  "bearer": "vhf",
  "source": {
    "id": "airframes",
    "name": "airframes.io",
    "class": "events",
    "format": "airframes.io",
    "source_type": "vhf",
    "frequency": 131.725
  },
  "aircraft": {
    "icao24": "caa172",
    "aircraft_id": 13237,
    "registration": "A7-BAH"
  },
  "kinematics": {
    "position": {
      "latitude": 51.313533782958984,
      "longitude": 2.698688507080078
    },
    "altitude_ft": 34000,
    "derived_from": "adsc_basic"
  },
  "label": "B6",
  "text": "/JEDAAYA.ADS.A7-BAH07247D580F5A484D0A8D9A0D24AB6808FE45EE418E24C4680566C3E8400E6740CEC004BB4F",
  "app": {
    "Arinc622": {
      "atsu_address": "JEDAAYA",
      "imi": "ADS",
      "registration": "A7-BAH",
      "payload": {
        "type": "Adsc",
        "data": {
          "tags": [
            {
              "BasicReport": {
                "latitude": 51.313533782958984,
                "longitude": 2.698688507080078,
                "altitude_ft": 34000,
                "timestamp_seconds_past_hour": 675.375,
                "nav_redundancy_ok": false,
                "position_accuracy_code": 5,
                "tcas_ok": true
              }
            },
            {
              "PredictedRoute": {
                "next_latitude": 51.56656265258789,
                "next_longitude": 1.5808296203613281,
                "next_altitude_ft": 24292,
                "next_eta_seconds": 398,
                "next_next_latitude": 51.70389175415039,
                "next_next_longitude": 0.9494590759277344,
                "next_next_altitude_ft": 16004
              }
            },
            {
              "EarthReferenceData": {
                "heading_or_track_degrees": 290.390625,
                "heading_invalid": false,
                "speed": 413.5,
                "vertical_speed_ft_per_min": 16
              }
            }
          ]
        }
      }
    }
  }
}
```

The exact nested body depends on the decoded protocol, but the top-level envelope is designed to make mixed sources easier to consume.

## Sinks

By default, decoded events are written to stdout as JSONL.

```toml
[output]
jsonl = "-"
```

Write to a file and optionally publish to Redis:

```toml
[output]
jsonl = "datalink.jsonl"
redis_url = "redis://localhost:6379"
```

Redis topics are selected from decoded application type where possible, including:

- `datalink-sq`
- `datalink-acars`
- `datalink-cpdlc`
- `datalink-hfdl`
- `datalink-other`
