use std::{fs, process::Command};

#[test]
fn cli_renders_audible_stereo_wav_without_devices() {
    let path = std::env::temp_dir().join(format!("mx44-rust-render-{}.wav", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_mx44-rust"))
        .args([
            "--render",
            path.to_str().unwrap(),
            "--duration",
            "2",
            "--sample-rate",
            "8000",
            "--program",
            "2",
        ])
        .env(
            "XDG_CONFIG_HOME",
            std::env::temp_dir().join(format!("mx44-rust-no-config-{}", std::process::id())),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut reader = hound::WavReader::open(&path).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_rate, 8000);
    assert_eq!(reader.duration(), 16000);
    let samples: Vec<i16> = reader.samples().map(Result::unwrap).collect();
    let rms = (samples
        .iter()
        .map(|x| (*x as f64 / 32768.0).powi(2))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt();
    assert!(rms > 0.005 && rms < 0.8, "unexpected RMS {rms}");
    drop(reader);
    fs::remove_file(path).unwrap();
}

#[test]
fn cli_rejects_invalid_duration_and_unsafe_load_save_combination() {
    let output = Command::new(env!("CARGO_BIN_EXE_mx44-rust"))
        .args(["--render", "unused.wav", "--duration", "NaN"])
        .env(
            "XDG_CONFIG_HOME",
            std::env::temp_dir().join(format!("mx44-rust-no-config-{}", std::process::id())),
        )
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--duration"));
    let output = Command::new(env!("CARGO_BIN_EXE_mx44-rust"))
        .args(["--load-from", "nonexistent.json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--save-to"));
}
