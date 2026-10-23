//! Approximate import of little-endian, 480-byte Mx44 C patch records.
use crate::patch::{CHANNELS, Envelope, Operator, PROGRAMS, Patch, Session, Sound, Waveform};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::Path,
};

const RECORD_BYTES: usize = 480;
const BANK_BYTES: usize = RECORD_BYTES * PROGRAMS;
const STATE_BYTES: usize = RECORD_BYTES * (PROGRAMS + CHANNELS) + CHANNELS * 2;
const MAX_BYTES: usize = STATE_BYTES + 13;
type Issues = BTreeSet<&'static str>;

pub struct Import {
    pub session: Session,
    pub warnings: Vec<String>,
}

pub fn load(path: &Path, source_sample_rate: u32) -> Result<Import> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("read legacy bank {}", path.display()))?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    from_bytes(&bytes, source_sample_rate)
        .with_context(|| format!("import legacy bank {}", path.display()))
}

pub fn from_bytes(bytes: &[u8], source_sample_rate: u32) -> Result<Import> {
    ensure!(
        (8_000..=192_000).contains(&source_sample_rate),
        "legacy source sample rate must be between 8000 and 192000 Hz"
    );
    ensure!(
        [BANK_BYTES, STATE_BYTES, STATE_BYTES + 12, MAX_BYTES].contains(&bytes.len()),
        "unsupported legacy bank length {}; expected 61440, 69152, 69164 or 69165 bytes \
         (little-endian Mx44, 480-byte records)",
        bytes.len()
    );
    let mut counts = BTreeMap::new();
    let mut convert = |index| {
        let mut issues = Issues::new();
        let patch = convert_patch(
            &bytes[index * RECORD_BYTES..(index + 1) * RECORD_BYTES],
            index,
            source_sample_rate as f32,
            &mut issues,
        );
        for issue in issues {
            *counts.entry(issue).or_insert(0_usize) += 1;
        }
        patch
    };
    let patches = (0..PROGRAMS).map(&mut convert).collect::<Vec<_>>();
    let mut session = Session {
        version: 1,
        channels: [patches[0].sound; CHANNELS],
        patches,
        programs: [0; CHANNELS],
        master: 0.5,
    };
    if bytes.len() != BANK_BYTES {
        session.channels = std::array::from_fn(|channel| convert(PROGRAMS + channel).sound);
        let start = RECORD_BYTES * (PROGRAMS + CHANNELS);
        for channel in 0..CHANNELS {
            let program = bytes[start + channel];
            ensure!(
                (program as usize) < PROGRAMS,
                "legacy channel {} has invalid program number {program}",
                channel + 1
            );
            session.programs[channel] = program;
        }
    }
    let mut warnings = vec![format!(
        "Approximate conversion, not emulation: tuning, modulation, mixing and velocity response \
         differ. Sample-based values use {source_sample_rate} Hz, MIDI A4 and velocity 100."
    )];
    warnings.extend(
        counts
            .into_iter()
            .map(|(issue, count)| format!("{issue} ({count} preset/channel records).")),
    );
    if bytes.len() == BANK_BYTES {
        warnings.push("Bank-only input: all channels start with preset 1.".into());
    } else {
        let modes = &bytes[STATE_BYTES - CHANNELS..STATE_BYTES];
        if modes.iter().any(|mode| *mode != 0) {
            warnings.push(
                "Legacy mono/legato channel modes are not supported; using polyphony.".into(),
            );
        }
        if bytes.len() > STATE_BYTES {
            warnings.push(
                "Legacy shruti/temperament data is not imported; using equal temperament.".into(),
            );
        }
    }
    session
        .validate()
        .context("validate converted legacy bank")?;
    Ok(Import { session, warnings })
}

struct Record<'a>(&'a [u8]);

impl Record<'_> {
    fn word(&self, offset: usize) -> f32 {
        i16::from_le_bytes([self.0[offset], self.0[offset + 1]]) as f32
    }
}

fn limited(value: f32, min: f32, max: f32, issue: &'static str, issues: &mut Issues) -> f32 {
    if !(min..=max).contains(&value) {
        issues.insert(issue);
    }
    value.clamp(min, max)
}

fn convert_patch(bytes: &[u8], index: usize, rate: f32, issues: &mut Issues) -> Patch {
    let r = Record(bytes);
    let name_bytes = &bytes[320..352];
    let name_bytes = &name_bytes[..name_bytes.iter().position(|b| *b == 0).unwrap_or(32)];
    let name = match std::str::from_utf8(name_bytes) {
        Ok(name) => name.to_owned(),
        Err(_) => {
            issues.insert("Non-UTF-8 names decoded as Latin-1");
            name_bytes.iter().map(|byte| char::from(*byte)).collect()
        }
    };
    if name.chars().any(char::is_control) {
        issues.insert("Control characters removed from patch names");
    }
    let name: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let name = if name.trim().is_empty() {
        format!("Legacy {:03}", index + 1)
    } else {
        name.trim().to_owned()
    };
    if bytes[..320].iter().chain(&bytes[352..]).all(|b| *b == 0) {
        return Patch {
            name,
            sound: Sound {
                operators: [Operator {
                    level: 0.0,
                    envelope: Envelope {
                        levels: [0.0; 8],
                        times: [0.001; 8],
                        loop_sustain: true,
                    },
                    ..Operator::default()
                }; 4],
                ..Sound::default()
            },
        };
    }
    let operators = std::array::from_fn(|op| {
        let ratio = limited(
            r.word(208 + op * 2) / 512.0,
            0.125,
            32.0,
            "Zero, sub-range or excessive oscillator ratios clipped to 0.125..32",
            issues,
        );
        // The original detune is an additive phase increment, not cents.
        let d = (r.word(232 + op * 2) as i64 * 127) >> 7;
        let delta = (((d * d) >> 15) * d + (d << 5)) * 2;
        let hz = 440.0 * r.word(208 + op * 2) / 512.0
            + (delta as f64 * 2.0 * rate as f64 / 4_294_967_296.0) as f32;
        let detune = if hz > 0.0 {
            1200.0 * (hz / (440.0 * ratio)).log2()
        } else {
            issues.insert("Non-positive/fixed oscillator frequencies cannot be represented");
            -100.0
        };
        let detune = limited(
            detune,
            -100.0,
            100.0,
            "Reference-note detuning clipped to +/-100 cents",
            issues,
        );
        let volume = limited(
            r.word(64 + op * 4) / 32768.0,
            0.0,
            1.0,
            "Negative operator mixer levels omitted",
            issues,
        );
        let balance = r.word(66 + op * 4) / 32768.0;
        let left = 1.0 - 2.0 * balance.max(0.0);
        let right = 1.0 + 2.0 * balance.min(0.0);
        if left < 0.0 || right < 0.0 {
            issues.insert("Mixer polarity inversions replaced by absolute channel gains");
        }
        let pan = right.abs().atan2(left.abs()) * 4.0 / std::f32::consts::PI - 1.0;
        let level = (volume * volume + volume) * 0.5 * (left * left + right * right).sqrt()
            / std::f32::consts::SQRT_2;
        let velocity = limited(
            r.word(296 + op * 2) / 32000.0,
            0.0,
            1.0,
            "Inverse/out-of-range velocity response clipped to 0..1",
            issues,
        );
        if bytes[248 + op] > 1 {
            issues.insert("Secondary harmonics above the octave replaced by octave doubling");
        }
        if bytes[252 + op] & (1 | 8 | 16 | 64) != 0 {
            issues.insert("Operator self-shaping, low-pass and internal modulation flags omitted");
        }
        if bytes[252 + op] & (2 | 4 | 128) != 0 {
            issues.insert("Per-operator wheel/expression assignments omitted");
        }
        if bytes[252 + op] & 32 != 0 || r.word(304 + op * 4) != 0.0 || r.word(306 + op * 4) != 0.0 {
            issues.insert("Initial phase and phase-follow controls omitted");
        }
        if r.word(216 + op * 4) != 0.0 {
            issues.insert("Intonation/portamento envelopes omitted");
        }
        if r.word(264 + op * 4) != 0.0 || r.word(266 + op * 4) != 0.0 {
            issues.insert("Keyboard-dependent level scaling omitted");
        }
        let key_attack = (32000.0 - r.word(280 + op * 4) * 69.0 / 128.0) / 32768.0;
        let key_sustain = (32000.0 - r.word(282 + op * 4) * 69.0 / 128.0) / 32768.0;
        let velocity_attack = (32000.0 - r.word(240 + op * 2) * 100.0 / 128.0) / 32768.0;
        let controller = (127.0_f32 / 128.0).powi(2);
        let scales = [
            (key_attack * velocity_attack).powi(2) * controller,
            (key_attack * velocity_attack).powi(2) * controller,
            key_attack.powi(2) * controller,
            key_attack.powi(2) * controller,
            key_sustain.powi(2) * controller,
            key_sustain.powi(2) * controller,
            key_attack.powi(2) * controller,
            key_attack.powi(2),
        ];
        if r.word(240 + op * 2) != 0.0 || r.word(280 + op * 4) != 0.0 || r.word(282 + op * 4) != 0.0
        {
            issues.insert("Dynamic envelope key/velocity timing baked at A4, velocity 100");
        }
        if r.word(80 + op * 16) != 0.0 {
            issues.insert("Nonzero envelope initial levels replaced by zero");
        }
        // Legacy levels[0] is the initial value; sustain returns to levels[4].
        let levels = std::array::from_fn(|stage| {
            if stage == 7 {
                0.0
            } else {
                let source = [1, 2, 3, 4, 5, 4, 7][stage];
                limited(
                    r.word(80 + op * 16 + source * 2) / 8000.0,
                    0.0,
                    1.0,
                    "Envelope levels clipped to 0..1",
                    issues,
                )
            }
        });
        let times = std::array::from_fn(|stage| {
            let time = r.word(144 + op * 16 + stage * 2);
            if time < 0.0 {
                issues.insert("Negative envelope times replaced by the minimum duration");
            }
            let samples = (time * 8.0 * scales[stage]).max(0.0);
            let seconds = ((samples as u32 / 16 + 1) * 16) as f32 / rate;
            limited(
                seconds,
                0.001,
                10.0,
                "Envelope stage times clipped to 1 ms..10 s",
                issues,
            )
        });
        Operator {
            ratio,
            detune,
            level,
            pan,
            velocity,
            octave_mix: if bytes[248 + op] == 0 { 0.0 } else { 0.5 },
            wave: Waveform::Mx,
            envelope: Envelope {
                levels,
                times,
                loop_sustain: true,
            },
        }
    });
    let lfo_rate =
        ((325000.0 - 2048.0 * (r.word(366) + r.word(368))) * rate / (32.0 * 4_294_967_296.0)).abs();
    let amount = ((r.word(370) + r.word(372)).abs() / 262144.0 + 1.0).log2() * 12.0;
    if bytes[364..378].iter().any(|b| *b != 0) {
        issues.insert("Dual-rate/enveloped/operator-phase LFO reduced to an average sine LFO");
    }
    if bytes[352..364].iter().any(|b| *b != 0) {
        issues.insert("Reserved legacy effect fields ignored; new drive/delay disabled");
    }
    if bytes[378..398].iter().any(|b| *b != 0) {
        issues.insert("Reserved GS/controller and modulation-delay fields omitted");
    }
    Patch {
        name,
        sound: Sound {
            operators,
            pm: std::array::from_fn(|dst| {
                std::array::from_fn(|src| {
                    r.word((dst * 4 + src) * 2) * std::f32::consts::PI / 32768.0
                })
            }),
            am: std::array::from_fn(|dst| {
                std::array::from_fn(|src| r.word(32 + (dst * 4 + src) * 2) / 32768.0)
            }),
            lfo_rate: limited(
                lfo_rate,
                0.05,
                20.0,
                "LFO rate clipped to 0.05..20 Hz",
                issues,
            ),
            lfo_depth: limited(
                amount,
                0.0,
                1.0,
                "LFO depth clipped to one semitone",
                issues,
            ),
            ..Sound::default()
        },
    }
}
