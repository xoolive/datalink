# vdlm2dec reference

`vdlm2dec` is a VDL2 decoder from the same author as `acarsdec`. It is a secondary VDL2 reference for this workspace.

## Why vdlm2dec

`vdlm2dec` has a simpler codebase than `dumpvdl2` and is useful for understanding VDL2 demod and AVLC framing without the full X.25/CLNP/COTP complexity.

It decodes up to 8 VDL2 channels simultaneously and outputs ACARS messages. It uses `libacars` for application-layer decode (same as `dumpvdl2`).

## What we compare

- **VDL2 demod**: D8PSK symbol recovery, burst detection, training sequence handling
- **AVLC frame parse**: I/S/U frame structure, FCS validation
- **Multi-channel handling**: simultaneous decode on multiple frequencies

## What we don't compare

- X.25 reassembly (vdlm2dec does not implement this; use dumpvdl2 for X.25 reference)
- JSON output schema (different from this workspace)

## Repository

`../../github/vdlm2dec`

## Next reads

- `docs/20-decoders/dumpvdl2.md` for primary VDL2 reference
- `docs/10-modes/vdl2.md` for VDL2 bearer details
