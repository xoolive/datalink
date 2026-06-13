# Getting Started

Aviation datalink is easier to understand if you read it from the outside in:

1. **Radio frequencies**, i.e., where signals are transmitted.
2. **Bearers**, i.e., the physical/link technologies: VHF ACARS, VDL Mode 2, HFDL, SATCOM
3. **Wrappers and network layers**, i.e., ACARS frames, AVLC, X.25, ARINC 622, ATN B1
4. **Applications**, i.e., CPDLC, ADS-C, ATIS, OOOI, weather, maintenance, and airline operations.

`datalink` is a receiver/decoder for this stack.

## Communication vs. Surveillance

ADS-B and Mode S are mostly **surveillance systems**. They answer questions like: where is the aircraft, how fast is it moving, and what transponder address identifies it?

Datalink answers a different class of questions:

- what did ATC instruct the crew to do?
- what did the crew acknowledge?
- what surveillance contract is active over oceanic airspace?
- what route, waypoint, or next-report information did the aircraft send?
- what airline operational events happened: out, off, on, in, ATIS, weather, maintenance?

!!! warning

    The confusing part is that _“datalink”_ is not one protocol. It is **a family of radio bearers and application protocols** stacked on top of each other.

## Frequencies and receiver tools

Different datalink systems live in different parts of the spectrum.

They may need different receiver setups. The key take-away is that, unlike ADS-B or AIS, **you cannot receive everything at once** with a single RTL-SDR setup.

| System        | Typical frequencies        | Coverage                | Common tools in the ecosystem                                                                          |
| ------------- | -------------------------- | ----------------------- | ------------------------------------------------------------------------------------------------------ |
| VHF ACARS     | mainly 129–131 MHz         | Line of sight           | [`acarsdec`](https://github.com/TLeconte/acarsdec)                                                     |
| VDL Mode 2    | 136–137 MHz                | Line of sight           | [`dumpvdl2`](https://github.com/szpajder/dumpvdl2), [`vdlm2dec`](https://github.com/szpajder/vdlm2dec) |
| HFDL          | roughly 2–22 MHz HF        | Long range (oceanic)    | [`dumphfdl`](https://github.com/szpajder/dumphfdl)                                                     |
| Inmarsat AERO | around 1.5 GHz L-band (\*) | Satellite footprint     | [`JAERO`](https://github.com/jontio/JAERO)                                                             |
| Iridium AoI   | around 1.6 GHz L-band      | Global, including polar | [`iridium-toolkit`](https://github.com/muccc/iridium-toolkit)                                          |

(\*) also C-band feeder links

!!! note

    The tool names above are ecosystem references, not required dependencies. `datalink` is intended to provide a unified Rust path for the bearers it supports.

For reference, here are frequencies for other surveillance systems:

| System              | Typical frequencies | Coverage      | Common tools in the ecosystem                                                                                                                    |
| ------------------- | ------------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| ADS-B / Mode&nbsp;S | 1090 MHz            | Line of sight | [`jet1090`](https://mode-s.org/jet1090/), [`dump1090`](https://github.com/flightaware/dump1090), [`readsb`](https://github.com/wiedehopf/readsb) |
| AIS (ships)         | ±162 MHz            | Line of sight | [`ship162`](https://github.com/xoolive/ship162), [`AIS-catcher`](https://github.com/jvde-github/AIS-catcher)                                     |

### Known VHF ACARS channels

VHF ACARS channels are region-dependent.

When no explicit channel list is configured, `datalink` first tries to choose known channels that fit inside the source bandwidth. If bandwidth metadata is missing or too narrow, it falls back to a compact default scan list.

|   Frequency | Support                    |   Frequency | Support                     |
| ----------: | -------------------------- | ----------: | --------------------------- |
| 129.125 MHz | Global ARINC primary       | 130.025 MHz | USA/Canada ARINC secondary  |
| 130.425 MHz | USA ARINC additional       | 130.450 MHz | USA/Canada ARINC additional |
| 131.125 MHz | USA ARINC additional       | 131.450 MHz | Japan                       |
| 131.475 MHz | Air Canada company channel | 131.525 MHz | Europe SITA secondary       |
| 131.550 MHz | Global SITA primary        | 131.725 MHz | Europe SITA                 |
| 131.850 MHz | New European channel       |             |                             |

### Known VDL Mode 2 channels

VDL2 channels cluster around 136–137 MHz.

`datalink` uses the same idea: choose known channels that fit the source bandwidth when possible, or fall back to default scan channels.

|   Frequency | Support          |   Frequency | Support                 |
| ----------: | ---------------- | ----------: | ----------------------- |
| 136.100 MHz | USA ARINC        | 136.650 MHz | USA ARINC               |
| 136.675 MHz | Europe ARINC     | 136.700 MHz | USA ARINC               |
| 136.725 MHz | Europe ARINC     | 136.750 MHz | Additional USA / Europe |
| 136.775 MHz | Europe SITA      | 136.800 MHz | USA SITA                |
| 136.825 MHz | Europe ARINC     | 136.850 MHz | North America SITA      |
| 136.875 MHz | Europe SITA      | 136.900 MHz | European secondary      |
| 136.975 MHz | Worldwide common |             |                         |

### Others

HFDL is different: frequencies change with station, propagation, time of day, and operational use. A receiver often monitors one or more known HF channels rather than a compact VHF band.

The docs.rs page contains a list of known HDFL frequencies.  
<https://docs.rs/crate/datalink/latest>

## Bearers

A **bearer** is the physical/link technology that carries the data.

### VHF ACARS / POA

VHF ACARS is the original "plain old ACARS" (sometimes written as POA) link.

It carries ACARS messages directly over a low-rate VHF signal (MSK modulation).

```text
VHF signal → demodulator → ACARS frame → label/application decoder
```

It is simple compared to other bearers: the ACARS frame is the main container.

### VDL Mode 2

VDL2 is a higher-throughput VHF (DMSK modulation) digital link. It carries bursts that decode into **AVLC** frames.

```text
VDL2 burst → demodulator → AVLC frame → payload
```

The AVLC payload can take two important paths:

```text
AVLC I-frame → ACARS prefix → ACARS frame → ARINC 622 / ACARS applications
AVLC I-frame → X.25 packet  → CLNP/COTP/ULCS → ATN B1 applications
```

This is why VDL2 output can contain both ACARS-style traffic and ATN B1 CPDLC traffic.

### HFDL

HFDL uses HF radio for long-range datalink, especially where VHF line-of-sight coverage is unavailable.

```text
HF signal → HFDL demodulator → HFDL frame → decoded payload
```

HF reception depends heavily on propagation. Native HFDL work in `datalink` is experimental, but decoded HFDL messages are still normalized into the same output envelope.

### SATCOM and Iridium

Satellite ACARS bearers are important in the real world because they provide oceanic, remote, and polar coverage. They are part of the conceptual stack, and Airframes.io may expose decoded satellite-origin messages, but native SATCOM/Iridium RF reception is not currently a `datalink` target.

## Wrappers and containers

Once a bearer has produced bytes or frames, the next question is to identify the container.

### ACARS frame

ACARS is **both** a legacy system and a common message container used across bearers.

```text
mode · registration · ack · label · block_id · text · checksum
```

The `label` is the first application router:

| Label family | Common use                                                   |
| ------------ | ------------------------------------------------------------ |
| `H1`, `T1`   | ARINC 622 envelopes, ADS-C, CPDLC, AFN, AOC, related traffic |
| `A0`, `B0`   | AFN contact/logon                                            |
| `A9`, `B9`   | ATIS delivery or request                                     |
| `B1`         | Oceanic clearance                                            |
| `QF`, `QQ`   | OOOI events: Out, Off, On, In                                |
| `5Z`, `80`   | Airline operational control reports                          |
| `SA`         | Media advisory / link state                                  |
| `MA`         | MIAM file transfer                                           |

The `text` field may be human-readable, airline-specific, or another structured envelope.

### AVLC frame

AVLC is the VDL2 link-layer wrapper.

It includes source and destination addresses (as 24-bit ICAO addresses, the same as in ADS-B), link control, payload, and frame check sequence.

```text
VDL2 → AVLC(dst, src, control, payload, fcs)
```

AVLC tells you who is talking to whom at the VDL2 link layer: aircraft, ground stations, all-stations broadcasts, XID management frames, flow-control frames, or I-frames carrying user data.

### HFDL frame

HFDL has its own frame structure and ground-station network. It is not simply “ACARS over VHF on HF”; it has its own link behavior before any application payload is interpreted.

### ARINC 622 envelope

Many high-value ACARS messages use ARINC 622 inside ACARS text:

```text
/ATSU.IMI.REGISTRATION payload checksum
```

| Field        | Example                           | Meaning                                       |
| ------------ | --------------------------------- | --------------------------------------------- |
| ATSU address | `CZQX`, `EGGX`, `KZWY`            | Air traffic service unit or datalink facility |
| IMI          | `ADS`, `AT1`, `CR1`, `CC1`, `DR1` | Application selector                          |
| Registration | `G-EZBJ`, `A7-ANR`                | Aircraft registration                         |
| Payload      | binary/text                       | Application-specific body                     |

Typical IMIs:

| IMI   | Meaning                                 |
| ----- | --------------------------------------- |
| `ADS` | ADS-C report, request, or contract data |
| `AT1` | FANS-1/A CPDLC message body             |
| `CR1` | CPDLC connect request                   |
| `CC1` | CPDLC connect confirm                   |
| `DR1` | CPDLC disconnect request                |
| `DIS` | ADS-C disconnect/control                |

### ATN B1 over X.25

VDL2 can also carry a non-ACARS network path:

```text
VDL2 AVLC → X.25 → CLNP → COTP → ULCS → ATN B1 CPDLC
```

This is structurally very different from FANS-1/A over ARINC 622, but the application result may still be a CPDLC message such as "CONTACT EGTT 129.605".

## Applications

### ADS-C

**ADS-C** means _Automatic Dependent Surveillance – Contract_.

It is different from ADS-B, where B means _Broadcast_; ADS-C is a negotiated contract between a ground facility and an aircraft.

ADS-C reports can contain:

- position, altitude, and timestamp;
- flight id;
- next and next-next waypoint predictions;
- ground speed, track, and vertical rate;
- heading, airspeed, Mach;
- wind and temperature;
- airframe identifier;
- event or emergency report context.

### CPDLC

**CPDLC** means _Controller-Pilot Data Link Communications_.

It is structured text communication between controller and pilot.

A typical session is:

- connect request
- connect confirm
- uplink/downlink messages
- disconnect

Messages include clearances, altitude instructions, route clearances, contact instructions, WILCO/UNABLE responses, position reports, and logical acknowledgements.

CPDLC can arrive through two major paths:

| Path           | Wrapper stack                           |
| -------------- | --------------------------------------- |
| FANS-1/A CPDLC | ACARS → ARINC 622 `AT1`                 |
| ATN B1 CPDLC   | VDL2 → AVLC → X.25 → CLNP → COTP → ULCS |

Both CPDLC implementations follow slightly different standards.

### Others

Most traffic observed on datalink is not very meaningful application payload but messages for maintaining the communication channel (ping, acknowledgement).

Examples of other useful applications include:

- Airline Operational Control (AOC) messages
- ATIS requests and deliveries;
- OOOI timestamps: Out, Off, On, In;
- weather and METAR bundles;
- ETA and slash-field reports;
- maintenance and performance messages;
- link-state/media advisory messages;
- airline-specific free text or binary payloads.

These messages help enrich a trajectory with an operational timeline.
