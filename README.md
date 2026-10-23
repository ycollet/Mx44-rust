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
The original C synth's binary `.mx44patch` files are **not compatible**. The old
file format, exact fixed-point oscillator behavior, microtonal temperaments,
GS envelope controllers and original LFO/waveshaping modes are not ported.
This instrument is deliberately similar in architecture, not identical in sound.

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
to avoid stuck notes. Oscillators are not oversampled: aggressive modulation
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
