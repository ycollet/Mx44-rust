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

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
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

## Attribution and license

Inspired by [Mx44](https://github.com/linuxmao-org/Mx44), originally by
**Jens M Andreasen** (2002-2006), with later contributions including
**James Morris**. The local C/GTK2 source was consulted for the synthesis
architecture and envelope design. No original binary patches are bundled.

Distributed under **GPL-2.0-or-later**, consistent with Mx44. See `COPYING`.
