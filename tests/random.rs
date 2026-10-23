use mx44_rust::{
    engine::Synth,
    midi::MidiEvent,
    patch::{Session, Sound},
};
use rand::{SeedableRng, rngs::StdRng};

#[test]
fn randomized_sounds_are_valid_varied_and_gain_limited() {
    let mut rng = StdRng::seed_from_u64(44);
    let mut previous = Sound::default();
    for _ in 0..1024 {
        let sound = Sound::random(&mut rng);
        sound.validate().unwrap();
        assert_ne!(sound, previous);
        assert_eq!(sound.operators[0].ratio, 1.0);
        assert!(sound.operators[0].level >= 0.28);
        let gain: f32 = sound.operators.iter().map(|op| op.level).sum();
        assert!((gain - 0.8).abs() < 0.000_001);
        assert!(sound.delay_feedback <= 0.45);
        assert!(sound.drive <= 0.25);
        previous = sound;
    }
}

#[test]
fn randomized_sounds_render_audibly_and_release() {
    let session = Session::default();
    let mut synth = Synth::new(8000, 4, &session);
    let mut rng = StdRng::seed_from_u64(2026);
    for _ in 0..32 {
        synth.panic();
        synth.set_sound(0, Sound::random(&mut rng));
        synth.event(MidiEvent::NoteOn {
            channel: 0,
            key: 60,
            velocity: 105,
        });
        let mut energy = 0.0;
        for _ in 0..4000 {
            for sample in synth.next_frame() {
                assert!(sample.is_finite() && sample.abs() <= 1.0);
                energy += sample * sample;
            }
        }
        let rms = (energy / 8000.0_f32).sqrt();
        assert!(
            rms > 0.005 && rms < 0.3,
            "unexpected randomized sound RMS {rms}"
        );
        synth.event(MidiEvent::NoteOff {
            channel: 0,
            key: 60,
        });
        for _ in 0..9600 {
            synth.next_frame();
        }
        assert_eq!(synth.active_voices(), 0);
    }
}
