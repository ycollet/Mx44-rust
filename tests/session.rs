use mx44_rust::{
    engine::Synth,
    midi::MidiEvent,
    patch::{Session, Sound},
    plugin,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "mx44-rust-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn session_round_trips_and_overwrites_atomically() {
    let path = path("session.json");
    let mut session = Session::default();
    session.channels[3].pm[0][2] = 2.75;
    session.programs[3] = 4;
    session.patches[4].name = "Custom pad".into();
    session.save(&path).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert_eq!(loaded.channels[3], session.channels[3]);
    assert_eq!(loaded.programs, session.programs);
    assert_eq!(loaded.patches[4].name, "Custom pad");
    session.master = 0.123;
    session.save(&path).unwrap();
    assert_eq!(Session::load(&path).unwrap().master, 0.123);
    fs::remove_file(path).unwrap();
}

#[test]
fn invalid_sessions_are_rejected_without_overwriting() {
    let path = path("invalid.json");
    let mut session = Session::default();
    session.save(&path).unwrap();
    let original = fs::read(&path).unwrap();
    session.channels[0].delay_feedback = 1.01;
    assert!(session.save(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    session.channels[0].delay_feedback = 0.3;
    session.channels[0].operators[0].ratio = f32::NAN;
    assert!(session.validate().is_err());
    session = Session::default();
    session.channels[0].operators[0].envelope.levels[7] = 0.1;
    assert!(session.validate().is_err());
    session = Session::default();
    session.patches.pop();
    assert!(session.validate().is_err());
    fs::write(&path, r#"{"version":999}"#).unwrap();
    assert!(Session::load(&path).is_err());
    fs::remove_file(path).unwrap();
}

#[test]
fn preexisting_save_temp_file_is_never_removed() {
    let path = path("occupied.json");
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let temporary = path.parent().unwrap().join(name);
    fs::write(&temporary, "do not delete").unwrap();
    assert!(Session::default().save(&path).is_err());
    assert_eq!(fs::read_to_string(&temporary).unwrap(), "do not delete");
    assert!(!path.exists());
    fs::remove_file(temporary).unwrap();
}

#[test]
fn factory_bank_preserves_original_programs_and_adds_distinct_valid_sounds() {
    let factory = Session::factory_patches();
    assert_eq!(
        factory[..6]
            .iter()
            .map(|patch| patch.name.as_str())
            .collect::<Vec<_>>(),
        [
            "Init - Mx tone",
            "Glass electric piano",
            "Rubber FM bass",
            "Inharmonic bells",
            "Slow orbit pad",
            "Four drawbars",
        ]
    );
    assert_eq!(factory.len(), 60);
    let specs = plugin::parameter_specs();
    for (index, patch) in factory.iter().enumerate() {
        patch.sound.validate().unwrap();
        assert!(
            factory[..index]
                .iter()
                .all(|other| other.sound != patch.sound && other.name != patch.name),
            "duplicate factory sound or name: {}",
            patch.name
        );
        let values = plugin::encode(&patch.sound, 0.5);
        assert_eq!(plugin::decode(&values), (patch.sound, 0.5));
        for (value, spec) in values.iter().zip(&specs) {
            assert!((*value >= spec.min) && (*value <= spec.max));
        }
    }
    let session = Session::default();
    session.validate().unwrap();
    for (slot, patch) in session.patches.iter().zip(&factory) {
        assert_eq!(slot.name, patch.name);
        assert_eq!(slot.sound, patch.sound);
    }
    assert!(
        session.patches[factory.len()..]
            .iter()
            .all(|patch| patch.sound == Sound::default())
    );
    assert_eq!(session.channels, [Sound::default(); 16]);
    assert_eq!(session.programs, [0; 16]);
}

#[test]
fn imported_factory_sounds_produce_finite_audible_output() {
    let session = Session::default();
    let mut synth = Synth::new(8000, 4, &session);
    for patch in Session::factory_patches().iter().skip(6) {
        synth.reset();
        synth.set_sound(0, patch.sound);
        synth.event(MidiEvent::NoteOn {
            channel: 0,
            key: 69,
            velocity: 100,
        });
        let mut peak = 0.0_f32;
        for _ in 0..24000 {
            for value in synth.next_frame() {
                assert!(value.is_finite() && value.abs() <= 1.0, "{}", patch.name);
                peak = peak.max(value.abs());
            }
        }
        assert!(peak > 0.00001, "silent factory sound: {}", patch.name);
    }
}

#[test]
fn loading_an_existing_session_does_not_fill_its_slots_with_new_factory_sounds() {
    let path = path("existing-bank.json");
    let mut session = Session::default();
    for (index, patch) in session.patches.iter_mut().enumerate().skip(6) {
        patch.name = format!("My slot {index}");
        patch.sound = Sound::default();
    }
    session.patches[6].sound.pm[0][1] = 1.25;
    session.programs[3] = 6;
    session.channels[3] = session.patches[6].sound;
    session.save(&path).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert_eq!(
        serde_json::to_value(&loaded).unwrap(),
        serde_json::to_value(&session).unwrap()
    );
    fs::remove_file(path).unwrap();
}
