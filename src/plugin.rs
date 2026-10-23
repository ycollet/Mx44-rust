//! Shared host parameter layout. Keep the order stable for project compatibility.
use crate::{
    engine::Synth,
    midi::MidiEvent,
    patch::{CHANNELS, Session, Sound, Waveform},
};

pub const PARAM_COUNT: usize = 131;
pub type Values = [f32; PARAM_COUNT];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Linear,
    Logarithmic,
    Toggle,
    Waveform,
}

#[derive(Clone, Debug)]
pub struct ParameterSpec {
    pub symbol: String,
    pub name: String,
    pub group: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub kind: Kind,
}

pub fn parameter_specs() -> Vec<ParameterSpec> {
    use Kind::*;
    let defaults = encode(&Sound::default(), 0.5);
    let mut specs = Vec::with_capacity(PARAM_COUNT);
    let mut add = |symbol: String, name: String, group: String, min, max, kind| {
        let default = defaults[specs.len()];
        specs.push(ParameterSpec {
            symbol,
            name,
            group,
            min,
            max,
            default,
            kind,
        });
    };
    add(
        "master".into(),
        "Master".into(),
        "Global".into(),
        0.0,
        1.0,
        Linear,
    );
    for op in 0..4 {
        let group = format!("Operator {}", op + 1);
        for (symbol, name, min, max, kind) in [
            ("ratio", "Ratio", 0.125, 32.0, Logarithmic),
            ("detune", "Detune (cents)", -100.0, 100.0, Linear),
            ("level", "Output", 0.0, 1.0, Linear),
            ("pan", "Pan", -1.0, 1.0, Linear),
            ("velocity", "Velocity", 0.0, 1.0, Linear),
            ("octave_mix", "Octave mix", 0.0, 1.0, Linear),
            ("wave", "Waveform", 0.0, 3.0, Waveform),
            ("loop", "Loop sustain", 0.0, 1.0, Toggle),
        ] {
            add(
                format!("op{}_{}", op + 1, symbol),
                name.into(),
                group.clone(),
                min,
                max,
                kind,
            );
        }
        for stage in 0..7 {
            add(
                format!("op{}_env_level{}", op + 1, stage + 1),
                format!("Stage {} level", stage + 1),
                group.clone(),
                0.0,
                1.0,
                Linear,
            );
        }
        for stage in 0..8 {
            add(
                format!("op{}_env_time{}", op + 1, stage + 1),
                format!("Stage {} time (s)", stage + 1),
                group.clone(),
                0.001,
                10.0,
                Logarithmic,
            );
        }
    }
    for (matrix, max) in [("pm", 8.0), ("am", 1.0)] {
        for dst in 0..4 {
            for src in 0..4 {
                add(
                    format!("{matrix}_{}_{}", dst + 1, src + 1),
                    format!("Op {} -> Op {}", src + 1, dst + 1),
                    matrix.to_uppercase(),
                    -max,
                    max,
                    Linear,
                );
            }
        }
    }
    for (symbol, name, min, max, kind) in [
        ("lfo_rate", "LFO rate (Hz)", 0.05, 20.0, Logarithmic),
        ("lfo_depth", "Vibrato (semitones)", 0.0, 1.0, Linear),
        ("drive", "Drive", 0.0, 1.0, Linear),
        ("delay_time", "Delay time (s)", 0.01, 1.5, Linear),
        ("delay_feedback", "Delay feedback", 0.0, 0.95, Linear),
        ("delay_mix", "Delay mix", 0.0, 1.0, Linear),
    ] {
        add(symbol.into(), name.into(), "Effects".into(), min, max, kind);
    }
    assert_eq!(specs.len(), PARAM_COUNT);
    specs
}

pub fn encode(sound: &Sound, master: f32) -> Values {
    let mut values = [0.0; PARAM_COUNT];
    let mut out = values.iter_mut();
    *out.next().unwrap() = master;
    for op in &sound.operators {
        let wave = match op.wave {
            Waveform::Sine => 0.0,
            Waveform::Mx => 1.0,
            Waveform::Triangle => 2.0,
            Waveform::SoftSaw => 3.0,
        };
        for value in [
            op.ratio,
            op.detune,
            op.level,
            op.pan,
            op.velocity,
            op.octave_mix,
            wave,
            if op.envelope.loop_sustain { 1.0 } else { 0.0 },
        ] {
            *out.next().unwrap() = value;
        }
        for value in op.envelope.levels[..7]
            .iter()
            .chain(op.envelope.times.iter())
        {
            *out.next().unwrap() = *value;
        }
    }
    for value in sound.pm.iter().flatten().chain(sound.am.iter().flatten()) {
        *out.next().unwrap() = *value;
    }
    for value in [
        sound.lfo_rate,
        sound.lfo_depth,
        sound.drive,
        sound.delay_time,
        sound.delay_feedback,
        sound.delay_mix,
    ] {
        *out.next().unwrap() = value;
    }
    values
}

pub fn decode(values: &Values) -> (Sound, f32) {
    let mut input = values.iter().copied();
    let mut sound = Sound::default();
    let master = input.next().unwrap();
    for op in &mut sound.operators {
        op.ratio = input.next().unwrap();
        op.detune = input.next().unwrap();
        op.level = input.next().unwrap();
        op.pan = input.next().unwrap();
        op.velocity = input.next().unwrap();
        op.octave_mix = input.next().unwrap();
        op.wave = match input.next().unwrap().round() as u8 {
            0 => Waveform::Sine,
            1 => Waveform::Mx,
            2 => Waveform::Triangle,
            _ => Waveform::SoftSaw,
        };
        op.envelope.loop_sustain = input.next().unwrap() >= 0.5;
        for value in op.envelope.levels[..7]
            .iter_mut()
            .chain(op.envelope.times.iter_mut())
        {
            *value = input.next().unwrap();
        }
    }
    for value in sound
        .pm
        .iter_mut()
        .flatten()
        .chain(sound.am.iter_mut().flatten())
    {
        *value = input.next().unwrap();
    }
    sound.lfo_rate = input.next().unwrap();
    sound.lfo_depth = input.next().unwrap();
    sound.drive = input.next().unwrap();
    sound.delay_time = input.next().unwrap();
    sound.delay_feedback = input.next().unwrap();
    sound.delay_mix = input.next().unwrap();
    (sound, master)
}

/// The host owns timing, parameter state and MIDI; no device or file access.
pub struct PluginEngine {
    synth: Synth,
    previous: Option<Values>,
}

impl PluginEngine {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            synth: Synth::with_shared_delay(sample_rate, 32, &Session::default()),
            previous: None,
        }
    }

    /// Values must be finite and within the shared parameter specification.
    pub fn set_values(&mut self, values: &Values) {
        if self.previous.as_ref() != Some(values) {
            let (sound, master) = decode(values);
            for channel in 0..CHANNELS {
                self.synth.set_sound(channel, sound);
            }
            self.synth.set_master(master);
            self.previous = Some(*values);
        }
    }

    pub fn reset(&mut self) {
        self.synth.reset();
    }
    pub fn set_master(&mut self, value: f32) {
        self.synth.set_master(value);
    }
    pub fn event(&mut self, event: MidiEvent) {
        // The host's parameter state is authoritative, not a hidden MIDI program bank.
        if !matches!(event, MidiEvent::Program { .. }) {
            self.synth.event(event);
        }
    }
    pub fn choke(&mut self, channel: u8, note: u8) {
        self.synth.choke(channel, note);
    }
    pub fn next_frame(&mut self) -> [f32; 2] {
        self.synth.next_frame()
    }

    pub fn is_active(&self) -> bool {
        self.synth.is_active()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};
    use std::collections::HashSet;

    #[test]
    fn schema_matches_sound_and_has_unique_stable_symbols() {
        let specs = parameter_specs();
        assert_eq!(
            specs
                .iter()
                .map(|s| &s.symbol)
                .collect::<HashSet<_>>()
                .len(),
            PARAM_COUNT
        );
        let defaults = std::array::from_fn(|i| specs[i].default);
        assert_eq!(decode(&defaults), (Sound::default(), 0.5));
        let mut rng = StdRng::seed_from_u64(44);
        for _ in 0..128 {
            let sound = Sound::random(&mut rng);
            let values = encode(&sound, 0.35);
            assert_eq!(decode(&values), (sound, 0.35));
            for (spec, value) in specs.iter().zip(values) {
                assert!(
                    value.is_finite() && (spec.min..=spec.max).contains(&value),
                    "{}",
                    spec.symbol
                );
            }
        }
        assert_eq!(specs[0].symbol, "master");
        assert_eq!(specs[93].symbol, "pm_1_1");
        assert_eq!(specs[130].symbol, "delay_mix");
    }

    #[test]
    fn host_choke_wildcards_stop_matching_voices() {
        let mut plugin = PluginEngine::new(8000);
        plugin.set_values(&encode(&Sound::default(), 0.5));
        for (channel, key) in [(0, 60), (15, 60), (15, 64)] {
            plugin.event(MidiEvent::NoteOn {
                channel,
                key,
                velocity: 100,
            });
        }
        plugin.choke(u8::MAX, 60);
        assert_eq!(plugin.synth.active_voices(), 1);
        plugin.choke(15, u8::MAX);
        assert_eq!(plugin.synth.active_voices(), 0);
        assert_eq!(plugin.next_frame(), [0.0; 2]);
    }

    #[test]
    fn reset_silences_tails_and_restores_channel_controllers() {
        let mut plugin = PluginEngine::new(8000);
        plugin.set_values(&encode(&Sound::default(), 0.5));
        plugin.event(MidiEvent::NoteOn {
            channel: 15,
            key: 60,
            velocity: 100,
        });
        for _ in 0..100 {
            plugin.next_frame();
        }
        plugin.event(MidiEvent::Control {
            channel: 15,
            controller: 7,
            value: 0,
        });
        plugin.reset();
        assert_eq!(plugin.next_frame(), [0.0; 2]);
        plugin.event(MidiEvent::NoteOn {
            channel: 15,
            key: 60,
            velocity: 100,
        });
        assert!((0..1000).any(|_| plugin.next_frame()[0].abs() > 0.01));
        plugin.choke(15, 60);
        assert_eq!(plugin.next_frame(), [0.0; 2]);
    }

    #[test]
    fn silent_gaps_keep_pending_echoes_alive_until_history_is_empty() {
        let mut plugin = PluginEngine::new(8000);
        plugin.set_values(&encode(
            &Sound {
                delay_time: 1.5,
                delay_feedback: 0.0,
                delay_mix: 1.0,
                ..Sound::default()
            },
            0.5,
        ));
        assert!(!plugin.is_active());
        plugin.event(MidiEvent::NoteOn {
            channel: 15,
            key: 60,
            velocity: 100,
        });
        for _ in 0..160 {
            plugin.next_frame();
        }
        plugin.choke(15, 60);
        for _ in 0..1000 {
            assert_eq!(plugin.next_frame(), [0.0; 2]);
            assert!(plugin.is_active());
        }
        let mut heard_echo = false;
        for _ in 0..24_000 {
            heard_echo |= plugin.next_frame()[0].abs() > 0.001;
        }
        assert!(heard_echo);
        assert!(!plugin.is_active());
        plugin.event(MidiEvent::NoteOn {
            channel: 0,
            key: 67,
            velocity: 110,
        });
        assert!(plugin.is_active());
    }
}
