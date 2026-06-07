use acars::decode::{
    acars::{parse_acars_frame, MessageDirection},
    compact::compact_value,
};
use acars::demod::hfdl::{diagnose_channel, HfdlDemodConfig};
use clap::{Parser, ValueEnum};
use rustfft::num_complex::Complex;
use serde_json::json;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::PathBuf;

const DEFAULT_HFDL_CHANNELS_KHZ: &[f64] = &[
    6529.0, 6532.0, 6535.0, 6559.0, 6565.0, 6589.0, 6596.0, 6619.0, 6628.0, 6646.0, 6652.0, 6661.0,
    6712.0, 8825.0, 8834.0, 8843.0, 8885.0, 8886.0, 8894.0, 8912.0, 8921.0, 8927.0, 8936.0, 8939.0,
    8942.0, 8948.0, 8957.0, 8977.0, 10027.0, 10030.0, 10060.0, 10063.0, 10066.0, 10081.0, 10084.0,
    10087.0, 10093.0, 11184.0, 11306.0, 11312.0, 11318.0, 11321.0, 11327.0, 11348.0, 11354.0,
    11384.0, 11387.0, 13264.0, 13270.0, 13276.0, 13303.0, 13312.0, 13315.0, 13321.0, 13324.0,
    13342.0, 13351.0,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum SampleFormat {
    U8,
    Cs16,
    Cf32,
    /// 16-bit stereo WAV I/Q file (auto-selected for .wav sources when --format is omitted).
    Wav16,
}

impl SampleFormat {
    fn bytes_per_complex(self) -> usize {
        match self {
            Self::U8 => 2,
            Self::Cs16 => 4,
            Self::Cf32 => 8,
            Self::Wav16 => 4,
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "HF Data Link decoder for I/Q file captures")]
pub(crate) struct Options {
    /// I/Q file source, e.g. file://capture.raw or ~/capture.raw.
    source: Option<String>,

    /// I/Q sample format
    #[arg(long, value_enum, default_value_t = SampleFormat::Cf32)]
    format: SampleFormat,

    /// Recording center frequency in Hz
    #[arg(long, default_value_t = 10_000_000)]
    center_freq: u32,

    /// Recording sample rate in samples/s
    #[arg(long, default_value_t = 8_000_000)]
    sample_rate: u32,

    /// HFDL channel frequencies, in Hz or kHz. Defaults to known channels within the recording bandwidth.
    #[arg(long, num_args = 1..)]
    channel: Option<Vec<f64>>,

    /// Start offset in seconds for file decoding
    #[arg(long, default_value_t = 0.0)]
    start_second: f64,

    /// Maximum seconds to decode from the file
    #[arg(long, default_value_t = 20.0)]
    max_seconds: f64,

    /// Print demod/decode counters to stderr at end
    #[arg(long)]
    stats: bool,
}

pub(crate) fn run(options: Options) -> anyhow::Result<()> {
    decode_mode(&options)
}

pub(crate) fn decode_file_values(
    source: &str,
    format: Option<&str>,
    center_freq: Option<u32>,
    sample_rate: Option<u32>,
    channels: Option<Vec<u32>>,
    start_second: f64,
    max_seconds: f64,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let options = Options {
        source: Some(source.to_string()),
        format: format
            .and_then(parse_sample_format)
            .unwrap_or(SampleFormat::Cf32),
        center_freq: center_freq.unwrap_or(10_000_000),
        sample_rate: sample_rate.unwrap_or(8_000_000),
        channel: channels.map(|v| v.into_iter().map(|hz| hz as f64).collect()),
        start_second,
        max_seconds,
        stats: false,
    };
    collect_decoded_pdus(&options)
}

fn parse_sample_format(value: &str) -> Option<SampleFormat> {
    match value.to_ascii_lowercase().as_str() {
        "u8" | "cu8" => Some(SampleFormat::U8),
        "cs16" => Some(SampleFormat::Cs16),
        "cf32" => Some(SampleFormat::Cf32),
        "wav16" | "wav" => Some(SampleFormat::Wav16),
        _ => None,
    }
}

fn decode_mode(options: &Options) -> anyhow::Result<()> {
    for parsed in collect_decoded_pdus(options)? {
        println!("{}", serde_json::to_string(&parsed)?)
    }
    Ok(())
}

fn collect_decoded_pdus(options: &Options) -> anyhow::Result<Vec<serde_json::Value>> {
    let source = options
        .source
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("missing source; pass an explicit I/Q file source"))?;
    let path = normalize_source_path(source);
    let format = effective_format(&path, options.format);
    let sample_rate = effective_sample_rate(&path, format, options.sample_rate)?;
    let center_freq = effective_center_freq(&path, format, options.center_freq);
    let channels = channels_khz_for(options, sample_rate, center_freq);
    anyhow::ensure!(
        !channels.is_empty(),
        "no HFDL channels selected; pass --channel or use a wider/centered recording"
    );
    let samples = read_complex_window(
        &path,
        format,
        sample_rate,
        options.start_second,
        options.max_seconds,
    )?;
    let mut pdu_ok = 0u64;
    let mut candidate_count = 0u64;
    let mut frame_sync_count = 0u64;
    let mut out = Vec::new();
    for &channel_khz in &channels {
        let diagnostics = diagnose_channel(
            &samples,
            &HfdlDemodConfig {
                input_sample_rate: sample_rate,
                center_freq_hz: center_freq as f64,
                channel_khz,
                use_symbol_sync: true,
            },
        )
        .map_err(anyhow::Error::msg)?;
        frame_sync_count += diagnostics.frame_hits.len() as u64;
        candidate_count += diagnostics.pdu_candidates.len() as u64;
        for candidate in &diagnostics.pdu_candidates {
            let mut parsed = parse_hfdl_pdu(&candidate.bytes);
            if parsed.get("fcs_ok").and_then(|v| v.as_bool()) != Some(true) {
                continue;
            }
            pdu_ok += 1;
            if let Some(obj) = parsed.as_object_mut() {
                obj.insert("event".into(), "pdu".into());
                obj.insert("channel_khz".into(), channel_khz.into());
                obj.insert("m1".into(), candidate.m1.into());
                obj.insert("raw_hex".into(), hex::encode_upper(&candidate.bytes).into());
            }
            out.push(parsed);
        }
    }
    if options.stats {
        eprintln!(
            "datalink hfdl stats: channels={} frame_sync={} candidates={} pdu_ok={}",
            channels.len(),
            frame_sync_count,
            candidate_count,
            pdu_ok
        );
    }
    Ok(out)
}

fn read_complex_window(
    path: &str,
    format: SampleFormat,
    sample_rate: u32,
    start_second: f64,
    max_seconds: f64,
) -> anyhow::Result<Vec<Complex<f32>>> {
    if format == SampleFormat::Wav16 {
        return read_wav_complex_window(path, start_second, max_seconds);
    }
    let mut reader = BufReader::new(File::open(path)?);
    seek_to_second(&mut reader, format, sample_rate, start_second)?;
    let count = (sample_rate as f64 * max_seconds).ceil() as usize;
    let mut raw = vec![0u8; count * format.bytes_per_complex()];
    let mut filled = 0usize;
    while filled < raw.len() {
        let n = reader.read(&mut raw[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    raw.truncate(filled - (filled % format.bytes_per_complex()));
    let mut out = vec![Complex::new(0.0f32, 0.0f32); raw.len() / format.bytes_per_complex()];
    decode_complex_bytes(format, &raw, &mut out);
    Ok(out)
}

fn read_wav_complex_window(
    path: &str,
    start_second: f64,
    max_seconds: f64,
) -> anyhow::Result<Vec<Complex<f32>>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.channels == 2,
        "HFDL WAV input expects stereo I/Q, got {} channels",
        spec.channels
    );
    anyhow::ensure!(
        spec.sample_format == hound::SampleFormat::Int && spec.bits_per_sample == 16,
        "HFDL WAV input currently supports 16-bit PCM stereo only"
    );
    let start_frames = (start_second * spec.sample_rate as f64).round() as usize;
    let max_frames = (max_seconds * spec.sample_rate as f64).ceil() as usize;
    let mut samples = reader.samples::<i16>();
    for _ in 0..start_frames.saturating_mul(2) {
        if samples.next().is_none() {
            return Ok(Vec::new());
        }
    }
    let mut out = Vec::with_capacity(max_frames);
    for _ in 0..max_frames {
        let Some(i) = samples.next() else { break };
        let Some(q) = samples.next() else { break };
        out.push(Complex::new(i? as f32 / 32768.0, q? as f32 / 32768.0));
    }
    Ok(out)
}

fn seek_to_second<R: Seek>(
    reader: &mut R,
    format: SampleFormat,
    sample_rate: u32,
    start_second: f64,
) -> anyhow::Result<()> {
    if start_second <= 0.0 {
        return Ok(());
    }
    let byte_offset =
        (start_second * sample_rate as f64).round() as u64 * format.bytes_per_complex() as u64;
    reader.seek(SeekFrom::Start(byte_offset))?;
    Ok(())
}

fn decode_complex_bytes(format: SampleFormat, raw: &[u8], out: &mut [Complex<f32>]) {
    match format {
        SampleFormat::U8 => {
            for (idx, chunk) in raw.chunks_exact(2).enumerate() {
                out[idx].re = (chunk[0] as f32 - 127.5) / 128.0;
                out[idx].im = (chunk[1] as f32 - 127.5) / 128.0;
            }
        }
        SampleFormat::Cs16 => {
            for (idx, chunk) in raw.chunks_exact(4).enumerate() {
                let i = i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0;
                let q = i16::from_le_bytes([chunk[2], chunk[3]]) as f32 / 32768.0;
                out[idx].re = i;
                out[idx].im = q;
            }
        }
        SampleFormat::Wav16 => unreachable!("WAV samples are decoded by read_wav_complex_window"),
        SampleFormat::Cf32 => {
            for (idx, chunk) in raw.chunks_exact(8).enumerate() {
                out[idx].re = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                out[idx].im = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
            }
        }
    }
}

fn parse_hfdl_pdu(buf: &[u8]) -> serde_json::Value {
    if buf.is_empty() {
        return json!({ "bearer": "hfdl", "parse_ok": false, "error": "empty PDU" });
    }
    if buf[0] & 1 != 0 {
        parse_mpdu(buf)
    } else {
        parse_spdu(buf)
    }
}

fn parse_spdu(buf: &[u8]) -> serde_json::Value {
    let fcs_ok = hfdl_fcs_ok(buf, 64);
    let mut out = json!({
        "bearer": "hfdl",
        "pdu": "spdu",
        "parse_ok": buf.len() >= 66,
        "fcs_ok": fcs_ok,
        "len": buf.len(),
        "ground_station_id": buf.get(1).map(|v| v & 0x7f),
    });
    if buf.len() >= 66 {
        let obj = out.as_object_mut().unwrap();
        obj.insert("version".into(), ((buf[0] >> 2) & 3).into());
        obj.insert("rls_in_use".into(), (buf[0] & 2 != 0).into());
        obj.insert("iso8208_supported".into(), (buf[0] & 0x20 != 0).into());
        obj.insert(
            "change_note".into(),
            spdu_change_note((buf[0] & 0xc0) >> 6).into(),
        );
        obj.insert(
            "tdma_frame_index".into(),
            ((buf[2] as u16) | (((buf[3] & 0x0f) as u16) << 8)).into(),
        );
        obj.insert("tdma_frame_offset".into(), (buf[3] >> 4).into());
        obj.insert("min_priority".into(), (buf[52] & 0x0f).into());
        obj.insert(
            "system_table_version".into(),
            ((buf[53] as u16) | (((buf[54] & 0x0f) as u16) << 8)).into(),
        );
        obj.insert("ground_stations".into(), json!([
            {"id": buf[1] & 0x7f, "utc_sync": buf[1] & 0x80 != 0, "frequencies_in_use_mask": ((buf[54] >> 4) as u32) | ((buf[55] as u32) << 4) | ((buf[56] as u32) << 12)},
            {"id": buf[57] & 0x7f, "utc_sync": buf[57] & 0x80 != 0, "frequencies_in_use_mask": (buf[58] as u32) | ((buf[59] as u32) << 8) | (((buf[60] & 0x0f) as u32) << 16)},
            {"id": (buf[60] >> 4) | ((buf[61] & 0x07) << 4), "utc_sync": buf[61] & 0x08 != 0, "frequencies_in_use_mask": ((buf[61] >> 4) as u32) | ((buf[62] as u32) << 4) | ((buf[63] as u32) << 12)}
        ]));
    }
    out
}

fn parse_mpdu(buf: &[u8]) -> serde_json::Value {
    let downlink = buf[0] & 0x02 != 0;
    if downlink {
        parse_downlink_mpdu(buf)
    } else {
        parse_uplink_mpdu(buf)
    }
}

fn parse_downlink_mpdu(buf: &[u8]) -> serde_json::Value {
    if buf.len() < 8 {
        return json!({ "bearer": "hfdl", "pdu": "mpdu", "direction": "downlink", "parse_ok": false, "error": "too short" });
    }
    let lpdu_count = ((buf[0] >> 2) & 0x0f) as usize;
    let header_len = 6 + lpdu_count;
    let fcs_ok = hfdl_fcs_ok(buf, header_len);
    let lpdu_lengths: Vec<usize> = buf
        .get(6..6 + lpdu_count)
        .unwrap_or_default()
        .iter()
        .map(|v| *v as usize + 1)
        .collect();
    let parse_ok = buf.len() >= header_len + 2;
    let lpdus = if parse_ok {
        let data_start = header_len + 2;
        parse_lpdu_list(
            &lpdu_lengths,
            buf.get(data_start..).unwrap_or_default(),
            MessageDirection::AirToGround,
        )
    } else {
        Vec::new()
    };
    json!({
        "bearer": "hfdl",
        "pdu": "mpdu",
        "direction": "downlink",
        "parse_ok": parse_ok,
        "fcs_ok": fcs_ok,
        "len": buf.len(),
        "src_aircraft_id": buf[2],
        "dst_ground_station_id": buf[1] & 0x7f,
        "lpdu_count": lpdu_count,
        "lpdu_lengths": lpdu_lengths,
        "lpdus": lpdus,
    })
}

fn parse_uplink_mpdu(buf: &[u8]) -> serde_json::Value {
    if buf.len() < 5 {
        return json!({ "bearer": "hfdl", "pdu": "mpdu", "direction": "uplink", "parse_ok": false, "error": "too short" });
    }
    let aircraft_count = (((buf[0] & 0x70) >> 4) + 1) as usize;
    let mut pos = 2usize;
    let mut aircraft_headers = Vec::new();
    for _ in 0..aircraft_count {
        if pos + 2 > buf.len() {
            break;
        }
        let aircraft_id = buf[pos];
        let lpdu_count = (buf[pos + 1] >> 4) as usize;
        pos += 2;
        let lengths: Vec<usize> = buf
            .get(pos..pos + lpdu_count)
            .unwrap_or_default()
            .iter()
            .map(|v| *v as usize + 1)
            .collect();
        pos += lpdu_count;
        aircraft_headers.push((aircraft_id, lengths));
    }
    let fcs_ok = hfdl_fcs_ok(buf, pos);
    let parse_ok = buf.len() >= pos + 2;
    let mut data = buf.get(pos + 2..).unwrap_or_default();
    let aircraft: Vec<_> = aircraft_headers
        .into_iter()
        .map(|(aircraft_id, lengths)| {
            let lpdus = parse_lpdu_list(&lengths, data, MessageDirection::GroundToAir);
            let consumed: usize = lengths.iter().sum();
            data = data.get(consumed..).unwrap_or_default();
            json!({
                "aircraft_id": aircraft_id,
                "lpdu_count": lengths.len(),
                "lpdu_lengths": lengths,
                "lpdus": lpdus,
            })
        })
        .collect();
    json!({
        "bearer": "hfdl",
        "pdu": "mpdu",
        "direction": "uplink",
        "parse_ok": parse_ok,
        "fcs_ok": fcs_ok,
        "len": buf.len(),
        "src_ground_station_id": buf[1] & 0x7f,
        "aircraft": aircraft,
    })
}

fn hfdl_fcs_ok(buf: &[u8], header_len: usize) -> Option<bool> {
    if buf.len() < header_len + 2 {
        return None;
    }
    let got = u16::from_le_bytes([buf[header_len], buf[header_len + 1]]);
    let expected = hfdl_fcs(&buf[..header_len]);
    Some(got == expected)
}

fn hfdl_fcs(data: &[u8]) -> u16 {
    crc16_ccitt_reflected(data, 0xffff) ^ 0xffff
}

fn parse_lpdu_list(
    lengths: &[usize],
    mut data: &[u8],
    acars_direction: MessageDirection,
) -> Vec<serde_json::Value> {
    lengths
        .iter()
        .enumerate()
        .map(|(idx, len)| {
            let lpdu = data.get(..*len).unwrap_or(data);
            data = data.get(*len..).unwrap_or_default();
            parse_lpdu(idx, lpdu, acars_direction)
        })
        .collect()
}

fn parse_lpdu(index: usize, buf: &[u8], acars_direction: MessageDirection) -> serde_json::Value {
    if buf.len() < 3 {
        return json!({
            "index": index,
            "parse_ok": false,
            "error": "too short",
            "len": buf.len(),
        });
    }
    let body_len = buf.len() - 2;
    let fcs_ok = hfdl_fcs_ok(buf, body_len);
    let body = &buf[..body_len];
    let lpdu_type = body[0];
    let mut out = json!({
        "index": index,
        "parse_ok": true,
        "fcs_ok": fcs_ok,
        "len": buf.len(),
        "type": format!("0x{lpdu_type:02X}"),
        "type_name": lpdu_type_name(lpdu_type),
    });

    if let Some(obj) = out.as_object_mut() {
        match lpdu_type {
            0x0D | 0x1D if body.len() > 1 => {
                obj.insert("hfnpdu".into(), parse_hfnpdu(&body[1..], acars_direction));
            }
            0x2F | 0x3F if body.len() >= 5 => {
                obj.insert("icao24".into(), icao_hex(&body[1..4]).into());
                obj.insert("reason_code".into(), body[4].into());
            }
            0x5F | 0x9F if body.len() >= 5 => {
                obj.insert("icao24".into(), icao_hex(&body[1..4]).into());
                obj.insert("aircraft_id".into(), body[4].into());
            }
            0x4F | 0x8F | 0xBF if body.len() >= 4 => {
                obj.insert("icao24".into(), icao_hex(&body[1..4]).into());
            }
            _ => {}
        }
    }
    out
}

fn parse_hfnpdu(buf: &[u8], acars_direction: MessageDirection) -> serde_json::Value {
    if buf.is_empty() {
        return json!({ "parse_ok": false, "error": "empty HFNPDU" });
    }
    if buf[0] != 0xFF {
        return json!({
            "parse_ok": false,
            "error": "not an HFNPDU",
            "raw_hex": hex::encode_upper(buf),
        });
    }
    if buf.len() < 2 {
        return json!({ "parse_ok": false, "error": "too short", "raw_hex": hex::encode_upper(buf) });
    }
    let hfnpdu_type = buf[1];
    let mut out = json!({
        "parse_ok": true,
        "type": format!("0x{hfnpdu_type:02X}"),
        "type_name": hfnpdu_type_name(hfnpdu_type),
    });
    if let Some(obj) = out.as_object_mut() {
        match hfnpdu_type {
            0xD0 if buf.len() >= 5 => {
                obj.insert("total_pdu_count".into(), ((buf[2] >> 4) + 1).into());
                obj.insert("pdu_sequence".into(), (buf[2] & 0x0f).into());
                obj.insert(
                    "system_table_version".into(),
                    ((buf[3] as u16 >> 4) | ((buf[4] as u16) << 4)).into(),
                );
            }
            0xD1 if buf.len() >= 47 => {
                obj.insert("performance".into(), parse_performance_data(buf));
            }
            0xD2 if buf.len() >= 4 => {
                obj.insert(
                    "request_data".into(),
                    u16::from_le_bytes([buf[2], buf[3]]).into(),
                );
            }
            0xD5 if buf.len() >= 15 => {
                obj.insert("frequency_data".into(), parse_frequency_data(buf));
            }
            0xFF => {
                let acars_bytes = &buf[2..];
                match parse_acars_frame(acars_bytes, acars_direction) {
                    Ok(msg) => {
                        let raw = serde_json::to_value(&msg)
                            .unwrap_or_else(|e| json!({ "serialize_error": e.to_string() }));
                        obj.insert("acars".into(), compact_value(raw, false));
                    }
                    Err(err) => {
                        obj.insert(
                            "acars".into(),
                            json!({
                                "parse_ok": false,
                                "error": err.to_string(),
                                "raw_hex": hex::encode_upper(acars_bytes),
                            }),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn parse_performance_data(buf: &[u8]) -> serde_json::Value {
    let flight_id = ascii_trim(&buf[2..8]);
    let lat_raw = (buf[8] as u32) | ((buf[9] as u32) << 8) | (((buf[10] & 0x0f) as u32) << 16);
    let lon_raw =
        ((buf[10] as u32 & 0xf0) >> 4) | ((buf[11] as u32) << 4) | ((buf[12] as u32) << 12);
    let utc = 2 * u16::from_le_bytes([buf[13], buf[14]]) as u32;
    json!({
        "version": buf[15],
        "flight_id": flight_id,
        "position": { "lat": parse_hfdl_coordinate(lat_raw), "lon": parse_hfdl_coordinate(lon_raw) },
        "time_utc": format_hms(utc),
        "flight_leg": buf[16],
        "ground_station_id": buf[17] & 0x7f,
        "frequency_id": buf[18],
        "frequency_search_count": {
            "previous_leg": u16::from_le_bytes([buf[19], buf[20]]),
            "current_leg": u16::from_le_bytes([buf[21], buf[22]]),
        },
        "hf_data_disabled_duration_sec": {
            "previous_leg": u16::from_le_bytes([buf[23], buf[24]]),
            "current_leg": u16::from_le_bytes([buf[25], buf[26]]),
        },
        "mpdus_received": mpdu_stats(&buf[27..31]),
        "mpdus_received_with_errors": mpdu_stats(&buf[31..35]),
        "spdus_received": u16::from_le_bytes([buf[35], buf[36]]),
        "spdus_missed": buf[37],
        "mpdus_transmitted": mpdu_stats(&buf[38..42]),
        "mpdus_delivered": mpdu_stats(&buf[42..46]),
        "frequency_change_code": buf[46] & 0x0f,
        "frequency_change_reason": frequency_change_reason(buf[46] & 0x0f),
    })
}

fn parse_frequency_data(buf: &[u8]) -> serde_json::Value {
    let flight_id = ascii_trim(&buf[2..8]);
    let lat_raw = (buf[8] as u32) | ((buf[9] as u32) << 8) | (((buf[10] & 0x0f) as u32) << 16);
    let lon_raw =
        ((buf[10] as u32 & 0xf0) >> 4) | ((buf[11] as u32) << 4) | ((buf[12] as u32) << 12);
    let utc = 2 * u16::from_le_bytes([buf[13], buf[14]]) as u32;
    let mut freqs = Vec::new();
    let mut pos = 15usize;
    while pos + 6 <= buf.len() && freqs.len() < 6 {
        freqs.push(json!({
            "ground_station_id": buf[pos] & 0x7f,
            "propagating_frequencies_mask": (buf[pos + 1] as u32) | ((buf[pos + 2] as u32) << 8) | (((buf[pos + 3] & 0x0f) as u32) << 16),
            "heard_frequencies_mask": ((buf[pos + 3] as u32 & 0xf0) >> 4) | ((buf[pos + 4] as u32) << 4) | ((buf[pos + 5] as u32) << 12),
        }));
        pos += 6;
    }
    json!({
        "flight_id": flight_id,
        "position": { "lat": parse_hfdl_coordinate(lat_raw), "lon": parse_hfdl_coordinate(lon_raw) },
        "time_utc": format_hms(utc),
        "propagating_frequency_count": freqs.len(),
        "ground_stations": freqs,
    })
}

fn mpdu_stats(bytes: &[u8]) -> serde_json::Value {
    json!({ "300bps": bytes[3], "600bps": bytes[2], "1200bps": bytes[1], "1800bps": bytes[0] })
}

fn parse_hfdl_coordinate(raw: u32) -> f64 {
    let signed = if raw & (1 << 19) != 0 {
        raw as i32 - (1 << 20)
    } else {
        raw as i32
    };
    signed as f64 * 180.0 / 0x7ffff as f64
}

fn format_hms(total_seconds: u32) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        total_seconds / 3600,
        (total_seconds % 3600) / 60,
        total_seconds % 60
    )
}

fn ascii_trim(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_matches(|c| c == '\0' || c == ' ')
        .to_string()
}

fn frequency_change_reason(code: u8) -> &'static str {
    match code {
        0 => "First frequency search in this flight leg",
        1 => "Too many NACKs",
        2 => "SPDUs no longer received",
        3 => "HFDL disabled",
        4 => "Ground station frequency change",
        5 => "Ground station down / channel down",
        6 => "Poor uplink channel quality",
        7 => "No change",
        _ => "Unknown",
    }
}

fn spdu_change_note(code: u8) -> &'static str {
    match code {
        0 => "None",
        1 => "Channel down",
        2 => "Upcoming frequency change",
        3 => "Ground station down",
        _ => "Unknown",
    }
}

fn lpdu_type_name(typ: u8) -> &'static str {
    match typ {
        0x0D => "Unnumbered data",
        0x1D => "Unnumbered acked data",
        0x2F => "Logon denied",
        0x3F => "Logoff request",
        0x4F => "Logon resume",
        0x5F => "Logon resume confirm",
        0x8F => "Logon request normal",
        0x9F => "Logon confirm",
        0xBF => "Logon request DLS",
        _ => "Unknown",
    }
}

fn hfnpdu_type_name(typ: u8) -> &'static str {
    match typ {
        0xD0 => "System table partial",
        0xD1 => "Performance data",
        0xD2 => "System table request",
        0xD5 => "Frequency data",
        0xDE => "Delayed echo",
        0xFF => "Enveloped data",
        _ => "Unknown",
    }
}

fn icao_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn crc16_ccitt_reflected(data: &[u8], init: u16) -> u16 {
    let mut crc = init;
    for &byte in data {
        crc ^= byte as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0x8408;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

fn channels_khz_for(options: &Options, sample_rate: u32, center_freq: u32) -> Vec<f64> {
    if let Some(channels) = &options.channel {
        return channels.iter().copied().map(to_khz).collect();
    }

    let center_khz = center_freq as f64 / 1000.0;
    let usable_half_bw_khz = sample_rate as f64 * 0.40 / 1000.0;
    let lo = center_khz - usable_half_bw_khz;
    let hi = center_khz + usable_half_bw_khz;
    DEFAULT_HFDL_CHANNELS_KHZ
        .iter()
        .copied()
        .filter(|freq| *freq >= lo && *freq <= hi)
        .collect()
}

fn to_khz(freq: f64) -> f64 {
    if freq > 100_000.0 {
        freq / 1000.0
    } else {
        freq
    }
}

fn effective_format(path: &str, requested: SampleFormat) -> SampleFormat {
    if requested == SampleFormat::Cf32 && path.to_ascii_lowercase().ends_with(".wav") {
        SampleFormat::Wav16
    } else {
        requested
    }
}

fn effective_center_freq(path: &str, format: SampleFormat, requested: u32) -> u32 {
    if format == SampleFormat::Wav16 && requested == 10_000_000 {
        if let Some(freq) = infer_khz_from_filename(path) {
            return freq;
        }
    }
    requested
}

fn infer_khz_from_filename(path: &str) -> Option<u32> {
    let name = std::path::Path::new(path).file_name()?.to_string_lossy();
    let lower = name.to_ascii_lowercase();
    let khz_pos = lower.rfind("khz")?;
    let prefix = &lower[..khz_pos];
    let digits_rev: String = prefix
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits_rev.is_empty() {
        return None;
    }
    let digits: String = digits_rev.chars().rev().collect();
    digits.parse::<u32>().ok().map(|khz| khz * 1000)
}

fn effective_sample_rate(path: &str, format: SampleFormat, requested: u32) -> anyhow::Result<u32> {
    if format == SampleFormat::Wav16 {
        let reader = hound::WavReader::open(path)?;
        Ok(reader.spec().sample_rate)
    } else {
        Ok(requested)
    }
}

fn normalize_source_path(source: &str) -> String {
    let path = source.strip_prefix("file://").unwrap_or(source);
    expanduser(path).to_string_lossy().into_owned()
}

fn expanduser(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_check_accepts_constructed_spdu() {
        let mut pdu = vec![0u8; 66];
        pdu[0] = 0x00;
        pdu[1] = 12;
        let fcs = crc16_ccitt_reflected(&pdu[..64], 0xffff) ^ 0xffff;
        pdu[64..66].copy_from_slice(&fcs.to_le_bytes());
        let parsed = parse_hfdl_pdu(&pdu);
        assert_eq!(parsed["pdu"], "spdu");
        assert_eq!(parsed["fcs_ok"], true);
    }

    #[test]
    fn parse_downlink_mpdu_extracts_hfnpdu_payload() {
        let hfnpdu = [0xff, 0xd2, 0x34, 0x12];
        let mut lpdu = vec![0x0d];
        lpdu.extend_from_slice(&hfnpdu);
        let lpdu_fcs = hfdl_fcs(&lpdu);
        lpdu.extend_from_slice(&lpdu_fcs.to_le_bytes());

        let lpdu_len = lpdu.len();
        let mut pdu = vec![0x07, 0x8c, 0x2a, 0, 0, 0, (lpdu_len - 1) as u8];
        let hdr_fcs = hfdl_fcs(&pdu);
        pdu.extend_from_slice(&hdr_fcs.to_le_bytes());
        pdu.extend_from_slice(&lpdu);

        let parsed = parse_hfdl_pdu(&pdu);
        assert_eq!(parsed["pdu"], "mpdu");
        assert_eq!(parsed["direction"], "downlink");
        assert_eq!(parsed["fcs_ok"], true);
        assert_eq!(parsed["lpdus"][0]["fcs_ok"], true);
        assert_eq!(parsed["lpdus"][0]["hfnpdu"]["type"], "0xD2");
        assert_eq!(parsed["lpdus"][0]["hfnpdu"]["request_data"], 0x1234);
    }

    #[test]
    fn parse_uplink_mpdu_uses_high_nibble_lpdu_count() {
        let mut lpdu = vec![0x1d, 0xff, 0xff, 0x01, 0x02, 0x03];
        let lpdu_fcs = hfdl_fcs(&lpdu);
        lpdu.extend_from_slice(&lpdu_fcs.to_le_bytes());

        let mut pdu = vec![0x01, 0x8c, 0x2a, 0x10, (lpdu.len() - 1) as u8];
        let hdr_fcs = hfdl_fcs(&pdu);
        pdu.extend_from_slice(&hdr_fcs.to_le_bytes());
        pdu.extend_from_slice(&lpdu);

        let parsed = parse_hfdl_pdu(&pdu);
        assert_eq!(parsed["pdu"], "mpdu");
        assert_eq!(parsed["direction"], "uplink");
        assert_eq!(parsed["fcs_ok"], true);
        assert_eq!(parsed["aircraft"][0]["lpdu_count"], 1);
        assert_eq!(parsed["aircraft"][0]["lpdus"][0]["hfnpdu"]["type"], "0xFF");
        assert_eq!(
            parsed["aircraft"][0]["lpdus"][0]["hfnpdu"]["acars"]["parse_ok"],
            false
        );
    }

    #[test]
    fn auto_channels_cover_10mhz_capture() {
        let options = Options {
            source: Some("dummy".into()),
            format: SampleFormat::Cf32,
            center_freq: 10_000_000,
            sample_rate: 8_000_000,
            channel: None,
            start_second: 0.0,
            max_seconds: 1.0,
            stats: false,
        };
        let channels = channels_khz_for(&options, options.sample_rate, options.center_freq);
        assert!(channels.contains(&10081.0));
        assert!(channels.contains(&11387.0));
        assert!(!channels.contains(&6529.0));
    }
}
