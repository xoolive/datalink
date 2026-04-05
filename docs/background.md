# Background and context

This page provides protocol and operational context for the `datalink` workspace without going into code-level implementation details.

## 1) The datalink problem space

Air-ground digital communications are not a single protocol. They are a stack of:

- bearer technologies (how bits move over radio/satellite),
- link/network transports (how frames/packets are carried),
- message/application protocols (what operational meaning the message has).

In practice, the same application can appear over different bearers, and the same bearer can carry different applications.

## 2) Bearers in scope and near-scope

### VDL Mode 2 (VHF)

- Digital VHF datalink bearer used widely for ATS/AOC traffic.
- Commonly monitored channels include 136.975 MHz (CSC) and, regionally, 136.725/136.775/136.875 MHz.
- In this project, this is the current demodulated bearer path via the `vdl136` frontend.

### Classic VHF ACARS (POA)

- Earlier ACARS-over-VHF bearer family (still widely used operationally).
- Initial target channels for this frontend are `131.525`, `131.725`, and `131.825` MHz.
- Planned as the `acars131` in-house frontend.

## 3) ACARS, ARINC 622, CPDLC, ADS-C: how they relate

These are often confused because they are at different layers.

- **ACARS**: message transport/envelope family (labels, block framing, text body).
- **ARINC 622**: application envelope conventions carried on ACARS for ATS applications.
- **CPDLC**: controller-pilot communications application protocol.
- **ADS-C**: surveillance-contract reporting application protocol.

So ARINC 622 is not a replacement for ACARS; it is an app-level framing/routing convention layered over ACARS transport.

## 4) VDL2 internal protocol chain (why AVLC/X.25 matter)

When traffic is on VDL2, a typical decode path can be:

```text
VDL2 RF burst
  -> AVLC frame
  -> payload dispatch (ACARS marker or X.25)
  -> X.25 / CLNP / COTP (for some ATN paths)
  -> app payload (e.g. CPDLC)
```

This is why VDL2 tooling needs both:

- ACARS-aware parsing, and
- X.25/CLNP/COTP-aware parsing.

## 5) Why parity references are needed

Air-ground datalink decoding has many edge cases (fragmentation, bit-level framing details, direction-sensitive interpretation, label/IMI routing nuances).

The project uses known references to reduce ambiguity:

- `dumpvdl2` for VDL2 bearer/link behavior,
- `libacars` for app-layer semantics and broad application coverage,
- `acarsdec` / JAERO for operational workflows and additional vectors.

The goal is behavioral parity where needed, while keeping a Rust-native architecture and stable output conventions.

## 6) Current maturity snapshot (conceptual)

- Strong: VDL2 demod path (`vdl136`), AVLC parsing, ACARS parsing, ADS-C decode core.
- Partial: CPDLC structured depth, full ARINC 622 app routing in all paths, X.25/COTP reassembly parity.
- Planned: MIAM, Media Advisory, OHMA native decode modules; POA VHF in-house frontend (`acars131`).

## 7) Design direction

The project direction is:

1. Keep bearer demodulation in-house across all frontends.
2. Feed all frontends into the shared `acars` decode core.
3. Keep outputs consistent across bearers via a normalized metadata contract.
4. Expand app coverage incrementally without breaking stable JSON consumers.

For layer diagrams and coverage tables, see `docs/architecture.md`.
For end-to-end runtime flow, see `docs/decoding-pipeline.md`.
For concrete implementation sequencing, see `plan.md`.
