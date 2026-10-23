# Mx44 Rust

A standalone Rust synthesizer inspired by Jens M Andreasen's **Mx44**.
This is a new implementation, not an emulation or a source-compatible port.
It replaces GTK2, x86 MMX and the JACK-only audio path with an egui editor,
portable floating-point DSP, CPAL audio and midir MIDI input.

## Features

- Four operators per voice, each with ratio, detune, velocity response, pan,
  octave doubling and sine, Mx-style parabolic, triangle or soft-saw waveforms.
- Full 4x4 phase- and amplitude-modulation matrices, including self-feedback.
  Routes use the previous sample, so feedback has a defined one-sample delay.
- Independent eight-stage envelopes: two attack, two decay, two sustain and
  two release stages. Sustain can loop; release starts at the current level.
- 16 independent MIDI channels, 128 editable preset slots, 1-256 shared voices,
  sustain pedal, voice stealing, pitch bend, vibrato and channel aftertouch.
- Stereo operator mixing, per-channel drive and ping-pong delay.
- Six starter sounds: Mx tone, electric piano, bass, bells, pad and organ.
- Computer/on-screen keyboard, headless MIDI operation and offline WAV rendering.
- Validated, versioned JSON sessions, saved by temporary-file/rename replacement.

## Build and run

Use a current stable Rust toolchain with Rust 2024 support (Rust 1.88 or newer).
On Debian/Ubuntu, install the native audio and window-system development packages:

```sh
sudo apt install build-essential pkg-config libasound2-dev \
  libx11-dev libxi-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev
cargo run --release
```

On Fedora:

```sh
sudo dnf install gcc gcc-c++ pkgconf-pkg-config alsa-lib-devel \
  libX11-devel libXi-devel libxkbcommon-devel wayland-devel mesa-libGL-devel
cargo run --release
```

macOS is also supported using CoreAudio/CoreMIDI and a Rust toolchain.
No GTK or JACK development packages are required. CPAL uses ALSA on Linux;
select an ALSA device routed through PipeWire/PulseAudio if that is your setup.
This version does **not** expose JACK audio/MIDI ports.

```sh
cargo run --release -- --help
cargo run --release -- --list-audio
cargo run --release -- --audio-device "unique device name"
cargo install --path .
mx44-rust
```

Build the release profile for actual playing; debug builds are not intended for
low-latency use. The output device supplies the live sample rate and buffer size.

## Playing and MIDI

By default the synth creates a virtual MIDI input named **Mx44 Rust**. On Linux,
connect a keyboard or sequencer with a patchbay or ALSA's `aconnect`:

```sh
aconnect -l
aconnect <keyboard-client>:<port> <mx44-client>:<port>
```

Alternatively, open an existing input directly:

```sh
mx44-rust --list-midi
mx44-rust --midi-port 0
mx44-rust --midi-port "unique input name"
mx44-rust --no-midi
mx44-rust --console --voices 64
```

Linux MIDI needs a functioning ALSA sequencer (`/dev/snd/seq`; the `snd_seq`
kernel module). If MIDI initialization fails, fix that setup or use `--no-midi`
to play with the editor's keyboard. There is no silent fallback.

Play the computer keys **A W S E D F T G Y H U J K O L P ;** chromatically,
starting at C of the selected octave. Text fields capture typing; click out of
them to play. Mouse keys sound while held. **Escape / PANIC** stops all voices
and clears the delay buffers. Focus loss releases editor-generated notes.
Keyboard notes remember their original channel and octave until released.

MIDI note-on/off, program change, channel pressure and pitch bend (+/-2 semitones)
are supported on all channels. Displayed presets/channels are 1-based; MIDI is
0-based. Controllers:

| CC | Function |
|---|---|
| 1 | Additional vibrato depth |
| 7 / 11 | Channel volume / expression |
| 10 | Pan |
| 64 | Sustain pedal |
| 70 / 74 | Phase-modulation intensity (not a filter cutoff) |
| 120 | Immediate all sound off on this channel, including delay |
| 121 | Reset channel controllers |
| 123 | All notes off (respects sustain) |

Select a channel to edit its working sound. Operator output levels affect the
audible mix, not modulation sends: a muted operator can still modulate another.
Matrix rows are destinations, columns are sources; PM values are radians.
Operator and matrix edits affect held notes; envelope edits apply to new notes.
**Copy/Paste** transfers one complete operator.

**Random** generates a new working sound on the selected channel, including
operators, envelopes, both modulation matrices, LFO and effects. It favors
harmonic ratios and moderate modulation/effect levels, keeps an audible
fundamental and normalizes total operator output. Master volume, other channels
and stored presets are unchanged. Play a new note to hear the new envelopes;
use **Store in preset** and **Save session** to keep a discovery in the bank.

## Patches and sessions

The default state is `$XDG_CONFIG_HOME/mx44-rust/session.json`, or
`~/.config/mx44-rust/session.json` when XDG_CONFIG_HOME is unset. A missing
default file starts with factory presets; malformed files are reported, never
silently replaced. A normal window close or console Ctrl-C saves the session.
Forced termination does not save.

**Store in preset** copies the channel's working sound into the selected bank
slot. **Save session** writes the entire bank, all 16 channel working sounds,
selected programs and master volume to disk. Selecting/recalling a preset
replaces that channel's unsaved working edits. Storing a preset does not change
other channels already playing it; recall it there to pick up the update.
Controller positions and held notes are not persisted.

```sh
mx44-rust --save-to ./my-bank.json
mx44-rust --load-from ./my-bank.json --save-to ./edited-bank.json
```

An explicit `--load-from` requires `--save-to`, except for read-only rendering.
The original C synth's binary `.mx44patch` files must be converted first; they
cannot be loaded directly as JSON. Exact fixed-point oscillator behavior,
microtonal temperaments, GS envelope controllers and original LFO/waveshaping
modes are not ported.
This instrument is deliberately similar in architecture, not identical in sound.

### Audio engine differences from the original

Both engines use four operators, PM/AM matrices and eight-stage envelopes, but
the Rust engine is a new implementation, **not an emulation**:

| Area | Original Mx44 | Rust engine |
|---|---|---|
| Arithmetic | Fixed-point/MMX arithmetic with integer wrapping and saturation | Floating-point DSP with different clipping behavior |
| Oscillators | Parabolic wave components, self-shaping and selectable secondary harmonics | Simplified Mx approximation, sine, triangle and soft saw; octave mixing does not reproduce every original harmonic mode |
| Modulation | Original fixed-point PM/AM scaling, saturation and operator-specific wheel assignments | Different scaling, clipping and controller mappings; feedback-heavy sounds can change significantly |
| Envelopes | Nonzero initial levels, key/velocity-dependent timing and level scaling | Envelopes start at zero, with different timing and velocity response; original keyboard scaling is not implemented |
| LFO | Two rates, evolving depth and operator-specific phases | A simpler sine LFO shared by operators on each MIDI channel |
| Processing | Per-operator low-pass filtering and resonant waveshaping | Original filtering/shaping is absent; drive and stereo delay are separate new effects |
| Tuning | Original temperaments, shruti settings and additive frequency offsets | Equal temperament, frequency ratios and cents-based detuning |

Simple tones may be reasonably close, while complex original presets can sound
very different. Importing preserves their approximate structure and settings,
not their exact sound. Floating point is not inherently less accurate or lower
quality: the largest audible differences come from oscillator algorithms,
nonlinear behavior and control mappings. Closer reproduction would require a
dedicated legacy-compatible synthesis mode, which is not currently implemented.

### Importing original Mx44 banks

An approximate conversion of the supplied Mx44 `data/mx44patch` bank is included
in `presets/mx44-legacy.json`. Original preset data is from Jens M Andreasen's
Mx44. Open it without overwriting the bundled copy:

```sh
cargo run --release -- --load-from presets/mx44-legacy.json --save-to my-legacy-session.json
```

Convert another original bank without opening audio, MIDI or a window:

```sh
cargo run --release -- --import-legacy ../Mx44_clone/data/mx44patch \
  --save-to my-imported-bank.json
```

The importer reads little-endian 480-byte records: either a 128-preset bank
(61,440 bytes), or a bank with 16 editable channel sounds, selected programs and
mono flags (69,152 bytes), optionally followed by shruti/temperament data
(69,164/69,165 bytes). Other sizes are rejected. `--save-to` is required and may
replace an existing destination session; the original input cannot be the
destination. Conversion does not change the default user session.

Names, operator tuning, PM/AM routes, mixer proportions and envelope targets are
translated. Editable channel sounds are preserved separately from their stored
presets. Detune and envelope timing are approximated at MIDI A4, velocity 100,
assuming the original ran at 48 kHz. Use `--sample-rate 44100` during import if
the source used 44.1 kHz; the binary format does not store its sample rate.

Conversion prints warnings for clipped values and unsupported features,
including initial phase/envelope levels, inverse velocity, keyboard scaling,
intonation, operator shaping/filtering, higher secondary harmonics, mono/legato
and microtonal tuning. The dual-rate LFO becomes an average sine LFO.
Reserved original FX fields do not enable new drive/delay effects.
These are playable approximations, **not sound-identical presets**.

## Offline rendering

Render a short demo phrase without opening audio, MIDI or a window:

```sh
cargo run --release -- --render demo.wav --program 2 --duration 8
mx44-rust --render bells.wav --program 4 --sample-rate 48000
mx44-rust --load-from my-bank.json --render custom.wav
```

Files are stereo 16-bit PCM. The final third is reserved for note releases and
delay tails; long envelope/effect tails may extend beyond the requested length.
Rendering does not save or modify the session.

## VST3 and CLAP plugins

Both formats use [NIH-plug](https://github.com/robbert-vdh/nih-plug), pinned to a
specific revision. The standalone app remains unchanged. LV2 is not included.

```sh
cargo xtask bundle mx44-plugin --release
```

Bundles are written to:

```text
target/bundled/Mx44 Rust.vst3
target/bundled/Mx44 Rust.clap
```

The build targets the current operating system and CPU architecture; build on
Linux to produce Linux plugins. The plugin has no ALSA, CPAL or MIDI-device
dependency: audio and MIDI come exclusively from the host. Its embedded editor
uses OpenGL/X11 on Linux (install the window-system development packages above).
NIH-plug dependencies are fetched from Git on the first build.

Copy the bundles into your host's plugin search directories, then rescan:

| Platform | VST3 | CLAP |
|---|---|---|
| Linux | `~/.vst3/` | `~/.clap/` |
| macOS | `~/Library/Audio/Plug-Ins/VST3/` | `~/Library/Audio/Plug-Ins/CLAP/` |

For example, on Linux:

```sh
mkdir -p ~/.vst3 ~/.clap
cp -R "target/bundled/Mx44 Rust.vst3" ~/.vst3/
cp -R "target/bundled/Mx44 Rust.clap" ~/.clap/
```

Load **Mx44 Rust** as an instrument on a MIDI track. Each instance provides
stereo output and 32 voices, receives notes/controllers on all 16 MIDI channels,
and uses **one shared sound** across those channels. Use separate plugin
instances for independent sounds. There is no audio input, device selector,
standalone keyboard or automatic access to the standalone session file.
The plugin uses one shared stereo delay bus to reduce per-instance memory:
about 0.55 MiB at 48 kHz or 2.20 MiB at 192 kHz. MIDI CC120 stops voices on
its channel and clears **all plugin echoes**; other channels' voices continue.
The standalone retains independent per-channel delays.

The plugin exposes 131 sound/master parameters for host automation, with stable
IDs and sample-timed MIDI. Parameter automation is applied at block boundaries;
sample-accurate parameter automation is deliberately disabled because the pinned
NIH-plug CLAP wrapper applies a block's first automation event early.
Master gain is smoothed.
All operator settings, both matrices, envelopes, LFO and effects are editable
in its embedded editor and the host's generic parameter editor. The final
envelope level remains fixed at zero. **Random** changes the sound through the
host's parameter system without changing master volume; the six factory sounds
are available from **Load factory sound**. **PANIC** clears voices and effects.

Save your host project or a host preset to retain the sound and editor state.
Plugin state is managed by the host, not `session.json`. MIDI program changes,
MPE/polyphonic expression and the standalone's 128-slot bank editor are not
implemented in the plugin; select factory sounds in the editor instead.
Envelope edits apply to newly played notes, as in the standalone.

## Development

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
cargo test --workspace
cargo test -p mx44-rust --no-default-features
```

The DSP and MIDI parser are in the library; the audio/MIDI runtime and editor
are separate binary modules. The audio callback owns its synthesizer and reads
a bounded, allocation-free command queue; UI/MIDI producers never lock the
audio callback. Queue overflow is reported and requests emergency all-sound-off
to avoid stuck notes. Oscillator/pan/effect coefficients are cached and refreshed
on sound/controller changes; SoftSaw derives its harmonics from one sine/cosine
pair. Panic and CC120 invalidate delay history without
clearing large buffers on the audio thread. Dormant delays skip processing;
feedback below -240 dBFS is flushed to zero so tails eventually finish.
The plugin permits host suspension only after all voices, delay history and
gain smoothing have finished; its LFOs pause while the host suspends processing.
Oscillators are not oversampled: aggressive modulation
and high harmonics can alias. A final soft limiter bounds the output but is
not a substitute for conservative monitoring volume.

The current implementation has been built and smoke-tested on macOS. Linux
audio/MIDI hardware integration has not yet been verified on a Linux host.
Plugin tests exercise CLAP discovery, parameter/state round trips, stereo
processing and sample-timed note/choke events through NIH-plug's actual CLAP
entry point. Third-party DAW/editor compatibility still needs testing in your
chosen host.

To run the same host checks against a packaged binary, build the bundles and
set `MX44_CLAP_BINARY` to the `.clap` file on Linux, or to
`target/bundled/Mx44 Rust.clap/Contents/MacOS/Mx44 Rust` on macOS:

```sh
MX44_CLAP_BINARY="/absolute/path/to/plugin/binary" \
  cargo test -p mx44-plugin --test clap_host packaged_clap -- --ignored
```

## Attribution and license

Inspired by [Mx44](https://github.com/linuxmao-org/Mx44), originally by
**Jens M Andreasen** (2002-2006), with later contributions including
**James Morris**. The local C/GTK2 source was consulted for the synthesis
architecture and envelope design. No original binary patches are bundled.

The standalone and shared DSP are **GPL-2.0-or-later**, consistent with Mx44;
see `COPYING`. The `mx44-plugin` package and its combined VST3/CLAP binaries are
**GPL-3.0-or-later**; see `COPYING-GPL3`. NIH-plug itself is ISC-licensed, but
its VST3 bindings require GPLv3-compatible distribution. The shared DSP's
"or later" license permits its inclusion in these GPLv3 plugin binaries.
