# Output format

By default, `datalink` emits newline-delimited JSON on stdout.  
Every decoded row uses a common `DecodedEvent` envelope.

## JSONL format

`datalink` normalizes all supported input paths into a common event shape.

The output includes:

- `timestamp`: Unix epoch seconds, when an absolute time is known (see below);
- `time_offset`: seconds from the start of the recording or live run to the frame (I/Q sources);
- `time_source`: where `timestamp` comes from: `capture`, `wallclock`, or `upstream`;
- `bearer`: `vhf`, `vdl2`, `hfdl`, `decoded`, or `unknown`;
- `source`: file, SDR, websocket, or standalone decode metadata;
- `receiver`: channel information for I/Q receivers;
- `aircraft`: best-effort ICAO24, aircraft id, or registration;
- `kinematics`: best-effort position, speed, altitude, or meteorological values;
- `raw_frame_hex`: raw frame bytes when available;
- `message`: the protocol-specific decoded body.

### Timestamps

How `timestamp` is set depends on the source:

| Source | `timestamp` | `time_source` |
| --- | --- | --- |
| Live SDR | arrival time of the samples containing the frame (see below) | `wallclock` |
| File with `start_time` set (`--start-time`, `?start_time=`, or `start_time` in a TOML source) | `start_time` + `time_offset` | `capture` |
| Gqrx (`gqrx_YYYYMMDD_HHMMSS_…`) or SDRuno (`SDRuno_YYYYMMDD_HHMMSSZ_…`) file | start encoded in the filename + `time_offset` | `capture` |
| Other files and stdin | omitted | omitted |
| Airframes.io | upstream timestamp | `upstream` |

`start_time` accepts RFC 3339 (`2026-06-03T08:30:11Z`) or Unix epoch seconds:

```sh
datalink vdl2 --format cu8 --center-freq 136850000 --sample-rate 1050000 \
  --start-time 2026-06-01T12:00:00Z capture.cu8
```

Decoding a recording never uses the time of decoding as the event time, so replayed captures cannot be mistaken for live reception. `time_offset` is set on all I/Q events (VHF, VDL2 and HFDL) and is useful for comparing decoders on the same recording. It marks the end of the frame; for HFDL, the time also includes `--start-second` when decoding a window.

For live SDR input, each chunk of samples is stamped with the time it arrives and frames are placed within their chunk by sample count. If decoding cannot keep up with the sample rate, the SDR driver drops samples; timestamps still follow real time, and `datalink` prints a warning on stderr when the received samples fall behind real time (first at 2 s, then each time the lag doubles). There, `time_offset` is the wall-clock time since the first sample.

An ADS-C event captured from the airframes.io websocket feed looks like this:

```json
{
  "event": "message",
  "timestamp": 1781378212.80346,
  "bearer": "unknown",
  "source": {
    "id": "airframes",
    "name": "airframes.io",
    "class": "events",
    "format": "airframes.io"
  },
  "aircraft": {
    "icao24": "3c4582",
    "aircraft_id": 2727,
    "registration": "D-AALB"
  },
  "kinematics": {
    "position": {
      "latitude": 45.166098,
      "longitude": 24.740809
    },
    "altitude_ft": 37000,
    "track": 116.67,
    "derived_from": "airframes_flight"
  },
  "message": {
    "kind": "airframes",
    "data": {
      "payload": {
        "label": "A6",
        "text": "/PIKCPYA.ADS.D-AALB08060A1812B113227921E314E5DD",
        "from_hex": "90",
        "to_hex": "3C4582",
        "latitude": 0.0,
        "longitude": 0.0,
        "altitude": 0.0,
        "track": null,
        "source_type": "unknown",
        "timestamp": 1781378212.80346,
        "created_at": 1781378212.802201,
        "frequency": 0.0,
        "id": "6885605082",
        "airframe_id": 2727,
        "flight_id": 5544633336,
        "tail": "D-AALB",
        "link_direction": "uplink",
        "airframe": {
          "icao": "3C4582",
          "tail": "D-AALB"
        },
        "flight": {
          "latitude": 45.166098,
          "longitude": 24.740809,
          "altitude": 37000.0,
          "track": 116.67
        }
      },
      "dst": {
        "icao24": "3c4582",
        "addr_type": "aircraft"
      },
      "app": {
        "kind": "arinc622",
        "data": {
          "atsu_address": "PIKCPYA",
          "imi": "ADS",
          "registration": "D-AALB",
          "payload": {
            "kind": "adsc",
            "data": {
              "atsu_address": "PIKCPYA",
              "registration": "D-AALB",
              "tags": [
                {
                  "kind": "event_contract_request",
                  "data": {
                    "contract_number": 6,
                    "groups": [
                      {
                        "kind": "lateral_deviation_change",
                        "data": {
                          "threshold_nm": 3.0
                        }
                      },
                      {
                        "kind": "vertical_speed_change",
                        "data": {
                          "threshold_ft_per_min": -5056
                        }
                      },
                      {
                        "kind": "altitude_range",
                        "data": {
                          "ceiling_ft": 220425,
                          "floor_ft": 216675
                        }
                      },
                      {
                        "kind": "report_waypoint_changes"
                      }
                    ]
                  }
                }
              ]
            }
          }
        }
      }
    }
  }
}
```

For an ACARS frame decoded directly by the VHF path, the top-level envelope is the same, but `message.kind` is `"acars"` and `message.data` contains the ACARS fields such as `label`, `text`, and `app`.

CPDLC elements also include `fragments`, a phrase AST compiled from the bundled CPDLC catalog templates. Each fragment is either literal text or a typed value slot. Consumers resolve a value slot from the element body: if `body.kind` equals the slot, use `body.data`; otherwise use `body.data[slot]`:

```json
{
  "id": 20,
  "catalog_name": "uM20Altitude",
  "fragments": [
    {
      "kind": "text",
      "data": "CLIMB TO AND MAINTAIN "
    },
    {
      "kind": "value",
      "data": "altitude"
    }
  ],
  "body": {
    "kind": "altitude",
    "data": {
      "kind": "flight_level",
      "data": 320
    }
  },
  "is_additional": false
}
```

```json
{
  "id": 163,
  "catalog_name": "uM163ICAOfacilitydesignationTp4table",
  "fragments": [
    {
      "kind": "value",
      "data": "icao_facility_designation"
    },
    {
      "kind": "text",
      "data": " "
    },
    {
      "kind": "value",
      "data": "tp4_table"
    }
  ],
  "body": {
    "kind": "icao_facility_designation_tp4_table",
    "data": {
      "icao_facility_designation": {
        "kind": "icao",
        "data": "FNAN"
      },
      "tp4_table": "label_a"
    }
  },
  "is_additional": false
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

- `datalink-acars`
- `datalink-adsc`
- `datalink-cpdlc`
- `datalink-hfdl`
- `datalink-sq`
- `datalink-vdl2`
- `datalink-x25`
- `datalink-xid`
- `datalink-unknown`
