#![cfg(feature = "cli")]

use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const EXPECTED_MPDU: &str = "0784FA0004003103CE0DFFD100000000000024DF61AEC02064017B0404150004000000000000000007000000001000000000000100000001016087000000000000000000";

fn decode(source: &str, format: &str, input: Vec<u8>, extra: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_datalink"))
        .args([
            "hfdl",
            source,
            "--format",
            format,
            "--sample-rate",
            "62500",
            "--center-freq",
            "11404000",
            "--channel",
            "11387",
            "--max-seconds",
            "5",
            "--stats",
        ])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let deadline = Instant::now() + Duration::from_secs(120);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            let _ = writer.join();
            panic!(
                "HFDL process timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    // Early validation errors may close the pipe before the writer finishes.
    let _ = writer.join().unwrap();
    output
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn empty_stdin_finishes_successfully() {
    let output = decode("-", "cs16", vec![], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("pdu_ok=0"));
}

#[test]
fn complete_short_stdin_accepts_all_raw_formats() {
    for (format, sample) in [
        ("u8", vec![128, 128]),
        ("cs16", vec![0; 4]),
        ("cf32", vec![0; 8]),
    ] {
        let output = decode("-", format, sample, &[]);
        assert!(output.status.success(), "{format}: {}", stderr(&output));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn truncated_stdin_is_an_error_for_each_format() {
    for (format, sample_bytes) in [("u8", 2), ("cs16", 4), ("cf32", 8)] {
        // Exercise both immediate and deferred truncation after a complete sample.
        for len in [sample_bytes - 1, sample_bytes + 1] {
            let output = decode("-", format, vec![0; len], &[]);
            assert!(!output.status.success(), "accepted {len} bytes as {format}");
            assert!(stderr(&output).contains("truncated"), "{}", stderr(&output));
            assert!(output.stdout.is_empty());
        }
    }
}

#[test]
fn wav_stdin_is_rejected_with_an_actionable_error() {
    let output = decode("-", "wav16", vec![], &[]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("raw IQ"), "{}", stderr(&output));
}

#[test]
fn invalid_stream_windows_are_rejected() {
    for args in [
        vec!["--sample-rate", "0"],
        vec!["--max-seconds", "NaN"],
        vec!["--max-seconds", "inf"],
        vec!["--start-second", "NaN"],
        vec!["--start-second=-1"],
        vec!["--max-seconds=-1"],
        vec!["--max-seconds", "1e100"],
    ] {
        let output = decode("-", "cs16", vec![], &args);
        assert!(!output.status.success(), "accepted {args:?}");
        assert!(!stderr(&output).contains("panicked"), "{}", stderr(&output));
    }
}

#[test]
#[ignore = "requires private IQ and WAV recordings; see docs/sources.md"]
fn private_capture_matches_reference_for_file_stdin_and_wav() {
    let raw = std::env::var("DATALINK_HFDL_FIXTURE")
        .expect("set DATALINK_HFDL_FIXTURE to the private 5-second CS16 excerpt");
    let wav = std::env::var("DATALINK_HFDL_WAV_FIXTURE")
        .expect("set DATALINK_HFDL_WAV_FIXTURE to the original SDRuno WAV");
    let file = decode(&raw, "cs16", vec![], &[]);
    let pipe = decode("-", "cs16", std::fs::read(&raw).unwrap(), &[]);
    let wav = decode(&wav, "wav16", vec![], &["--start-second", "10"]);
    let mut messages = Vec::new();
    for output in [&file, &pipe, &wav] {
        assert!(output.status.success(), "{}", stderr(output));
        let events: Vec<serde_json::Value> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events.len(), 1, "{}", stderr(output));
        let event = &events[0];
        assert_eq!(event["raw_frame_hex"], EXPECTED_MPDU);
        assert_eq!(event["message"]["hfdl"]["fcs"], "pass");
        assert_eq!(
            event["message"]["hfdl"]["pdu"]["mpdu"]["downlink"]["lpdus"][0]["fcs"],
            "pass"
        );
        assert_eq!(event["receiver"]["channel_hz"], 11_387_000);
        messages.push(event["message"].clone());
    }
    assert_eq!(messages[0], messages[1]);
    assert_eq!(messages[0], messages[2]);
}
