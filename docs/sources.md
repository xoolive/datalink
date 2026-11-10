# Sources

A source describes where samples or decoded events come from. The same source model is used by standalone bearer commands and merged receiver configuration.

!!! tip "Get started!"

    The easiest path to get some data is to start with the VHF or VDL2 subcommands:

    ```sh
    datalink vhf --help
    datalink vdl2 --help
    ```

## File and stdin input

Raw I/Q files need enough metadata to describe the recording:

```sh
datalink vdl2 --format cf32 --center-freq 136500000 --sample-rate 1800000 \
  capture.raw
```

This is equivalent to the following syntax:

```sh
datalink vdl2 file://capture.raw?center_freq=136.5M&sample_rate=1.8M&format=cf32
```

!!! tip

    `datalink` can infer some metadata from common capture filenames, WAV metadata, including SDRuno and GQRX-style names.

    ```sh
    # Pass GQRX raw files
    datalink vdl2 gqrx_20260518_114025_136500000_1800000_fc.raw
    # Pass SDRUno file from https://www.sdrplay.com/iq-demo-files/
    datalink vhf SDRuno_20200908_152020Z_129535kHz.wav
    ```

Set explicit values when inference is ambiguous.

### HFDL raw IQ on stdin

Use `-` to read raw HFDL IQ from stdin. There is no filename or WAV header to
infer metadata from, so specify the format, sample rate and center frequency:

```sh
cat capture.cs16 | datalink hfdl - --format cs16 --sample-rate 62500 \
  --center-freq 11404000 --channel 11387 --max-seconds 5
```

Supported stdin formats are `u8` (alias `cu8`), `cs16` (aliases `s16`, `s16le`,
`s16_le`) and `cf32` (alias `f32`). Without a format setting, HFDL retains its
`cf32` default. Source configuration format settings take precedence over the
standalone command's format/default. WAV stdin is not supported: pass a WAV
file path instead, or extract its raw interleaved IQ before piping it.

`--start-second` consumes and discards samples from stdin; `--max-seconds`
limits the retained sample duration, not wall-clock time. A shorter stream ends
at EOF. Empty stdin succeeds without events; an incomplete IQ sample or read
error encountered while reading the window fails the command rather than
silently accepting damaged input. These checks do not inspect bytes beyond the
requested window. The existing seekable raw/WAV file readers are unchanged.

The frontend buffers the requested window before decoding; it is not a
continuous streaming decoder. Close the producer's pipe at EOF. A stalled
producer is not bounded by `--max-seconds`, and no stronger idle-stdin cancellation
guarantee is added here.

#### Optional positive-capture regression

The ordinary test suite uses tiny generated IQ inputs and injected read errors.
A separate ignored CLI test compares raw file input, piped stdin and a WAV
window against the same known, FCS-valid 68-byte MPDU, cross-checked with
`dumphfdl` 1.7.0. It requires local captures, which are not distributed with the
repository:

```sh
DATALINK_HFDL_FIXTURE=/path/to/hfdl-11387-start10-duration5.cs16 \
DATALINK_HFDL_WAV_FIXTURE=/path/to/SDRuno_20200904_143937Z_11404kHz.wav \
  cargo test -p datalink --all-features --test hfdl_stdin \
    private_capture_matches_reference_for_file_stdin_and_wav -- --ignored
```

The raw fixture is original WAV seconds [10, 15), PCM signed 16-bit little-endian
interleaved IQ at 62500 samples/s, center 11404000 Hz. Extract it without
resampling or changing channel order:

```sh
ffmpeg -i SDRuno_20200904_143937Z_11404kHz.wav -ss 10 -t 5 \
  -c:a pcm_s16le -f s16le hfdl-11387-start10-duration5.cs16
```

Its SHA-256 is `0be8112edca5056061463fc105776aea1a4fe3595261fdde39c829241b157a72`.
This establishes input-path equivalence on one positive fixture, not complete
HFDL demodulation accuracy or full-capture parity with the reference decoder.

## SDR input

The standalone `vhf`, `vdl2`, and `hfdl` commands accept RTL-SDR, Airspy, HackRF.

```sh
datalink vhf rtlsdr://  # comes with a reasonable set of default values
datalink vdl2 hackrf://  # comes with a reasonable set of default values
```

SoapySDR is also covered if compiled with the `soapy` feature.

You may specify more options:

```sh
datalink vhf rtlsdr://0?center_freq=131.7M&sample_rate=1.05M&channel=131.725M
datalink vdl2 rtlsdr://0?center_freq=136.85M&sample_rate=1.05M&channel=136.875M,136.975M&gain=auto
datalink vdl2 airspy://0?center_freq=136.85M&sample_rate=6M
datalink hfdl hackrf://0?center_freq=10M&sample_rate=8M
```

Common parameters are `name`, `center_freq`/`freq`, `sample_rate`/`rate`, `channel`/`channels`, `format`/`iq_format`, `gain`, `bias_tee`, `amp_enable`/`rf_amp`, `rf_gain`, `lna_gain`/`if_gain`, `mixer_gain`, and `vga_gain`/`bb_gain`.

!!! tip

    If you have enough sample rate to cover VHF and VDL2 with the same SDR device, look at how to handle that with [configuration files](config.md)

## Airframes.io websocket source

Airframes.io is an event source rather than an I/Q source. Use the standalone command:

```sh
datalink airframes
```

See the public [Airframes.io docs](https://github.com/airframesio/docs) for ecosystem background.
