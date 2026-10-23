use mx44_rust::{
    midi::MidiEvent,
    patch::{Session, Sound},
    plugin::{self, Kind, PARAM_COUNT, ParameterSpec, PluginEngine, Values},
};
use nih_plug::prelude::*;
use nih_plug_egui::{EguiState, create_egui_editor, egui, widgets};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Params)]
struct Control {
    #[id = "value"]
    value: FloatParam,
}

#[derive(Params)]
struct SynthParams {
    #[persist = "editor-state"]
    editor: Arc<EguiState>,
    // Host IDs are value_1..value_131. Never reorder this array.
    #[nested(array)]
    controls: Vec<Control>,
}

impl Default for SynthParams {
    fn default() -> Self {
        let controls = plugin::parameter_specs()
            .iter()
            .enumerate()
            .map(|(i, spec)| {
                let range = if spec.kind == Kind::Logarithmic {
                    FloatRange::Skewed {
                        min: spec.min,
                        max: spec.max,
                        factor: FloatRange::skew_factor(-2.0),
                    }
                } else {
                    FloatRange::Linear {
                        min: spec.min,
                        max: spec.max,
                    }
                };
                let mut value = FloatParam::new(
                    format!("{} - {}", spec.group, spec.name),
                    spec.default,
                    range,
                );
                match spec.kind {
                    Kind::Toggle => {
                        value = value
                            .with_step_size(1.0)
                            .with_value_to_string(Arc::new(|v| {
                                if v >= 0.5 { "On" } else { "Off" }.into()
                            }));
                    }
                    Kind::Waveform => {
                        value = value
                            .with_step_size(1.0)
                            .with_value_to_string(Arc::new(|v| {
                                ["Sine", "Mx", "Triangle", "Soft saw"][(v.round() as usize).min(3)]
                                    .into()
                            }));
                    }
                    _ => {}
                }
                if i == 0 {
                    value = value.with_smoother(SmoothingStyle::Linear(10.0));
                }
                Control { value }
            })
            .collect();
        Self {
            editor: EguiState::from_size(1280, 850),
            controls,
        }
    }
}

impl SynthParams {
    fn values(&self) -> Values {
        std::array::from_fn(|i| self.controls[i].value.value())
    }

    fn set_sound(&self, setter: &ParamSetter, sound: &Sound) {
        let values = plugin::encode(sound, self.controls[0].value.value());
        for (control, value) in self.controls.iter().zip(values).skip(1) {
            setter.begin_set_parameter(&control.value);
            setter.set_parameter(&control.value, value);
            setter.end_set_parameter(&control.value);
        }
    }
}

pub struct Mx44Plugin {
    params: Arc<SynthParams>,
    engine: Option<PluginEngine>,
    panic_requested: Arc<AtomicBool>,
}

impl Default for Mx44Plugin {
    fn default() -> Self {
        Self {
            params: Arc::new(SynthParams::default()),
            engine: None,
            panic_requested: Arc::new(AtomicBool::new(false)),
        }
    }
}

fn parameter_ui(
    ui: &mut egui::Ui,
    params: &SynthParams,
    specs: &[ParameterSpec],
    setter: &ParamSetter,
    index: usize,
) {
    ui.push_id(index, |ui| {
        ui.label(&specs[index].name);
        ui.add(widgets::ParamSlider::for_param(
            &params.controls[index].value,
            setter,
        ));
    });
}

impl Plugin for Mx44Plugin {
    const NAME: &'static str = "Mx44 Rust";
    const VENDOR: &'static str = "Mx44 Rust";
    const URL: &'static str = "https://github.com/linuxmao-org/Mx44";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    // The pinned NIH CLAP wrapper applies a buffer's first automation event early.
    // Keep automation block-based consistently across both formats.
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let panic_requested = self.panic_requested.clone();
        let specs = plugin::parameter_specs();
        let factory = Session::default();
        create_egui_editor(
            params.editor.clone(),
            (),
            |_, _| {},
            move |ctx, setter, _| {
                egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Mx44 Rust");
                    ui.label("Master");
                    ui.add(widgets::ParamSlider::for_param(&params.controls[0].value, setter));
                    if ui.button("Random").on_hover_text("Randomize the sound, keeping master volume").clicked() {
                        params.set_sound(setter, &Sound::random(&mut rand::rng()));
                    }
                    egui::ComboBox::from_id_salt("factory")
                        .selected_text("Load factory sound")
                        .show_ui(ui, |ui| {
                            for patch in &factory.patches[..6] {
                                if ui.button(&patch.name).clicked() {
                                    params.set_sound(setter, &patch.sound);
                                }
                            }
                        });
                    if ui.button("PANIC").clicked() {
                        panic_requested.store(true, Ordering::Release);
                    }
                });
                ui.label("32 voices / stereo / host MIDI on all channels / settings saved with your project");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.columns(4, |columns| {
                        for (op, column) in columns.iter_mut().enumerate() {
                            column.heading(format!("Operator {}", op + 1));
                            for index in (1 + op * 23)..(1 + (op + 1) * 23) {
                                parameter_ui(column, &params, &specs, setter, index);
                            }
                            column.label("Final release level: 0");
                        }
                    });
                    ui.separator();
                    ui.columns(3, |columns| {
                        for (matrix, column) in columns[..2].iter_mut().enumerate() {
                            column.heading(if matrix == 0 { "Phase modulation" } else { "Amplitude modulation" });
                            for index in (93 + matrix * 16)..(93 + (matrix + 1) * 16) {
                                parameter_ui(column, &params, &specs, setter, index);
                            }
                        }
                        columns[2].heading("LFO / effects");
                        for index in 125..PARAM_COUNT {
                            parameter_ui(&mut columns[2], &params, &specs, setter, index);
                        }
                    });
                    ui.label("Envelope edits apply to new notes. All other sound edits affect held notes.");
                });
            });
            },
        )
    }

    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        if !config.sample_rate.is_finite() || !(8000.0..=384000.0).contains(&config.sample_rate) {
            nih_error!("Unsupported Mx44 sample rate: {}", config.sample_rate);
            return false;
        }
        let mut engine = PluginEngine::new(config.sample_rate.round() as u32);
        engine.set_values(&self.params.values());
        self.engine = Some(engine);
        true
    }

    fn reset(&mut self) {
        if let Some(engine) = &mut self.engine {
            engine.reset();
        }
        self.panic_requested.store(false, Ordering::Relaxed);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let Some(engine) = &mut self.engine else {
            for samples in buffer.iter_samples() {
                for sample in samples {
                    *sample = 0.0;
                }
            }
            return ProcessStatus::Error("Mx44 Rust was not initialized");
        };
        engine.set_values(&self.params.values());
        if self.panic_requested.swap(false, Ordering::AcqRel) {
            engine.reset();
        }
        let mut event = context.next_event();
        for (timing, mut samples) in buffer.iter_samples().enumerate() {
            while event.is_some_and(|e| e.timing() <= timing as u32) {
                dispatch(engine, event.unwrap());
                event = context.next_event();
            }
            engine.set_master(self.params.controls[0].value.smoothed.next());
            let frame = engine.next_frame();
            for (channel, sample) in samples.iter_mut().enumerate() {
                *sample = frame[channel];
            }
        }
        // Events on the block boundary affect the following block, not an earlier sample.
        while let Some(remaining) = event {
            dispatch(engine, remaining);
            event = context.next_event();
        }
        if engine.is_active() || self.params.controls[0].value.smoothed.is_smoothing() {
            ProcessStatus::KeepAlive
        } else {
            ProcessStatus::Normal
        }
    }
}

fn dispatch(engine: &mut PluginEngine, event: NoteEvent<()>) {
    let midi = match event {
        NoteEvent::NoteOn {
            channel,
            note,
            velocity,
            ..
        } => Some(MidiEvent::NoteOn {
            channel,
            key: note,
            velocity: if velocity > 0.0 {
                (velocity.clamp(0.0, 1.0) * 127.0).round().max(1.0) as u8
            } else {
                0
            },
        }),
        NoteEvent::NoteOff { channel, note, .. } => Some(MidiEvent::NoteOff { channel, key: note }),
        NoteEvent::Choke { channel, note, .. } => {
            engine.choke(channel, note);
            None
        }
        NoteEvent::MidiCC {
            channel, cc, value, ..
        } => Some(MidiEvent::Control {
            channel,
            controller: cc,
            value: (value.clamp(0.0, 1.0) * 127.0).round() as u8,
        }),
        NoteEvent::MidiPitchBend { channel, value, .. } => Some(MidiEvent::Bend {
            channel,
            value: (value.clamp(0.0, 1.0) * 16384.0).round().min(16383.0) as u16,
        }),
        NoteEvent::MidiChannelPressure {
            channel, pressure, ..
        } => Some(MidiEvent::Pressure {
            channel,
            value: (pressure.clamp(0.0, 1.0) * 127.0).round() as u8,
        }),
        _ => None,
    };
    if let Some(event) = midi {
        engine.event(event);
    }
}

impl ClapPlugin for Mx44Plugin {
    const CLAP_ID: &'static str = "org.mx44.rust.synth";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Four-operator FM/AM matrix synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for Mx44Plugin {
    const VST3_CLASS_ID: [u8; 16] = *b"Mx44RustSynth001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_clap!(Mx44Plugin);

// The upstream VST3 COM macro emits this compatibility warning on recent Rust.
#[allow(semicolon_in_expressions_from_non_local_macros)]
mod vst3_export {
    use super::Mx44Plugin;
    nih_plug::nih_export_vst3!(Mx44Plugin);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn parameters_match_shared_schema_and_have_stable_ids() {
        let params = SynthParams::default();
        let mapping = params.param_map();
        assert_eq!(mapping.len(), PARAM_COUNT);
        assert_eq!(
            mapping
                .iter()
                .map(|(id, _, _)| id)
                .collect::<HashSet<_>>()
                .len(),
            PARAM_COUNT
        );
        assert_eq!(mapping[0].0, "value_1");
        assert_eq!(mapping[130].0, "value_131");
        assert_eq!(plugin::decode(&params.values()), (Sound::default(), 0.5));
        assert!(params.serialize_fields().contains_key("editor-state"));
    }

    #[test]
    fn host_notes_and_chokes_reach_the_synth() {
        let mut engine = PluginEngine::new(8000);
        engine.set_values(&plugin::encode(&Sound::default(), 0.5));
        dispatch(
            &mut engine,
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 3,
                note: 60,
                velocity: 0.8,
            },
        );
        assert!((0..1000).any(|_| engine.next_frame()[0].abs() > 0.01));
        dispatch(
            &mut engine,
            NoteEvent::Choke {
                timing: 0,
                voice_id: None,
                channel: 3,
                note: 60,
            },
        );
        assert_eq!(engine.next_frame(), [0.0; 2]);
    }
}
