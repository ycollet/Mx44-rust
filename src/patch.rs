use anyhow::{Context, Result, ensure};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

pub const OPERATORS: usize = 4;
pub const PROGRAMS: usize = 128;
pub const CHANNELS: usize = 16;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub levels: [f32; 8],
    pub times: [f32; 8],
    pub loop_sustain: bool,
}

impl Default for Envelope {
    fn default() -> Self {
        Self {
            levels: [1.0, 0.95, 0.75, 0.7, 0.65, 0.7, 0.25, 0.0],
            times: [0.008, 0.04, 0.15, 0.15, 0.4, 0.4, 0.12, 0.3],
            loop_sustain: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    #[default]
    Mx,
    Triangle,
    SoftSaw,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Operator {
    pub ratio: f32,
    pub detune: f32,
    pub level: f32,
    pub pan: f32,
    pub velocity: f32,
    pub octave_mix: f32,
    pub wave: Waveform,
    pub envelope: Envelope,
}

impl Default for Operator {
    fn default() -> Self {
        Self {
            ratio: 1.0,
            detune: 0.0,
            level: 0.0,
            pan: 0.0,
            velocity: 0.7,
            octave_mix: 0.0,
            wave: Waveform::Mx,
            envelope: Envelope::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sound {
    pub operators: [Operator; OPERATORS],
    // Matrices are indexed [destination][source], including self-feedback.
    pub pm: [[f32; OPERATORS]; OPERATORS],
    pub am: [[f32; OPERATORS]; OPERATORS],
    pub lfo_rate: f32,
    pub lfo_depth: f32,
    pub drive: f32,
    pub delay_time: f32,
    pub delay_feedback: f32,
    pub delay_mix: f32,
}

impl Default for Sound {
    fn default() -> Self {
        let mut operators = [Operator::default(); OPERATORS];
        operators[0].level = 0.8;
        Self {
            operators,
            pm: [[0.0; OPERATORS]; OPERATORS],
            am: [[0.0; OPERATORS]; OPERATORS],
            lfo_rate: 5.0,
            lfo_depth: 0.0,
            drive: 0.0,
            delay_time: 0.3,
            delay_feedback: 0.3,
            delay_mix: 0.0,
        }
    }
}

fn range(value: f32, min: f32, max: f32, field: &str) -> Result<()> {
    ensure!(
        value.is_finite() && (min..=max).contains(&value),
        "{field} must be finite and between {min} and {max}, got {value}"
    );
    Ok(())
}

impl Sound {
    pub fn random(rng: &mut impl Rng) -> Self {
        let ratios = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];
        let waves = [
            Waveform::Sine,
            Waveform::Mx,
            Waveform::Triangle,
            Waveform::SoftSaw,
        ];
        let operators = std::array::from_fn(|i| {
            let sustain = rng.random_range(0.2..=0.7);
            Operator {
                ratio: if i == 0 {
                    1.0
                } else {
                    ratios[rng.random_range(0..ratios.len())]
                },
                detune: rng.random_range(-12.0..=12.0),
                level: if i == 0 {
                    1.0
                } else {
                    rng.random_range(0.0..=0.6)
                },
                pan: rng.random_range(-0.7..=0.7),
                velocity: rng.random_range(0.3..=0.9),
                octave_mix: rng.random_range(0.0..=0.5),
                wave: waves[rng.random_range(0..waves.len())],
                envelope: Envelope {
                    levels: [
                        rng.random_range(0.8..=1.0),
                        rng.random_range(0.7..=0.9),
                        rng.random_range(sustain..=0.8),
                        sustain,
                        rng.random_range(0.2..=0.7),
                        sustain,
                        rng.random_range(0.0..=0.2),
                        0.0,
                    ],
                    times: [
                        rng.random_range(0.004..=0.12),
                        rng.random_range(0.01..=0.15),
                        rng.random_range(0.05..=0.4),
                        rng.random_range(0.05..=0.4),
                        rng.random_range(0.1..=0.8),
                        rng.random_range(0.1..=0.8),
                        rng.random_range(0.05..=0.5),
                        rng.random_range(0.1..=0.6),
                    ],
                    loop_sustain: rng.random_bool(0.35),
                },
            }
        });
        let mut sound = Self {
            operators,
            pm: [[0.0; OPERATORS]; OPERATORS],
            am: [[0.0; OPERATORS]; OPERATORS],
            lfo_rate: rng.random_range(0.2..=8.0),
            lfo_depth: rng.random_range(0.0..=0.2),
            drive: rng.random_range(0.0..=0.25),
            delay_time: rng.random_range(0.08..=0.6),
            delay_feedback: rng.random_range(0.1..=0.45),
            delay_mix: rng.random_range(0.0..=0.25),
        };
        // Keep total carrier gain consistent, with an audible fundamental.
        let total = sound.operators.iter().map(|op| op.level).sum::<f32>();
        for op in &mut sound.operators {
            op.level *= 0.8 / total;
        }
        for dst in 0..OPERATORS {
            for src in 0..OPERATORS {
                if rng.random_bool(0.35) {
                    let amount = if src == dst { 0.3 } else { 2.0 };
                    sound.pm[dst][src] = rng.random_range(-amount..=amount);
                }
                if rng.random_bool(0.2) {
                    sound.am[dst][src] = rng.random_range(-0.25..=0.25);
                }
            }
        }
        sound
    }

    pub fn validate(&self) -> Result<()> {
        for op in &self.operators {
            range(op.ratio, 0.125, 32.0, "ratio")?;
            range(op.detune, -100.0, 100.0, "detune")?;
            range(op.level, 0.0, 1.0, "level")?;
            range(op.pan, -1.0, 1.0, "pan")?;
            range(op.velocity, 0.0, 1.0, "velocity")?;
            range(op.octave_mix, 0.0, 1.0, "octave mix")?;
            for value in op.envelope.levels {
                range(value, 0.0, 1.0, "envelope level")?;
            }
            ensure!(
                op.envelope.levels[7] == 0.0,
                "last envelope level must be zero"
            );
            for value in op.envelope.times {
                range(value, 0.001, 10.0, "envelope time")?;
            }
        }
        for row in self.pm {
            for value in row {
                range(value, -8.0, 8.0, "phase modulation")?;
            }
        }
        for row in self.am {
            for value in row {
                range(value, -1.0, 1.0, "amplitude modulation")?;
            }
        }
        range(self.lfo_rate, 0.05, 20.0, "LFO rate")?;
        range(self.lfo_depth, 0.0, 1.0, "LFO depth")?;
        range(self.drive, 0.0, 1.0, "drive")?;
        range(self.delay_time, 0.01, 1.5, "delay time")?;
        range(self.delay_feedback, 0.0, 0.95, "delay feedback")?;
        range(self.delay_mix, 0.0, 1.0, "delay mix")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub name: String,
    pub sound: Sound,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub version: u32,
    pub patches: Vec<Patch>,
    pub programs: [u8; CHANNELS],
    pub channels: [Sound; CHANNELS],
    pub master: f32,
}

impl Default for Session {
    fn default() -> Self {
        let mut patches = Vec::with_capacity(PROGRAMS);
        let init = Sound::default();
        patches.push(Patch {
            name: "Init - Mx tone".into(),
            sound: init,
        });
        let mut ep = init;
        ep.operators[0].wave = Waveform::Sine;
        ep.operators[1].ratio = 3.0;
        ep.operators[1].wave = Waveform::Sine;
        ep.operators[1].envelope.levels = [1.0, 0.6, 0.2, 0.03, 0.03, 0.03, 0.01, 0.0];
        ep.pm[0][1] = 3.0;
        ep.delay_mix = 0.18;
        patches.push(Patch {
            name: "Glass electric piano".into(),
            sound: ep,
        });
        let mut bass = ep;
        bass.operators[1].ratio = 1.0;
        bass.pm[0][1] = 2.4;
        bass.delay_mix = 0.0;
        bass.drive = 0.25;
        patches.push(Patch {
            name: "Rubber FM bass".into(),
            sound: bass,
        });
        let mut bell = ep;
        bell.operators[1].ratio = 3.51;
        bell.pm[0][1] = 5.0;
        bell.operators[0].envelope.levels = [1.0, 0.8, 0.5, 0.15, 0.15, 0.15, 0.05, 0.0];
        bell.operators[0].envelope.times[6] = 0.6;
        bell.operators[0].envelope.times[7] = 1.2;
        bell.delay_mix = 0.3;
        patches.push(Patch {
            name: "Inharmonic bells".into(),
            sound: bell,
        });
        let mut pad = init;
        for (i, op) in pad.operators.iter_mut().enumerate() {
            op.level = 0.25;
            op.detune = (i as f32 - 1.5) * 5.0;
            op.pan = (i as f32 - 1.5) / 2.0;
            op.envelope.times[0] = 0.8;
            op.envelope.times[6] = 0.5;
            op.envelope.times[7] = 0.9;
            op.envelope.loop_sustain = true;
        }
        pad.lfo_depth = 0.12;
        pad.delay_mix = 0.3;
        patches.push(Patch {
            name: "Slow orbit pad".into(),
            sound: pad,
        });
        let mut organ = init;
        for (i, op) in organ.operators.iter_mut().enumerate() {
            op.ratio = [1.0, 2.0, 3.0, 4.0][i];
            op.level = [0.45, 0.25, 0.15, 0.1][i];
            op.wave = Waveform::Sine;
            op.envelope.levels = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.3, 0.0];
        }
        patches.push(Patch {
            name: "Four drawbars".into(),
            sound: organ,
        });
        while patches.len() < PROGRAMS {
            patches.push(Patch {
                name: format!("Init {:03}", patches.len() + 1),
                sound: init,
            });
        }
        Self {
            version: 1,
            patches,
            programs: [0; CHANNELS],
            channels: [init; CHANNELS],
            master: 0.5,
        }
    }
}

impl Session {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported session version {}",
            self.version
        );
        ensure!(
            self.patches.len() == PROGRAMS,
            "a bank must contain 128 patches"
        );
        for patch in &self.patches {
            ensure!(patch.name.len() <= 128, "patch name exceeds 128 bytes");
            patch
                .sound
                .validate()
                .with_context(|| format!("invalid patch '{}'", patch.name))?;
        }
        for program in self.programs {
            ensure!((program as usize) < PROGRAMS, "program out of range");
        }
        for sound in &self.channels {
            sound.validate()?;
        }
        range(self.master, 0.0, 1.0, "master volume")
    }

    pub fn load(path: &Path) -> Result<Self> {
        use std::io::Read;
        let file = fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 4 * 1024 * 1024, "session file exceeds 4 MiB");
        let session: Self = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "parse {} (Mx44 C binary patches are not supported)",
                path.display()
            )
        })?;
        session.validate()?;
        Ok(session)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        let file_name = path.file_name().context("save path must name a file")?;
        let mut temporary = file_name.to_os_string();
        temporary.push(format!(".{}.tmp", std::process::id()));
        let temporary = parent.join(temporary);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("create temporary save {}", temporary.display()))?;
        let result = (|| -> Result<()> {
            use std::io::Write;
            file.write_all(&serde_json::to_vec_pretty(self)?)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if let Err(error) = result {
            fs::remove_file(&temporary).context("clean up failed save")?;
            return Err(error).with_context(|| format!("save {}", path.display()));
        }
        Ok(())
    }
}
