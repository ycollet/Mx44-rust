use mx44_rust::{
    engine::Synth,
    midi::MidiEvent,
    patch::{Session, Sound},
    plugin::{PluginEngine, encode},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
};

struct CountingAllocator;
thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.with(|count| count.set(count.get() + 1));
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn rendering_live_edits_and_resets_do_not_allocate() {
    let session = Session::default();
    let mut standalone = Synth::new(48_000, 32, &session);
    let mut plugin = PluginEngine::new(48_000);
    let sound = Sound {
        delay_time: 1.5,
        delay_mix: 0.5,
        ..Sound::default()
    };
    let values = encode(&sound, 0.5);
    COUNTING.with(|enabled| enabled.set(true));
    for index in 0..512 {
        let note = MidiEvent::NoteOn {
            channel: (index % 16) as u8,
            key: (48 + index % 24) as u8,
            velocity: 110,
        };
        standalone.set_sound(index % 16, sound);
        plugin.set_values(&values);
        standalone.event(note);
        plugin.event(note);
        for _ in 0..128 {
            black_box(standalone.next_frame());
            black_box(plugin.next_frame());
        }
        if index % 8 == 0 {
            standalone.panic();
            plugin.reset();
        } else {
            let clear = MidiEvent::Control {
                channel: (index % 16) as u8,
                controller: 120,
                value: 0,
            };
            standalone.event(clear);
            plugin.event(clear);
        }
    }
    COUNTING.with(|enabled| enabled.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}
