# Configuration

Merged receiver mode runs when `datalink` starts without a bearer subcommand:

```sh
datalink --config datalink.toml
```

The file is TOML with `[output]`, one or more `[[sources]]`, and optional `[[sources.receivers]]` for I/Q sources.

## Sample rate values

The sample rate basically determines the bandwidth and the number of channels you can listen to.

Any reasonable sample rate value should work with `datalink` you may prefer:

- a multiple of 12.5 kHz for classic VHF ACARS
- a multiple of 105 kHz for VDL2

## Complete example

```toml
[output]
jsonl = "-"
redis_url = "redis://localhost:6379"

[[sources]]
id = "wideband-vhf-vdl2"
name = "8 MHz VHF/VDL2 capture"
file = "~/captures/gqrx_20260603_083011_134000000_8400000_fc.raw"
format = "cf32"
center_freq = 134_000_000
sample_rate = 8_400_000

[[sources.receivers]]
bearer = "vhf"
channels = [131_525_000, 131_725_000, 131_825_000]

[[sources.receivers]]
bearer = "vdl2"
channels = [136_675_000, 136_775_000, 136_875_000, 136_975_000]

[[sources]]
id = "airframes-live"
name = "Airframes.io live feed"
websocket = "airframes://"
```

## Output

| Key         | Meaning                                        |
| ----------- | ---------------------------------------------- |
| `jsonl`     | `-` for stdout or a path for JSON Lines output |
| `redis_url` | Optional Redis pub/sub URL                     |

## Sources

I/Q sources can be files, stdin, or SDR devices. They need a center frequency and sample rate unless these can be inferred.

```toml
[[sources]]
id = "hackrf-vhf-vdl2"
name = "HackRF wideband"
hackrf = { device = 0 }
center_freq = 133_062_500
sample_rate = 8_400_000
rf_gain = true
lna_gain = 24
vga_gain = 26
```

Event sources currently use `websocket` and do not contain receiver blocks.

## Receivers

Receivers attach protocol-specific demodulators to an I/Q source.

```toml
[[sources.receivers]]
bearer = "hfdl"
channels = [8_912_000, 10_081_000, 11_387_000]
```

Valid I/Q receiver bearers are `vhf`, `vdl2`, and `hfdl`.

If channels are omitted, `datalink` selects known channels that fit inside the configured source bandwidth.

## Validation notes

- Source ids must be unique and non-empty.
- I/Q sources must have at least one receiver.
- Event sources must not have receiver blocks.
- Channels must fit within the source bandwidth.
- Use explicit `format`, `center_freq`, and `sample_rate` for raw captures when filename inference is not reliable.
