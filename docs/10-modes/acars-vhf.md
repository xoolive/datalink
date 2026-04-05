# Classic VHF ACARS (POA)

Classic VHF ACARS (also called "Plain Old ACARS" or POA) is the original ACARS bearer. It uses MSK (Minimum Shift Keying) modulation at 2400 baud on VHF channels between 129 and 131 MHz.

## Frequencies

Common channels (regional usage varies):

- **131.525 MHz**
- **131.550 MHz**
- **131.725 MHz**
- **131.825 MHz**
- **129.125 MHz**
- **130.025 MHz**
- **130.425 MHz**
- **130.450 MHz**
- **131.125 MHz**

Initial `acars131` frontend targets 131.525, 131.725, 131.825 MHz. Check the airframes.io frequency reports for your region to see which channels are active.

Not all channels are in use everywhere. In some regions, only 2-3 channels see traffic. In others, 6+ channels are active. A wide-bandwidth SDR (2 MHz) can cover 3-4 channels simultaneously.

## Modulation

MSK at 2400 baud, continuous (not bursted like VDL2). The signal is always on when a message is being sent, no time-slotting.

The demod pipeline:

1. Downmix to baseband (channel center to 0 Hz)
2. Low-pass filter and decimate to ~12.5 kHz
3. MSK symbol recovery (track phase, slice symbols)
4. ACARS frame sync (preamble detection and byte alignment)
5. Extract frame, validate CRC

Implementation: `crates/acars/src/demod/acars131.rs`

## Frame structure

Classic ACARS frames are simpler than VDL2 AVLC frames. No link-layer envelope, just:

1. Preamble (alternating bit pattern for sync)
2. Sync word (marks start of frame)
3. ACARS message (mode, aircraft reg, label, block ID, message text)
4. CRC (parity check)

The frame format is the same as ACARS-over-VDL2, but without the AVLC wrapper.

## Validation status

The `acars131` frontend was tested against `acarsdec` using a multi-channel IQ capture (2 MHz sample rate, 83 seconds, 3 channels). Both tools decoded 7 messages from the same file. Message text matched except for minor control-character differences (SOH/STX handling).

This is an initial validation. More test captures and edge-case comparison are needed.

## Parity reference

- `acarsdec`: primary classic ACARS reference (multi-channel demod, mature codebase)

`acarsdec` has been in production use for years and is considered the baseline for classic ACARS behavior.

## Implementation status

**Frontend**: `acars131` (file inputs, initial)
**Demod**: `crates/acars/src/demod/acars131.rs` (MSK demod + frame sync)
**ACARS parse**: shared with VDL2 (`crates/acars/src/decode/acars.rs`)

The demod is working but has not been tested on a wide range of captures. Edge cases (weak signals, frequency drift, multi-path) are still being validated.

## Next reads

- `docs/00-overview/architecture.md` for layer comparison (VHF ACARS vs VDL2)
- `docs/20-decoders/acarsdec.md` for reference tool notes
- `docs/10-modes/vdl2.md` for comparison with VDL2
