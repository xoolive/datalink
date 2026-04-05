# acarsdec reference

`acarsdec` is the primary classic VHF ACARS reference for this workspace. It has been in production use for years and is considered the baseline for classic ACARS demod and operations.

## Why acarsdec

Classic VHF ACARS uses MSK modulation. Demod edge cases include frequency drift, weak signal handling, and multi-path interference. The ACARS frame sync and CRC validation need to be robust against these conditions.

`acarsdec` has been tested on thousands of hours of real-world captures and is the operational standard for classic ACARS decoding.

## What we compare

- **Demod behavior**: MSK symbol recovery, frame sync, weak signal handling
- **Frame extraction**: preamble detection, sync word alignment, CRC validation
- **Multi-channel handling**: simultaneous decode on multiple frequencies
- **Message output**: ACARS frame parse (mode, reg, label, text)

## Validation example

The `acars131` frontend was validated against `acarsdec` using a 2 MHz, 83-second IQ capture with 3 active channels (131.525, 131.725, 131.825 MHz). Both tools decoded 7 messages. Message text matched except for minor control-character differences (SOH/STX byte handling).

This is initial validation. More captures and edge-case testing are needed.

## Input format note

`acarsdec -f` (WAV file input) expects 12.5 kHz demodulated channel streams, not arbitrary SDR IQ wrapper files. If you have a raw IQ capture at 2 MHz, you need to convert it to raw I/Q format (e.g., `.cs16`) for `acars131`, or preprocess it to 12.5 kHz channel streams for `acarsdec`.

## Repository

`../../github/acarsdec`

Key files for reference:
- `acars.c`: ACARS frame sync and parse
- `msk.c`: MSK demod
- `rtl.c`, `airspy.c`, `sdrplay.c`: SDR interfaces (not directly comparable, different frontend architecture)

## Next reads

- `docs/10-modes/acars-vhf.md` for classic ACARS bearer details
- `docs/20-decoders/reference-policy.md` for how references are used
