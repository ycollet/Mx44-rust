use crate::{
    midi::MidiEvent,
    patch::{CHANNELS, Envelope, PROGRAMS, Session, Sound, Waveform},
};
use std::f32::consts::{FRAC_PI_2, TAU};

#[derive(Clone, Copy)]
struct EnvelopeState {
    shape: Envelope,
    stage: usize,
    remaining: u32,
    level: f32,
    delta: f32,
}

impl EnvelopeState {
    fn new(shape: Envelope, rate: f32) -> Self {
        let mut env = Self {
            shape,
            stage: 0,
            remaining: 0,
            level: 0.0,
            delta: 0.0,
        };
        env.enter(0, rate);
        env
    }

    fn enter(&mut self, stage: usize, rate: f32) {
        self.stage = stage;
        self.remaining = (self.shape.times[stage] * rate).round().max(1.0) as u32;
        self.delta = (self.shape.levels[stage] - self.level) / self.remaining as f32;
    }

    fn release(&mut self, rate: f32) {
        if self.stage < 6 {
            self.enter(6, rate);
        }
    }

    fn next(&mut self, rate: f32) -> f32 {
        if self.stage == 8 {
            return 0.0;
        }
        if self.remaining > 0 {
            self.level += self.delta;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.level = self.shape.levels[self.stage];
                match self.stage {
                    5 if self.shape.loop_sustain => self.enter(4, rate),
                    5 => {}
                    7 => {
                        self.stage = 8;
                        self.level = 0.0;
                    }
                    n => self.enter(n + 1, rate),
                }
            }
        }
        self.level.clamp(0.0, 1.0)
    }
}

struct Voice {
    channel: usize,
    key: u8,
    velocity: f32,
    age: u64,
    held: bool,
    released: bool,
    phase: [f32; 4],
    previous: [f32; 4],
    envelopes: [EnvelopeState; 4],
}

impl Voice {
    fn new(channel: usize, key: u8, velocity: u8, age: u64, sound: &Sound, rate: f32) -> Self {
        Self {
            channel,
            key,
            velocity: velocity as f32 / 127.0,
            age,
            held: true,
            released: false,
            phase: [0.0; 4],
            previous: [0.0; 4],
            envelopes: std::array::from_fn(|i| {
                EnvelopeState::new(sound.operators[i].envelope, rate)
            }),
        }
    }

    fn release(&mut self, rate: f32) {
        self.released = true;
        for envelope in &mut self.envelopes {
            envelope.release(rate);
        }
    }

    fn finished(&self) -> bool {
        self.envelopes.iter().all(|e| e.stage == 8)
    }

    fn render(&mut self, sound: &Sound, channel: &Channel, lfo: f32, rate: f32) -> [f32; 2] {
        let frequency =
            440.0 * 2.0_f32.powf((self.key as f32 - 69.0 + channel.bend * 2.0 + lfo) / 12.0);
        let mut next = [0.0; 4];
        let mut output = [0.0; 2];
        for (dst, value) in next.iter_mut().enumerate() {
            let op = &sound.operators[dst];
            let mut pm = 0.0;
            let mut am = 1.0;
            for src in 0..4 {
                pm += sound.pm[dst][src] * self.previous[src] * channel.timbre;
                am += sound.am[dst][src] * self.previous[src];
            }
            let hz = frequency * op.ratio * 2.0_f32.powf(op.detune / 1200.0);
            // Suppress oscillators above Nyquist; modulation can still alias, as in the original.
            let audible = if hz < rate * 0.49 { 1.0 } else { 0.0 };
            let phase = self.phase[dst] + pm;
            let basic = waveform(op.wave, phase);
            let doubled = waveform(op.wave, 2.0 * phase + std::f32::consts::PI / 3.0);
            let env = self.envelopes[dst].next(rate);
            let velocity = 1.0 - op.velocity + op.velocity * self.velocity;
            *value = ((basic * (1.0 - op.octave_mix) + doubled * op.octave_mix)
                * env
                * env
                * velocity
                * am.clamp(-2.0, 2.0)
                * audible)
                .clamp(-4.0, 4.0);
            self.phase[dst] = (self.phase[dst] + TAU * hz.min(rate) / rate).rem_euclid(TAU);
            let angle = ((op.pan + channel.pan).clamp(-1.0, 1.0) + 1.0) * FRAC_PI_2 * 0.5;
            output[0] += *value * op.level * angle.cos();
            output[1] += *value * op.level * angle.sin();
        }
        self.previous = next;
        let volume = channel.volume * channel.expression * 0.18;
        [output[0] * volume, output[1] * volume]
    }
}

fn waveform(wave: Waveform, phase: f32) -> f32 {
    match wave {
        Waveform::Sine => phase.sin(),
        Waveform::Mx => {
            // Two parabolic sine approximations, 60 degrees apart.
            let bump = |p: f32| {
                let x = (p / TAU + 0.5).rem_euclid(1.0) * 2.0 - 1.0;
                4.0 * x * (1.0 - x.abs())
            };
            (bump(phase) + bump(phase + std::f32::consts::PI / 3.0)) * 0.577_350_26
        }
        Waveform::Triangle => phase.sin().asin() / FRAC_PI_2,
        Waveform::SoftSaw => {
            (phase.sin() + (2.0 * phase).sin() * 0.5 + (3.0 * phase).sin() / 3.0) / 1.5
        }
    }
}

#[derive(Clone, Copy)]
struct Channel {
    program: u8,
    volume: f32,
    expression: f32,
    pan: f32,
    bend: f32,
    modulation: f32,
    pressure: f32,
    timbre: f32,
    sustain: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            program: 0,
            volume: 1.0,
            expression: 1.0,
            pan: 0.0,
            bend: 0.0,
            modulation: 0.0,
            pressure: 0.0,
            timbre: 1.0,
            sustain: false,
        }
    }
}

struct Delay {
    buffer: Vec<[f32; 2]>,
    cursor: usize,
}

impl Delay {
    fn new(rate: f32) -> Self {
        Self {
            buffer: vec![[0.0; 2]; (rate * 1.6).ceil() as usize + 1],
            cursor: 0,
        }
    }

    fn next(&mut self, input: [f32; 2], sound: &Sound, rate: f32) -> [f32; 2] {
        let offset = (sound.delay_time * rate).round() as usize;
        let index = (self.cursor + self.buffer.len() - offset) % self.buffer.len();
        let wet = self.buffer[index];
        self.buffer[self.cursor] = [
            input[0] + wet[1] * sound.delay_feedback,
            input[1] + wet[0] * sound.delay_feedback,
        ];
        self.cursor = (self.cursor + 1) % self.buffer.len();
        [
            input[0] + wet[0] * sound.delay_mix,
            input[1] + wet[1] * sound.delay_mix,
        ]
    }
}

pub struct Synth {
    rate: f32,
    voices: Vec<Option<Voice>>,
    channels: [Channel; CHANNELS],
    sounds: [Sound; CHANNELS],
    bank: [Sound; PROGRAMS],
    delays: [Delay; CHANNELS],
    lfo_phase: [f32; CHANNELS],
    age: u64,
    master: f32,
}

impl Synth {
    /// Session must validate; sample rate must be in 8,000..=384,000 and voices in 1..=256.
    pub fn new(rate: u32, voices: usize, session: &Session) -> Self {
        assert!((8_000..=384_000).contains(&rate));
        assert!((1..=256).contains(&voices));
        assert!(session.validate().is_ok(), "invalid synthesizer session");
        Self {
            rate: rate as f32,
            voices: (0..voices).map(|_| None).collect(),
            channels: std::array::from_fn(|i| Channel {
                program: session.programs[i],
                ..Channel::default()
            }),
            sounds: session.channels,
            bank: std::array::from_fn(|i| session.patches[i].sound),
            delays: std::array::from_fn(|_| Delay::new(rate as f32)),
            lfo_phase: [0.0; CHANNELS],
            age: 0,
            master: session.master,
        }
    }

    pub fn set_sound(&mut self, channel: usize, sound: Sound) {
        if let Some(slot) = self.sounds.get_mut(channel) {
            *slot = sound;
        }
    }

    pub fn set_preset(&mut self, program: usize, sound: Sound) {
        if let Some(slot) = self.bank.get_mut(program) {
            *slot = sound;
        }
    }

    pub fn set_master(&mut self, volume: f32) {
        self.master = volume.clamp(0.0, 1.0);
    }

    pub fn program(&self, channel: usize) -> u8 {
        self.channels[channel].program
    }
    pub fn active_voices(&self) -> usize {
        self.voices.iter().flatten().count()
    }

    pub fn panic(&mut self) {
        for voice in &mut self.voices {
            *voice = None;
        }
        for channel in &mut self.channels {
            channel.sustain = false;
        }
        for delay in &mut self.delays {
            delay.buffer.fill([0.0; 2]);
        }
    }

    pub fn event(&mut self, event: MidiEvent) {
        use MidiEvent::*;
        let channel = match event {
            NoteOn { channel, .. }
            | NoteOff { channel, .. }
            | Control { channel, .. }
            | Program { channel, .. }
            | Bend { channel, .. }
            | Pressure { channel, .. } => channel as usize,
        };
        if channel >= CHANNELS {
            return;
        }
        match event {
            NoteOn {
                key, velocity: 0, ..
            }
            | NoteOff { key, .. } => {
                // Release the oldest held instance, so overlapping repeated notes remain paired.
                if let Some(voice) = self
                    .voices
                    .iter_mut()
                    .flatten()
                    .filter(|v| v.channel == channel && v.key == key && v.held)
                    .min_by_key(|v| v.age)
                {
                    voice.held = false;
                    if !self.channels[channel].sustain {
                        voice.release(self.rate);
                    }
                }
            }
            NoteOn { key, velocity, .. } if key < 128 && velocity < 128 => {
                let slot = self
                    .voices
                    .iter()
                    .position(Option::is_none)
                    .unwrap_or_else(|| {
                        self.voices
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, v)| {
                                let v = v.as_ref().unwrap();
                                (!v.released, v.age)
                            })
                            .unwrap()
                            .0
                    });
                self.age += 1;
                self.voices[slot] = Some(Voice::new(
                    channel,
                    key,
                    velocity,
                    self.age,
                    &self.sounds[channel],
                    self.rate,
                ));
            }
            Program { program, .. } if (program as usize) < PROGRAMS => {
                self.channels[channel].program = program;
                self.sounds[channel] = self.bank[program as usize];
            }
            Bend { value, .. } => {
                self.channels[channel].bend = (value.min(16383) as f32 - 8192.0) / 8192.0
            }
            Pressure { value, .. } => {
                self.channels[channel].pressure = value.min(127) as f32 / 127.0
            }
            Control {
                controller, value, ..
            } => {
                let normalized = value.min(127) as f32 / 127.0;
                match controller {
                    1 => self.channels[channel].modulation = normalized,
                    7 => self.channels[channel].volume = normalized,
                    10 => {
                        self.channels[channel].pan = ((value as f32 - 64.0) / 63.0).clamp(-1.0, 1.0)
                    }
                    11 => self.channels[channel].expression = normalized,
                    64 => {
                        self.channels[channel].sustain = value >= 64;
                        if value < 64 {
                            self.release_unheld(channel);
                        }
                    }
                    70 | 74 => self.channels[channel].timbre = normalized * 2.0,
                    120 => {
                        for voice in &mut self.voices {
                            if voice.as_ref().is_some_and(|v| v.channel == channel) {
                                *voice = None;
                            }
                        }
                        self.delays[channel].buffer.fill([0.0; 2]);
                    }
                    121 => {
                        let program = self.channels[channel].program;
                        self.channels[channel] = Channel {
                            program,
                            ..Channel::default()
                        };
                        self.release_unheld(channel);
                    }
                    123 => {
                        for voice in self
                            .voices
                            .iter_mut()
                            .flatten()
                            .filter(|v| v.channel == channel)
                        {
                            voice.held = false;
                            if !self.channels[channel].sustain {
                                voice.release(self.rate);
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn release_unheld(&mut self, channel: usize) {
        for voice in self
            .voices
            .iter_mut()
            .flatten()
            .filter(|v| v.channel == channel && !v.held)
        {
            voice.release(self.rate);
        }
    }

    pub fn next_frame(&mut self) -> [f32; 2] {
        let mut mixes = [[0.0; 2]; CHANNELS];
        let mut lfos = [0.0; CHANNELS];
        for (i, lfo) in lfos.iter_mut().enumerate() {
            let channel = self.channels[i];
            *lfo = self.lfo_phase[i].sin()
                * (self.sounds[i].lfo_depth + channel.modulation * 0.5 + channel.pressure * 0.25);
            self.lfo_phase[i] =
                (self.lfo_phase[i] + TAU * self.sounds[i].lfo_rate / self.rate).rem_euclid(TAU);
        }
        for slot in &mut self.voices {
            if let Some(voice) = slot {
                let ch = voice.channel;
                let sample =
                    voice.render(&self.sounds[ch], &self.channels[ch], lfos[ch], self.rate);
                mixes[ch][0] += sample[0];
                mixes[ch][1] += sample[1];
                if voice.finished() {
                    *slot = None;
                }
            }
        }
        let mut out = [0.0; 2];
        for (i, mix) in mixes.iter_mut().enumerate() {
            let sound = &self.sounds[i];
            if sound.drive > 0.0 {
                let gain = 1.0 + sound.drive * 10.0;
                for value in mix.iter_mut() {
                    *value = (*value * gain).tanh() / gain.sqrt();
                }
            }
            let frame = self.delays[i].next(*mix, sound, self.rate);
            out[0] += frame[0];
            out[1] += frame[1];
        }
        [(out[0] * self.master).tanh(), (out[1] * self.master).tanh()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const RATE: u32 = 8_000;
    fn note(channel: u8, key: u8) -> MidiEvent {
        MidiEvent::NoteOn {
            channel,
            key,
            velocity: 110,
        }
    }
    fn off(channel: u8, key: u8) -> MidiEvent {
        MidiEvent::NoteOff { channel, key }
    }
    fn cc(channel: u8, controller: u8, value: u8) -> MidiEvent {
        MidiEvent::Control {
            channel,
            controller,
            value,
        }
    }
    fn advance(synth: &mut Synth, seconds: f32) {
        for _ in 0..(RATE as f32 * seconds) as usize {
            synth.next_frame();
        }
    }

    #[test]
    fn envelopes_loop_and_release_from_current_level() {
        let shape = Envelope {
            times: [0.01; 8],
            levels: [1.0, 0.9, 0.8, 0.7, 0.2, 0.7, 0.3, 0.0],
            loop_sustain: true,
        };
        let mut env = EnvelopeState::new(shape, 1000.0);
        for _ in 0..103 {
            env.next(1000.0);
        }
        assert!((4..=5).contains(&env.stage));
        let before = env.level;
        env.release(1000.0);
        assert_eq!(env.level, before);
        for _ in 0..20 {
            env.next(1000.0);
        }
        assert_eq!(env.stage, 8);
        assert_eq!(env.level, 0.0);
    }

    #[test]
    fn sustain_channels_and_release_are_independent() {
        let mut synth = Synth::new(RATE, 8, &Session::default());
        synth.event(cc(0, 64, 127));
        synth.event(note(0, 60));
        synth.event(note(1, 64));
        synth.event(off(0, 60));
        synth.event(off(1, 64));
        advance(&mut synth, 0.6);
        assert_eq!(synth.active_voices(), 1);
        synth.event(cc(0, 64, 0));
        advance(&mut synth, 0.6);
        assert_eq!(synth.active_voices(), 0);
    }

    #[test]
    fn voice_limit_repeat_notes_and_panic() {
        let mut synth = Synth::new(RATE, 2, &Session::default());
        synth.event(note(0, 60));
        synth.event(note(0, 60));
        synth.event(off(0, 60));
        assert_eq!(synth.voices.iter().flatten().filter(|v| v.held).count(), 1);
        synth.event(note(0, 70));
        assert_eq!(synth.active_voices(), 2);
        assert!(synth.voices.iter().flatten().any(|v| v.key == 70));
        synth.panic();
        assert_eq!(synth.active_voices(), 0);
        assert_eq!(synth.next_frame(), [0.0, 0.0]);
    }

    #[test]
    fn sine_pitch_is_440_hz_and_bend_is_two_semitones() {
        let mut session = Session::default();
        session.channels[0].operators[0].wave = Waveform::Sine;
        let mut synth = Synth::new(RATE, 4, &session);
        synth.event(note(0, 69));
        advance(&mut synth, 0.5);
        let crossings = |synth: &mut Synth| {
            let mut count = 0;
            let mut previous = 0.0;
            for _ in 0..RATE {
                let value = synth.next_frame()[0];
                if previous <= 0.0 && value > 0.0 {
                    count += 1;
                }
                previous = value;
            }
            count
        };
        assert!((439..=441).contains(&crossings(&mut synth)));
        synth.event(MidiEvent::Bend {
            channel: 0,
            value: 16383,
        });
        assert!((493..=495).contains(&crossings(&mut synth)));
    }

    #[test]
    fn muted_operators_still_modulate_and_both_matrices_affect_audio() {
        let session = Session::default();
        let mut baseline = Synth::new(RATE, 4, &session);
        let mut phase = Synth::new(RATE, 4, &session);
        let mut amplitude = Synth::new(RATE, 4, &session);
        let mut sound = session.channels[0];
        sound.operators[1].ratio = 2.75;
        sound.pm[0][1] = 3.0;
        phase.set_sound(0, sound);
        sound.pm[0][1] = 0.0;
        sound.am[0][1] = 1.0;
        amplitude.set_sound(0, sound);
        for synth in [&mut baseline, &mut phase, &mut amplitude] {
            synth.event(note(0, 60));
        }
        let mut pm_difference = 0.0;
        let mut am_difference = 0.0;
        for _ in 0..RATE / 4 {
            let plain = baseline.next_frame()[0];
            pm_difference += (phase.next_frame()[0] - plain).abs();
            am_difference += (amplitude.next_frame()[0] - plain).abs();
        }
        assert!(pm_difference > 1.0);
        assert!(am_difference > 1.0);
    }

    #[test]
    fn delay_is_stereo_ping_pong_at_the_requested_time() {
        let sound = Sound {
            delay_time: 0.01,
            delay_feedback: 0.5,
            delay_mix: 1.0,
            ..Sound::default()
        };
        let mut delay = Delay::new(1000.0);
        assert_eq!(delay.next([1.0, 0.0], &sound, 1000.0), [1.0, 0.0]);
        for _ in 1..10 {
            assert_eq!(delay.next([0.0; 2], &sound, 1000.0), [0.0; 2]);
        }
        assert_eq!(delay.next([0.0; 2], &sound, 1000.0), [1.0, 0.0]);
        for _ in 11..20 {
            delay.next([0.0; 2], &sound, 1000.0);
        }
        assert_eq!(delay.next([0.0; 2], &sound, 1000.0), [0.0, 0.5]);
    }

    #[test]
    fn controller_reset_releases_sustained_notes_and_sound_off_is_per_channel() {
        let mut synth = Synth::new(RATE, 4, &Session::default());
        synth.event(MidiEvent::Program {
            channel: 0,
            program: 4,
        });
        synth.event(note(0, 60));
        synth.event(note(1, 64));
        synth.event(cc(0, 64, 127));
        synth.event(off(0, 60));
        synth.event(cc(0, 121, 0));
        assert_eq!(synth.program(0), 4);
        assert!(
            synth
                .voices
                .iter()
                .flatten()
                .find(|v| v.channel == 0)
                .unwrap()
                .released
        );
        synth.event(cc(0, 120, 0));
        assert_eq!(synth.active_voices(), 1);
        assert_eq!(synth.voices.iter().flatten().next().unwrap().channel, 1);
    }

    #[test]
    fn all_presets_produce_finite_bounded_audio() {
        let session = Session::default();
        let mut synth = Synth::new(RATE, 8, &session);
        for program in 0..6 {
            synth.panic();
            synth.event(MidiEvent::Program {
                channel: 0,
                program,
            });
            for key in [48, 60, 64, 67] {
                synth.event(note(0, key));
            }
            let mut peak = 0.0_f32;
            for _ in 0..RATE {
                for value in synth.next_frame() {
                    assert!(value.is_finite() && value.abs() <= 1.0);
                    peak = peak.max(value.abs());
                }
            }
            assert!(peak > 0.005, "silent program {program}");
        }
    }
}
