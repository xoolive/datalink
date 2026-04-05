# Getting started (project-local)

This page is intentionally focused on this repository, not external feeder ecosystems.

## 1) Pick your starting mode

- Start with `vdl136` if you have VDL2 VHF I/Q captures.
- Use `acars131` for classic VHF ACARS captures (initial implementation, still being validated).
- Use `datalink` CLI when you already have demodulated payload/frame data.

## 2) Understand component roles

- `acars`: shared demod/decoder core
- `vdl136`: VDL2 frontend (I/Q -> decoded frames/messages)
- `acars131`: classic ACARS frontend (I/Q -> decoded frames/messages)
- `datalink`: payload decoder for direct frame/app parsing

## 3) Run a first decode

VDL2 example:

```bash
cargo run --release --bin vdl136 -- file --file sample.rtl --center-freq 136850000 --sample-rate 1050000
```

Classic ACARS example:

```bash
cargo run --release --bin acars131 -- --file sample.cs16 --format cs16 --center-freq 129535000 --sample-rate 2000000 --channel 129125000 129550000 130025000 --stats
```

Payload decode example:

```bash
cargo run --release --bin datalink -- adsc "/BOMASAI.ADS.VT-ANB072501A070A988CA73248F0E5DC10200000F5EE1ABC000102B885E0A19F5"
```

## 4) Validate against references

- VDL2 parity target: `dumpvdl2` / `vdlm2dec`
- VHF ACARS parity target: `acarsdec`
- App semantics reference: `libacars`

Keep external tools as references; keep demod implementations in-house.
