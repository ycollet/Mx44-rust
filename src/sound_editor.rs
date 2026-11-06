// Compiled by both frontends, which currently use different egui versions.
use super::egui::{self, Color32, RichText, Slider};
use mx44_rust::patch::{Operator, Sound, Waveform};

pub fn heading(ui: &mut egui::Ui) {
    ui.heading(RichText::new("Mx44").color(Color32::from_rgb(110, 210, 190)));
    ui.label("RUST / FOUR-OPERATOR MATRIX SYNTH");
}

pub fn show(ui: &mut egui::Ui, sound: &mut Sound, clipboard: &mut Option<Operator>) {
    egui::ScrollArea::both().show(ui, |ui| {
        ui.set_min_width(900.0);
        ui.columns(4, |columns| {
            for (i, column) in columns.iter_mut().enumerate() {
                operator(column, i, &mut sound.operators[i], clipboard);
            }
        });
        ui.add_space(12.0);
        ui.horizontal_top(|ui| {
            matrix(ui, "Phase modulation (radians)", &mut sound.pm, 8.0);
            matrix(ui, "Amplitude modulation", &mut sound.am, 1.0);
            effects(ui, sound);
        });
        ui.label("All 16 routes include one-sample-delayed feedback. Operator output level does not affect its modulation send.");
        ui.label("Edits update held notes; envelope settings apply to new notes.");
    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    fn draw(
        ctx: &egui::Context,
        sound: &mut Sound,
        clipboard: &mut Option<Operator>,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 940.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, sound, clipboard);
                });
            },
        )
    }

    #[test]
    fn drawing_preserves_host_and_standalone_sound_values() {
        let ctx = egui::Context::default();
        let mut rng = StdRng::seed_from_u64(44);
        let mut clipboard = None;
        for before in
            std::iter::once(Sound::default()).chain((0..4).map(|_| Sound::random(&mut rng)))
        {
            let mut sound = before;
            let output = draw(&ctx, &mut sound, &mut clipboard, vec![]);
            assert!(!output.shapes.is_empty());
            assert_eq!(sound, before);
            sound.validate().unwrap();
        }
    }

    #[test]
    fn copy_paste_transfers_one_complete_operator() {
        let ctx = egui::Context::default();
        let mut sound = Sound::random(&mut StdRng::seed_from_u64(44));
        let before = sound;
        let mut clipboard = None;
        let mut output = draw(&ctx, &mut sound, &mut clipboard, vec![]);
        for (label, index) in [("Copy", 0), ("Paste", 1)] {
            let position = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .nth(index)
                .expect("operator button is visible");
            for pressed in [true, false] {
                output = draw(
                    &ctx,
                    &mut sound,
                    &mut clipboard,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        }
        let mut expected = before;
        expected.operators[1] = before.operators[0];
        assert_eq!(clipboard, Some(before.operators[0]));
        assert_eq!(sound, expected);
        let values = mx44_rust::plugin::encode(&sound, 0.37);
        assert_eq!(mx44_rust::plugin::decode(&values), (expected, 0.37));
    }
}
