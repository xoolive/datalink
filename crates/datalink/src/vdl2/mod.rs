mod source;

use acars::decode::acars::{decode_acars_text_payload, MessageDirection};
use acars::decode::avlc::parse_avlc_frame;
use acars::demod::resample::{maybe_resample, ResampleAdapter};
use acars::demod::vdl2::{Vdl2Channel, SYMBOL_RATE};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use http::Uri;
use serde::Deserialize;
use serde_json::Value;
use source::{Address, Source, DEFAULT_CHUNK_SIZE};
use std::fs::{create_dir_all, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Default, Clone, Deserialize, Parser)]
#[command(about = "VDL2 frontend for I/Q and SDR inputs")]
pub(crate) struct Options {
    /// Dump a copy of decoded AVLC frames as JSONL
    #[arg(short, long)]
    output: Option<String>,
    /// Print demod/decode counters to stderr at end
    #[arg(long)]
    #[serde(default)]
    stats: bool,
    /// Write rejected AVLC frames as NDJSON (frame + parse error)
    #[arg(long)]
    reject_log: Option<String>,
    /// Include the full nested decoder output under raw_decode
    #[arg(long)]
    #[serde(default)]
    raw: bool,
    /// Include frames with bad AVLC FCS in output JSON
    #[arg(long)]
    #[serde(default)]
    include_fcs_fail: bool,
    /// Write all AVLC candidates (ok/fail) as NDJSON
    #[arg(long)]
    candidate_log: Option<String>,
    /// Candidate log/output lower bound (seconds into recording)
    #[arg(long)]
    window_start_sec: Option<f64>,
    /// Candidate log/output upper bound (seconds into recording)
    #[arg(long)]
    window_end_sec: Option<f64>,
    /// Directory for per-channel demod trace NDJSON logs
    #[arg(long)]
    demod_trace_dir: Option<String>,
    /// Preamble sync threshold
    #[arg(long)]
    #[serde(default)]
    sync_threshold: Option<f32>,
    /// I/Q sample format for file input: cu8, cs8, cs16, cf32
    #[arg(long, default_value = "cu8")]
    #[serde(default)]
    format: Option<String>,
    /// Center frequency for file and SDR sources
    #[arg(long)]
    #[serde(default)]
    center_freq: Option<u32>,
    /// Sample rate for file and SDR sources
    #[arg(long)]
    #[serde(default)]
    sample_rate: Option<u32>,
    /// VDL2 channel frequencies in Hz
    #[arg(long, num_args = 1..)]
    #[serde(default)]
    channel: Option<Vec<u32>>,
    /// Source URL: file://, rtlsdr://, airspy://, hackrf://, soapy://
    #[serde(default)]
    source: Option<Source>,
}

#[derive(Default)]
struct DecodeStats {
    demod_frames: u64,
    avlc_ok: u64,
    avlc_fcs_ok: u64,
    avlc_fcs_fail: u64,
    avlc_parse_fail: u64,
}

pub(crate) async fn run(cli: Options) -> anyhow::Result<()> {
    let mut options = Options::default();
    merge_cli(&mut options, cli)?;

    anyhow::ensure!(
        options.source.is_some(),
        "missing source; pass an explicit source such as file://capture.rtl, -, or rtlsdr://"
    );
    if matches!(
        options.source.as_ref().map(|src| &src.address),
        Some(Address::Websocket { .. })
    ) {
        anyhow::bail!("websocket/Airframes.io sources belong to `datalink airframes.io`");
    }
    run_options(options, "vdl2").await
}

pub(crate) async fn run_airframes_simple(
    source: Option<String>,
    output: Option<String>,
    stats: bool,
    raw: bool,
) -> anyhow::Result<()> {
    let source = source.unwrap_or_else(|| "airframes://".to_string());
    let options = Options {
        output,
        stats,
        raw,
        source: Some(source.parse().map_err(anyhow::Error::msg)?),
        ..Options::default()
    };

    run_options(options, "airframes.io").await
}

async fn run_options(options: Options, stats_name: &str) -> anyhow::Result<()> {
    let mut output = if let Some(path) = options.output.as_deref() {
        Some(BufWriter::new(File::create(expanduser(path))?))
    } else {
        None
    };
    let mut reject_writer = if let Some(path) = options.reject_log.as_deref() {
        Some(BufWriter::new(File::create(expanduser(path))?))
    } else {
        None
    };
    let mut candidate_writer = if let Some(path) = options.candidate_log.as_deref() {
        Some(BufWriter::new(File::create(expanduser(path))?))
    } else {
        None
    };

    let mut total = DecodeStats::default();
    let src = options.source.as_ref().expect("source checked before run");
    let stats = decode_source(
        src,
        0,
        &options,
        output.as_mut(),
        reject_writer.as_mut(),
        candidate_writer.as_mut(),
    )
    .await?;
    total.demod_frames += stats.demod_frames;
    total.avlc_ok += stats.avlc_ok;
    total.avlc_fcs_ok += stats.avlc_fcs_ok;
    total.avlc_fcs_fail += stats.avlc_fcs_fail;
    total.avlc_parse_fail += stats.avlc_parse_fail;

    if let Some(w) = output.as_mut() {
        w.flush()?;
    }
    if let Some(w) = reject_writer.as_mut() {
        w.flush()?;
    }
    if let Some(w) = candidate_writer.as_mut() {
        w.flush()?;
    }

    if options.stats {
        eprintln!(
            "{} stats: demod_frames={} avlc_ok={} avlc_fcs_ok={} avlc_fcs_fail={} avlc_parse_fail={}",
            stats_name, total.demod_frames, total.avlc_ok, total.avlc_fcs_ok, total.avlc_fcs_fail, total.avlc_parse_fail
        );
    }

    Ok(())
}

fn merge_cli(options: &mut Options, cli: Options) -> anyhow::Result<()> {
    if cli.output.is_some() {
        options.output = cli.output;
    }
    if cli.stats {
        options.stats = true;
    }
    if cli.reject_log.is_some() {
        options.reject_log = cli.reject_log;
    }
    if cli.raw {
        options.raw = true;
    }
    if cli.include_fcs_fail {
        options.include_fcs_fail = true;
    }
    if cli.candidate_log.is_some() {
        options.candidate_log = cli.candidate_log;
    }
    if cli.window_start_sec.is_some() {
        options.window_start_sec = cli.window_start_sec;
    }
    if cli.window_end_sec.is_some() {
        options.window_end_sec = cli.window_end_sec;
    }
    if cli.demod_trace_dir.is_some() {
        options.demod_trace_dir = cli.demod_trace_dir;
    }
    if cli.sync_threshold.is_some() {
        options.sync_threshold = cli.sync_threshold;
    }
    if cli.source.is_some() {
        options.source = cli.source;
    }
    if cli.center_freq.is_some() {
        options.center_freq = cli.center_freq;
    }
    if cli.sample_rate.is_some() {
        options.sample_rate = cli.sample_rate;
    }
    if cli.channel.is_some() {
        options.channel = cli.channel;
    }
    if cli.format.as_deref() != Some("cu8") {
        options.format = cli.format;
    }
    apply_source_overrides(options);
    Ok(())
}

fn apply_source_overrides(options: &mut Options) {
    let Some(source) = options.source.as_mut() else {
        return;
    };
    if options.center_freq.is_some() {
        source.center_freq = options.center_freq;
    }
    if options.sample_rate.is_some() {
        source.sample_rate = options.sample_rate;
    }
    if options.channel.is_some() {
        source.channels = options.channel.clone();
    }
    if options.format.as_deref() != Some("cu8") {
        source.format = options.format.clone();
    }
}

async fn decode_source(
    src: &Source,
    source_index: usize,
    options: &Options,
    mut output: Option<&mut BufWriter<File>>,
    mut reject_writer: Option<&mut BufWriter<File>>,
    mut candidate_writer: Option<&mut BufWriter<File>>,
) -> anyhow::Result<DecodeStats> {
    if matches!(src.address, Address::Websocket { .. }) {
        return decode_websocket_source(src, source_index, options.raw, output).await;
    }

    let center_freq = src.center_freq();
    let raw_sample_rate = src.sample_rate();
    let channels = src.channels();
    let source_label = src.label();
    let sync_threshold = options.sync_threshold.unwrap_or(3.2);

    // Compute the nearest valid VDL2 demod rate (integer multiple of SYMBOL_RATE * SPS = 105 000)
    // and set up a transparent resampler if the source rate is not already valid.
    let vdl2_decimated_rate = SYMBOL_RATE * 10; // 105_000
    let (sample_rate, resample_rs) = maybe_resample(raw_sample_rate, vdl2_decimated_rate);
    let mut adapter = ResampleAdapter::new(resample_rs);
    if sample_rate != raw_sample_rate {
        eprintln!(
            "datalink vdl2: resampling {:.3} MHz → {:.3} MHz for VDL2 demod",
            raw_sample_rate as f64 / 1e6,
            sample_rate as f64 / 1e6
        );
    }

    let mut demods: Vec<Vdl2Channel> = channels
        .iter()
        .map(|&ch_freq| {
            let mut d = Vdl2Channel::new(
                sample_rate as f32,
                ch_freq as f32 - center_freq as f32,
                ch_freq as f32,
            );
            d.set_sync_threshold(sync_threshold);
            d
        })
        .collect();

    if let Some(dir) = options.demod_trace_dir.as_deref() {
        create_dir_all(dir)?;
        for (idx, d) in demods.iter_mut().enumerate() {
            let path = format!("{dir}/src_{source_index}_ch_{}.ndjson", channels[idx]);
            d.enable_trace(&path, options.window_start_sec, options.window_end_sec)?;
        }
    }

    let mut stream = open_source(src).await?;
    let run_start = SystemTime::now();
    let mut sample_index: u64 = 0;
    let mut stats = DecodeStats::default();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        for raw_sample in &chunk {
            // Feed through the resampler (passthrough if rate is already valid)
            for sample in adapter.feed(raw_sample.re, raw_sample.im) {
                sample_index = sample_index.saturating_add(1);
                let seconds_into_recording = sample_index as f64 / sample_rate as f64;
                let frame_ts = run_start + Duration::from_secs_f64(seconds_into_recording);
                let timestamp_unix = frame_ts
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or_default();
                for (idx, d) in demods.iter_mut().enumerate() {
                    for demod_frame in d.process_sample(sample.re, sample.im) {
                        stats.demod_frames += 1;
                        match parse_avlc_frame(&demod_frame.bytes) {
                            Ok(avlc) => {
                                stats.avlc_ok += 1;
                                if avlc.fcs_ok {
                                    stats.avlc_fcs_ok += 1;
                                } else {
                                    stats.avlc_fcs_fail += 1;
                                }
                                if let Some(w) = candidate_writer.as_mut() {
                                    if in_window(
                                        seconds_into_recording,
                                        options.window_start_sec,
                                        options.window_end_sec,
                                    ) {
                                        let lcf_key = match &avlc.lcf {
                                            acars::decode::avlc::AvlcLcf::I { .. } => "I",
                                            acars::decode::avlc::AvlcLcf::S { .. } => "S",
                                            acars::decode::avlc::AvlcLcf::U { .. } => "U",
                                        };
                                        let payload = avlc
                                            .payload
                                            .as_ref()
                                            .map(|p| match p {
                                                acars::decode::avlc::AvlcPayload::Acars(_) => {
                                                    "Acars"
                                                }
                                                acars::decode::avlc::AvlcPayload::X25(_) => "X25",
                                                acars::decode::avlc::AvlcPayload::Xid(_) => "Xid",
                                                acars::decode::avlc::AvlcPayload::Unknown(_) => {
                                                    "Unknown"
                                                }
                                            })
                                            .unwrap_or("None");
                                        let cand = serde_json::json!({"source": source_label, "source_index": source_index, "sample_index": sample_index, "seconds_into_recording": seconds_into_recording, "channel_mhz": channel_hz(channels[idx]), "parse_ok": true, "fcs_ok": avlc.fcs_ok, "src": avlc.src.icao24, "dst": avlc.dst.icao24, "role": avlc.role, "lcf": lcf_key, "payload_class": payload, "frame": bytes_to_hex(&demod_frame.bytes)});
                                        writeln!(w, "{}", serde_json::to_string(&cand)?)?;
                                    }
                                }
                                if !options.include_fcs_fail && !avlc.fcs_ok {
                                    continue;
                                }
                                if !in_window(
                                    seconds_into_recording,
                                    options.window_start_sec,
                                    options.window_end_sec,
                                ) {
                                    continue;
                                }
                                let _snr_db = demod_frame.signal_dbfs - demod_frame.noise_dbfs;
                                let channel_hz = channels[idx] as u64;
                                let mut obj = serde_json::to_value(&avlc)?;
                                if let serde_json::Value::Object(ref mut m) = obj {
                                    m.insert("timestamp".into(), timestamp_unix.into());
                                    m.insert(
                                        "frame".into(),
                                        bytes_to_hex(&demod_frame.bytes).into(),
                                    );
                                    m.insert(
                                        "metadata".into(),
                                        serde_json::json!({
                                            "bearer": "vdl2",
                                            "channel_mhz": channel_hz as f64 / 1_000_000.0,
                                        }),
                                    );
                                }
                                let obj =
                                    acars::decode::compact::compact_avlc_value(obj, options.raw);
                                let line = serde_json::to_string(&obj)?;
                                println!("{line}");
                                if let Some(w) = output.as_mut() {
                                    writeln!(w, "{line}")?;
                                }
                            }
                            Err(err) => {
                                stats.avlc_parse_fail += 1;
                                if let Some(w) = candidate_writer.as_mut() {
                                    if in_window(
                                        seconds_into_recording,
                                        options.window_start_sec,
                                        options.window_end_sec,
                                    ) {
                                        let cand = serde_json::json!({"source": source_label, "source_index": source_index, "sample_index": sample_index, "seconds_into_recording": seconds_into_recording, "channel_mhz": channel_hz(channels[idx]), "parse_ok": false, "parse_error": err.to_string(), "frame": bytes_to_hex(&demod_frame.bytes)});
                                        writeln!(w, "{}", serde_json::to_string(&cand)?)?;
                                    }
                                }
                                if let Some(w) = reject_writer.as_mut() {
                                    if in_window(
                                        seconds_into_recording,
                                        options.window_start_sec,
                                        options.window_end_sec,
                                    ) {
                                        let reject = serde_json::json!({"source": source_label, "source_index": source_index, "sample_index": sample_index, "seconds_into_recording": seconds_into_recording, "channel_mhz": channel_hz(channels[idx]), "frame_len": demod_frame.bytes.len(), "parse_error": err.to_string(), "frame": bytes_to_hex(&demod_frame.bytes)});
                                        writeln!(w, "{}", serde_json::to_string(&reject)?)?;
                                    }
                                }
                            }
                        }
                    } // end for demod_frame
                } // end for (idx, d)
            } // end for sample in adapter.feed
        } // end for raw_sample in chunk
    }
    Ok(stats)
}

async fn decode_websocket_source(
    src: &Source,
    source_index: usize,
    raw: bool,
    mut output: Option<&mut BufWriter<File>>,
) -> anyhow::Result<DecodeStats> {
    let Address::Websocket {
        websocket,
        token,
        events,
    } = &src.address
    else {
        unreachable!("decode_websocket_source called for non-websocket source")
    };
    let selected_events = events
        .clone()
        .unwrap_or_else(|| vec!["message".to_string()]);
    let capture_all = selected_events.iter().any(|event| event == "*");
    let mut request =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
            websocket.as_str(),
        )?;
    request.headers_mut().insert(
        "Origin",
        tokio_tungstenite::tungstenite::http::HeaderValue::from_static("https://app.airframes.io"),
    );
    let mut ws = websocket_connect(websocket, request).await?;
    while let Some(message) = ws.next().await {
        let message = message?;
        if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
            if text.starts_with('0') {
                break;
            }
        }
    }
    ws.send(tokio_tungstenite::tungstenite::Message::Text(format!(
        "40{}",
        serde_json::to_string(&serde_json::json!({ "token": token.as_deref().unwrap_or("") }))?
    )))
    .await?;

    let mut stats = DecodeStats::default();
    while let Some(message) = ws.next().await {
        let message = message?;
        match message {
            tokio_tungstenite::tungstenite::Message::Text(text) => {
                for packet in text.split('\u{1e}') {
                    if packet == "2" {
                        ws.send(tokio_tungstenite::tungstenite::Message::Text(
                            "3".to_string(),
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
                    stats.avlc_ok += 1;
                    let record = websocket_record(src, source_index, &event, payload, raw);
                    let line = serde_json::to_string(&record)?;
                    println!("{line}");
                    if let Some(w) = output.as_mut() {
                        writeln!(w, "{line}")?;
                    }
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
    Ok(stats)
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

fn websocket_record(
    src: &Source,
    source_index: usize,
    event: &str,
    payload: Value,
    raw: bool,
) -> Value {
    let decoded = if event == "message" {
        decode_airframes_message(&payload, raw)
    } else {
        None
    };
    let mut record = serde_json::json!({
        "source": src.label(),
        "source_index": source_index,
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
        .cloned()
        .unwrap_or(Value::Null);

    let Some(text) = text else {
        return Some(serde_json::json!({
            "path": "unknown",
            "message_class": "metadata_only",
            "summary": "Airframes VDL row without ACARS text payload",
            "airframes_id": row.get("id").cloned().unwrap_or(Value::Null),
            "timestamp": timestamp,
            "label": row.get("label").cloned().unwrap_or(Value::Null),
            "tail": row.get("tail").cloned().unwrap_or(Value::Null),
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

/// Connect to a websocket, routing through an HTTP CONNECT proxy if
/// HTTPS_PROXY / https_proxy is set in the environment (matching the behaviour
/// of the Python collection script).
async fn websocket_connect(
    url: &str,
    request: tokio_tungstenite::tungstenite::handshake::client::Request,
) -> anyhow::Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    // Install a CryptoProvider if none has been installed yet (required by rustls 0.23+).
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

        // TLS-wrap the tunnelled TCP stream with rustls, then hand to tungstenite.
        let root_store = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let tls_config = std::sync::Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth(),
        );
        let connector = tokio_rustls::TlsConnector::from(tls_config.clone());
        let domain = rustls::pki_types::ServerName::try_from(target_host.clone())
            .map_err(|e| anyhow::anyhow!("invalid TLS hostname {target_host}: {e}"))?;
        let tls = connector.connect(domain, tcp).await?;
        // Wrap the TlsStream in MaybeTlsStream so the return type matches the non-proxy path.
        let maybe_tls = tokio_tungstenite::MaybeTlsStream::Rustls(tls);
        let (ws, _) = tokio_tungstenite::client_async_with_config(request, maybe_tls, None).await?;
        Ok(ws)
    } else {
        Ok(tokio_tungstenite::connect_async(request).await?.0)
    }
}

async fn open_source(src: &Source) -> anyhow::Result<desperado::IqAsyncSource> {
    use desperado::IqAsyncSource;
    let center_freq = src.center_freq();
    let sample_rate = src.sample_rate();
    match &src.address {
        Address::File { file } if file == "-" => Ok(IqAsyncSource::from_stdin(
            center_freq,
            sample_rate,
            DEFAULT_CHUNK_SIZE,
            src.iq_format(),
        )),
        Address::File { file } => Ok(IqAsyncSource::from_file(
            file,
            center_freq,
            sample_rate,
            DEFAULT_CHUNK_SIZE,
            src.iq_format(),
        )
        .await?),
        #[cfg(feature = "rtlsdr")]
        Address::Rtlsdr { device, serial } => {
            let selector = if let Some(serial) = serial {
                desperado::rtlsdr::DeviceSelector::Filter {
                    manufacturer: None,
                    product: None,
                    serial: Some(serial.clone()),
                }
            } else {
                desperado::rtlsdr::DeviceSelector::Index(device.unwrap_or(0))
            };
            let cfg = desperado::rtlsdr::RtlSdrConfig {
                device: selector,
                center_freq,
                sample_rate,
                gain: src.gain(49.6),
                bias_tee: src.bias_tee.unwrap_or(false),
                freq_correction_ppm: 0,
            };
            Ok(IqAsyncSource::from_device_config(&desperado::DeviceConfig::RtlSdr(cfg)).await?)
        }
        #[cfg(feature = "airspy")]
        Address::Airspy { device, serial } => {
            let selector = if let Some(serial) = serial {
                desperado::airspy::DeviceSelector::Serial(parse_airspy_serial(serial)?)
            } else {
                desperado::airspy::DeviceSelector::Index(device.unwrap_or(0))
            };
            let cfg = desperado::airspy::AirspyConfig {
                device: selector,
                center_freq,
                sample_rate,
                gain: src.gain(50.0),
                bias_tee: src.bias_tee.unwrap_or(false),
                packing: false,
                lna_gain: None,
                mixer_gain: None,
                vga_gain: None,
                gain_mode: desperado::airspy::AirspyGainMode::Sensitivity,
            };
            Ok(IqAsyncSource::from_device_config(&desperado::DeviceConfig::Airspy(cfg)).await?)
        }
        #[cfg(feature = "hackrf")]
        Address::Hackrf { device } => {
            let cfg = desperado::hackrf::HackRfConfig {
                device_index: device.unwrap_or(0),
                center_freq: center_freq as u64,
                sample_rate,
                gain: hackrf_gain(src),
                amp_enable: src.amp_enable.unwrap_or(false),
                bias_tee: src.bias_tee.unwrap_or(false),
            };
            Ok(IqAsyncSource::from_device_config(&desperado::DeviceConfig::HackRf(cfg)).await?)
        }
        #[cfg(feature = "soapy")]
        Address::Soapy { soapy } => {
            let cfg = desperado::soapy::SoapyConfig {
                args: soapy.clone(),
                center_freq: center_freq as f64,
                sample_rate: sample_rate as f64,
                channel: 0,
                gain: src.gain(49.6),
                bias_tee: src.bias_tee.unwrap_or(false),
            };
            Ok(IqAsyncSource::from_device_config(&desperado::DeviceConfig::Soapy(cfg)).await?)
        }
        #[allow(unreachable_patterns)]
        _ => Err(anyhow::anyhow!("source type is not enabled in this build")),
    }
}

#[cfg(feature = "hackrf")]
fn hackrf_gain(src: &Source) -> desperado::Gain {
    let mut elements = Vec::new();
    if let Some(value_db) = src.lna_gain {
        elements.push(desperado::GainElement {
            name: desperado::GainElementName::Lna,
            value_db,
        });
    }
    if let Some(value_db) = src.vga_gain {
        elements.push(desperado::GainElement {
            name: desperado::GainElementName::Vga,
            value_db,
        });
    }
    if elements.is_empty() {
        src.gain(30.0)
    } else {
        desperado::Gain::Elements(elements)
    }
}

#[cfg(feature = "airspy")]
fn parse_airspy_serial(value: &str) -> anyhow::Result<u64> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return Ok(u64::from_str_radix(hex, 16)?);
    }
    value
        .parse::<u64>()
        .or_else(|_| u64::from_str_radix(value, 16))
        .map_err(Into::into)
}

fn expanduser(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn channel_hz(hz: u32) -> f64 {
    hz as f64 / 1_000_000.0
}

fn in_window(seconds: f64, start: Option<f64>, end: Option<f64>) -> bool {
    if let Some(s) = start {
        if seconds < s {
            return false;
        }
    }
    if let Some(e) = end {
        if seconds > e {
            return false;
        }
    }
    true
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join("")
}
