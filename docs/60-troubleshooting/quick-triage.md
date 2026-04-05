# Quick triage

When decode quality drops or you get zero messages, follow this checklist before digging into parser code.

## Step 1: Confirm input assumptions

**File inputs**:
- Is the file format correct? (`.cu8`, `.cs16`, `.rtl`, etc.)
- Is the sample rate what you think it is? (check file metadata or recording notes)
- Is the center frequency correct?

Mismatched sample rates are a common mistake. A file recorded at 2 Msps but decoded at 1.05 Msps will produce garbage.

**Live SDR inputs**:
- Is the SDR plugged in and recognized? (`rtl_test`)
- Is the antenna connected?
- Is the frequency correct for your region?

## Step 2: Check per-channel demod counters

Before blaming the parser, check demod counters. If the demod is not detecting frames, the parser never runs.

Run the decoder with stats output (e.g., `--stats` flag) and look at:
- Frames detected per channel
- Frames with valid FCS/CRC

If all channels show zero frames:
- Wrong frequency
- Wrong sample rate
- Antenna not connected
- Gain too low or too high (clipping)

If one channel shows zero frames but others work:
- That channel may not be active in your region
- Frequency error for that specific channel

## Step 3: Reduce monitored frequency count

If you are monitoring 6+ channels and seeing sample drops or low decode rates, try reducing to 2-3 channels. If that fixes it, you are CPU or USB bandwidth limited.

## Step 4: Compare against a reference decoder

Take a known-good IQ file and decode it with both this workspace and a reference decoder (`dumpvdl2` for VDL2, `acarsdec` for classic ACARS).

If the reference decoder works and this workspace does not, file an issue with the IQ file attached (if possible).

If neither decoder works, the IQ file may be bad (corrupted, wrong format, etc.).

## Step 5: Adjust DSP/decoder constants (last resort)

Only adjust demod parameters (timing recovery, gain, thresholds) after ruling out the above. Wrong parameters can mask real problems (like antenna issues).

## Next reads

- `docs/60-troubleshooting/decoder-and-feed-issues.md` for specific failure modes
- `docs/50-operations/monitoring-metrics.md` for ongoing health tracking
