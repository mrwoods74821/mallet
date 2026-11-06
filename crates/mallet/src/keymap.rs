//! Editor actions and their key bindings, starting from Hammer's defaults.
//!
//! Bindings live in one table so they can be rebound later without touching the code
//! that reacts to them.

use std::collections::HashSet;

use eframe::egui::{InputState, Key, Modifiers};

/// Something the user can trigger from the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    NewFile,
    CloseTab,
    NextTab,
    PreviousTab,
    OpenFile,
    SaveFile,
    Undo,
    Redo,
    GridSmaller,
    GridLarger,
    SelectionTool,
    BlockTool,
    EntityTool,
    Properties,
    /// Open the compile window (Hammer's Run Map).
    Compile,
    /// Compile again with the preset used last.
    CompileAgain,
    /// Open Mallet Options.
    Options,
    SelectAll,
    ClearSelection,
    DeleteSelection,
    CreateBlock,
    CenterViews2D,
    CenterView3D,
    ToggleMouseLook,
    MoveForward,
    MoveBack,
    StrafeLeft,
    StrafeRight,
    TurnLeft,
    TurnRight,
    LookUp,
    LookDown,
}

/// A key plus the exact modifiers that must be held with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    /// The key on its own.
    pub const fn key(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    /// Ctrl plus the key.
    pub const fn ctrl(key: Key) -> Self {
        Self {
            ctrl: true,
            ..Self::key(key)
        }
    }

    /// Shift plus the key.
    pub const fn shift(key: Key) -> Self {
        Self {
            shift: true,
            ..Self::key(key)
        }
    }

    /// Alt plus the key.
    pub const fn alt(key: Key) -> Self {
        Self {
            alt: true,
            ..Self::key(key)
        }
    }

    /// Ctrl+Shift plus the key.
    pub const fn ctrl_shift(key: Key) -> Self {
        Self {
            ctrl: true,
            shift: true,
            ..Self::key(key)
        }
    }

    fn modifiers_match(&self, modifiers: &Modifiers) -> bool {
        modifiers.ctrl == self.ctrl && modifiers.shift == self.shift && modifiers.alt == self.alt
    }

    /// Text shown next to menu entries, such as `Ctrl+O`.
    pub fn label(&self) -> String {
        let mut label = String::new();
        if self.ctrl {
            label.push_str("Ctrl+");
        }
        if self.shift {
            label.push_str("Shift+");
        }
        if self.alt {
            label.push_str("Alt+");
        }
        label.push_str(self.key.symbol_or_name());
        label
    }
}

/// The table of bindings.
pub struct KeyMap {
    bindings: Vec<(Action, Chord)>,
}

impl KeyMap {
    /// Hammer's default bindings for the actions Mallet supports so far.
    pub fn hammer_defaults() -> Self {
        Self {
            bindings: vec![
                (Action::NewFile, Chord::ctrl(Key::N)),
                (Action::CloseTab, Chord::ctrl(Key::W)),
                (Action::NextTab, Chord::ctrl(Key::Tab)),
                (Action::PreviousTab, Chord::ctrl_shift(Key::Tab)),
                (Action::OpenFile, Chord::ctrl(Key::O)),
                (Action::SaveFile, Chord::ctrl(Key::S)),
                (Action::Undo, Chord::ctrl(Key::Z)),
                (Action::Redo, Chord::ctrl(Key::Y)),
                (Action::GridSmaller, Chord::key(Key::OpenBracket)),
                (Action::GridLarger, Chord::key(Key::CloseBracket)),
                (Action::SelectionTool, Chord::shift(Key::S)),
                (Action::BlockTool, Chord::shift(Key::B)),
                (Action::EntityTool, Chord::shift(Key::E)),
                (Action::Properties, Chord::alt(Key::Enter)),
                (Action::Compile, Chord::key(Key::F9)),
                (Action::CompileAgain, Chord::shift(Key::F9)),
                (Action::SelectAll, Chord::ctrl(Key::A)),
                (Action::ClearSelection, Chord::key(Key::Escape)),
                (Action::DeleteSelection, Chord::key(Key::Delete)),
                (Action::CreateBlock, Chord::key(Key::Enter)),
                (Action::CenterViews2D, Chord::ctrl(Key::E)),
                (Action::CenterView3D, Chord::ctrl_shift(Key::E)),
                (Action::ToggleMouseLook, Chord::key(Key::Z)),
                (Action::MoveForward, Chord::key(Key::W)),
                (Action::MoveBack, Chord::key(Key::S)),
                (Action::StrafeLeft, Chord::key(Key::A)),
                (Action::StrafeRight, Chord::key(Key::D)),
                (Action::TurnLeft, Chord::key(Key::ArrowLeft)),
                (Action::TurnRight, Chord::key(Key::ArrowRight)),
                (Action::LookUp, Chord::key(Key::ArrowUp)),
                (Action::LookDown, Chord::key(Key::ArrowDown)),
            ],
        }
    }

    /// Every binding, in table order.
    pub fn bindings(&self) -> &[(Action, Chord)] {
        &self.bindings
    }

    /// The first chord bound to an action, for display in menus.
    pub fn chord(&self, action: Action) -> Option<Chord> {
        self.bindings
            .iter()
            .find(|(bound, _)| *bound == action)
            .map(|(_, chord)| *chord)
    }

    /// Menu label for an action's shortcut, or an empty string when it has none.
    pub fn label(&self, action: Action) -> String {
        self.chord(action).map(|chord| chord.label()).unwrap_or_default()
    }

    /// Whether the action was triggered this frame, with exactly its modifiers held.
    pub fn pressed(&self, input: &InputState, action: Action) -> bool {
        self.bindings.iter().any(|(bound, chord)| {
            *bound == action && input.key_pressed(chord.key) && chord.modifiers_match(&input.modifiers)
        })
    }

    /// Whether a movement action's key is held. Ctrl and Alt suppress it, and so does a
    /// key in `blocked`: one that went down as part of a shortcut such as Ctrl+S, which
    /// must not move the camera when Ctrl is let go first.
    pub fn held(&self, input: &InputState, action: Action, blocked: &HashSet<Key>) -> bool {
        !input.modifiers.ctrl
            && !input.modifiers.alt
            && self
                .bindings
                .iter()
                .any(|(bound, chord)| *bound == action && input.key_down(chord.key) && !blocked.contains(&chord.key))
    }
}
