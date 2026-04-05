# Iridium (context)

Iridium is a low-earth-orbit (LEO) satellite system that carries some aviation datalink traffic. It is out of scope for this workspace but relevant as an operations reference.

## Why Iridium is different

Iridium ACARS uses different framing and modulation than VHF ACARS, VDL2, HFDL, or Inmarsat L-band. The decode tools and protocols are distinct.

Iridium is documented in this workspace for two reasons:

1. **Multi-channel operations**: Iridium decode setups often monitor 10+ beams/channels simultaneously. This is a useful reference for multi-channel station design and monitoring (metrics, logging, fault detection).

2. **Protocol awareness**: Some aircraft send ACARS over both Iridium and VHF/VDL2/satcom. Understanding the full bearer landscape helps when analyzing multi-source message timelines.

## Reference

- `thebaldgeek.github.io/Iridium.md`: operations guide and decoder setup

This page exists to clarify scope. Iridium is not a planned frontend for this workspace.
