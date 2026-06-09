use acars::decode::acars::{decode_acars_text_payload, MessageDirection};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use http::Uri;
use serde_json::Value;
use std::str::FromStr;

use crate::merged::{
    Bearer, DecodedEvent, OutputConfig, OutputSink, ReceiverMetadata, SourceClass, SourceConfig,
    SourceMetadata,
};
use crate::util::unix_timestamp_value;

const DEFAULT_AIRFRAMES_WS: &str = "wss://ws.airframes.io/socket.io/?EIO=4&transport=websocket";

#[derive(Debug, Clone)]
pub(crate) struct Source {
    websocket: String,
    token: Option<String>,
    events: Vec<String>,
    name: Option<String>,
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

impl FromStr for Source {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let default = url::Url::parse("airframes://").unwrap();
        let url = default.join(input).map_err(|err| err.to_string())?;
        let mut source = match url.scheme() {
            "airframes" => Source::default(),
            "ws" | "wss" => Source {
                websocket: input.to_string(),
                ..Source::default()
            },
            other => return Err(format!("unsupported Airframes source scheme: {other}")),
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
    /// Include the original websocket payload under raw
    #[arg(long)]
    raw: bool,
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
        .parse()
        .map_err(anyhow::Error::msg)?;
    let output_config = OutputConfig {
        jsonl: options.output,
        raw: options.raw,
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
        .parse::<Source>()
        .map_err(anyhow::Error::msg)?;
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
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        format!(
            "40{}",
            serde_json::to_string(
                &serde_json::json!({ "token": source.token.as_deref().unwrap_or("") })
            )?
        )
        .into(),
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
                    let decoded = normalize_payload(
                        source_meta.clone(),
                        &event,
                        payload,
                        output.raw_enabled(),
                    )?;
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
    payload: Value,
    raw: bool,
) -> anyhow::Result<DecodedEvent> {
    let record = airframes_record(&source.name, event, payload, raw);
    let bearer = infer_bearer(&record);
    Ok(DecodedEvent {
        event: "message",
        timestamp: record.pointer("/decoded/timestamp").and_then(Value::as_f64),
        bearer,
        source,
        receiver: None::<ReceiverMetadata>,
        aircraft: aircraft_summary(&record),
        message: record.get("decoded").cloned().unwrap_or(Value::Null),
        raw: raw.then_some(record),
    })
}

fn parse_socketio_event(packet: &str) -> Option<(String, Value)> {
    let json = packet.strip_prefix("42")?;
    let value: Value = serde_json::from_str(json).ok()?;
    let array = value.as_array()?;
    let event = array.first()?.as_str()?.to_string();
    let payload = if array.len() == 2 {
        array[1].clone()
    } else {
        Value::Array(array.iter().skip(1).cloned().collect())
    };
    Some((event, payload))
}

fn airframes_record(source_label: &str, event: &str, payload: Value, raw: bool) -> Value {
    let decoded = if event == "message" {
        decode_airframes_message(&payload, raw)
    } else {
        None
    };
    let mut record = serde_json::json!({
        "source": source_label,
        "bearer": "airframes.io",
        "event": event,
        "decoded": decoded,
    });
    if raw {
        record
            .as_object_mut()
            .unwrap()
            .insert("raw".into(), payload);
    }
    record
}

fn decode_airframes_message(payload: &Value, include_raw: bool) -> Option<Value> {
    let row = if payload.is_array() {
        payload.as_array()?.first()?
    } else {
        payload
    };
    let text = row.get("text").and_then(Value::as_str);
    let label = row.get("label").and_then(Value::as_str).unwrap_or_default();
    let link_direction = row.get("link_direction").and_then(Value::as_str);
    let direction = infer_airframes_direction(label, link_direction);
    let timestamp = row
        .get("timestamp")
        .or_else(|| row.get("created_at"))
        .and_then(unix_timestamp_value)
        .map(Value::from)
        .unwrap_or(Value::Null);

    let Some(text) = text else {
        return Some(serde_json::json!({
            "path": "unknown",
            "message_class": "metadata_only",
            "summary": "Airframes row without ACARS text payload",
            "airframes_id": row.get("id").cloned().unwrap_or(Value::Null),
            "timestamp": timestamp,
            "label": row.get("label").cloned().unwrap_or(Value::Null),
            "tail": row.get("tail").cloned().unwrap_or(Value::Null),
            "src": airframes_addr_value(row, "from_hex"),
            "dst": airframes_addr_value(row, "to_hex"),
            "source_type": row.get("source_type").cloned().unwrap_or(Value::Null),
            "frequency": row.get("frequency").cloned().unwrap_or(Value::Null),
            "app": Value::Null,
        }));
    };

    let normalized_text = normalize_arinc622_text(text).unwrap_or_else(|| text.to_string());
    let app = decode_acars_text_payload(label, None, &normalized_text, direction);
    let raw_val = serde_json::json!({
        "timestamp": timestamp,
        "label": label,
        "tail": row.get("tail").cloned().unwrap_or(Value::Null),
        "text": text,
        "direction": direction,
        "src": airframes_addr_value(row, "from_hex"),
        "dst": airframes_addr_value(row, "to_hex"),
        "data": app,
        "metadata": {
            "bearer": "airframes.io",
            "source": row.get("source").cloned().unwrap_or(Value::Null),
            "source_type": row.get("source_type").cloned().unwrap_or(Value::Null),
            "frequency": row.get("frequency").cloned().unwrap_or(Value::Null),
            "link_direction": link_direction,
            "airframes_id": row.get("id").cloned().unwrap_or(Value::Null),
        }
    });
    Some(acars::decode::compact::compact_acars_value(
        raw_val,
        include_raw,
    ))
}

fn airframes_addr_value(row: &Value, key: &str) -> Value {
    let Some(hex) = row.get(key).and_then(Value::as_str) else {
        return Value::Null;
    };
    let addr = hex.trim().to_ascii_lowercase();
    if addr.len() != 6 || !addr.chars().all(|c| c.is_ascii_hexdigit()) {
        return serde_json::json!({ "icao24": addr });
    }
    let aircraft_icao = row
        .pointer("/airframe/icao")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);
    let addr_type = if aircraft_icao.as_deref() == Some(addr.as_str()) {
        "aircraft"
    } else {
        "ground_station"
    };
    serde_json::json!({ "icao24": addr, "type": addr_type })
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

fn infer_bearer(record: &Value) -> Bearer {
    let source_type = record
        .pointer("/decoded/metadata/source_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if source_type.contains("vdl") {
        Bearer::Vdl2
    } else if source_type.contains("hfdl") || source_type.contains("hf") {
        Bearer::Hfdl
    } else if source_type.contains("acars") || source_type.contains("vhf") {
        Bearer::Vhf
    } else if record.get("decoded").is_some_and(|v| !v.is_null()) {
        Bearer::Decoded
    } else {
        Bearer::Unknown
    }
}

fn aircraft_summary(record: &Value) -> Option<Value> {
    let mut obj = serde_json::Map::new();
    if let Some(icao) = record
        .pointer("/decoded/src/icao24")
        .and_then(Value::as_str)
    {
        obj.insert("icao24".into(), Value::from(icao));
    }
    if obj.is_empty() {
        None
    } else {
        Some(Value::Object(obj))
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
    fn normalize_payload_sets_bearer() {
        let meta = SourceMetadata {
            id: "airframes".into(),
            name: "Airframes".into(),
            class: SourceClass::Events,
            format: Some("airframes.io".into()),
        };
        let payload = serde_json::json!({"label":"SA", "text":"hello", "from_hex":"A1B2C3", "source_type":"vdl2", "timestamp": 123.0});
        let event = normalize_payload(meta, "message", payload, true).unwrap();
        assert_eq!(event.bearer, Bearer::Vdl2);
        assert!(event.raw.is_some());
    }

    #[test]
    fn normalize_payload_converts_timestamp_string() {
        let meta = SourceMetadata {
            id: "airframes".into(),
            name: "Airframes".into(),
            class: SourceClass::Events,
            format: Some("airframes.io".into()),
        };
        let payload = serde_json::json!({"label":"SA", "text":"hello", "from_hex":"A1B2C3", "source_type":"vdl2", "timestamp": "2026-05-22T08:37:19.050Z"});
        let event = normalize_payload(meta, "message", payload, false).unwrap();
        assert_eq!(event.timestamp, Some(1779439039.05));
        assert_eq!(
            event.message.get("timestamp").and_then(Value::as_f64),
            Some(1779439039.05)
        );
    }
}
