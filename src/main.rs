mod icon;
mod runtime;
mod ui;

use anyhow::{Context, Result, ensure};
use clap::Parser;
use mx44_rust::{
    engine::Synth,
    legacy,
    midi::MidiEvent,
    patch::{CHANNELS, Session},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Parser)]
#[command(version, about = "Four-operator matrix synthesizer inspired by Mx44")]
struct Options {
    /// Run without a window (Ctrl-C to save and exit)
    #[arg(long)]
    console: bool,
    #[arg(long, default_value = "32", value_parser = clap::value_parser!(u16).range(1..=256))]
    voices: u16,
    /// Read a Rust JSON session (never the original C binary format)
    #[arg(long)]
    load_from: Option<PathBuf>,
    /// Convert a little-endian Mx44 C bank to JSON, without opening devices
    #[arg(long, requires = "save_to", conflicts_with_all = [
        "load_from", "render", "console", "program", "list_audio", "list_midi",
        "voices", "no_midi", "midi_port", "audio_device", "duration"
    ])]
    import_legacy: Option<PathBuf>,
    /// Write session here instead of the default user state
    #[arg(long)]
    save_to: Option<PathBuf>,
    /// Do not open MIDI; useful with only the on-screen keyboard
    #[arg(long)]
    no_midi: bool,
    /// Existing MIDI input index or unique name fragment; otherwise create a virtual input
    #[arg(long)]
    midi_port: Option<String>,
    #[arg(long)]
    list_midi: bool,
    #[arg(long)]
    list_audio: bool,
    /// Unique output device name fragment (default: system output)
    #[arg(long)]
    audio_device: Option<String>,
    /// Render a demo phrase to a stereo WAV without audio, MIDI or a display
    #[arg(long)]
    render: Option<PathBuf>,
    /// Offline render length, including release tail, in seconds (2..=600)
    #[arg(long, default_value = "8")]
    duration: f32,
    /// Offline rate, or original rate for legacy conversion; live audio follows the device
    #[arg(long, default_value = "48000", value_parser = clap::value_parser!(u32).range(8000..=192000))]
    sample_rate: u32,
    /// Initial preset on channel 1, numbered 1..=128
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=128))]
    program: Option<u8>,
}

fn default_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path).join("mx44-rust/session.json"));
    }
    Ok(
        PathBuf::from(std::env::var_os("HOME").context("HOME is not set; specify --save-to")?)
            .join(".config/mx44-rust/session.json"),
    )
}

fn render(options: &Options, session: &Session, path: &std::path::Path) -> Result<()> {
    ensure!(
        options.duration.is_finite() && (2.0..=600.0).contains(&options.duration),
        "--duration must be finite and between 2 and 600 seconds"
    );
    let mut synth = Synth::new(options.sample_rate, options.voices as usize, session);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: options.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("create {}", path.display()))?;
    let frames = (options.duration * options.sample_rate as f32).round() as u64;
    let beat = (options.sample_rate as f32 * 0.4) as u64;
    let keys = [60, 64, 67, 72, 67, 64, 60, 55];
    for frame in 0..frames {
        if frame % beat == 0 && frame < frames * 2 / 3 {
            let key = keys[(frame / beat) as usize % keys.len()];
            synth.event(MidiEvent::NoteOn {
                channel: 0,
                key,
                velocity: 105,
            });
        }
        if frame % beat == beat * 3 / 4 {
            let key = keys[(frame / beat) as usize % keys.len()];
            synth.event(MidiEvent::NoteOff { channel: 0, key });
        }
        // Leave the final third for envelopes and effects to decay.
        if frame == frames * 2 / 3 {
            synth.event(MidiEvent::Control {
                channel: 0,
                controller: 123,
                value: 0,
            });
        }
        for sample in synth.next_frame() {
            writer.write_sample((sample * i16::MAX as f32).round() as i16)?;
        }
    }
    writer.finalize()?;
    println!(
        "Rendered {} Hz stereo WAV: {}",
        options.sample_rate,
        path.display()
    );
    Ok(())
}

fn run() -> Result<()> {
    let options = Options::parse();
    if let Some(source) = &options.import_legacy {
        let target = options
            .save_to
            .as_ref()
            .context("--import-legacy requires --save-to")?;
        let source_path = std::fs::canonicalize(source)
            .with_context(|| format!("read legacy bank {}", source.display()))?;
        let imported = legacy::load(source, options.sample_rate)?;
        ensure!(
            target.file_name().is_some(),
            "import output path must name a file"
        );
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create import directory {}", parent.display()))?;
        match std::fs::canonicalize(target) {
            Ok(path) => ensure!(
                path != source_path,
                "cannot overwrite the original legacy bank"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("resolve {}", target.display()));
            }
        }
        imported.session.save(target)?;
        for warning in &imported.warnings {
            eprintln!("Import warning: {warning}");
        }
        println!("Imported 128 legacy presets into {}", target.display());
        return Ok(());
    }
    if options.list_audio {
        return runtime::list_audio();
    }
    if options.list_midi {
        return runtime::list_midi();
    }
    ensure!(
        options.load_from.is_none() || options.save_to.is_some() || options.render.is_some(),
        "--load-from requires --save-to, except for read-only --render"
    );
    ensure!(
        !(options.no_midi && options.midi_port.is_some()),
        "--no-midi conflicts with --midi-port"
    );
    let save_path = match &options.save_to {
        Some(path) => Some(path.clone()),
        None if options.render.is_some() && options.load_from.is_some() => None,
        None => Some(default_path()?),
    };
    let load_path = options.load_from.as_ref().or(save_path.as_ref());
    let mut session = if let Some(path) = load_path {
        match std::fs::metadata(path) {
            Ok(_) => Session::load(path)?,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && options.load_from.is_none() =>
            {
                Session::default()
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        }
    } else {
        Session::default()
    };
    if let Some(program) = options.program {
        session.programs[0] = program - 1;
        session.channels[0] = session.patches[(program - 1) as usize].sound;
    }
    if let Some(path) = &options.render {
        return render(&options, &session, path);
    }
    let audio = runtime::Audio::start(
        &session,
        options.voices as usize,
        options.audio_device.as_deref(),
    )?;
    println!("Audio: {}", audio.description);
    let _midi = if options.no_midi {
        None
    } else {
        Some(runtime::open_midi(
            audio.control.clone(),
            options.midi_port.as_deref(),
        )?)
    };
    let path = save_path.context("missing save path")?;
    if options.console {
        let running = Arc::new(AtomicBool::new(true));
        let signal = running.clone();
        ctrlc::set_handler(move || {
            signal.store(false, Ordering::Release);
        })?;
        println!("Mx44 Rust listening for MIDI. Ctrl-C saves and exits.");
        while running.load(Ordering::Acquire)
            && !audio.control.stats.audio_failed.load(Ordering::Acquire)
        {
            std::thread::sleep(Duration::from_millis(100));
        }
        for ch in 0..CHANNELS {
            if audio.control.stats.revisions[ch].load(Ordering::Acquire) > 0 {
                let program = audio.control.stats.programs[ch].load(Ordering::Relaxed);
                session.programs[ch] = program;
                session.channels[ch] = session.patches[program as usize].sound;
            }
        }
        session.save(&path)?;
        ensure!(
            !audio.control.stats.audio_failed.load(Ordering::Acquire),
            "audio stream stopped unexpectedly"
        );
    } else {
        session = ui::run(session, audio, path.clone())?;
        session.save(&path)?;
    }
    println!("Session saved to {}", path.display());
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Mx44 Rust: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
