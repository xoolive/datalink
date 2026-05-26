# vdl136

VDL Mode 2 frontend for IQ files and live SDR sources.

## Sources

`vdl136` accepts jet1090-style source URLs:

```sh
vdl136 rtlsdr://0?gain=40&bias_tee=true
vdl136 airspy://0?sample_rate=6000000&channel=136875000
vdl136 hackrf://0?center_freq=136850000&rf_amp=true&if_gain=32&bb_gain=20&channel=136875000&channel=136975000
vdl136 soapy://driver=rtlsdr?gain=40
vdl136 file:///data/vdl2.cu8?format=cu8&sample_rate=1050000&center_freq=136850000
vdl136 'airframes://live?event=message'
vdl136 'wss://ws.airframes.io/socket.io/?EIO=4&transport=websocket'
```

The legacy file form still works:

```sh
vdl136 file --file /data/vdl2.cu8 --center-freq 136850000 --channel 136875000 136975000
```

Common query parameters:

- `center_freq` / `freq`: tuner center frequency in Hz, or with `k`, `M`, `G` suffix.
- `sample_rate` / `rate`: sample rate in Hz, or with `k`, `M`, `G` suffix.
- `channel`: VDL2 channel frequency. Repeat it or use comma-separated values.
- `gain`: manual gain in dB.
- `bias_tee`: `true`, `false`, `1`, `0`, `yes`, `no`, `on`, `off`.
- HackRF-specific gains: `amp_enable`/`rf_amp`, `lna_gain`/`if_gain`, `vga_gain`/`bb_gain`.
- `format`: file IQ format (`cu8`, `cs8`, `cs16`, `cf32`).
- `name`: source name added to output JSON.
- WebSocket-specific: `token` for the Socket.IO auth payload and `event`/`events` for comma-separated Socket.IO event filters (`message` by default, `*` for all).

Defaults:

- center frequency: `136850000`
- sample rate: `1050000`
- channels: `136875000`, `136975000`
- file format: `cu8`
- RTL-SDR gain: `49.6` dB
- Airspy gain: `50.0`
- HackRF gain: `30.0`
- Soapy gain: `49.6` dB
- bias tee: `false`
- sync threshold: `3.2`

## Config file

Configuration is loaded from `$XDG_CONFIG_HOME/vdl136/config.toml` when present.
Set `VDL136_CONFIG=/path/to/config.toml` to override it.

Example:

```toml
stats = true
output = "~/vdl136.jsonl"
sync_threshold = 3.2

[[sources]]
rtlsdr = { device = 0 }
gain = 40.0
bias_tee = false
center_freq = 136850000
sample_rate = 1050000
channels = [136875000, 136975000]

[[sources]]
file = "~/captures/vdl2.cu8"
format = "cu8"
center_freq = 136850000
sample_rate = 1050000
channels = [136875000]
```

## Output

Decoded AVLC frames are emitted as JSON lines. `vdl136` adds source and timing metadata:

- `bearer = "vdl2"`
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
- demod quality fields: `signal_dbfs`, `noise_dbfs`, `snr_db`, `ppm_error`

WebSocket sources emit JSON lines with both forms preserved:

- `raw`: the original Socket.IO/Airframes event payload.
- `decoded`: the Rust-side interpretation. For Airframes `message` events this includes selected row fields and, when the row text contains an ARINC 622 envelope, the native decoded ARINC 622/CPDLC/ADS-C payload. Rows that cannot be decoded still keep `raw` and include `decoded.ok = false` with a reason/error.

Use `--output path.jsonl` to also write a JSONL copy to disk.

Existing debug options remain available: `--stats`, `--reject-log`, `--candidate-log`, `--include-fcs-fail`, `--demod-trace-dir`, and window filters.

## Hardware checks

Before running live SDR sources, verify devices with the vendor tools:

- RTL-SDR: `rtl_test -t` or `rtl_eeprom`
- Airspy: `airspy_info`
- HackRF: `hackrf_info`
- SoapySDR: `SoapySDRUtil --find` or `SoapySDRUtil --probe`
