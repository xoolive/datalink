# Receiver system planning

This page covers receiver layout decisions for this workspace. The focus is on software and channel strategy, not hardware selection or antenna details (see external references for that).

## Start simple

**One SDR, VDL2 only**

Run `vdl136` with 3-4 channels around 136 MHz (e.g., 136.725, 136.775, 136.875, 136.975). This is the easiest entry point and gives you ATS/AOC traffic immediately.

Sample rate: 1.05 MHz covers ~2 MHz bandwidth, enough for 4-6 VDL2 channels. Higher sample rates are not needed unless you are monitoring more channels or widely-spaced frequencies.

**One SDR, classic VHF ACARS only**

Run `acars131` with 2-3 channels around 131 MHz (e.g., 131.525, 131.725, 131.825). This works if your region has active classic ACARS traffic.

Check the airframes.io frequency reports for your area before committing to a channel set. Not all regions use all channels.

## Scaling up

**Two SDRs: VDL2 + classic ACARS**

Run `vdl136` on one SDR (136 MHz band) and `acars131` on another (131 MHz band). This captures both bearer types without channel conflicts.

You need:
- Two SDRs with different serial numbers (use `rtl_eeprom` to set them)
- Antenna splitter or separate antennas
- Enough CPU to run both decoders (Pi 4 or x86 low-power box works)

**Three+ SDRs: More channels or frequency groups**

If your region has heavy VDL2 traffic on channels outside the 2 MHz span (e.g., 136.725 and 136.975 are 250 kHz apart, but you also want to monitor 136.650 or 136.800), add another SDR.

Same for classic ACARS: if 129.125, 130.025, 131.525, and 131.825 are all active, you may need two SDRs to cover the full spread.

## Channel selection strategy

**Don't add channels blindly**

More channels use more CPU and increase the chance of missing frames due to processing lag or sample drops. Add channels based on observed traffic, not assumptions.

**Monitor per-channel yield**

Track how many frames/messages you get per channel per hour. If a channel produces <10 messages/day, consider dropping it unless it is critical for your use case (e.g., monitoring a specific ATC region).

**Check regional reports**

The airframes.io frequency stats reports show which channels are active in different regions. Use these as a starting point, then adjust based on your local observations.

## CPU and sample rate considerations

VDL2 demod is more CPU-intensive than classic ACARS demod (D8PSK vs MSK, tighter timing requirements). On a Pi 4:

- 3-channel VDL2 at 1.05 Msps: ~60-80% CPU (single core)
- 3-channel classic ACARS at 2 Msps: ~40-60% CPU (single core)

If you are dropping samples or seeing decode gaps, reduce the number of channels or lower the sample rate (if coverage allows).

## SDR serial numbers

If you run multiple SDRs, set unique serial numbers. The default serial (`00000001` or similar) makes it hard to assign specific SDRs to specific decoders.

```bash
rtl_eeprom -d 0 -s 136    # VDL2 SDR
rtl_eeprom -d 1 -s 131    # Classic ACARS SDR
```

Label the physical SDRs so you know which is which when you replug them.

## HFDL

HFDL is a separate track. It requires different SDRs (HF-capable: Airspy, SDRPlay, not RTL-SDR), different antennas (HF long-wire or loop), and different decode software (`dumphfdl`).

HFDL is out of scope for this workspace at the moment. See `docs/10-modes/hfdl.md` for context.

## Next reads

- `docs/10-modes/vdl2.md` for VDL2 channel and demod details
- `docs/10-modes/acars-vhf.md` for classic ACARS channels
- External references (`thebaldgeek.github.io`, `airframesio-docs`) for SDR and antenna selection
