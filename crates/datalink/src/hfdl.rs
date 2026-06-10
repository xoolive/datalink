use crate::source::{Address, Source};
#[cfg(feature = "hackrf")]
use crate::util::hackrf_gain;
#[cfg(feature = "airspy")]
use crate::util::parse_airspy_serial;
use crate::util::{expanduser, infer_capture_params, redis_topic_for_record, RedisPublisher};
use acars::decode::hfdl::{parse_hfdl_pdu, FcsResult};
use acars::demod::hfdl::{diagnose_channel, HfdlDemodConfig};
use clap::{Parser, ValueEnum};
use futures_util::StreamExt;
use rustfft::num_complex::Complex;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};

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
#[command(about = "HF Data Link frontend for WAV and I/Q captures")]
pub(crate) struct Options {
    /// WAV, I/Q, or SDR source, e.g. file://capture.wav, rtlsdr://0, hackrf://0, or ~/capture.raw.
    source: Option<Source>,

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

    /// Publish decoded PDUs to application-specific Redis pub/sub topics
    #[arg(long, value_name = "REDIS URL")]
    redis_url: Option<String>,

    /// Retry interval (seconds) when publishing to Redis fails; 0 disables retry
    #[arg(long)]
    redis_retry_interval: Option<u64>,
}

pub(crate) async fn run(options: Options) -> anyhow::Result<()> {
    decode_mode(&options).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn decode_file_values(
    source: &str,
    format: Option<&str>,
    center_freq: Option<u32>,
    sample_rate: Option<u32>,
    channels: Option<Vec<u32>>,
    start_second: f64,
    max_seconds: f64,
    source_meta: &crate::merged::SourceMetadata,
    receiver_bearer: crate::merged::Bearer,
) -> anyhow::Result<Vec<crate::merged::DecodedEvent>> {
    let options = Options {
        source: Some(source.parse()?),
        format: format
            .and_then(parse_sample_format)
            .unwrap_or(SampleFormat::Cf32),
        center_freq: center_freq.unwrap_or(10_000_000),
        sample_rate: sample_rate.unwrap_or(8_000_000),
        channel: channels.map(|v| v.into_iter().map(|hz| hz as f64).collect()),
        start_second,
        max_seconds,
        stats: false,
        redis_url: None,
        redis_retry_interval: None,
    };

    let mut out = Vec::new();
    for parsed in collect_decoded_pdus(&options, source_meta, receiver_bearer).await? {
        out.push(parsed);
    }
    Ok(out)
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

async fn decode_mode(options: &Options) -> anyhow::Result<()> {
    let mut redis = if let Some(url) = options.redis_url.as_deref() {
        Some(
            RedisPublisher::connect_with_prefix(
                url,
                options.redis_retry_interval.unwrap_or(5),
                "datalink hfdl",
            )
            .await?,
        )
    } else {
        None
    };

    let source_meta = crate::merged::SourceMetadata {
        id: "hfdl_cli".into(),
        name: options
            .source
            .as_ref()
            .map(|s| s.label())
            .unwrap_or_else(|| "hfdl".into()),
        class: crate::merged::SourceClass::Iq,
        format: None,
    };

    for parsed in collect_decoded_pdus(options, &source_meta, crate::merged::Bearer::Hfdl).await? {
        let line = serde_json::to_string(&parsed)?;
        println!("{line}");
        if let Some(redis) = redis.as_mut() {
            redis
                .publish(redis_topic_for_record(&parsed.message), &line)
                .await;
        }
    }
    Ok(())
}

async fn collect_decoded_pdus(
    options: &Options,
    source_meta: &crate::merged::SourceMetadata,
    receiver_bearer: crate::merged::Bearer,
) -> anyhow::Result<Vec<crate::merged::DecodedEvent>> {
    let source = options
        .source
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing source; pass an explicit I/Q or SDR source"))?;
    let (samples, sample_rate, center_freq, channels) = samples_for_source(source, options).await?;
    anyhow::ensure!(
        !channels.is_empty(),
        "no HFDL channels selected; pass --channel or use a wider/centered recording"
    );
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
        )?;
        frame_sync_count += diagnostics.frame_hits.len() as u64;
        candidate_count += diagnostics.pdu_candidates.len() as u64;
        for candidate in &diagnostics.pdu_candidates {
            let Ok(parsed) = parse_hfdl_pdu(&candidate.bytes) else {
                continue;
            };
            if parsed.fcs == FcsResult::Pass {
                continue;
            }
            pdu_ok += 1;
            let pmsg = crate::merged::ProtocolMessage::Hfdl(Box::new(parsed));

            let event = crate::merged::DecodedEvent {
                event: "message",
                timestamp: None,
                bearer: receiver_bearer,
                source: source_meta.clone(),
                receiver: Some(crate::merged::ReceiverMetadata {
                    bearer: receiver_bearer,
                    channel_hz: Some((channel_khz * 1000.0).round() as u32),
                }),
                aircraft: crate::merged::aircraft_summary(&pmsg),
                kinematics: pmsg.kinematics(),
                raw_frame_hex: Some(hex::encode_upper(&candidate.bytes)),
                message: pmsg,
            };
            out.push(event);
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

async fn samples_for_source(
    source: &Source,
    options: &Options,
) -> anyhow::Result<(Vec<Complex<f32>>, u32, u32, Vec<f64>)> {
    let mut effective_source = source.clone();
    if effective_source.center_freq.is_none() {
        effective_source.center_freq = Some(options.center_freq);
    }
    if effective_source.sample_rate.is_none() {
        effective_source.sample_rate = Some(options.sample_rate);
    }
    let sample_rate = effective_source.sample_rate.unwrap_or(options.sample_rate);
    let center_freq = effective_source.center_freq.unwrap_or(options.center_freq);
    let configured_channels = options.channel.clone().or_else(|| {
        effective_source
            .channels
            .clone()
            .map(|channels| channels.into_iter().map(|hz| hz as f64).collect())
    });
    let channels = channels_khz_for(configured_channels.as_ref(), sample_rate, center_freq);

    let samples = match &effective_source.address {
        Address::File { file } => {
            let path = expanduser(file.strip_prefix("file://").unwrap_or(file));
            let path = path.to_string_lossy();
            let inferred = infer_capture_params(&path);
            let requested_format = effective_source
                .format
                .as_deref()
                .and_then(parse_sample_format)
                .unwrap_or(options.format);
            let format = effective_format(&path, requested_format);
            let sample_rate = effective_sample_rate(
                &path,
                format,
                inferred
                    .and_then(|params| params.sample_rate)
                    .unwrap_or(sample_rate),
            )?;
            let center_freq = inferred
                .map(|params| params.center_freq)
                .unwrap_or_else(|| effective_center_freq(&path, format, center_freq));
            let channels = channels_khz_for(configured_channels.as_ref(), sample_rate, center_freq);
            let samples = read_complex_window(
                &path,
                format,
                sample_rate,
                options.start_second,
                options.max_seconds,
            )?;
            return Ok((samples, sample_rate, center_freq, channels));
        }
        _ => read_sdr_window(&effective_source, options.start_second, options.max_seconds).await?,
    };

    Ok((samples, sample_rate, center_freq, channels))
}

async fn read_sdr_window(
    source: &Source,
    start_second: f64,
    max_seconds: f64,
) -> anyhow::Result<Vec<Complex<f32>>> {
    let sample_rate = source.sample_rate.unwrap_or(8_000_000);
    let skip = (start_second.max(0.0) * sample_rate as f64).round() as usize;
    let keep = (max_seconds.max(0.0) * sample_rate as f64).ceil() as usize;
    let mut stream = open_source(source).await?;
    let mut seen = 0usize;
    let mut out = Vec::with_capacity(keep);
    while out.len() < keep {
        let Some(chunk_result) = stream.next().await else {
            break;
        };
        let chunk = chunk_result?;
        for sample in chunk {
            if seen >= skip && out.len() < keep {
                out.push(Complex::new(sample.re, sample.im));
            }
            seen = seen.saturating_add(1);
            if out.len() >= keep {
                break;
            }
        }
    }
    Ok(out)
}

async fn open_source(src: &Source) -> anyhow::Result<desperado::IqAsyncSource> {
    use desperado::{DeviceConfig, IqAsyncSource};

    let center_freq = src.center_freq.unwrap_or(10_000_000);
    let sample_rate = src.sample_rate.unwrap_or(8_000_000);
    match &src.address {
        Address::File { file } if file == "-" => Ok(IqAsyncSource::from_stdin(
            center_freq,
            sample_rate,
            65_536,
            src.iq_format(),
        )),
        Address::File { file } => {
            Ok(
                IqAsyncSource::from_file(file, center_freq, sample_rate, 65_536, src.iq_format())
                    .await?,
            )
        }
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
            Ok(IqAsyncSource::from_device_config(&DeviceConfig::RtlSdr(cfg)).await?)
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
            Ok(IqAsyncSource::from_device_config(&DeviceConfig::Airspy(cfg)).await?)
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
            Ok(IqAsyncSource::from_device_config(&DeviceConfig::HackRf(cfg)).await?)
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
            Ok(IqAsyncSource::from_device_config(&DeviceConfig::Soapy(cfg)).await?)
        }
        #[allow(unreachable_patterns)]
        _ => Err(anyhow::anyhow!("source type is not enabled in this build")),
    }
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

fn channels_khz_for(
    configured_channels: Option<&Vec<f64>>,
    sample_rate: u32,
    center_freq: u32,
) -> Vec<f64> {
    if let Some(channels) = configured_channels {
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
        if let Some(params) = infer_capture_params(path) {
            return params.center_freq;
        }
    }
    requested
}

fn effective_sample_rate(path: &str, format: SampleFormat, requested: u32) -> anyhow::Result<u32> {
    if format == SampleFormat::Wav16 {
        let reader = hound::WavReader::open(path)?;
        Ok(reader.spec().sample_rate)
    } else {
        Ok(requested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acars::decode::hfdl::Mpdu;

    fn hfdl_fcs(data: &[u8]) -> u16 {
        crc16_ccitt_reflected(data, 0xffff) ^ 0xffff
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

    #[test]
    fn crc_check_accepts_constructed_spdu() {
        let mut pdu = vec![0u8; 66];
        pdu[0] = 0x00;
        pdu[1] = 12;
        let fcs = crc16_ccitt_reflected(&pdu[..64], 0xffff) ^ 0xffff;
        pdu[64..66].copy_from_slice(&fcs.to_le_bytes());
        let parsed = parse_hfdl_pdu(&pdu).unwrap();
        assert!(matches!(parsed.pdu, acars::decode::hfdl::HfdlPdu::Spdu(_)));
        assert!(parsed.fcs == FcsResult::Pass);
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

        let parsed = parse_hfdl_pdu(&pdu).unwrap();
        let acars::decode::hfdl::HfdlPdu::Mpdu(Mpdu::Downlink(dl)) = parsed.pdu else {
            panic!()
        };
        assert!(parsed.fcs == FcsResult::Pass);
        assert!(dl.lpdus[0].fcs == FcsResult::Pass);
        if let acars::decode::hfdl::LpduData::Hfnpdu { hfnpdu, .. } = &dl.lpdus[0].data {
            if let acars::decode::hfdl::Hfnpdu::SystemTableRequest { request_data, .. } =
                &hfnpdu.data
            {
                assert_eq!(*request_data, 0x1234);
            } else {
                panic!("expected SystemTableRequest");
            }
        } else {
            panic!("expected Hfnpdu LPDU type");
        }
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

        let parsed = parse_hfdl_pdu(&pdu).unwrap();
        assert!(parsed.fcs == FcsResult::Pass);
        let acars::decode::hfdl::HfdlPdu::Mpdu(Mpdu::Uplink(ul)) = parsed.pdu else {
            panic!()
        };
        assert_eq!(ul.aircraft[0].lpdu_count, 1);
        if let acars::decode::hfdl::LpduData::Hfnpdu { hfnpdu, .. } = &ul.aircraft[0].lpdus[0].data
        {
            if let acars::decode::hfdl::Hfnpdu::Unknown { raw_hex, .. } = &hfnpdu.data {
                assert_eq!(hfnpdu.kind_code, "0xFF");
                assert_eq!(raw_hex, "010203");
            } else {
                panic!("expected Hfnpdu::Unknown (due to invalid acars frame)");
            }
        } else {
            panic!("expected Hfnpdu LPDU type");
        }
    }

    #[test]
    fn auto_channels_cover_10mhz_capture() {
        let options = Options {
            source: Some("dummy".parse().unwrap()),
            format: SampleFormat::Cf32,
            center_freq: 10_000_000,
            sample_rate: 8_000_000,
            channel: None,
            start_second: 0.0,
            max_seconds: 1.0,
            stats: false,
            redis_url: None,
            redis_retry_interval: None,
        };
        let channels = channels_khz_for(
            options.channel.as_ref(),
            options.sample_rate,
            options.center_freq,
        );
        assert!(channels.contains(&10081.0));
        assert!(channels.contains(&11387.0));
        assert!(!channels.contains(&6529.0));
    }

    #[test]
    fn gqrx_inferred_params_cover_hfdl_channels() {
        let params = infer_capture_params("gqrx_20260518_114025_10000000_8000000_fc.raw").unwrap();
        let channels = channels_khz_for(None, params.sample_rate.unwrap(), params.center_freq);
        assert_eq!(params.format, Some("cf32"));
        assert!(channels.contains(&10081.0));
        assert!(channels.contains(&11387.0));
        assert!(!channels.contains(&6529.0));
    }
}
