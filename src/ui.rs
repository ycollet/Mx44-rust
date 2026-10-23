use crate::runtime::{Audio, Command};
use anyhow::{Context, Result};
use eframe::egui::{self, Color32, Key, RichText, Slider};
use mx44_rust::{
    midi::MidiEvent,
    patch::{CHANNELS, Operator, Session, Sound, Waveform},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};

const KEYS: [Key; 17] = [
    Key::A,
    Key::W,
    Key::S,
    Key::E,
    Key::D,
    Key::F,
    Key::T,
    Key::G,
    Key::Y,
    Key::H,
    Key::U,
    Key::J,
    Key::K,
    Key::O,
    Key::L,
    Key::P,
    Key::Semicolon,
];
const KEY_LABELS: [&str; 17] = [
    "A", "W", "S", "E", "D", "F", "T", "G", "Y", "H", "U", "J", "K", "O", "L", "P", ";",
];

struct Editor {
    session: Session,
    audio: Audio,
    channel: usize,
    octave: u8,
    status: String,
    save_path: PathBuf,
    patch_name: String,
    seen_revisions: [u64; CHANNELS],
    keyboard_notes: [Option<(u8, u8)>; 17],
    mouse_note: Option<(u8, u8)>,
    clipboard: Option<Operator>,
    result: Arc<Mutex<Option<Session>>>,
}

pub fn run(session: Session, audio: Audio, save_path: PathBuf) -> Result<Session> {
    let result = Arc::new(Mutex::new(None));
    let output = result.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mx44 Rust - Matrix Synthesizer")
            .with_app_id(crate::icon::APP_ID)
            .with_icon(crate::icon::window_icon()?)
            .with_inner_size([1440.0, 940.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Mx44 Rust",
        options,
        Box::new(move |cc| {
            #[cfg(target_os = "macos")]
            crate::icon::set_dock_icon()?;
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            let patch_name = session.patches[session.programs[0] as usize].name.clone();
            Ok(Box::new(Editor {
                session,
                audio,
                channel: 0,
                octave: 4,
                status: "Ready. Play A W S E D F T G Y H U J K, or connect a MIDI keyboard.".into(),
                save_path,
                patch_name,
                seen_revisions: [0; CHANNELS],
                keyboard_notes: [None; 17],
                mouse_note: None,
                clipboard: None,
                result: output,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("open editor: {e}"))?;
    let session = result
        .lock()
        .map_err(|_| anyhow::anyhow!("editor result lock poisoned"))?
        .take()
        .context("editor closed without returning session")?;
    Ok(session)
}

impl Editor {
    fn send(&mut self, command: Command) -> bool {
        match self.audio.control.send(command) {
            Ok(()) => true,
            Err(error) => {
                self.status = format!("Error: {error:#}");
                false
            }
        }
    }

    fn sync_programs(&mut self) {
        for ch in 0..CHANNELS {
            let revision = self.audio.control.stats.revisions[ch].load(Ordering::Acquire);
            if revision != self.seen_revisions[ch] {
                let program = self.audio.control.stats.programs[ch].load(Ordering::Relaxed);
                self.session.programs[ch] = program;
                self.session.channels[ch] = self.session.patches[program as usize].sound;
                self.seen_revisions[ch] = revision;
                if ch == self.channel {
                    self.patch_name = self.session.patches[program as usize].name.clone();
                    self.status = format!(
                        "MIDI program change: channel {}, preset {}",
                        ch + 1,
                        program + 1
                    );
                }
            }
        }
    }

    fn release_keyboard(&mut self) {
        for i in 0..KEYS.len() {
            if let Some((channel, key)) = self.keyboard_notes[i].take() {
                self.send(Command::Midi(MidiEvent::NoteOff { channel, key }));
            }
        }
        if let Some((channel, key)) = self.mouse_note.take() {
            self.send(Command::Midi(MidiEvent::NoteOff { channel, key }));
        }
    }

    fn keyboard_input(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.focused) {
            self.release_keyboard();
            return;
        }
        for (i, key) in KEYS.iter().enumerate() {
            if ctx.input(|input| input.key_released(*key))
                && let Some((channel, note)) = self.keyboard_notes[i].take()
            {
                self.send(Command::Midi(MidiEvent::NoteOff { channel, key: note }));
            }
            if !ctx.wants_keyboard_input()
                && ctx.input(|input| input.key_pressed(*key))
                && self.keyboard_notes[i].is_none()
            {
                let note = (self.octave + 1) * 12 + i as u8;
                let channel = self.channel as u8;
                if self.send(Command::Midi(MidiEvent::NoteOn {
                    channel,
                    key: note,
                    velocity: 110,
                })) {
                    self.keyboard_notes[i] = Some((channel, note));
                }
            }
        }
        if !ctx.input(|i| i.pointer.primary_down())
            && let Some((channel, key)) = self.mouse_note.take()
        {
            self.send(Command::Midi(MidiEvent::NoteOff { channel, key }));
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.release_keyboard();
            self.send(Command::Panic);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Mx44").color(Color32::from_rgb(110, 210, 190)));
            ui.label("RUST / FOUR-OPERATOR MATRIX SYNTH");
            ui.separator();
            let mut master = self.session.master;
            if ui
                .add(Slider::new(&mut master, 0.0..=1.0).text("Master"))
                .changed()
                && self.send(Command::Master(master))
            {
                self.session.master = master;
            }
            if ui.button("Random")
                .on_hover_text("Generate a new sound on this channel without overwriting its preset")
                .clicked()
            {
                let sound = Sound::random(&mut rand::rng());
                if self.send(Command::Sound(self.channel, sound)) {
                    self.session.channels[self.channel] = sound;
                    self.patch_name = "Random sound".into();
                    self.status = format!(
                        "Randomized channel {}. Play a new note to hear the envelopes; Store in preset to keep it.",
                        self.channel + 1
                    );
                }
            }
            if ui.button("PANIC [Esc]").clicked() {
                self.release_keyboard();
                self.send(Command::Panic);
            }
            if ui.button("Save session").clicked() {
                match self.session.save(&self.save_path) {
                    Ok(()) => self.status = format!("Saved {}", self.save_path.display()),
                    Err(error) => self.status = format!("Save failed: {error:#}"),
                }
            }
        });
        ui.horizontal(|ui| {
            let old_channel = self.channel;
            egui::ComboBox::from_id_salt("channel")
                .selected_text(format!("MIDI channel {}", self.channel + 1))
                .show_ui(ui, |ui| {
                    for i in 0..CHANNELS {
                        ui.selectable_value(&mut self.channel, i, format!("Channel {}", i + 1));
                    }
                });
            if self.channel != old_channel {
                self.patch_name = self.session.patches[self.session.programs[self.channel] as usize].name.clone();
            }
            let old = self.session.programs[self.channel];
            let mut selected = old;
            egui::ComboBox::from_id_salt("program").width(240.0)
                .selected_text(format!("{:03}  {}", old as u16 + 1, self.session.patches[old as usize].name))
                .show_ui(ui, |ui| {
                    for (i, patch) in self.session.patches.iter().enumerate() {
                        ui.selectable_value(&mut selected, i as u8, format!("{:03}  {}", i + 1, patch.name));
                    }
                });
            let reload = ui.button("Recall").on_hover_text("Discard channel edits and recall the selected preset").clicked();
            if (selected != old || reload) && self.send(Command::Select(self.channel as u8, selected)) {
                self.session.programs[self.channel] = selected;
                self.session.channels[self.channel] = self.session.patches[selected as usize].sound;
                self.patch_name = self.session.patches[selected as usize].name.clone();
            }
            ui.add(egui::TextEdit::singleline(&mut self.patch_name).desired_width(180.0).char_limit(32));
            if ui.button("Store in preset").on_hover_text("Overwrite this bank slot with the channel's edits; Save session writes it to disk").clicked() {
                let program = self.session.programs[self.channel] as usize;
                let sound = self.session.channels[self.channel];
                if self.send(Command::Preset(program, sound)) {
                    self.session.patches[program].sound = sound;
                    self.session.patches[program].name = self.patch_name.clone();
                    self.status = format!("Stored preset {}. Save session to write the bank to disk.", program + 1);
                }
            }
        });
    }

    fn piano(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Keyboard octave");
            ui.add(Slider::new(&mut self.octave, 0..=7));
            ui.label("A W S E D F T G Y H U J K O L P ;");
        });
        ui.horizontal(|ui| {
            for (i, label) in KEY_LABELS.iter().enumerate() {
                let key = (self.octave + 1) * 12 + i as u8;
                let black = matches!(key % 12, 1 | 3 | 6 | 8 | 10);
                let playing = self
                    .keyboard_notes
                    .iter()
                    .flatten()
                    .any(|n| *n == (self.channel as u8, key))
                    || self.mouse_note == Some((self.channel as u8, key));
                let color = if playing {
                    Color32::from_rgb(45, 150, 125)
                } else if black {
                    Color32::from_gray(35)
                } else {
                    Color32::from_gray(190)
                };
                let text = RichText::new(format!("{}\n{label}", note_name(key))).color(
                    if black || playing {
                        Color32::WHITE
                    } else {
                        Color32::BLACK
                    },
                );
                let response = ui.add_sized(
                    [44.0, if black { 52.0 } else { 66.0 }],
                    egui::Button::new(text)
                        .fill(color)
                        .sense(egui::Sense::click_and_drag()),
                );
                if response.is_pointer_button_down_on()
                    && self.mouse_note != Some((self.channel as u8, key))
                {
                    if let Some((channel, old)) = self.mouse_note.take() {
                        self.send(Command::Midi(MidiEvent::NoteOff { channel, key: old }));
                    }
                    let channel = self.channel as u8;
                    if self.send(Command::Midi(MidiEvent::NoteOn {
                        channel,
                        key,
                        velocity: 110,
                    })) {
                        self.mouse_note = Some((channel, key));
                    }
                }
            }
        });
    }
}

fn note_name(note: u8) -> String {
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", names[(note % 12) as usize], note as i16 / 12 - 1)
}

fn operator(ui: &mut egui::Ui, index: usize, op: &mut Operator, clipboard: &mut Option<Operator>) {
    ui.push_id(index, |ui| {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.heading(format!("Operator {}", index + 1));
                if ui.small_button("Copy").clicked() {
                    *clipboard = Some(*op);
                }
                if ui
                    .add_enabled(clipboard.is_some(), egui::Button::new("Paste").small())
                    .clicked()
                    && let Some(value) = clipboard
                {
                    *op = *value;
                }
            });
            egui::ComboBox::from_id_salt("wave")
                .selected_text(format!("{:?}", op.wave))
                .show_ui(ui, |ui| {
                    for wave in [
                        Waveform::Sine,
                        Waveform::Mx,
                        Waveform::Triangle,
                        Waveform::SoftSaw,
                    ] {
                        ui.selectable_value(&mut op.wave, wave, format!("{wave:?}"));
                    }
                });
            ui.add(
                Slider::new(&mut op.ratio, 0.125..=32.0)
                    .logarithmic(true)
                    .text("Ratio"),
            );
            ui.add(Slider::new(&mut op.detune, -100.0..=100.0).text("Cents"));
            ui.add(Slider::new(&mut op.level, 0.0..=1.0).text("Output"));
            ui.add(Slider::new(&mut op.pan, -1.0..=1.0).text("Pan"));
            ui.add(Slider::new(&mut op.velocity, 0.0..=1.0).text("Velocity"));
            ui.add(Slider::new(&mut op.octave_mix, 0.0..=1.0).text("Octave mix"));
            ui.separator();
            ui.label("Envelope: time (seconds) / level");
            ui.checkbox(&mut op.envelope.loop_sustain, "Loop sustain stages 5-6");
            egui::Grid::new("envelope").striped(true).show(ui, |ui| {
                for stage in 0..8 {
                    let name = match stage {
                        0 | 1 => "Attack",
                        2 | 3 => "Decay",
                        4 | 5 => "Sustain",
                        _ => "Release",
                    };
                    ui.label(format!("{} {name}", stage + 1));
                    ui.add(
                        egui::DragValue::new(&mut op.envelope.times[stage])
                            .speed(0.005)
                            .range(0.001..=10.0)
                            .suffix(" s"),
                    );
                    if stage == 7 {
                        ui.label("0.00");
                    } else {
                        ui.add(
                            egui::DragValue::new(&mut op.envelope.levels[stage])
                                .speed(0.005)
                                .range(0.0..=1.0)
                                .fixed_decimals(2),
                        );
                    }
                    ui.end_row();
                }
            });
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width().min(260.0), 55.0),
                egui::Sense::hover(),
            );
            let mut points = vec![rect.left_bottom()];
            let total = op.envelope.times.iter().sum::<f32>();
            let mut time = 0.0;
            for stage in 0..8 {
                time += op.envelope.times[stage];
                points.push(egui::pos2(
                    rect.left() + rect.width() * time / total,
                    rect.bottom() - rect.height() * op.envelope.levels[stage],
                ));
            }
            ui.painter().add(egui::Shape::line(
                points,
                egui::Stroke::new(1.5_f32, Color32::from_rgb(110, 210, 190)),
            ));
        });
    });
}

fn matrix(ui: &mut egui::Ui, name: &str, values: &mut [[f32; 4]; 4], max: f32) {
    ui.group(|ui| {
        ui.strong(name);
        ui.label("Rows = destination, columns = source");
        egui::Grid::new(name).striped(true).show(ui, |ui| {
            ui.label("To / From");
            for src in 0..4 {
                ui.label(format!("Op {}", src + 1));
            }
            ui.end_row();
            for (dst, row) in values.iter_mut().enumerate() {
                ui.label(format!("Op {}", dst + 1));
                for value in row {
                    ui.add(
                        egui::DragValue::new(value)
                            .speed(0.01)
                            .range(-max..=max)
                            .fixed_decimals(2),
                    );
                }
                ui.end_row();
            }
        });
    });
}

fn effects(ui: &mut egui::Ui, sound: &mut Sound) {
    ui.group(|ui| {
        ui.strong("LFO / stereo effects");
        ui.add(
            Slider::new(&mut sound.lfo_rate, 0.05..=20.0)
                .logarithmic(true)
                .text("LFO Hz"),
        );
        ui.add(Slider::new(&mut sound.lfo_depth, 0.0..=1.0).text("Vibrato (semitones)"));
        ui.add(Slider::new(&mut sound.drive, 0.0..=1.0).text("Drive"));
        ui.add(Slider::new(&mut sound.delay_time, 0.01..=1.5).text("Delay seconds"));
        ui.add(Slider::new(&mut sound.delay_feedback, 0.0..=0.95).text("Feedback"));
        ui.add(Slider::new(&mut sound.delay_mix, 0.0..=1.0).text("Delay mix"));
    });
}

impl eframe::App for Editor {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_programs();
        self.keyboard_input(ctx);
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| self.toolbar(ui));
        egui::TopBottomPanel::bottom("keyboard").show(ctx, |ui| {
            self.piano(ui);
            ui.separator();
            let stats = &self.audio.control.stats;
            let peak = f32::from_bits(stats.peak.load(Ordering::Relaxed));
            ui.horizontal(|ui| {
                ui.label(format!("{} voices", stats.voices.load(Ordering::Relaxed)));
                ui.add(
                    egui::ProgressBar::new(peak)
                        .desired_width(120.0)
                        .text("Output"),
                );
                ui.label(&self.audio.description);
                let overflow = stats.overflow.load(Ordering::Relaxed);
                if overflow > 0 {
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        format!("{overflow} dropped commands (emergency stop)"),
                    );
                }
                if stats.audio_failed.load(Ordering::Acquire) {
                    ui.colored_label(Color32::LIGHT_RED, "AUDIO STREAM FAILED");
                }
            });
            ui.label(&self.status);
        });
        let before = self.session.channels[self.channel];
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                ui.set_min_width(900.0);
                let sound = &mut self.session.channels[self.channel];
                ui.columns(4, |columns| {
                    for (i, column) in columns.iter_mut().enumerate() {
                        operator(column, i, &mut sound.operators[i], &mut self.clipboard);
                    }
                });
                ui.add_space(12.0);
                ui.horizontal_top(|ui| {
                    matrix(ui, "Phase modulation (radians)", &mut sound.pm, 8.0);
                    matrix(ui, "Amplitude modulation", &mut sound.am, 1.0);
                    effects(ui, sound);
                });
                ui.label("All 16 routes include one-sample-delayed feedback. Operator output level does not affect its modulation send.");
                ui.label("Edits update held notes; envelope settings apply to new notes. Store in preset before switching programs.");
            });
        });
        let sound = self.session.channels[self.channel];
        if sound != before && !self.send(Command::Sound(self.channel, sound)) {
            self.session.channels[self.channel] = before;
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        self.sync_programs();
        match self.result.lock() {
            Ok(mut result) => *result = Some(self.session.clone()),
            Err(error) => eprintln!("Could not return editor session: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_controls_draw_without_a_window() {
        let ctx = egui::Context::default();
        let mut sound = Sound::default();
        let mut clipboard = None;
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for i in 0..4 {
                    operator(ui, i, &mut sound.operators[i], &mut clipboard);
                }
                matrix(ui, "PM", &mut sound.pm, 8.0);
                matrix(ui, "AM", &mut sound.am, 1.0);
                effects(ui, &mut sound);
            });
        });
        assert!(!output.shapes.is_empty());
        sound.validate().unwrap();
    }
}
