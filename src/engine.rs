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
    frequency: f32,
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
            frequency: 440.0 * 2.0_f32.powf((key as f32 - 69.0) / 12.0),
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

    fn render(
        &mut self,
        sound: &Sound,
        channel: &Channel,
        coefficients: &Coefficients,
        pitch: f32,
        rate: f32,
    ) -> [f32; 2] {
        let frequency = self.frequency * pitch;
        let mut next = [0.0; 4];
        let mut output = [0.0; 2];
        for (dst, value) in next.iter_mut().enumerate() {
            if self.envelopes[dst].stage == 8 {
                continue;
            }
            let op = &sound.operators[dst];
            let mut pm = 0.0;
            let mut am = 1.0;
            for src in 0..4 {
                pm += sound.pm[dst][src] * self.previous[src] * channel.timbre;
                am += sound.am[dst][src] * self.previous[src];
            }
            let hz = frequency * coefficients.ratios[dst];
            // Suppress oscillators above Nyquist; modulation can still alias, as in the original.
            let audible = if hz < rate * 0.49 { 1.0 } else { 0.0 };
            let phase = self.phase[dst] + pm;
            let oscillator = match op.octave_mix {
                0.0 => waveform(op.wave, phase),
                1.0 => waveform(op.wave, 2.0 * phase + std::f32::consts::PI / 3.0),
                mix => {
                    waveform(op.wave, phase) * (1.0 - mix)
                        + waveform(op.wave, 2.0 * phase + std::f32::consts::PI / 3.0) * mix
                }
            };
            let env = self.envelopes[dst].next(rate);
            let velocity = 1.0 - op.velocity + op.velocity * self.velocity;
            *value = (oscillator * env * env * velocity * am.clamp(-2.0, 2.0) * audible)
                .clamp(-4.0, 4.0);
            let phase = self.phase[dst] + TAU * hz.min(rate) / rate;
            self.phase[dst] = if phase >= TAU {
                phase.rem_euclid(TAU)
            } else {
                phase
            };
            output[0] += *value * coefficients.gains[dst][0];
            output[1] += *value * coefficients.gains[dst][1];
        }
        self.previous = next;
        [
            output[0] * coefficients.volume,
            output[1] * coefficients.volume,
        ]
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
            // Harmonic identities replace three sine calls with one sin/cos pair.
            let (sin, cos) = phase.sin_cos();
            let second = 2.0 * sin * cos;
            let third = sin * (3.0 - 4.0 * sin * sin);
            (sin + second * 0.5 + third / 3.0) / 1.5
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
    written: usize,
    nonzero: usize,
}

impl Delay {
    fn new(rate: f32) -> Self {
        Self {
            buffer: vec![[0.0; 2]; (rate * 1.5).ceil() as usize + 1],
            cursor: 0,
            written: 0,
            nonzero: 0,
        }
    }

    fn clear(&mut self) {
        self.written = 0;
        self.nonzero = 0;
    }

    fn is_silent(&self) -> bool {
        self.nonzero == 0
    }

    fn next(&mut self, input: [f32; 2], sound: &Sound, offset: usize) -> [f32; 2] {
        if self.is_silent() && input == [0.0; 2] {
            return input;
        }
        let index = if self.cursor >= offset {
            self.cursor - offset
        } else {
            self.cursor + self.buffer.len() - offset
        };
        let wet = if offset <= self.written {
            self.buffer[index]
        } else {
            [0.0; 2]
        };
        if self.written == self.buffer.len() {
            self.nonzero -= usize::from(self.buffer[self.cursor] != [0.0; 2]);
        }
        let mut next = [
            input[0] + wet[1] * sound.delay_feedback,
            input[1] + wet[0] * sound.delay_feedback,
        ];
        // Flush inaudible feedback below -240 dBFS, including subnormal fixed points.
        for sample in &mut next {
            if sample.abs() < 1e-12 {
                *sample = 0.0;
            }
        }
        self.buffer[self.cursor] = next;
        self.nonzero += usize::from(next != [0.0; 2]);
        self.written = (self.written + 1).min(self.buffer.len());
        self.cursor += 1;
        if self.cursor == self.buffer.len() {
            self.cursor = 0;
        }
        [
            input[0] + wet[0] * sound.delay_mix,
            input[1] + wet[1] * sound.delay_mix,
        ]
    }
}

struct Coefficients {
    ratios: [f32; 4],
    gains: [[f32; 2]; 4],
    volume: f32,
    pitch: f32,
    lfo_step: f32,
    delay_offset: usize,
    drive_gain: f32,
    drive_scale: f32,
}

impl Coefficients {
    fn new(sound: &Sound, channel: &Channel, rate: f32) -> Self {
        let drive_gain = 1.0 + sound.drive * 10.0;
        Self {
            ratios: sound
                .operators
                .map(|op| op.ratio * 2.0_f32.powf(op.detune / 1200.0)),
            gains: sound.operators.map(|op| {
                let angle = ((op.pan + channel.pan).clamp(-1.0, 1.0) + 1.0) * FRAC_PI_2 * 0.5;
                let (sin, cos) = angle.sin_cos();
                [op.level * cos, op.level * sin]
            }),
            volume: channel.volume * channel.expression * 0.18,
            pitch: 2.0_f32.powf(channel.bend * 2.0 / 12.0),
            lfo_step: TAU * sound.lfo_rate / rate,
            delay_offset: (sound.delay_time * rate).round() as usize,
            drive_gain,
            drive_scale: 1.0 / drive_gain.sqrt(),
        }
    }
}

pub struct Synth {
    rate: f32,
    voices: Vec<Option<Voice>>,
    channels: [Channel; CHANNELS],
    sounds: [Sound; CHANNELS],
    bank: [Sound; PROGRAMS],
    delays: Vec<Delay>,
    coefficients: [Coefficients; CHANNELS],
    voice_counts: [usize; CHANNELS],
    lfo_phase: [f32; CHANNELS],
    age: u64,
    master: f32,
}

impl Synth {
    /// Session must validate; sample rate must be in 8,000..=384,000 and voices in 1..=256.
    pub fn new(rate: u32, voices: usize, session: &Session) -> Self {
        Self::with_delay_count(rate, voices, session, CHANNELS)
    }

    /// For the plugin's single shared sound; CC120 clears the shared effect history.
    pub(crate) fn with_shared_delay(rate: u32, voices: usize, session: &Session) -> Self {
        Self::with_delay_count(rate, voices, session, 1)
    }

    fn with_delay_count(rate: u32, voices: usize, session: &Session, delay_count: usize) -> Self {
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
            delays: (0..delay_count).map(|_| Delay::new(rate as f32)).collect(),
            coefficients: std::array::from_fn(|i| {
                Coefficients::new(&session.channels[i], &Channel::default(), rate as f32)
            }),
            voice_counts: [0; CHANNELS],
            lfo_phase: [0.0; CHANNELS],
            age: 0,
            master: session.master,
        }
    }

    pub fn set_sound(&mut self, channel: usize, sound: Sound) {
        if let Some(slot) = self.sounds.get_mut(channel) {
            *slot = sound;
            self.refresh(channel);
        }
    }

    fn refresh(&mut self, channel: usize) {
        self.coefficients[channel] =
            Coefficients::new(&self.sounds[channel], &self.channels[channel], self.rate);
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
        self.voice_counts.iter().sum()
    }

    pub fn is_active(&self) -> bool {
        self.active_voices() != 0 || self.delays.iter().any(|delay| !delay.is_silent())
    }

    pub fn panic(&mut self) {
        for voice in &mut self.voices {
            *voice = None;
        }
        self.voice_counts.fill(0);
        for channel in &mut self.channels {
            channel.sustain = false;
        }
        for delay in &mut self.delays {
            delay.clear();
        }
    }

    pub fn reset(&mut self) {
        self.panic();
        for channel in &mut self.channels {
            *channel = Channel {
                program: channel.program,
                ..Channel::default()
            };
        }
        self.lfo_phase.fill(0.0);
        self.age = 0;
        for channel in 0..CHANNELS {
            self.refresh(channel);
        }
    }

    /// A value of `u8::MAX` matches all channels or keys, as in CLAP choke events.
    pub fn choke(&mut self, channel: u8, key: u8) {
        for voice in &mut self.voices {
            if voice.as_ref().is_some_and(|v| {
                (channel == u8::MAX || v.channel == channel as usize)
                    && (key == u8::MAX || v.key == key)
            }) {
                self.voice_counts[voice.as_ref().unwrap().channel] -= 1;
                *voice = None;
            }
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
                if let Some(previous) = &self.voices[slot] {
                    self.voice_counts[previous.channel] -= 1;
                }
                self.voice_counts[channel] += 1;
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
                self.refresh(channel);
            }
            Bend { value, .. } => {
                self.channels[channel].bend = (value.min(16383) as f32 - 8192.0) / 8192.0;
                self.refresh(channel);
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
                        self.voice_counts[channel] = 0;
                        let bus = if self.delays.len() == 1 { 0 } else { channel };
                        self.delays[bus].clear();
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
                if matches!(controller, 7 | 10 | 11 | 121) {
                    self.refresh(channel);
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
        let mut pitches = [1.0; CHANNELS];
        for (i, pitch) in pitches.iter_mut().enumerate() {
            let channel = self.channels[i];
            if self.voice_counts[i] != 0 {
                let depth =
                    self.sounds[i].lfo_depth + channel.modulation * 0.5 + channel.pressure * 0.25;
                *pitch = if depth == 0.0 {
                    self.coefficients[i].pitch
                } else {
                    2.0_f32.powf((channel.bend * 2.0 + self.lfo_phase[i].sin() * depth) / 12.0)
                };
            }
            let phase = self.lfo_phase[i] + self.coefficients[i].lfo_step;
            self.lfo_phase[i] = if phase >= TAU { phase - TAU } else { phase };
        }
        if !self.is_active() {
            return [0.0; 2];
        }
        for slot in &mut self.voices {
            if let Some(voice) = slot {
                let ch = voice.channel;
                let sample = voice.render(
                    &self.sounds[ch],
                    &self.channels[ch],
                    &self.coefficients[ch],
                    pitches[ch],
                    self.rate,
                );
                mixes[ch][0] += sample[0];
                mixes[ch][1] += sample[1];
                if voice.finished() {
                    self.voice_counts[ch] -= 1;
                    *slot = None;
                }
            }
        }
        let mut out = [0.0; 2];
        for (i, mix) in mixes.iter_mut().enumerate() {
            let sound = &self.sounds[i];
            if sound.drive > 0.0 && *mix != [0.0; 2] {
                for value in mix.iter_mut() {
                    *value = (*value * self.coefficients[i].drive_gain).tanh()
                        * self.coefficients[i].drive_scale;
                }
            }
            let frame = if self.delays.len() == 1 {
                *mix
            } else {
                self.delays[i].next(*mix, sound, self.coefficients[i].delay_offset)
            };
            out[0] += frame[0];
            out[1] += frame[1];
        }
        if self.delays.len() == 1 {
            out = self.delays[0].next(out, &self.sounds[0], self.coefficients[0].delay_offset);
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
        assert_eq!(delay.next([1.0, 0.0], &sound, 10), [1.0, 0.0]);
        for _ in 1..10 {
            assert_eq!(delay.next([0.0; 2], &sound, 10), [0.0; 2]);
        }
        assert_eq!(delay.next([0.0; 2], &sound, 10), [1.0, 0.0]);
        for _ in 11..20 {
            delay.next([0.0; 2], &sound, 10);
        }
        assert_eq!(delay.next([0.0; 2], &sound, 10), [0.0, 0.5]);
    }

    #[test]
    fn soft_saw_harmonics_match_the_original_formula() {
        for step in -25_000..25_000 {
            let phase = step as f32 / 37.0;
            let original =
                (phase.sin() + (2.0 * phase).sin() * 0.5 + (3.0 * phase).sin() / 3.0) / 1.5;
            assert!((waveform(Waveform::SoftSaw, phase) - original).abs() < 2e-5);
        }
    }

    #[test]
    fn delay_invalidation_matches_clearing_after_wraps_and_time_changes() {
        let sound = Sound {
            delay_mix: 0.8,
            delay_feedback: 0.75,
            ..Sound::default()
        };
        let mut delay = Delay::new(1000.0);
        let mut reference = vec![[0.0; 2]; delay.buffer.len()];
        let mut cursor = 0;
        for frame in 0..20_000 {
            if matches!(frame, 31 | 32 | 1540 | 3777 | 9000) {
                delay.clear();
                reference.fill([0.0; 2]);
            }
            let offset = [10, 1500, 17, 300][frame / 173 % 4];
            let input = if frame % 5 == 0 {
                [(frame as f32).sin(), (frame as f32).cos()]
            } else {
                [0.0; 2]
            };
            let wet = reference[(cursor + reference.len() - offset) % reference.len()];
            reference[cursor] = [
                input[0] + wet[1] * sound.delay_feedback,
                input[1] + wet[0] * sound.delay_feedback,
            ];
            cursor = (cursor + 1) % reference.len();
            let expected = [
                input[0] + wet[0] * sound.delay_mix,
                input[1] + wet[1] * sound.delay_mix,
            ];
            let actual = delay.next(input, &sound, offset);
            for side in 0..2 {
                assert!(
                    (actual[side] - expected[side]).abs() < 1e-10,
                    "frame {frame}"
                );
            }
        }
    }

    #[test]
    fn delay_tails_finish_and_stale_history_never_returns() {
        let sound = Sound {
            delay_mix: 1.0,
            delay_feedback: 0.95,
            ..Sound::default()
        };
        let mut delay = Delay::new(1000.0);
        delay.next([1.0, 0.0], &sound, 10);
        assert!(!delay.is_silent());
        for _ in 0..20_000 {
            delay.next([0.0; 2], &sound, 10);
        }
        assert!(delay.is_silent());
        for offset in [1500, 10, 100] {
            assert_eq!(delay.next([0.0; 2], &sound, offset), [0.0; 2]);
        }
        delay.next([1.0, 0.0], &sound, 10);
        delay.clear();
        let cursor = delay.cursor;
        assert!(delay.buffer.iter().any(|frame| *frame != [0.0; 2]));
        for _ in 0..512 {
            delay.clear();
        }
        assert_eq!(delay.cursor, cursor);
        assert_eq!(delay.next([0.0; 2], &sound, 10), [0.0; 2]);
    }

    #[test]
    fn shared_delay_reduces_storage_and_cc120_preserves_other_voices() {
        let mut session = Session::default();
        session.channels.fill(Sound {
            delay_time: 0.01,
            delay_mix: 0.8,
            drive: 0.3,
            ..Sound::default()
        });
        let mut shared = Synth::with_shared_delay(RATE, 8, &session);
        let mut independent = Synth::new(RATE, 8, &session);
        assert_eq!(shared.delays.len(), 1);
        assert_eq!(independent.delays.len(), CHANNELS);
        assert_eq!(shared.delays[0].buffer.len(), 12_001);
        for synth in [&mut shared, &mut independent] {
            synth.event(note(0, 60));
            synth.event(note(15, 67));
        }
        for _ in 0..4000 {
            let left = shared.next_frame();
            let right = independent.next_frame();
            for side in 0..2 {
                assert!((left[side] - right[side]).abs() < 1e-6);
            }
        }
        shared.event(cc(0, 120, 0));
        assert!(shared.delays[0].is_silent());
        assert_eq!(shared.active_voices(), 1);
        assert_eq!(shared.voice_counts[15], 1);
        independent.event(cc(0, 120, 0));
        assert!(independent.delays[0].is_silent());
        assert!(!independent.delays[15].is_silent());
        shared.choke(15, u8::MAX);
        assert!(!shared.is_active());
        for _ in 0..20_000 {
            assert_eq!(shared.next_frame(), [0.0; 2]);
        }
    }

    #[test]
    fn coefficients_follow_live_sound_program_and_controller_changes() {
        let session = Session::default();
        let mut synth = Synth::new(RATE, 2, &session);
        synth.event(note(0, 60));
        let mut sound = session.channels[0];
        sound.operators[0].ratio = 2.5;
        sound.operators[0].detune = -12.0;
        sound.operators[0].pan = 0.3;
        sound.operators[0].level = 0.7;
        sound.lfo_rate = 7.0;
        sound.drive = 0.4;
        sound.delay_time = 1.5;
        synth.set_sound(0, sound);
        for event in [
            cc(0, 10, 23),
            cc(0, 7, 100),
            cc(0, 11, 50),
            MidiEvent::Bend {
                channel: 0,
                value: 12_000,
            },
            cc(0, 121, 0),
            MidiEvent::Program {
                channel: 0,
                program: 4,
            },
        ] {
            synth.event(event);
            let expected = Coefficients::new(&synth.sounds[0], &synth.channels[0], RATE as f32);
            let actual = &synth.coefficients[0];
            assert_eq!(actual.ratios, expected.ratios);
            assert_eq!(actual.gains, expected.gains);
            assert_eq!(actual.volume, expected.volume);
            assert_eq!(actual.pitch, expected.pitch);
            assert_eq!(actual.delay_offset, expected.delay_offset);
            assert_eq!(actual.drive_scale, expected.drive_scale);
            assert_eq!(actual.lfo_step, expected.lfo_step);
        }
        assert_eq!(synth.active_voices(), 1);
        synth.reset();
        assert_eq!(synth.coefficients[0].pitch, 1.0);
        assert_eq!(synth.coefficients[0].volume, 0.18);
    }

    #[test]
    fn cached_rendering_matches_uncached_waveforms_and_modulation() {
        fn uncached(voice: &mut Voice, sound: &Sound, channel: &Channel) -> [f32; 2] {
            let rate = RATE as f32;
            let frequency =
                440.0 * 2.0_f32.powf((voice.key as f32 - 69.0 + channel.bend * 2.0) / 12.0);
            let mut next = [0.0; 4];
            let mut output = [0.0; 2];
            for (dst, value) in next.iter_mut().enumerate() {
                let op = &sound.operators[dst];
                let mut pm = 0.0;
                let mut am = 1.0;
                for src in 0..4 {
                    pm += sound.pm[dst][src] * voice.previous[src] * channel.timbre;
                    am += sound.am[dst][src] * voice.previous[src];
                }
                let hz = frequency * op.ratio * 2.0_f32.powf(op.detune / 1200.0);
                let phase = voice.phase[dst] + pm;
                let basic = waveform(op.wave, phase);
                let doubled = waveform(op.wave, 2.0 * phase + std::f32::consts::PI / 3.0);
                let env = voice.envelopes[dst].next(rate);
                let velocity = 1.0 - op.velocity + op.velocity * voice.velocity;
                *value = ((basic * (1.0 - op.octave_mix) + doubled * op.octave_mix)
                    * env
                    * env
                    * velocity
                    * am.clamp(-2.0, 2.0)
                    * if hz < rate * 0.49 { 1.0 } else { 0.0 })
                .clamp(-4.0, 4.0);
                voice.phase[dst] = (voice.phase[dst] + TAU * hz.min(rate) / rate).rem_euclid(TAU);
                let angle = ((op.pan + channel.pan).clamp(-1.0, 1.0) + 1.0) * FRAC_PI_2 * 0.5;
                output[0] += *value * op.level * angle.cos();
                output[1] += *value * op.level * angle.sin();
            }
            voice.previous = next;
            let volume = channel.volume * channel.expression * 0.18;
            [output[0] * volume, output[1] * volume]
        }

        let channel = Channel {
            pan: -0.2,
            volume: 0.75,
            expression: 0.8,
            timbre: 0.7,
            ..Channel::default()
        };
        for wave in [
            Waveform::Sine,
            Waveform::Mx,
            Waveform::Triangle,
            Waveform::SoftSaw,
        ] {
            for mix in [0.0, 0.35, 1.0] {
                let mut sound = Sound::default();
                for (i, op) in sound.operators.iter_mut().enumerate() {
                    op.wave = wave;
                    op.ratio = (1 << i) as f32;
                    op.detune = -13.5;
                    op.pan = i as f32 * 0.3 - 0.5;
                    op.level = 0.2;
                    op.octave_mix = mix;
                }
                sound.pm = [[0.3; 4]; 4];
                sound.am = [[0.02; 4]; 4];
                let coefficients = Coefficients::new(&sound, &channel, RATE as f32);
                let mut cached = Voice::new(0, 69, 110, 0, &sound, RATE as f32);
                let mut reference = Voice::new(0, 69, 110, 0, &sound, RATE as f32);
                for _ in 0..4000 {
                    let actual = cached.render(
                        &sound,
                        &channel,
                        &coefficients,
                        coefficients.pitch,
                        RATE as f32,
                    );
                    let expected = uncached(&mut reference, &sound, &channel);
                    for side in 0..2 {
                        assert!((actual[side] - expected[side]).abs() < 1e-6);
                    }
                }
            }
        }
    }

    #[test]
    fn voice_counts_follow_stealing_chokes_releases_and_panic() {
        let mut synth = Synth::new(RATE, 2, &Session::default());
        synth.event(note(0, 60));
        synth.event(note(1, 64));
        synth.event(note(15, 67));
        assert_eq!(synth.voice_counts[0], 0);
        assert_eq!(synth.voice_counts[1], 1);
        assert_eq!(synth.voice_counts[15], 1);
        synth.choke(1, u8::MAX);
        synth.event(off(15, 67));
        advance(&mut synth, 0.6);
        assert_eq!(synth.active_voices(), 0);
        assert_eq!(synth.voices.iter().flatten().count(), 0);
        synth.event(note(0, 60));
        synth.panic();
        assert_eq!(synth.voice_counts, [0; CHANNELS]);
        assert!(!synth.is_active());
    }

    #[test]
    fn silent_channels_preserve_lfo_phase_and_wake_on_notes() {
        let mut synth = Synth::new(RATE, 2, &Session::default());
        let phase = synth.lfo_phase[15];
        for _ in 0..100 {
            assert_eq!(synth.next_frame(), [0.0; 2]);
        }
        assert_ne!(synth.lfo_phase[15], phase);
        synth.event(note(15, 60));
        assert!(synth.is_active());
        assert!((0..100).any(|_| synth.next_frame()[0] != 0.0));
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
