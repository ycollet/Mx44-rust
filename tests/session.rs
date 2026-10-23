use mx44_rust::patch::Session;
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
