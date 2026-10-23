#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MidiEvent {
    NoteOn {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        key: u8,
    },
    Control {
        channel: u8,
        controller: u8,
        value: u8,
    },
    Program {
        channel: u8,
        program: u8,
    },
    Bend {
        channel: u8,
        value: u16,
    },
    Pressure {
        channel: u8,
        value: u8,
    },
}

pub fn parse(bytes: &[u8]) -> Option<MidiEvent> {
    let status = *bytes.first()?;
    let channel = status & 0x0f;
    let data = |index| bytes.get(index).copied().filter(|v| *v < 128);
    match status & 0xf0 {
        0x80 => {
            data(2)?;
            Some(MidiEvent::NoteOff {
                channel,
                key: data(1)?,
            })
        }
        0x90 => {
            let (key, velocity) = (data(1)?, data(2)?);
            Some(if velocity == 0 {
                MidiEvent::NoteOff { channel, key }
            } else {
                MidiEvent::NoteOn {
                    channel,
                    key,
                    velocity,
                }
            })
        }
        0xb0 => Some(MidiEvent::Control {
            channel,
            controller: data(1)?,
            value: data(2)?,
        }),
        0xc0 => Some(MidiEvent::Program {
            channel,
            program: data(1)?,
        }),
        0xd0 => Some(MidiEvent::Pressure {
            channel,
            value: data(1)?,
        }),
        0xe0 => Some(MidiEvent::Bend {
            channel,
            value: data(1)? as u16 | ((data(2)? as u16) << 7),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_channels_bend_and_velocity_zero() {
        assert_eq!(
            parse(&[0x93, 60, 0]),
            Some(MidiEvent::NoteOff {
                channel: 3,
                key: 60
            })
        );
        assert_eq!(
            parse(&[0xef, 0, 64]),
            Some(MidiEvent::Bend {
                channel: 15,
                value: 8192
            })
        );
        assert_eq!(parse(&[0x90, 60]), None);
        assert_eq!(parse(&[0x90, 255, 80]), None);
        assert_eq!(parse(&[0xf8]), None);
    }
}
