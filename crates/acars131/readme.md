# acars131

Classic VHF ACARS frontend for IQ files and live SDR sources.

## Sources

`acars131` accepts jet1090-style source URLs:

```sh
acars131 rtlsdr://0?gain=40&bias_tee=true
acars131 airspy://0?sample_rate=6000000&channel=131725000
acars131 hackrf://0?center_freq=131700000&rf_amp=true&if_gain=32&bb_gain=20&channel=131525000&channel=131725000
acars131 soapy://driver=rtlsdr?gain=40
acars131 file:///data/acars.cu8?format=cu8&sample_rate=1050000&center_freq=131700000
```

Supported source schemes depend on enabled features: `rtlsdr`, `airspy`, `hackrf`, `soapy`, and `sdr`.
Default builds enable the usual SDR set.

Common query parameters:

- `center_freq` / `freq`: tuner center frequency in Hz, or with `k`, `M`, `G` suffix.
- `sample_rate` / `rate`: sample rate in Hz, or with `k`, `M`, `G` suffix.
- `channel`: ACARS channel frequency. Repeat it or use comma-separated values.
- `gain`: manual gain in dB.
- `bias_tee`: `true`, `false`, `1`, `0`, `yes`, `no`, `on`, `off`.
- HackRF-specific gains: `amp_enable`/`rf_amp`, `lna_gain`/`if_gain`, `vga_gain`/`bb_gain`.
- `format`: file IQ format (`cu8`, `cs8`, `cs16`, `cf32`).
- `name`: source name added to output JSON.

Defaults:

- center frequency: `131700000`
- sample rate: `1050000`
- channels: `131525000`, `131725000`, `131825000`
- file format: `cu8`

## Config file

Configuration is loaded from `$XDG_CONFIG_HOME/acars131/config.toml` when present.
Set `ACARS131_CONFIG=/path/to/config.toml` to override it.

Example:

```toml
stats = true
output = "~/acars131.jsonl"

[[sources]]
rtlsdr = { device = 0 }
gain = 40.0
bias_tee = false
center_freq = 131700000
sample_rate = 1050000
channels = [131525000, 131725000, 131825000]

[[sources]]
file = "~/captures/acars.cu8"
format = "cu8"
center_freq = 131700000
sample_rate = 1050000
channels = [131725000]
```

## Output

Decoded messages are emitted as JSON lines. In addition to the parsed ACARS fields,
`acars131` adds source and timing metadata:

- `bearer = "acars_vhf"`
- `source`
- `source_index`
- `center_freq_hz`
- `sample_rate`
- `frequency_hz`
- `channel_mhz`
- `sample_index`
- `seconds_into_recording`
- `timestamp_unix`
- `raw_frame_hex`

Use `--output path.jsonl` to also write a JSONL copy to disk.

## Hardware checks

Before running live SDR sources, verify devices with the vendor tools:

- RTL-SDR: `rtl_test -t` or `rtl_eeprom`
- Airspy: `airspy_info`
- HackRF: `hackrf_info`
- SoapySDR: `SoapySDRUtil --find` or `SoapySDRUtil --probe`
