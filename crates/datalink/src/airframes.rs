use acars::decode::acars::{decode_acars_text_payload, MessageDirection};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use http::Uri;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

use crate::merged::{
    Bearer, DecodedEvent, OutputConfig, OutputSink, ProtocolMessage, ReceiverMetadata, SourceClass,
    SourceConfig, SourceMetadata,
};

const DEFAULT_AIRFRAMES_WS: &str = "wss://ws.airframes.io/socket.io/?EIO=4&transport=websocket";

#[derive(Debug, Clone)]
pub(crate) struct Source {
    websocket: String,
    token: Option<String>,
    events: Vec<String>,
    name: Option<String>,
}

#[derive(Serialize)]
struct AirframesAuth<'a> {
    token: &'a str,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct AirframesPayload {
    pub label: Option<String>,
    pub text: Option<String>,
    pub from_hex: Option<String>,
    pub to_hex: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f64>,
    pub track: Option<f64>,
    #[serde(default)]
    pub source_type: Bearer,
    #[serde(deserialize_with = "crate::util::deserialize_timestamp", default)]
    pub timestamp: Option<f64>,
    #[serde(deserialize_with = "crate::util::deserialize_timestamp", default)]
    pub created_at: Option<f64>,
    pub frequency: Option<f64>,
    pub id: Option<String>,
    pub airframe_id: Option<u64>,
    pub flight_id: Option<u64>,
    pub tail: Option<String>,
    pub link_direction: Option<String>,
    pub airframe: Option<AirframesAirframe>,
    pub flight: Option<AirframesFlight>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct AirframesAirframe {
    pub icao: Option<String>,
    pub tail: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct AirframesFlight {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f64>,
    pub track: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AirframesMessage {
    #[serde(flatten)]
    pub payload: AirframesPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<acars::decode::payload::AcarsAppPayload>,
}

impl Source {
    fn label(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| "airframes.io".to_string())
    }
}

impl Default for Source {
    fn default() -> Self {
        Self {
            websocket: DEFAULT_AIRFRAMES_WS.to_string(),
            token: None,
            events: vec!["message".to_string()],
            name: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum AirframesSourceError {
    #[error("unsupported Airframes source scheme: {0}")]
    UnsupportedScheme(String),
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),
}

impl FromStr for Source {
    type Err = AirframesSourceError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let default = url::Url::parse("airframes://").unwrap();
        let url = default.join(input)?;
        let mut source = match url.scheme() {
            "airframes" => Source::default(),
            "ws" | "wss" => Source {
                websocket: input.to_string(),
                ..Source::default()
            },
            other => return Err(AirframesSourceError::UnsupportedScheme(other.to_string())),
        };
        if let Some(query) = url.query() {
            for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
                match key.as_ref() {
                    "name" => source.name = Some(value.into_owned()),
                    "token" => source.token = Some(value.into_owned()),
                    "event" | "events" => {
                        let parsed: Vec<String> = value
                            .split(',')
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect();
                        if !parsed.is_empty() {
                            source.events = parsed;
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

#[derive(Debug, Default, Clone, Parser)]
#[command(about = "Airframes.io websocket event source")]
pub(crate) struct Options {
    /// Airframes source URL; defaults to airframes://
    source: Option<String>,
    /// Dump a copy of decoded websocket rows as JSONL
    #[arg(short, long)]
    output: Option<String>,
    /// Publish decoded application messages to Redis pub/sub topics
    #[arg(long, value_name = "REDIS URL")]
    redis_url: Option<String>,
    /// Retry interval (seconds) when publishing to Redis fails; 0 disables retry
    #[arg(long, default_value_t = 5)]
    redis_retry_interval: u64,
}

pub(crate) async fn run(options: Options) -> anyhow::Result<()> {
    let source: Source = options
        .source
        .as_deref()
        .unwrap_or("airframes://")
        .parse()?;
    let output_config = OutputConfig {
        jsonl: options.output,
        redis_url: options.redis_url,
        redis_retry_interval: Some(options.redis_retry_interval),
    };
    let mut output = OutputSink::new(&output_config).await?;
    run_source(
        &source,
        SourceMetadata {
            id: "airframes".to_string(),
            name: source.label(),
            class: SourceClass::Events,
            format: Some("airframes.io".to_string()),
        },
        &mut output,
    )
    .await?;
    output.flush()?;
    Ok(())
}

pub(crate) async fn run_config_source(
    source: &SourceConfig,
    output: &mut OutputSink,
) -> anyhow::Result<()> {
    let mut airframes = source
        .websocket
        .as_deref()
        .unwrap_or("airframes://")
        .parse::<Source>()?;
    airframes.name = Some(source.display_name().to_string());
    run_source(&airframes, SourceMetadata::from_config(source)?, output).await
}

async fn run_source(
    source: &Source,
    source_meta: SourceMetadata,
    output: &mut OutputSink,
) -> anyhow::Result<()> {
    let selected_events = source.events.clone();
    let capture_all = selected_events.iter().any(|event| event == "*");
    let mut request =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
            source.websocket.as_str(),
        )?;
    request.headers_mut().insert(
        "Origin",
        tokio_tungstenite::tungstenite::http::HeaderValue::from_static("https://app.airframes.io"),
    );
    let mut ws = websocket_connect(&source.websocket, request).await?;
    while let Some(message) = ws.next().await {
        let message = message?;
        if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
            if text.starts_with('0') {
                break;
            }
        }
    }

    let auth = AirframesAuth {
        token: source.token.as_deref().unwrap_or(""),
    };
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        format!("40{}", serde_json::to_string(&auth)?).into(),
    ))
    .await?;

    while let Some(message) = ws.next().await {
        let message = message?;
        match message {
            tokio_tungstenite::tungstenite::Message::Text(text) => {
                for packet in text.split('\u{1e}') {
                    if packet == "2" {
                        ws.send(tokio_tungstenite::tungstenite::Message::Text(
                            "3".to_string().into(),
                        ))
                        .await?;
                        continue;
                    }
                    let Some((event, payload)) = parse_socketio_event(packet) else {
                        continue;
                    };
                    if !capture_all && !selected_events.iter().any(|wanted| wanted == &event) {
                        continue;
                    }
                    let decoded = normalize_payload(source_meta.clone(), &event, payload)?;
                    output.emit(decoded).await?;
                }
            }
            tokio_tungstenite::tungstenite::Message::Ping(payload) => {
                ws.send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                    .await?;
            }
            tokio_tungstenite::tungstenite::Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn normalize_payload(
    source: SourceMetadata,
    event: &str,
    payload: AirframesPayload,
) -> anyhow::Result<DecodedEvent> {
    let bearer = payload.source_type;

    let app = if event == "message" {
        payload.text.as_deref().map(|text| {
            let normalized_text = normalize_arinc622_text(text).unwrap_or_else(|| text.to_string());
            let direction = infer_airframes_direction(
                payload.label.as_deref().unwrap_or(""),
                payload.link_direction.as_deref(),
            );
            decode_acars_text_payload(
                payload.label.as_deref().unwrap_or(""),
                None,
                &normalized_text,
                direction,
            )
        })
    } else {
        None
    };

    let timestamp = payload.timestamp.or(payload.created_at);
    let raw_kinematics = airframes_payload_kinematics(&payload);
    let app_kinematics = app.as_ref().and_then(|app| {
        use acars::decode::compact::ExtractKinematics;
        app.kinematics()
    });
    let kinematics = match (raw_kinematics, app_kinematics) {
        (Some(raw), Some(app)) => Some(raw.merge(app)),
        (Some(raw), None) => Some(raw),
        (None, Some(app)) => Some(app),
        (None, None) => None,
    };
    let af_msg = AirframesMessage { payload, app };
    let pmsg = ProtocolMessage::Airframes(Box::new(af_msg));

    Ok(DecodedEvent {
        event: "message",
        timestamp,
        bearer,
        source,
        receiver: None::<ReceiverMetadata>,
        aircraft: crate::merged::aircraft_summary(&pmsg),
        kinematics,
        raw_frame_hex: None,
        message: pmsg,
    })
}

fn parse_socketio_event(packet: &str) -> Option<(String, AirframesPayload)> {
    let json = packet.strip_prefix("42")?;
    serde_json::from_str(json).ok()
}

pub(crate) fn extract_airframes_aircraft(row: &AirframesPayload) -> Option<String> {
    if let Some(icao) = row
        .airframe
        .as_ref()
        .and_then(|a| a.icao.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(icao.to_ascii_lowercase());
    }
    if let Some(hex) = &row.from_hex {
        let addr = hex.trim().to_ascii_lowercase();
        if addr.len() == 6 {
            return Some(addr);
        }
    }
    None
}

pub(crate) fn extract_airframes_registration(row: &AirframesPayload) -> Option<String> {
    row.tail
        .as_deref()
        .or_else(|| row.airframe.as_ref().and_then(|a| a.tail.as_deref()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn airframes_payload_kinematics(
    payload: &AirframesPayload,
) -> Option<acars::decode::compact::Kinematics> {
    let payload_kinematics = kinematics_from_airframes_position(
        payload.latitude,
        payload.longitude,
        payload.altitude,
        payload.track,
        "airframes_payload",
    );
    let flight_kinematics = payload.flight.as_ref().and_then(|flight| {
        kinematics_from_airframes_position(
            flight.latitude,
            flight.longitude,
            flight.altitude,
            flight.track,
            "airframes_flight",
        )
    });
    match (payload_kinematics, flight_kinematics) {
        (Some(payload), Some(flight)) => Some(payload.merge(flight)),
        (Some(payload), None) => Some(payload),
        (None, Some(flight)) => Some(flight),
        (None, None) => None,
    }
}

fn kinematics_from_airframes_position(
    latitude: Option<f64>,
    longitude: Option<f64>,
    altitude: Option<f64>,
    track: Option<f64>,
    derived_from: &str,
) -> Option<acars::decode::compact::Kinematics> {
    let (Some(latitude), Some(longitude)) = (latitude, longitude) else {
        return None;
    };
    if latitude == 0.0 && longitude == 0.0 {
        return None;
    }
    Some(acars::decode::compact::Kinematics {
        position: Some(acars::decode::compact::Position {
            latitude,
            longitude,
        }),
        altitude_ft: normalize_airframes_altitude(altitude),
        track,
        ground_speed_knots: None,
        derived_from: Some(derived_from.to_string()),
    })
}

fn normalize_airframes_altitude(altitude: Option<f64>) -> Option<i32> {
    altitude.and_then(|value| {
        let rounded = value.round();
        ((i32::MIN as f64)..=(i32::MAX as f64))
            .contains(&rounded)
            .then_some(rounded as i32)
    })
}

fn normalize_arinc622_text(text: &str) -> Option<String> {
    if text.starts_with('/') {
        return has_arinc622_imi(text).then(|| text.to_string());
    }
    for token in text.split_whitespace().rev() {
        if has_arinc622_imi(token) {
            return Some(format!("/{token}"));
        }
    }
    None
}

fn has_arinc622_imi(text: &str) -> bool {
    [".AT1.", ".CR1.", ".CC1.", ".DR1.", ".ADS."]
        .iter()
        .any(|needle| text.contains(needle))
}

fn infer_airframes_direction(label: &str, link_direction: Option<&str>) -> MessageDirection {
    match label {
        "AA" => MessageDirection::GroundToAir,
        "BA" => MessageDirection::AirToGround,
        _ => match link_direction {
            Some("uplink") => MessageDirection::GroundToAir,
            Some("downlink") => MessageDirection::AirToGround,
            _ => MessageDirection::Unknown,
        },
    }
}

async fn websocket_connect(
    url: &str,
    request: tokio_tungstenite::tungstenite::handshake::client::Request,
) -> anyhow::Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let proxy_env = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .ok();

    if let Some(proxy_url) = proxy_env {
        let proxy_uri: Uri = proxy_url.parse()?;
        let proxy_host = proxy_uri
            .host()
            .ok_or_else(|| anyhow::anyhow!("proxy URL has no host"))?
            .to_string();
        let proxy_port = proxy_uri.port_u16().unwrap_or(8080);
        let target_uri: Uri = url.parse()?;
        let target_host = target_uri
            .host()
            .ok_or_else(|| anyhow::anyhow!("target URL has no host"))?
            .to_string();
        let target_port = target_uri.port_u16().unwrap_or(443);
        let connect_target = format!("{target_host}:{target_port}");
        let connect_req = format!(
            "CONNECT {connect_target} HTTP/1.1\r\nHost: {connect_target}\r\nProxy-Connection: keep-alive\r\n\r\n"
        );

        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut tcp = tokio::net::TcpStream::connect(format!("{proxy_host}:{proxy_port}")).await?;
        tcp.write_all(connect_req.as_bytes()).await?;
        let mut buf = [0u8; 4096];
        let mut n = 0usize;
        loop {
            let r = tcp.read(&mut buf[n..]).await?;
            if r == 0 {
                anyhow::bail!("proxy closed during CONNECT");
            }
            n += r;
            if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            if n >= buf.len() {
                anyhow::bail!("proxy CONNECT response too large");
            }
        }
        let status_line = std::str::from_utf8(&buf[..n])?
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
        if !status_line.contains("200") {
            anyhow::bail!("proxy CONNECT failed: {status_line}");
        }
        let root_store = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let tls_config = std::sync::Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth(),
        );
        let connector = tokio_rustls::TlsConnector::from(tls_config);
        let server_name = rustls::pki_types::ServerName::try_from(target_host.clone())?;
        let tls = connector.connect(server_name, tcp).await?;
        let stream = tokio_tungstenite::MaybeTlsStream::Rustls(tls);
        let (ws, _) = tokio_tungstenite::client_async(request, stream).await?;
        Ok(ws)
    } else {
        let (ws, _) = tokio_tungstenite::connect_async(request).await?;
        Ok(ws)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_airframes_url() {
        let src: Source = "airframes://live?event=message&token=test".parse().unwrap();
        assert!(src.websocket.starts_with("wss://ws.airframes.io/"));
        assert_eq!(src.token.as_deref(), Some("test"));
        assert_eq!(src.events, vec!["message"]);
    }

    #[test]
    fn normalizes_airframes_payload_as_event_source() {
        let meta = SourceMetadata {
            id: "airframes".into(),
            name: "Airframes".into(),
            class: SourceClass::Events,
            format: Some("airframes.io".into()),
        };
        let payload = serde_json::from_str(
            r#"{
            "label": "SA",
            "text": "hello",
            "from_hex": "A1B2C3",
            "source_type": "vdl2",
            "timestamp": 123.0
        }"#,
        )
        .unwrap();
        let event = crate::airframes::normalize_payload(meta, "message", payload).unwrap();
        assert_eq!(event.bearer, Bearer::Vdl2);
    }

    #[test]
    fn parses_live_socketio_payload_with_created_at_and_timestamp() {
        let packet = r#"42["message",{
            "id":"6853529784",
            "created_at":"2026-06-09T20:34:43.090487Z",
            "updated_at":"2026-06-09T20:34:43.090487Z",
            "timestamp":"2026-06-09T20:34:42.157Z",
            "source_type":"acars",
            "frequency":131.725,
            "label":"H1",
            "tail":"C-FRAX",
            "airframe":{"icao":"C02CFC"}
        }]"#;

        let (event, payload) = parse_socketio_event(packet).expect("parse socket.io event");
        assert_eq!(event, "message");
        assert!(payload.created_at.is_some());
        assert!(payload.timestamp.is_some());
        assert_eq!(payload.source_type, Bearer::Vhf);

        let meta = SourceMetadata {
            id: "airframes".into(),
            name: "Airframes".into(),
            class: SourceClass::Events,
            format: Some("airframes.io".into()),
        };
        let normalized = normalize_payload(meta, &event, payload).expect("normalize payload");
        assert_eq!(normalized.timestamp, Some(1_781_037_282.157));
    }

    #[test]
    fn normalizes_airframes_flight_kinematics_and_identity() {
        let meta = SourceMetadata {
            id: "airframes".into(),
            name: "Airframes".into(),
            class: SourceClass::Events,
            format: Some("airframes.io".into()),
        };
        let payload = serde_json::from_str(
            r#"{
            "source_type": "vdl2",
            "airframe_id": 6624,
            "airframe": {"icao": "A4463E", "tail": "N3749D"},
            "latitude": 0,
            "longitude": 0,
            "flight": {
                "latitude": 60.797974,
                "longitude": -148.846657,
                "altitude": 17000,
                "track": 295.81
            }
        }"#,
        )
        .unwrap();

        let event = normalize_payload(meta, "message", payload).expect("normalize payload");
        let aircraft = event.aircraft.expect("aircraft summary");
        assert_eq!(aircraft.icao24.as_deref(), Some("a4463e"));
        assert_eq!(aircraft.aircraft_id, Some(6624));
        assert_eq!(aircraft.registration.as_deref(), Some("N3749D"));

        let kinematics = event.kinematics.expect("kinematics");
        assert_eq!(kinematics.position.unwrap().latitude, 60.797974);
        assert_eq!(kinematics.position.unwrap().longitude, -148.846657);
        assert_eq!(kinematics.altitude_ft, Some(17000));
        assert_eq!(kinematics.track, Some(295.81));
        assert_eq!(kinematics.derived_from.as_deref(), Some("airframes_flight"));
    }

    #[test]
    fn prefers_top_level_airframes_coordinates_over_flight_coordinates() {
        let meta = SourceMetadata {
            id: "airframes".into(),
            name: "Airframes".into(),
            class: SourceClass::Events,
            format: Some("airframes.io".into()),
        };
        let payload = serde_json::from_str(
            r#"{
            "source_type": "vdl2",
            "latitude": 51.4706,
            "longitude": -0.461941,
            "track": 92.5,
            "flight": {
                "latitude": 60.797974,
                "longitude": -148.846657,
                "altitude": 17000,
                "track": 295.81
            }
        }"#,
        )
        .unwrap();

        let event = normalize_payload(meta, "message", payload).expect("normalize payload");
        let kinematics = event.kinematics.expect("kinematics");
        assert_eq!(kinematics.position.unwrap().latitude, 51.4706);
        assert_eq!(kinematics.position.unwrap().longitude, -0.461941);
        assert_eq!(kinematics.altitude_ft, Some(17000));
        assert_eq!(kinematics.track, Some(92.5));
        assert_eq!(
            kinematics.derived_from.as_deref(),
            Some("airframes_payload")
        );
    }
}
