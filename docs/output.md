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
        "kind": "Arinc622",
        "data": {
          "atsu_address": "PIKCPYA",
          "imi": "ADS",
          "registration": "D-AALB",
          "payload": {
            "kind": "Adsc",
            "data": {
              "atsu_address": "PIKCPYA",
              "registration": "D-AALB",
              "tags": [
                {
                  "kind": "EventContractRequest",
                  "data": {
                    "contract_number": 6,
                    "groups": [
                      {
                        "kind": "lateral_deviation_change",
                        "value": {
                          "threshold_nm": 3.0
                        }
                      },
                      {
                        "kind": "vertical_speed_change",
                        "value": {
                          "threshold_ft_per_min": -5056
                        }
                      },
                      {
                        "kind": "altitude_range",
                        "value": {
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
