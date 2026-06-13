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

## SDR input

The standalone `vhf`, `vdl2`, and `hfdl` commands accept RTL-SDR, Airspy, HackRF.

```sh
datalink vhf rtlsdr://  # comes with a reasonable set of default values
datalink vdl2 hackrf://  # comes with a reasonable set of default values
```

SoapySDR is also covered if compiled with the `soapy` feature.

You may specify more options

```sh
datalink vhf rtlsdr://0?center_freq=131.7M&sample_rate=1.05M&channel=131.725M
rtlsdr://0?center_freq=136.85M&sample_rate=1.05M&channel=136.875M,136.975M&gain=auto
datalink vdl2 airspy://0?center_freq=136.85M&sample_rate=6M
datalink hfdl hackrf://0?center_freq=10M&sample_rate=8M
```

Common parameters are `name`, `center_freq`/`freq`, `sample_rate`/`rate`, `channel`/`channels`, `format`/`iq_format`, `gain`, `bias_tee`, `rf_gain`, `lna_gain`/`if_gain`, `mixer_gain`, and `vga_gain`/`bb_gain`.

!!! tip

    If you have enough sample rate to cover VHF and VDL2 with the same SDR device, look at how to handle that with [configuration files](config.md)

## Airframes.io websocket source

Airframes.io is an event source rather than an I/Q source. Use the standalone command:

```sh
datalink airframes
```

See the public [Airframes.io docs](https://github.com/airframesio/docs) for ecosystem background.
