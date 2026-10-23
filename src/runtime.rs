use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use midir::{MidiInput, MidiInputConnection};
use mx44_rust::{
    engine::Synth,
    midi::{self, MidiEvent},
    patch::{CHANNELS, Session, Sound},
};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};

// Inline patch data avoids allocation and destruction on the audio thread.
#[allow(clippy::large_enum_variant)]
pub enum Command {
    Midi(MidiEvent),
    Sound(usize, Sound),
    Preset(usize, Sound),
    Select(u8, u8),
    Master(f32),
    Panic,
}

pub struct Stats {
    pub voices: AtomicUsize,
    pub peak: AtomicU32,
    pub programs: [AtomicU8; CHANNELS],
    pub revisions: [AtomicU64; CHANNELS],
    pub overflow: AtomicU64,
    pub audio_failed: AtomicBool,
    emergency_stop: AtomicBool,
}

#[derive(Clone)]
pub struct Control {
    producer: Arc<Mutex<Producer<Command>>>,
    pub stats: Arc<Stats>,
}

impl Control {
    pub fn send(&self, command: Command) -> Result<()> {
        let mut producer = self
            .producer
            .lock()
            .map_err(|_| anyhow::anyhow!("audio command queue poisoned"))?;
        if producer.push(command).is_err() {
            self.stats.overflow.fetch_add(1, Ordering::Relaxed);
            self.stats.emergency_stop.store(true, Ordering::Release);
            bail!("audio command queue full; emergency all-sound-off requested");
        }
        Ok(())
    }
}

pub struct Audio {
    pub control: Control,
    pub description: String,
    _stream: cpal::Stream,
}

pub fn list_audio() -> Result<()> {
    let host = cpal::default_host();
    for device in host.output_devices()? {
        println!("{}", device.name()?);
    }
    Ok(())
}

impl Audio {
    pub fn start(session: &Session, voices: usize, device_name: Option<&str>) -> Result<Self> {
        let host = cpal::default_host();
        let device = if let Some(name) = device_name {
            let mut matching = Vec::new();
            for device in host.output_devices()? {
                if device.name()?.contains(name) {
                    matching.push(device);
                }
            }
            if matching.len() != 1 {
                bail!(
                    "audio device '{name}' matched {} devices; use --list-audio",
                    matching.len()
                );
            }
            matching.remove(0)
        } else {
            host.default_output_device()
                .context("no default audio output device")?
        };
        let supported = device
            .default_output_config()
            .context("get default output format")?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        if !(8_000..=384_000).contains(&config.sample_rate.0) {
            bail!("unsupported device sample rate {}", config.sample_rate.0);
        }
        let description = format!(
            "{} / {} Hz / {} channels",
            device.name()?,
            config.sample_rate.0,
            config.channels
        );
        let synth = Synth::new(config.sample_rate.0, voices, session);
        let (producer, consumer) = RingBuffer::new(512);
        let stats = Arc::new(Stats {
            voices: AtomicUsize::new(0),
            peak: AtomicU32::new(0),
            programs: std::array::from_fn(|i| AtomicU8::new(session.programs[i])),
            revisions: std::array::from_fn(|_| AtomicU64::new(0)),
            overflow: AtomicU64::new(0),
            audio_failed: AtomicBool::new(false),
            emergency_stop: AtomicBool::new(false),
        });
        let stream = match format {
            cpal::SampleFormat::F32 => {
                build_stream::<f32>(&device, &config, synth, consumer, stats.clone())?
            }
            cpal::SampleFormat::I16 => {
                build_stream::<i16>(&device, &config, synth, consumer, stats.clone())?
            }
            cpal::SampleFormat::U16 => {
                build_stream::<u16>(&device, &config, synth, consumer, stats.clone())?
            }
            cpal::SampleFormat::I32 => {
                build_stream::<i32>(&device, &config, synth, consumer, stats.clone())?
            }
            _ => bail!("unsupported audio sample format {format}; choose another output device"),
        };
        stream.play().context("start audio output")?;
        Ok(Self {
            control: Control {
                producer: Arc::new(Mutex::new(producer)),
                stats,
            },
            description,
            _stream: stream,
        })
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut synth: Synth,
    mut consumer: Consumer<Command>,
    stats: Arc<Stats>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    let error_stats = stats.clone();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                // Bounded work per callback even under a stream of MIDI traffic.
                for _ in 0..512 {
                    let Ok(command) = consumer.pop() else { break };
                    match command {
                        Command::Midi(event) => {
                            synth.event(event);
                            if let MidiEvent::Program { channel, .. } = event {
                                let ch = channel as usize;
                                if ch < CHANNELS {
                                    stats.programs[ch].store(synth.program(ch), Ordering::Relaxed);
                                    stats.revisions[ch].fetch_add(1, Ordering::Release);
                                }
                            }
                        }
                        Command::Sound(channel, sound) => synth.set_sound(channel, sound),
                        Command::Preset(program, sound) => synth.set_preset(program, sound),
                        Command::Select(channel, program) => {
                            synth.event(MidiEvent::Program { channel, program });
                            stats.programs[channel as usize].store(program, Ordering::Relaxed);
                        }
                        Command::Master(value) => synth.set_master(value),
                        Command::Panic => synth.panic(),
                    }
                }
                if stats.emergency_stop.swap(false, Ordering::AcqRel) {
                    synth.panic();
                }
                let mut peak = 0.0_f32;
                for frame in data.chunks_mut(channels) {
                    let sample = synth.next_frame();
                    peak = peak.max(sample[0].abs()).max(sample[1].abs());
                    if channels == 1 {
                        frame[0] = T::from_sample((sample[0] + sample[1]) * 0.5);
                    } else {
                        frame[0] = T::from_sample(sample[0]);
                        frame[1] = T::from_sample(sample[1]);
                        for extra in &mut frame[2..] {
                            *extra = T::from_sample(0.0);
                        }
                    }
                }
                stats.peak.store(peak.to_bits(), Ordering::Relaxed);
                stats.voices.store(synth.active_voices(), Ordering::Relaxed);
            },
            move |error| {
                error_stats.audio_failed.store(true, Ordering::Release);
                eprintln!("Audio stream failed: {error}");
            },
            None,
        )
        .context("build audio output stream")
}

pub fn list_midi() -> Result<()> {
    let input = MidiInput::new("Mx44 Rust")?;
    for (index, port) in input.ports().iter().enumerate() {
        println!("{index}: {}", input.port_name(port)?);
    }
    Ok(())
}

pub fn open_midi(
    control: Control,
    port_name: Option<&str>,
) -> Result<MidiInputConnection<Control>> {
    let mut input = MidiInput::new("Mx44 Rust").context("initialize MIDI input")?;
    input.ignore(midir::Ignore::None);
    let callback = |_timestamp: u64, bytes: &[u8], control: &mut Control| {
        if let Some(event) = midi::parse(bytes)
            && let Err(error) = control.send(Command::Midi(event))
        {
            eprintln!("MIDI: {error}");
        }
    };
    if let Some(name) = port_name {
        let ports = input.ports();
        let matching = if let Ok(index) = name.parse::<usize>() {
            vec![
                ports
                    .get(index)
                    .with_context(|| {
                        format!("MIDI input index {index} does not exist; use --list-midi")
                    })?
                    .clone(),
            ]
        } else {
            let mut matching = Vec::new();
            for port in ports {
                if input.port_name(&port)?.contains(name) {
                    matching.push(port);
                }
            }
            matching
        };
        if matching.len() != 1 {
            bail!(
                "MIDI port '{name}' matched {} ports; use --list-midi",
                matching.len()
            );
        }
        return input
            .connect(&matching[0], "Mx44 Rust input", callback, control)
            .map_err(|e| anyhow::anyhow!("connect MIDI: {e}"));
    }
    #[cfg(unix)]
    {
        use midir::os::unix::VirtualInput;
        input
            .create_virtual("Mx44 Rust", callback, control)
            .map_err(|e| anyhow::anyhow!("create virtual MIDI input: {e}"))
    }
    #[cfg(not(unix))]
    {
        let _ = (callback, control);
        bail!("select an input with --midi-port (see --list-midi), or use --no-midi")
    }
}
