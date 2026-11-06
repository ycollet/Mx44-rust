use crate::runtime::{Audio, Command};
use anyhow::{Context, Result};
use eframe::egui::{self, Color32, Key, RichText, Slider};
use mx44_rust::{
    midi::MidiEvent,
    patch::{CHANNELS, Operator, Patch, Session, Sound},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};

#[path = "sound_editor.rs"]
mod sound_editor;

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
    factory: Vec<Patch>,
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
                factory: Session::factory_patches(),
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
            sound_editor::heading(ui);
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
            if let Some(patch) = sound_editor::factory_picker(ui, &self.factory).cloned()
                && self.send(Command::Sound(self.channel, patch.sound))
            {
                self.session.channels[self.channel] = patch.sound;
                self.patch_name = patch.name;
                self.status = format!(
                    "Loaded {} on channel {}. Store in preset to keep it in the bank.",
                    self.patch_name, self.channel + 1
                );
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
            sound_editor::show(
                ui,
                &mut self.session.channels[self.channel],
                &mut self.clipboard,
            );
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
