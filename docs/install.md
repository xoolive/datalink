# Installation

`datalink` is currently easiest to install from source with Cargo.

Packaged builds for common platforms will be added later.

<!-- Packaging note: when adding release binaries, mirror the structure/style used by ../jet1090/docs/install.md. -->

## Install with Cargo

```sh
cargo install datalink
```

## Usage

Find about available subcommands:

```sh
datalink --help
```

| Subcommand | Purpose                                    |
| ---------- | ------------------------------------------ |
| vhf        | Classic VHF ACARS frontend                 |
| vdl2       | VDL Mode 2 frontend for I/Q and SDR inputs |
| hfdl       | HF Data Link frontend                      |
| decode     | Decode standalone payloads or frames       |
| airframes  | Decode airframes.io websocket feed         |

!!! tip "Get started!"

    The easiest path to get some data is to start with the VHF or VDL2 subcommands:

    ```sh
    datalink vhf --help
    datalink vdl2 --help
    ```

    Then check [Sources](sources.md) for more details

## Features

The default build includes the common receiver features:

| Feature     | Purpose                          | Default |
| ----------- | -------------------------------- | :-----: |
| `rtlsdr`    | Native RTL-SDR input             |    ☑    |
| `airspy`    | Native Airspy input              |    ☑    |
| `hackrf`    | Native HackRF input              |    ☑    |
| `websocket` | Airframes.io websocket ingestion |    ☑    |
| `soapy`     | Optional SoapySDR input          |         |

## SDR tools

If you use native SDR input, it may be good practice to install and test the vendor tools first:

- RTL-SDR: `rtl_test`, `rtl_eeprom`
- Airspy: `airspy_info`
- HackRF: `hackrf_info`
- SoapySDR: `SoapySDRUtil --find`

If you only use Airframes.io, standalone decode commands, or file captures, you can start without SDR hardware.
