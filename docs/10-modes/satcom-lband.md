# SATCOM L-band (context)

SATCOM ACARS uses satellite links (Inmarsat L-band, Iridium) for air-ground data. This is operationally important but not implemented in this workspace.

## L-band (Inmarsat)

Inmarsat satellites provide L-band downlinks around 1.5 GHz. Ground receivers can monitor ACARS messages sent from ground to aircraft (uplink direction as seen from the satellite).

Typical messages:
- ATC clearances
- Weather uplinks
- Dispatch messages
- Maintenance queries

L-band requires:
- L-band antenna (patch or helix)
- L-band LNA (low-noise amplifier)
- SDR capable of 1.5 GHz (RTL-SDR works)
- Decoder software (JAERO is the primary tool)

## Why this is documented here

SATCOM ACARS uses the same higher-layer protocols (ACARS messages, ARINC 622 applications) as VHF ACARS and VDL2. Understanding the bearer landscape helps when planning decode logic and output schemas.

If this workspace adds SATCOM support in the future, the ACARS/ADS-C/CPDLC parsers in `crates/acars/src/decode/` should work without changes.

## Reference tools

- `JAERO`: primary L-band SATCOM ACARS decoder
- `thebaldgeek.github.io/L-Band.md`: hardware and operations guide

## Next reads

- `docs/00-overview/architecture.md` for layer diagrams (bearer vs application)
- `docs/20-decoders/jaero.md` for JAERO reference notes
