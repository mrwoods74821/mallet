//! Synthetic keyboard and mouse input, queued by the `send_input` MCP tool and fed into
//! the window one frame at a time, so agents can exercise the same code paths as a user.

use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2};

/// Pointer steps between press and release of a synthetic drag; enough frames for egui
/// to recognise a drag and for tools to see intermediate positions.
const DRAG_STEPS: usize = 6;

/// Events delivered together in one frame, with the modifier keys held during it.
pub struct InputFrame {
    pub events: Vec<Event>,
    pub modifiers: Modifiers,
}

impl InputFrame {
    fn new(modifiers: Modifiers, events: Vec<Event>) -> Self {
        Self { events, modifiers }
    }
}

/// Parses modifier names: `ctrl`, `shift` and `alt`.
pub fn parse_modifiers(names: &[&str]) -> Result<Modifiers, String> {
    let mut modifiers = Modifiers::NONE;
    for name in names {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => {
                modifiers.ctrl = true;
                modifiers.command = true;
            }
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            other => return Err(format!("unknown modifier \"{other}\"; use ctrl, shift or alt")),
        }
    }
    Ok(modifiers)
}

/// Parses a mouse button name: `primary`, `secondary` or `middle`.
pub fn parse_button(name: &str) -> Result<PointerButton, String> {
    match name {
        "primary" | "left" => Ok(PointerButton::Primary),
        "secondary" | "right" => Ok(PointerButton::Secondary),
        "middle" => Ok(PointerButton::Middle),
        other => Err(format!("unknown button \"{other}\"; use primary, secondary or middle")),
    }
}

/// A key press and release, such as `Enter` or `B`, using egui's key names. The key is
/// held down for `frames` frames before it is released.
pub fn key_press(name: &str, modifiers: Modifiers, frames: usize) -> Result<Vec<InputFrame>, String> {
    let key = Key::from_name(name).ok_or_else(|| format!("unknown key \"{name}\""))?;
    let event = |pressed| Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    };
    let mut sequence = vec![InputFrame::new(modifiers, vec![event(true)])];
    for _ in 1..frames.max(1) {
        sequence.push(InputFrame::new(modifiers, Vec::new()));
    }
    sequence.push(InputFrame::new(modifiers, vec![event(false)]));
    Ok(sequence)
}

/// Moves the pointer to a screen position, presses and releases a button there.
pub fn click(at: Pos2, button: PointerButton, modifiers: Modifiers) -> Vec<InputFrame> {
    let press = |pressed| Event::PointerButton {
        pos: at,
        button,
        pressed,
        modifiers,
    };
    vec![
        InputFrame::new(modifiers, vec![Event::PointerMoved(at)]),
        InputFrame::new(modifiers, vec![press(true)]),
        InputFrame::new(modifiers, vec![press(false)]),
    ]
}

/// Presses a button at `from`, moves to `to` over several frames and releases there.
pub fn drag(from: Pos2, to: Pos2, button: PointerButton, modifiers: Modifiers) -> Vec<InputFrame> {
    let button_event = |pos, pressed| Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers,
    };
    let mut frames = vec![
        InputFrame::new(modifiers, vec![Event::PointerMoved(from)]),
        InputFrame::new(modifiers, vec![button_event(from, true)]),
    ];
    for step in 1..=DRAG_STEPS {
        let t = step as f32 / DRAG_STEPS as f32;
        frames.push(InputFrame::new(modifiers, vec![Event::PointerMoved(from.lerp(to, t))]));
    }
    frames.push(InputFrame::new(modifiers, vec![button_event(to, false)]));
    frames
}
