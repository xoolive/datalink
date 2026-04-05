# JAERO reference

JAERO is a satcom ACARS decoder for Inmarsat L-band. It is a reference for this workspace even though satcom is not implemented as a frontend.

## Why JAERO

JAERO decodes ACARS messages from satellite downlinks. It uses `libacars` for application-layer parsing and has app routing logic for ARINC 622 envelopes.

JAERO is relevant for:
- Understanding ARINC 622 app dispatch (how to route based on IMI)
- Seeing how `libacars` is integrated in a production decoder
- Reference for satcom-specific edge cases (if this workspace adds satcom support later)

## Current use

Satcom is out of scope for this workspace. JAERO is documented here for architectural reference and because it shares higher-layer protocols (ACARS, ARINC 622) with VHF bearers.

## Repository

`../../github/JAERO`

Key files for reference:
- `arincparse.cpp`: ARINC 622 envelope parsing and app dispatch

## Next reads

- `docs/10-modes/satcom-lband.md` for satcom context
- `docs/20-decoders/reference-policy.md` for how references are used
