# HFDL (High Frequency Data Link)

HFDL is a long-range datalink bearer using HF radio bands. It provides coverage in oceanic and remote regions where VHF range is insufficient.

## Why HF

VHF signals are line-of-sight. At cruise altitude (~40,000 ft), VHF range is roughly 200-250 nautical miles. This works over land where ground stations are dense, but not over oceans.

HF signals can propagate thousands of miles via ionospheric reflection (skywave propagation). HFDL uses HF for long-range data, covering areas like the North Atlantic, Pacific, and polar routes.

## Frequency bands

HFDL uses multiple HF frequency bands, with ground stations transmitting on different frequencies depending on time of day and ionospheric conditions. Typical bands:

- 2-6 MHz (low HF)
- 8-12 MHz (mid HF)
- 17-22 MHz (high HF)

Specific frequencies are assigned to HFDL ground stations. For example:
- San Francisco: 6559, 8927, 13276, 17919 kHz
- Reykjavik: 5508, 8834, 13321, 17919 kHz
- Canaries: 5652, 8939, 13312, 17928 kHz

Ground stations shift frequencies based on propagation forecasts (day/night, season, solar activity).

## Modulation and framing

HFDL uses PSK modulation (QPSK or 8PSK depending on mode) with interleaving and FEC to handle HF channel conditions (fading, interference, multi-path).

Frame structure includes:
- Downlink frames (ground station to aircraft)
- Uplink frames (aircraft to ground station)
- Squitters (periodic beacons from ground stations)

HFDL has its own link-layer protocol (not AVLC). Higher layers can carry ACARS messages and applications (ADS-C, CPDLC, AOC).

## Ground station network

HFDL is operated by ARINC. The global network has ~14 ground stations covering oceanic and remote areas:

- North Atlantic: Reykjavik, Shannon, New York
- Pacific: San Francisco, Molokai, Auck land
- Indian Ocean: Al Muharraq, Agana
- South Atlantic: Canaries, Johannesburg
- Polar: Hat Yai, others

Aircraft select the best ground station based on signal strength and availability.

## Payload types

HFDL carries ACARS-family traffic. The message formats are the same as VHF ACARS and VDL2 ACARS, but the bearer-level framing is different.

Common applications over HFDL:
- Position reports (ADS-C)
- Oceanic clearances (CPDLC)
- AOC messages (flight plans, weather, maintenance)
- Voice call setup (for HF voice circuits)

## Why HFDL is not implemented here (yet)

HFDL requires:
- Different demod (PSK variants, adaptive to HF propagation)
- Different framing (HFDL link layer, not AVLC)
- Different channel management (frequency prediction, multi-station selection)

This workspace focuses on VHF bearers (VDL2, classic ACARS). HFDL is a separate frontend track. Higher-layer decode logic (ACARS parsing, ADS-C, CPDLC) can be reused where the framing matches.

## Reference implementation

`dumphfdl` is the primary HFDL reference decoder. It supports:
- SoapySDR, Airspy, SDRPlay inputs
- Multi-frequency monitoring
- Systable (ground station frequency schedule) parsing
- ACARS and ARINC 622 application decode (via libacars)

See `docs/20-decoders/dumphfdl.md` for more.

## Context for this workspace

HFDL is documented here for architectural context. Applications like ADS-C and CPDLC can appear over HFDL, VDL2, or VHF ACARS. Understanding the bearer differences helps when planning decode logic reuse.

If/when an HFDL frontend is added to this workspace, the higher-layer parsers in `crates/acars/src/decode/` should work without changes.
