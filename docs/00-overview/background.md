# Background and context

Aviation datalink is a collection of radio systems for sending data between aircraft and ground. This page explains the problem space and where this software fits.

## The problem: layers, not a single protocol

Air-ground digital communications are not one thing. You have:

- **Bearer**: how bits move (VHF radio, HF radio, satellite)
- **Link/network**: how frames/packets are structured (AVLC, X.25)
- **Message transport**: how messages are packaged (ACARS framing)
- **Application**: what the message means (position reports, clearances, weather)

The same application can run over different bearers. A position report can come via VHF ACARS, VDL2, or satellite. The same bearer can carry different applications. VDL2 carries both simple text messages and complex ATC clearances.

This workspace decodes across all these layers, from raw radio samples to parsed application payloads.

## Bearers in this workspace

### VDL Mode 2 (VHF, 136 MHz band)

Digital VHF datalink used for ATS and AOC traffic. Common channels are 136.975 MHz (the "common signalling channel") and regional channels like 136.725, 136.775, 136.875 MHz.

Frontend: `vdl136` (file and SDR inputs)
Demod: shared in `crates/acars/src/demod/vdl2.rs`

VDL2 is the most mature path in this workspace. The demod handles D8PSK bursts, and the decode stack parses AVLC frames, dispatches ACARS or X.25 payloads, and extracts applications like ADS-C and CPDLC.

### Classic VHF ACARS (POA, 129-131 MHz)

Older ACARS-over-VHF system, still in wide use. Initial target channels: 131.525, 131.725, 131.825 MHz (regional usage varies).

Frontend: `acars131` (initial implementation)
Demod: `crates/acars/src/demod/acars131.rs`

The demod uses MSK demodulation and ACARS frame sync. Early validation shows parity with `acarsdec` on test captures (7/7 messages decoded from a multi-channel 83-second IQ file).

### HFDL (HF, variable frequencies)

Long-range HF bearer for oceanic and remote areas. Uses different modulation and channel management than VHF.

Status: not implemented as a frontend in this workspace. `dumphfdl` is the reference for HFDL bearer behavior. Higher-layer ACARS decode logic can be reused where the framing matches.

## Application-layer protocols: ACARS and ARINC 622

These terms get mixed up because they are at different layers.

**ACARS** is the message transport layer. It defines how messages are framed, labeled, and transmitted. An ACARS message has a mode, registration, label, block ID, message text, and CRC. ACARS runs over multiple bearers: classic VHF ACARS, VDL2, HFDL, and satellite.

**ARINC 622** is an application-level envelope used for ATS messages. It rides inside ACARS message text. An ARINC 622 payload has a format like `/<ATSU>.ADS.<REG><data><crc>`. The three-letter code after the first dot identifies the application (ADS for position reports, AT1/CC1/DR1 for CPDLC variants).

**ADS-C** is a surveillance application. Aircraft send position reports on a contract basis (periodic, event-driven, or demand). The payload is binary tag-length-value structures inside the ARINC 622 envelope.

**CPDLC** is a controller-pilot communications application. It supports things like clearance delivery, altitude requests, and route changes. CPDLC can appear in ARINC 622 envelopes over ACARS, or in X.25 payloads on VDL2.

## VDL2 internals: why X.25 and CLNP matter

VDL2 is more complex than classic ACARS. A VDL2 burst contains an AVLC frame (similar to HDLC). The frame can be:

- **I-frame** (information): carries payload data
- **S-frame** (supervisory): flow control, no payload
- **U-frame** (unnumbered): control functions like XID

I-frame payload dispatch depends on the first bytes:

- `FF FF 01`: ACARS marker, followed by an ACARS message
- Other: typically X.25 packet data

X.25 payloads can carry CLNP (Connectionless Network Protocol) and COTP (Connection-Oriented Transport Protocol) layers, which then carry application data like CPDLC.

This is why VDL2 decode tools need both ACARS parsers and X.25/CLNP/COTP parsers. A typical decode path:

```
VDL2 RF burst
  -> AVLC frame (CRC validated)
  -> I-frame payload
  -> X.25 packet
  -> CLNP/COTP
  -> CPDLC uplink message
```

The workspace handles this in `crates/acars/src/decode/avlc.rs` (frame parse), `decode/x25.rs` (X.25/CLNP/COTP), and `decode/acars.rs` (ACARS messages).

## Why reference implementations matter

Datalink protocols have many edge cases: fragmentation, direction-dependent interpretation, bit-level framing details, label and IMI routing rules. Specifications exist but are incomplete or ambiguous in places.

The workspace uses known-working decoders as behavioral references:

- `dumpvdl2`: VDL2 bearer and link-layer behavior (AVLC, XID, X.25 dispatch)
- `libacars`: application-layer semantics (ACARS, ADS-C, CPDLC, MIAM, OHMA, Media Advisory)
- `acarsdec`: classic VHF ACARS demod and operations
- `dumphfdl`: HFDL bearer behavior (future reference)

The goal is not to port these tools directly. The goal is to match their behavior where it matters (frame decode, application parse) while building a Rust-native, typed decoding stack with consistent output.

For example, `acars131` was validated against `acarsdec` using a multi-channel IQ capture. Both tools decoded 7 messages from the same file. The message text matched except for minor control-character differences.

## Current implementation status

**Strong** (working, tested against references):
- VDL2 demod (`vdl136` frontend)
- AVLC frame parsing and CRC validation
- ACARS message parsing (mode, reg, label, block, text, CRC)
- ADS-C tag decode (18 tag types implemented)

**Partial** (working but incomplete):
- CPDLC decode (X.25/CLNP/COTP path extracts free text, structured decode pending)
- ARINC 622 app routing (ADS dispatch works, full IMI routing in progress)
- X.25/COTP reassembly (basic parse works, multi-fragment reassembly partial)

**Initial** (implemented, validation in progress):
- Classic VHF ACARS demod (`acars131` frontend)
- MSK demodulation and frame sync for 131 MHz channels

**Planned** (not yet started):
- MIAM, Media Advisory, OHMA native parsers (currently pass through as text)
- HFDL frontend (bearer demod and framing)

See `docs/00-overview/architecture.md` for coverage tables and layer diagrams. See `plan.md` for implementation roadmap.

## Design direction

This workspace focuses on decoding, not on building an end-to-end datalink operations platform.

Specific goals:

1. **Keep demod in-house**. All frontends (VDL2, classic ACARS, future HFDL) implement their own demodulation. We use reference decoders for validation, not as runtime dependencies.

2. **Share decode logic**. All frontends feed into `crates/acars` decode library. ACARS parsing, ADS-C parsing, X.25 handling are shared.

3. **Consistent output schema**. All frontends produce JSON with the same structure for bearer metadata (frequency, timestamp, channel) and decoded payloads. Downstream tools should not need bearer-specific logic.

4. **Incremental app coverage**. Add ARINC 622 applications (MIAM, OHMA, Media Advisory) without breaking existing JSON consumers. Unknown applications stay as raw text.

This is a software framework, not a hardware guide. The docs explain protocols and decode paths. For SDR setup, antenna selection, and station operations, see the external references (`thebaldgeek.github.io`, `airframesio-docs`).

Next: `docs/00-overview/architecture.md` for layer diagrams and coverage tables.
