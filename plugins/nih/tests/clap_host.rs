use clap_sys::{
    audio_buffer::clap_audio_buffer,
    entry::clap_plugin_entry,
    events::*,
    ext::{params::*, state::*},
    factory::plugin_factory::*,
    host::clap_host,
    plugin::clap_plugin,
    process::*,
    stream::{clap_istream, clap_ostream},
    version::CLAP_VERSION,
};
use mx44_rust::plugin::PARAM_COUNT;
use std::{
    ffi::{CStr, c_char, c_void},
    io::{Cursor, Read},
    mem::{MaybeUninit, size_of},
    ptr,
};

unsafe extern "C" fn no_extension(_: *const clap_host, _: *const c_char) -> *const c_void {
    ptr::null()
}
unsafe extern "C" fn request(_: *const clap_host) {}

struct Instance(*const clap_plugin);
impl Drop for Instance {
    fn drop(&mut self) {
        // The instance is inactive before destruction and its host is still alive.
        unsafe {
            ((*self.0).destroy.unwrap())(self.0);
        }
    }
}

unsafe extern "C" fn event_count(list: *const clap_input_events) -> u32 {
    unsafe { (*(*list).ctx.cast::<Vec<*const clap_event_header>>()).len() as u32 }
}
unsafe extern "C" fn get_event(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    unsafe { (&*(*list).ctx.cast::<Vec<*const clap_event_header>>())[index as usize] }
}
unsafe extern "C" fn discard_event(
    _: *const clap_output_events,
    _: *const clap_event_header,
) -> bool {
    true
}

fn input(events: &mut Vec<*const clap_event_header>) -> clap_input_events {
    clap_input_events {
        ctx: (events as *mut Vec<*const clap_event_header>).cast(),
        size: Some(event_count),
        get: Some(get_event),
    }
}

unsafe extern "C" fn write_state(
    stream: *const clap_ostream,
    bytes: *const c_void,
    size: u64,
) -> i64 {
    unsafe {
        let output = &mut *(*stream).ctx.cast::<Vec<u8>>();
        output.extend_from_slice(std::slice::from_raw_parts(
            bytes.cast::<u8>(),
            size as usize,
        ));
        size as i64
    }
}
unsafe extern "C" fn read_state(stream: *const clap_istream, bytes: *mut c_void, size: u64) -> i64 {
    unsafe {
        let input = &mut *(*stream).ctx.cast::<Cursor<Vec<u8>>>();
        input
            .read(std::slice::from_raw_parts_mut(
                bytes.cast::<u8>(),
                size as usize,
            ))
            .unwrap() as i64
    }
}

fn header<T>(time: u32, type_: u16) -> clap_event_header {
    clap_event_header {
        size: size_of::<T>() as u32,
        time,
        space_id: CLAP_CORE_EVENT_SPACE_ID,
        type_,
        flags: 0,
    }
}

#[test]
fn clap_host_checks_discovery_state_audio_and_sample_accurate_events() {
    check_plugin(&mx44_plugin::clap_entry);
}

#[test]
#[ignore = "build the plugin bundle and set MX44_CLAP_BINARY to its shared library"]
fn packaged_clap_checks_discovery_state_audio_and_sample_accurate_events() {
    let path = std::env::var_os("MX44_CLAP_BINARY").expect("set MX44_CLAP_BINARY");
    // The library stays loaded until all plugin objects and entry-point references are gone.
    unsafe {
        let library = libloading::Library::new(path).unwrap();
        let entry = library
            .get::<*const clap_plugin_entry>(b"clap_entry\0")
            .unwrap();
        check_plugin(&**entry);
    }
}

fn check_plugin(clap_entry: &clap_plugin_entry) {
    // All pointers refer to stack/heap objects that outlive their synchronous ABI calls.
    unsafe {
        assert!((clap_entry.init.unwrap())(c".".as_ptr()));
        let factory = (clap_entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr())
            .cast::<clap_plugin_factory>();
        assert!(!factory.is_null());
        assert_eq!(((*factory).get_plugin_count.unwrap())(factory), 1);
        let descriptor = ((*factory).get_plugin_descriptor.unwrap())(factory, 0);
        assert_eq!(CStr::from_ptr((*descriptor).id), c"org.mx44.rust.synth");
        let host = clap_host {
            clap_version: CLAP_VERSION,
            host_data: ptr::null_mut(),
            name: c"Mx44 test host".as_ptr(),
            vendor: c"Mx44".as_ptr(),
            url: c"https://example.invalid".as_ptr(),
            version: c"1".as_ptr(),
            get_extension: Some(no_extension),
            request_restart: Some(request),
            request_process: Some(request),
            request_callback: Some(request),
        };
        let instance = Instance(((*factory).create_plugin.unwrap())(
            factory,
            &host,
            (*descriptor).id,
        ));
        let plugin = instance.0;
        assert!(!plugin.is_null());
        assert!(((*plugin).init.unwrap())(plugin));
        let params = &*((*plugin).get_extension.unwrap())(plugin, CLAP_EXT_PARAMS.as_ptr())
            .cast::<clap_plugin_params>();
        assert_eq!((params.count.unwrap())(plugin) as usize, PARAM_COUNT);
        for (index, spec) in mx44_rust::plugin::parameter_specs().iter().enumerate() {
            let mut info = MaybeUninit::<clap_param_info>::uninit();
            assert!((params.get_info.unwrap())(
                plugin,
                index as u32,
                info.as_mut_ptr()
            ));
            let info = info.assume_init();
            // NIH exposes continuous CLAP values normalized to 0..1.
            assert_eq!(info.min_value, 0.0);
            assert_eq!(info.max_value, 1.0);
            assert!((0.0..=1.0).contains(&info.default_value));
            assert!(
                CStr::from_ptr(info.name.as_ptr())
                    .to_string_lossy()
                    .contains(&spec.name)
            );
            assert_ne!(info.flags & CLAP_PARAM_IS_AUTOMATABLE, 0);
        }
        let mut info = MaybeUninit::<clap_param_info>::uninit();
        assert!((params.get_info.unwrap())(plugin, 0, info.as_mut_ptr()));
        let info = info.assume_init();
        assert_eq!(info.default_value, 0.5);
        let state = &*((*plugin).get_extension.unwrap())(plugin, CLAP_EXT_STATE.as_ptr())
            .cast::<clap_plugin_state>();
        let mut saved = Vec::<u8>::new();
        let output = clap_ostream {
            ctx: (&mut saved as *mut Vec<u8>).cast(),
            write: Some(write_state),
        };
        assert!((state.save.unwrap())(plugin, &output));
        assert!(!saved.is_empty());
        let output_events = clap_output_events {
            ctx: ptr::null_mut(),
            try_push: Some(discard_event),
        };
        let change = clap_event_param_value {
            header: header::<clap_event_param_value>(0, CLAP_EVENT_PARAM_VALUE),
            param_id: info.id,
            cookie: info.cookie,
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: 0.2,
        };
        let mut changes = vec![&change.header as *const _];
        (params.flush.unwrap())(plugin, &input(&mut changes), &output_events);
        let mut value = 0.0;
        assert!((params.get_value.unwrap())(plugin, info.id, &mut value));
        assert!((value - 0.2).abs() < 0.0001);
        let mut cursor = Cursor::new(saved);
        let input_state = clap_istream {
            ctx: (&mut cursor as *mut Cursor<Vec<u8>>).cast(),
            read: Some(read_state),
        };
        assert!((state.load.unwrap())(plugin, &input_state));
        assert!((params.get_value.unwrap())(plugin, info.id, &mut value));
        assert_eq!(value, 0.5);
        assert!(((*plugin).activate.unwrap())(plugin, 48000.0, 1, 1024));
        assert!(((*plugin).start_processing.unwrap())(plugin));

        let mut left = [f32::NAN; 1024];
        let mut right = [f32::NAN; 1024];
        let mut channels = [left.as_mut_ptr(), right.as_mut_ptr()];
        let mut audio = clap_audio_buffer {
            data32: channels.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let note = clap_event_note {
            header: header::<clap_event_note>(128, CLAP_EVENT_NOTE_ON),
            note_id: -1,
            port_index: 0,
            channel: 0,
            key: 60,
            velocity: 0.9,
        };
        let mut events = vec![&note.header as *const _];
        let mut input_events = input(&mut events);
        let mut process = clap_process {
            steady_time: 0,
            frames_count: 1024,
            transport: ptr::null(),
            audio_inputs: ptr::null(),
            audio_outputs: &mut audio,
            audio_inputs_count: 0,
            audio_outputs_count: 1,
            in_events: &input_events,
            out_events: &output_events,
        };
        assert_ne!(
            ((*plugin).process.unwrap())(plugin, &process),
            CLAP_PROCESS_ERROR
        );
        assert!(left[..128].iter().all(|v| *v == 0.0));
        assert!(left[128..].iter().any(|v| v.abs() > 0.01));
        assert!(
            left.iter()
                .chain(right.iter())
                .all(|v| v.is_finite() && v.abs() <= 1.0)
        );

        let mut level = MaybeUninit::<clap_param_info>::uninit();
        assert!((params.get_info.unwrap())(plugin, 3, level.as_mut_ptr()));
        let level = level.assume_init();
        let mute = clap_event_param_value {
            header: header::<clap_event_param_value>(256, CLAP_EVENT_PARAM_VALUE),
            param_id: level.id,
            cookie: level.cookie,
            value: 0.0,
            ..change
        };
        events = vec![&mute.header as *const _];
        input_events = input(&mut events);
        process.in_events = &input_events;
        process.steady_time += 1024;
        assert_ne!(
            ((*plugin).process.unwrap())(plugin, &process),
            CLAP_PROCESS_ERROR
        );
        // Parameter changes are intentionally block-based; notes remain sample-timed.
        assert!(left.iter().all(|v| *v == 0.0));

        let restore_level = clap_event_param_value {
            header: header::<clap_event_param_value>(0, CLAP_EVENT_PARAM_VALUE),
            value: level.default_value,
            ..mute
        };
        let choke = clap_event_note {
            header: header::<clap_event_note>(64, CLAP_EVENT_NOTE_CHOKE),
            ..note
        };
        events = vec![&restore_level.header as *const _, &choke.header as *const _];
        input_events = input(&mut events);
        process.in_events = &input_events;
        process.steady_time += 1024;
        assert_ne!(
            ((*plugin).process.unwrap())(plugin, &process),
            CLAP_PROCESS_ERROR
        );
        assert!(left[..64].iter().any(|v| v.abs() > 0.001));
        assert!(left[64..].iter().all(|v| *v == 0.0));
        ((*plugin).stop_processing.unwrap())(plugin);
        ((*plugin).deactivate.unwrap())(plugin);
        drop(instance);
        (clap_entry.deinit.unwrap())();
    }
}
