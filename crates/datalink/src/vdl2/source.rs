use serde::{Deserialize, Serialize};
use std::str::FromStr;
use url::Url;

pub const DEFAULT_CENTER_FREQ: u32 = 136_850_000;
pub const DEFAULT_SAMPLE_RATE: u32 = 1_050_000;
pub const DEFAULT_CHANNELS: &[u32] = &[136_875_000, 136_975_000];
pub const DEFAULT_CHUNK_SIZE: usize = 65_536;

use desperado::IqFormat;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", untagged)]
pub enum Address {
    File {
        file: String,
    },
    #[cfg(feature = "rtlsdr")]
    Rtlsdr {
        device: Option<usize>,
        serial: Option<String>,
    },
    #[cfg(feature = "airspy")]
    Airspy {
        device: Option<usize>,
        serial: Option<String>,
    },
    #[cfg(feature = "hackrf")]
    Hackrf {
        device: Option<usize>,
    },
    #[cfg(feature = "soapy")]
    Soapy {
        soapy: String,
    },
    Websocket {
        websocket: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        events: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    #[serde(flatten)]
    pub address: Address,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub center_freq: Option<u32>,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default, alias = "channel")]
    pub channels: Option<Vec<u32>>,
    #[serde(default)]
    pub gain: Option<f64>,
    #[serde(default)]
    pub bias_tee: Option<bool>,
    #[serde(default, alias = "rf_amp")]
    pub amp_enable: Option<bool>,
    #[serde(default, alias = "if_gain")]
    pub lna_gain: Option<f64>,
    #[serde(default, alias = "bb_gain")]
    pub vga_gain: Option<f64>,
    #[serde(default, alias = "iq_format")]
    pub format: Option<String>,
}

impl Source {
    pub fn center_freq(&self) -> u32 {
        self.center_freq.unwrap_or(DEFAULT_CENTER_FREQ)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.unwrap_or(DEFAULT_SAMPLE_RATE)
    }

    pub fn channels(&self) -> Vec<u32> {
        self.channels
            .clone()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| self.auto_channels())
    }

    /// When no channels are explicitly configured, return all standard VDL2 channels
    /// (25 kHz spacing) that fall within the recording's bandwidth.
    /// Falls back to DEFAULT_CHANNELS when the bandwidth is too narrow or matches the
    /// nominal center frequency exactly.
    fn auto_channels(&self) -> Vec<u32> {
        let center = self.center_freq();
        let sr = self.sample_rate();
        // Use 90% of half-bandwidth to stay away from the edges
        let half_bw = (sr as f64 * 0.45) as u32;
        let lo = center.saturating_sub(half_bw);
        let hi = center.saturating_add(half_bw);

        // Standard VDL2 channels: 136.600–137.000 MHz, 25 kHz spacing
        let candidates: Vec<u32> = (0..17)
            .map(|i| 136_600_000_u32 + i * 25_000)
            .filter(|&ch| ch >= lo && ch <= hi)
            .collect();

        if candidates.is_empty() {
            DEFAULT_CHANNELS.to_vec()
        } else {
            candidates
        }
    }

    pub fn label(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        match &self.address {
            Address::File { file } => format!("file:{file}"),
            #[cfg(feature = "rtlsdr")]
            Address::Rtlsdr { device, serial } => serial
                .as_ref()
                .map(|s| format!("rtlsdr:{s}"))
                .unwrap_or_else(|| format!("rtlsdr:{}", device.unwrap_or(0))),
            #[cfg(feature = "airspy")]
            Address::Airspy { device, serial } => serial
                .as_ref()
                .map(|s| format!("airspy:{s}"))
                .unwrap_or_else(|| format!("airspy:{}", device.unwrap_or(0))),
            #[cfg(feature = "hackrf")]
            Address::Hackrf { device } => format!("hackrf:{}", device.unwrap_or(0)),
            #[cfg(feature = "soapy")]
            Address::Soapy { soapy } => format!("soapy:{soapy}"),
            Address::Websocket { websocket, .. } => format!("websocket:{websocket}"),
        }
    }

    pub fn iq_format(&self) -> IqFormat {
        self.format
            .as_deref()
            .unwrap_or("cu8")
            .parse()
            .unwrap_or(IqFormat::Cu8)
    }

    #[cfg(feature = "sdr")]
    pub fn gain(&self, default: f64) -> desperado::Gain {
        self.gain
            .map(desperado::Gain::Manual)
            .unwrap_or(desperado::Gain::Manual(default))
    }
}

impl FromStr for Source {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "-" {
            return Ok(Source {
                address: Address::File {
                    file: "-".to_string(),
                },
                name: Some("stdin".to_string()),
                center_freq: None,
                sample_rate: None,
                channels: None,
                gain: None,
                bias_tee: None,
                amp_enable: None,
                lna_gain: None,
                vga_gain: None,
                format: None,
            });
        }
        let default = Url::parse("file://").unwrap();
        let url = default.join(s).map_err(|e| e.to_string())?;
        let address = match url.scheme() {
            "file" => {
                let file = if let Some(host) = url.host_str() {
                    format!("{}{}", host, url.path())
                } else {
                    url.path().to_string()
                };
                Address::File { file }
            }
            #[cfg(feature = "rtlsdr")]
            "rtlsdr" => {
                let host = url.host_str().unwrap_or("");
                if host.is_empty() {
                    Address::Rtlsdr {
                        device: Some(0),
                        serial: None,
                    }
                } else if let Ok(device) = host.parse::<usize>() {
                    Address::Rtlsdr {
                        device: Some(device),
                        serial: None,
                    }
                } else if let Some(serial) = host.strip_prefix("serial=") {
                    Address::Rtlsdr {
                        device: None,
                        serial: Some(serial.to_string()),
                    }
                } else {
                    Address::Rtlsdr {
                        device: Some(0),
                        serial: None,
                    }
                }
            }
            #[cfg(feature = "airspy")]
            "airspy" => {
                let host = url.host_str().unwrap_or("");
                if host.is_empty() {
                    Address::Airspy {
                        device: Some(0),
                        serial: None,
                    }
                } else if let Ok(device) = host.parse::<usize>() {
                    Address::Airspy {
                        device: Some(device),
                        serial: None,
                    }
                } else if let Some(serial) = host.strip_prefix("serial=") {
                    Address::Airspy {
                        device: None,
                        serial: Some(serial.to_string()),
                    }
                } else {
                    Address::Airspy {
                        device: Some(0),
                        serial: None,
                    }
                }
            }
            #[cfg(feature = "hackrf")]
            "hackrf" => {
                let device = url
                    .host_str()
                    .and_then(|s| s.parse::<usize>().ok())
                    .or(Some(0));
                Address::Hackrf { device }
            }
            #[cfg(feature = "soapy")]
            "soapy" => Address::Soapy {
                soapy: url.host_str().unwrap_or("").to_string(),
            },
            "ws" | "wss" => Address::Websocket {
                websocket: s.to_string(),
                token: None,
                events: None,
            },
            "airframes" => Address::Websocket {
                websocket: "wss://ws.airframes.io/socket.io/?EIO=4&transport=websocket".to_string(),
                token: None,
                events: None,
            },
            other => return Err(format!("unsupported source scheme: {other}")),
        };

        let mut source = Source {
            address,
            name: None,
            center_freq: None,
            sample_rate: None,
            channels: None,
            gain: None,
            bias_tee: None,
            amp_enable: None,
            lna_gain: None,
            vga_gain: None,
            format: None,
        };

        if let Some(query) = url.query() {
            for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
                match key.as_ref() {
                    "name" => source.name = Some(value.into_owned()),
                    "center_freq" | "freq" => source.center_freq = parse_hz(&value),
                    "sample_rate" | "rate" => source.sample_rate = parse_hz(&value),
                    "channel" | "channels" => {
                        let parsed: Vec<u32> = value.split(',').filter_map(parse_hz).collect();
                        if !parsed.is_empty() {
                            source.channels.get_or_insert_with(Vec::new).extend(parsed);
                        }
                    }
                    "gain" => source.gain = value.parse::<f64>().ok(),
                    "bias_tee" => source.bias_tee = parse_bool(&value),
                    "amp_enable" | "rf_amp" => source.amp_enable = parse_bool(&value),
                    "lna_gain" | "if_gain" => source.lna_gain = value.parse::<f64>().ok(),
                    "vga_gain" | "bb_gain" => source.vga_gain = value.parse::<f64>().ok(),
                    "format" | "iq_format" => source.format = Some(value.into_owned()),
                    "token" => {
                        if let Address::Websocket { token, .. } = &mut source.address {
                            *token = Some(value.into_owned());
                        }
                    }
                    "event" | "events" => {
                        if let Address::Websocket { events, .. } = &mut source.address {
                            let parsed: Vec<String> = value
                                .split(',')
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                                .collect();
                            if !parsed.is_empty() {
                                *events = Some(parsed);
                            }
                        }
                    }
                    _ if !key.is_empty() && value.is_empty() => {
                        source.name = Some(key.into_owned())
                    }
                    _ => {}
                }
            }
        }

        Ok(source)
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_hz(value: impl AsRef<str>) -> Option<u32> {
    let value = value.as_ref().trim();
    let (num, mult) = match value.chars().last()? {
        'k' | 'K' => (&value[..value.len() - 1], 1_000.0),
        'm' | 'M' => (&value[..value.len() - 1], 1_000_000.0),
        'g' | 'G' => (&value[..value.len() - 1], 1_000_000_000.0),
        _ => (value, 1.0),
    };
    num.parse::<f64>().ok().map(|v| (v * mult).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_file_url() {
        let src: Source = "file:///tmp/vdl2.cu8?format=cu8&sample_rate=1050000&center_freq=136850000&channel=136875000".parse().unwrap();
        assert_eq!(src.center_freq(), 136_850_000);
        assert_eq!(src.sample_rate(), 1_050_000);
        assert_eq!(src.channels(), vec![136_875_000]);
    }

    #[cfg(feature = "rtlsdr")]
    #[test]
    fn parse_rtlsdr_url() {
        let src: Source = "rtlsdr://0?gain=40&bias_tee=true&channel=136.875M"
            .parse()
            .unwrap();
        match src.address {
            Address::Rtlsdr { device, .. } => assert_eq!(device, Some(0)),
            _ => panic!("expected rtlsdr"),
        }
        assert_eq!(src.channels(), vec![136_875_000]);
        assert_eq!(src.bias_tee, Some(true));
    }

    #[cfg(feature = "soapy")]
    #[test]
    fn parse_soapy_url() {
        let src: Source = "soapy://driver=rtlsdr?gain=40&channel=136875000,136975000"
            .parse()
            .unwrap();
        match &src.address {
            Address::Soapy { soapy } => assert_eq!(soapy, "driver=rtlsdr"),
            _ => panic!("expected soapy"),
        }
        assert_eq!(src.channels(), vec![136_875_000, 136_975_000]);
    }

    #[cfg(feature = "hackrf")]
    #[test]
    fn parse_hackrf_gains() {
        let src: Source =
            "hackrf://0?center_freq=136.85M&channel=136.875M&rf_amp=true&if_gain=32&bb_gain=20"
                .parse()
                .unwrap();
        match src.address {
            Address::Hackrf { device } => assert_eq!(device, Some(0)),
            _ => panic!("expected hackrf"),
        }
        assert_eq!(src.amp_enable, Some(true));
        assert_eq!(src.lna_gain, Some(32.0));
        assert_eq!(src.vga_gain, Some(20.0));
    }

    #[test]
    fn parse_toml_file_source() {
        let text = r#"
file = "~/captures/vdl2.cu8"
format = "cu8"
center_freq = 136850000
sample_rate = 1050000
channels = [136875000, 136975000]
"#;
        let src: Source = toml::from_str(text).unwrap();
        assert_eq!(src.channels(), vec![136_875_000, 136_975_000]);
    }

    #[test]
    fn parse_airframes_websocket_source() {
        let src: Source = "airframes://live?event=message&token=test".parse().unwrap();
        match src.address {
            Address::Websocket {
                websocket,
                token,
                events,
            } => {
                assert!(websocket.starts_with("wss://ws.airframes.io/"));
                assert_eq!(token.as_deref(), Some("test"));
                assert_eq!(events, Some(vec!["message".to_string()]));
            }
            _ => panic!("expected websocket"),
        }
    }
}
