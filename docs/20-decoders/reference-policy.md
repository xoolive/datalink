# Reference policy

External projects are used for parity and workflow alignment, not copied architecture.

## Current reference roles

- `dumpvdl2`: primary VDL2 bearer behavior reference
- `vdlm2dec`: secondary VDL2 operational behavior reference
- `acarsdec`: primary classic VHF ACARS operational reference
- `dumphfdl`: primary HFDL bearer reference
- `libacars`: app-layer semantics reference (ADS-C/CPDLC/MIAM/OHMA/etc)
- `JAERO`: SATCOM workflow/routing reference

## Rules

1. Keep demod frontends in this workspace.
2. Use references for expected behavior and fixture extraction.
3. Preserve stable JSON conventions unless a deliberate schema change is documented.
