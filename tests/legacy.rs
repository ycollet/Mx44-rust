use mx44_rust::{
    engine::Synth,
    legacy,
    midi::MidiEvent,
    patch::{Session, Waveform},
};

const RECORD: usize = 480;
const BANK: usize = RECORD * 128;
const STATE: usize = RECORD * 144 + 32;

fn word(record: &mut [u8], offset: usize, value: i16) {
    record[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn patch() -> [u8; RECORD] {
    let mut bytes = [0; RECORD];
    bytes[320..331].copy_from_slice(b"Legacy test");
    for op in 0..4 {
        word(&mut bytes, 208 + op * 2, 512);
        word(&mut bytes, 296 + op * 2, 16000);
        for (stage, level) in [0, 8000, 6400, 4800, 3200, 1600, 0, 800]
            .into_iter()
            .enumerate()
        {
            word(&mut bytes, 80 + op * 16 + stage * 2, level);
            word(&mut bytes, 144 + op * 16 + stage * 2, 640);
        }
    }
    word(&mut bytes, 64, 16384);
    bytes
}

fn bank(record: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; BANK];
    bytes[..RECORD].copy_from_slice(record);
    bytes
}

#[test]
fn imports_little_endian_records_with_correct_matrix_and_envelope_layout() {
    let mut record = patch();
    word(&mut record, (2 * 4 + 1) * 2, 16384);
    word(&mut record, 32 + (4 + 3) * 2, -16384);
    word(&mut record, 210, 1024);
    let imported = legacy::from_bytes(&bank(&record), 48000).unwrap();
    let session = imported.session;
    session.validate().unwrap();
    assert_eq!(session.patches.len(), 128);
    assert_eq!(session.patches[0].name, "Legacy test");
    assert_eq!(session.patches[1].name, "Legacy 002");
    let sound = session.patches[0].sound;
    assert_eq!(sound.operators[0].wave, Waveform::Mx);
    assert_eq!(sound.operators[0].ratio, 1.0);
    assert_eq!(sound.operators[1].ratio, 2.0);
    assert_eq!(sound.operators[0].detune, 0.0);
    assert_eq!(sound.operators[0].level, 0.375);
    assert_eq!(sound.operators[0].pan, 0.0);
    assert_eq!(sound.operators[0].velocity, 0.5);
    assert_eq!(sound.pm[2][1], std::f32::consts::FRAC_PI_2);
    assert_eq!(sound.pm[1][2], 0.0);
    assert_eq!(sound.am[1][3], -0.5);
    assert_eq!(
        sound.operators[0].envelope.levels,
        [1.0, 0.8, 0.6, 0.4, 0.2, 0.4, 0.1, 0.0]
    );
    assert!(sound.operators[0].envelope.loop_sustain);
    assert!((0.09..0.11).contains(&sound.operators[0].envelope.times[0]));
    assert_eq!(session.channels, [sound; 16]);
    assert_eq!(session.programs, [0; 16]);
    assert!(
        session.patches[1]
            .sound
            .operators
            .iter()
            .all(|op| op.level == 0.0)
    );
    assert!(imported.warnings.iter().any(|w| w.contains("Bank-only")));
}

#[test]
fn preserves_editable_channel_sounds_and_selected_programs_in_all_state_variants() {
    for size in [STATE, STATE + 12, STATE + 13] {
        let mut bytes = bank(&patch());
        bytes.resize(size, 0);
        let mut edited = patch();
        word(&mut edited, 208, 1536);
        bytes[BANK..BANK + RECORD].copy_from_slice(&edited);
        bytes[RECORD * 144] = 42;
        bytes[STATE - 16] = 1;
        let imported = legacy::from_bytes(&bytes, 48000).unwrap();
        assert_eq!(imported.session.programs[0], 42);
        assert_eq!(imported.session.channels[0].operators[0].ratio, 3.0);
        assert_eq!(imported.session.patches[0].sound.operators[0].ratio, 1.0);
        assert!(imported.warnings.iter().any(|w| w.contains("mono/legato")));
        assert_eq!(
            imported.warnings.iter().any(|w| w.contains("temperament")),
            size > STATE
        );
    }
}

#[test]
fn malformed_lengths_program_numbers_and_source_rates_are_rejected() {
    for size in [0, 479, BANK - 1, BANK + 1, STATE - 1, STATE + 1, STATE + 14] {
        assert!(
            legacy::from_bytes(&vec![0; size], 48000).is_err(),
            "size {size}"
        );
    }
    for rate in [0, 7999, 192001, u32::MAX] {
        assert!(legacy::from_bytes(&bank(&patch()), rate).is_err());
    }
    let mut bytes = bank(&patch());
    bytes.resize(STATE, 0);
    bytes[RECORD * 144 + 15] = 255;
    let error = legacy::from_bytes(&bytes, 48000).err().unwrap();
    assert!(error.to_string().contains("channel 16"));
}

#[test]
fn approximations_and_clipping_are_reported_and_names_are_bounded() {
    let mut record = patch();
    record[320..352].fill(b'x');
    record[320] = 0xe9;
    record[321] = b'\n';
    record[248] = 7;
    record[252] = 255;
    word(&mut record, 208, 32767);
    word(&mut record, 296, -1000);
    word(&mut record, 80, 5000);
    word(&mut record, 82, 30000);
    word(&mut record, 144, -10);
    word(&mut record, 352, 1000);
    let imported = legacy::from_bytes(&bank(&record), 8000).unwrap();
    imported.session.validate().unwrap();
    let sound = imported.session.patches[0].sound;
    assert_eq!(sound.operators[0].ratio, 32.0);
    assert_eq!(sound.operators[0].velocity, 0.0);
    assert_eq!(sound.operators[0].envelope.levels[0], 1.0);
    assert_eq!(sound.operators[0].octave_mix, 0.5);
    assert_eq!(sound.delay_mix, 0.0);
    assert_eq!(sound.drive, 0.0);
    assert!(imported.session.patches[0].name.starts_with('\u{e9}'));
    assert!(!imported.session.patches[0].name.contains('\n'));
    assert!(imported.session.patches[0].name.len() <= 64);
    for text in [
        "Latin-1",
        "harmonics",
        "ratios",
        "velocity",
        "initial",
        "effect",
        "Negative envelope",
    ] {
        assert!(imported.warnings.iter().any(|w| w.contains(text)), "{text}");
    }
}

#[test]
fn sample_based_timing_and_additive_detuning_follow_the_source_rate() {
    let mut record = patch();
    word(&mut record, 232, 1000);
    let bytes = bank(&record);
    let low = legacy::from_bytes(&bytes, 48000).unwrap().session.patches[0].sound;
    let high = legacy::from_bytes(&bytes, 96000).unwrap().session.patches[0].sound;
    assert_eq!(
        low.operators[0].envelope.times[2],
        high.operators[0].envelope.times[2] * 2.0
    );
    assert!(low.operators[0].detune > 0.0);
    assert!(high.operators[0].detune > low.operators[0].detune);
}

#[test]
fn imported_sound_renders_and_releases() {
    let session = legacy::from_bytes(&bank(&patch()), 48000).unwrap().session;
    let mut synth = Synth::new(8000, 4, &session);
    synth.event(MidiEvent::NoteOn {
        channel: 0,
        key: 69,
        velocity: 100,
    });
    let mut peak = 0.0_f32;
    for _ in 0..8000 {
        for value in synth.next_frame() {
            assert!(value.is_finite() && value.abs() <= 1.0);
            peak = peak.max(value.abs());
        }
    }
    assert!(peak > 0.01);
    synth.event(MidiEvent::NoteOff {
        channel: 0,
        key: 69,
    });
    for _ in 0..8000 {
        synth.next_frame();
    }
    assert_eq!(synth.active_voices(), 0);
}

#[test]
fn supplied_converted_bank_loads_with_original_names_and_channel_state() {
    let session = Session::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("presets/mx44-legacy.json"),
    )
    .unwrap();
    assert_eq!(session.patches[0].name, "Amsterdam");
    assert_eq!(session.patches[7].name, "Bell Labs");
    assert_eq!(
        session.programs,
        [7, 60, 53, 0, 0, 3, 0, 1, 1, 5, 67, 1, 12, 12, 14, 7]
    );
}

#[cfg(feature = "standalone")]
mod cli {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mx44-legacy-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn command(source: &Path, target: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mx44-rust"));
        command
            .arg("--import-legacy")
            .arg(source)
            .arg("--save-to")
            .arg(target);
        command
    }

    #[test]
    fn converts_without_devices_and_rejects_invalid_inputs_without_output() {
        let source = path("mx44patch");
        let target = path("session.json");
        let bytes = bank(&patch());
        fs::write(&source, &bytes).unwrap();
        let output = command(&source, &target).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("Import warning"));
        assert_eq!(
            Session::load(&target).unwrap().patches[0].name,
            "Legacy test"
        );
        assert_eq!(fs::read(&source).unwrap(), bytes);
        fs::remove_file(&target).unwrap();
        fs::write(&source, b"invalid legacy data").unwrap();
        let output = command(&source, &target).output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported legacy bank length"));
        assert!(!target.exists());
        fs::remove_file(source).unwrap();
    }

    #[test]
    fn refuses_original_input_even_with_an_uncreated_parent_alias() {
        let directory = path("guard");
        fs::create_dir(&directory).unwrap();
        let source = directory.join("mx44patch");
        let bytes = bank(&patch());
        fs::write(&source, &bytes).unwrap();
        for target in [source.clone(), directory.join("missing/../mx44patch")] {
            let output = command(&source, &target).output().unwrap();
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("cannot overwrite"));
            assert_eq!(fs::read(&source).unwrap(), bytes);
        }
        fs::remove_dir(directory.join("missing")).unwrap();
        fs::remove_file(source).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_to_the_original_bank() {
        let source = path("source");
        let target = path("symlink");
        let bytes = bank(&patch());
        fs::write(&source, &bytes).unwrap();
        std::os::unix::fs::symlink(&source, &target).unwrap();
        let output = command(&source, &target).output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("cannot overwrite"));
        assert_eq!(fs::read(&source).unwrap(), bytes);
        fs::remove_file(target).unwrap();
        fs::remove_file(source).unwrap();
    }

    #[test]
    fn requires_an_explicit_destination_and_rejects_other_run_modes() {
        let output = Command::new(env!("CARGO_BIN_EXE_mx44-rust"))
            .args(["--import-legacy", "unused"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--save-to"));
        for flag in ["--console", "--list-audio", "--no-midi"] {
            let output = command(Path::new("unused"), Path::new("unused.json"))
                .arg(flag)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used"));
        }
    }
}
