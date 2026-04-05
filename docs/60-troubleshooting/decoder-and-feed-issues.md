# Decoder and feed issues

This page covers common failure modes and how to fix them.

## No messages decoded

**Checklist**:

1. **Input format/rate/frequency**: Triple-check these. A 2 Msps file decoded at 1.05 Msps produces noise.
2. **Channel list**: Are you monitoring the right channels for your region? Use airframes.io frequency reports as a guide.
3. **Gain**: Too low = weak signal, too high = clipping/overload. Try gain 30-40 dB as a starting point for RTL-SDR.
4. **Antenna**: Is it connected? Is it for the right frequency band? (ADSB antennas do not work for VHF ACARS.)

**Test with a known-good file**: Get a sample IQ file from someone else who is decoding successfully. If that works, your setup is fine and the problem is local signal/antenna.

## Low message count (but not zero)

**Check per-channel yield**:
- Use `--stats` or log parsing to see messages per channel per hour
- Drop channels with <10 messages/day unless you have a specific reason to monitor them

**Antenna placement**:
- VHF is line-of-sight. Higher is better.
- Indoors = significant signal loss
- Near metal roofs or walls = reflections and multi-path

**Cable loss**:
- VHF (130-136 MHz) has much lower coax loss than ADSB (1090 MHz), but long runs still matter
- Use RG-8x or better for runs >20 feet

**Time of day**:
- Aircraft traffic varies by time. Early morning and overnight see lower traffic in most regions.
- HFDL (future) has day/night propagation differences.

## Feed/output mismatches (when feeding aggregators)

**Station ID consistency**:
- Use the same station ID every time. Do not use dynamic IDs based on hostname or timestamp.

**Timestamp policy**:
- Use UTC, not local time
- Use epoch seconds or ISO 8601 strings, be consistent

**JSON schema**:
- Check the aggregator docs for required fields
- Some aggregators expect specific field names (`frequency` vs `frequency_hz`)

## Multi-SDR instability

**SDR serial numbers**:
- Set unique serial numbers with `rtl_eeprom`
- Label the physical SDRs so you know which is which

**USB power**:
- Use a powered USB hub if you are running 3+ SDRs
- Do not chain unpowered hubs

**Device selection**:
- Use serial numbers (`-s 136`) instead of device indexes (`-d 0`) where possible
- Device indexes can change if you replug SDRs

## Parser errors (rare)

If you are seeing parse errors on frames with valid CRC, this is a bug. Save the IQ file (or hex frame) and file an issue.

**Workaround**: Use the `datalink` CLI to decode the frame hex directly and see what the error is.

Example:
```bash
# Decode an AVLC frame from hex
cargo run --release --bin datalink -- avlc "03A1B677..."

# Decode an ACARS message from hex
cargo run --release --bin datalink -- acars "02..." --direction downlink
```

## Next reads

- `docs/60-troubleshooting/quick-triage.md` for diagnostic workflow
- `docs/50-operations/monitoring-metrics.md` for ongoing tracking
- `docs/10-modes/receiver-systems.md` for channel and SDR planning
