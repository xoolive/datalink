# VDL2 mode notes

VDL Mode 2 is a digital VHF datalink used for ATS (air traffic services) and AOC (airline operational control) communications. It uses D8PSK modulation and operates in the 136-137 MHz band.

## Frequencies

Common VDL2 channels:

- **136.975 MHz**: CSC (common signalling channel), monitored globally
- **136.725, 136.775, 136.875 MHz**: regional channels, usage varies

Regional usage depends on airspace and service provider (ARINC, SITA). The CSC (136.975) sees the most traffic and is a good starting point.

## Modulation and burst structure

VDL2 uses D8PSK (Differential 8-Phase Shift Keying) at 10.5 kbaud symbol rate. Transmissions are time-slotted bursts, not continuous. A typical burst:

1. Ramp-up (power stabilization)
2. Training sequence (known pattern for sync and channel estimation)
3. Payload (AVLC frame with bit interleaving and convolutional FEC)
4. Ramp-down

The demod in `crates/acars/src/demod/vdl2.rs` detects bursts by power threshold, locks symbol timing on the training sequence, and decodes the payload bits.

## Link layer: AVLC frames

VDL2 uses AVLC (Aviation VHF Link Control), which is similar to HDLC. Frame types:

- **I-frame** (Information): carries data payload. Can be ACARS or X.25.
- **S-frame** (Supervisory): flow control, acknowledgments. No user payload.
- **U-frame** (Unnumbered): control functions like XID (station identification/parameters).

Each AVLC frame has an FCS (frame check sequence, 16-bit CRC). Frames with bad FCS are discarded.

I-frame payload dispatch:
- If payload starts with `FF FF 01`: ACARS message follows
- Otherwise: assume X.25 packet data

## Payload types

**ACARS over VDL2**: ACARS message format is the same as classic VHF ACARS. The difference is the bearer (D8PSK burst vs MSK continuous) and link layer (AVLC vs direct ACARS framing).

**X.25 over VDL2**: X.25 is a packet-switching protocol. VDL2 X.25 payloads often carry CLNP (ISO connectionless network protocol) and COTP (connection-oriented transport protocol), which in turn carry application data like CPDLC.

## Parity references

- `dumpvdl2`: primary VDL2 reference (AVLC parsing, X.25 dispatch, multi-channel handling)
- `vdlm2dec`: secondary reference (same ecosystem as `acarsdec`, simpler codebase)

The workspace VDL2 demod was validated against these tools using multi-channel captures. Decode counts and frame structure match within expected variance (timing/SNR differences).

## Implementation status

**Frontend**: `vdl136` (file and SDR inputs)
**Demod**: `crates/acars/src/demod/vdl2.rs` (shared across frontends)
**AVLC parse**: `crates/acars/src/decode/avlc.rs`
**X.25 parse**: `crates/acars/src/decode/x25.rs`

Tested with IQ captures at 1.05 Msps (covers ~2 MHz, enough for 4-6 VDL2 channels). Performance on a 3-channel 100-second capture: ~17.8 seconds decode time, ~500 frames decoded.

## Next reads

- `docs/00-overview/architecture.md` for layer diagrams
- `docs/20-decoders/dumpvdl2.md` for reference tool notes
- `docs/40-pipelines/decoding-pipeline.md` for end-to-end flow
