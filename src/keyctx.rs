//! Context-aware meaning of Ctrl-C / Ctrl-Shift-C and Ctrl-A, decided in one place from
//! what has focus (or sits under the mouse), what is selected and what is running.
//! [`ctrl_c`] and [`ctrl_a`] are pure, so the whole table is unit-tested.

/// A part of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    #[default]
    None,
    Input,
    Sidebar,
    /// The chat / output pane.
    Body,
    Header,
    /// A pick list, the theme editor, help, the shell output or another modal.
    Overlay,
}

/// What the resolver looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ctx {
    /// Keyboard focus; `Overlay` while any overlay is open.
    pub focus: Pane,
    /// The pane under the mouse (last reported position).
    pub hovered: Pane,
    /// Text selected in the chat body (in-app selection).
    pub body_selection: bool,
    /// A range selected in the input box.
    pub input_selection: bool,
    pub input_has_text: bool,
    /// A request is in flight (waiting for the bot's reply).
    pub in_flight: bool,
    /// The open overlay handles Ctrl-C itself.
    pub overlay_own_ctrl_c: bool,
    /// A lone Ctrl-C asked "press again to quit" recently, with nothing done since.
    pub quit_armed: bool,
}

/// What Ctrl-C (or the copy shortcut Ctrl-Shift-C) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtrlC {
    CopyBodySelection,
    CopyInputSelection,
    CancelRequest,
    /// Ctrl-Shift-C with text and no selection: copy the whole input.
    CopyInput,
    /// Ctrl-C with text and no selection: clear it (Ctrl-Z brings it back).
    ClearInput,
    /// Close the overlay like Esc.
    CloseOverlay,
    /// Hand the key to the overlay.
    Overlay,
    /// Over the chat with nothing selected: say how to select.
    BodyHint,
    /// Copy shortcut with nothing to copy.
    NothingToCopy,
    /// "Press Ctrl-C again to quit".
    ArmQuit,
    Quit,
}

/// Resolve Ctrl-C (`copy` = Ctrl-Shift-C, which never quits, closes or cancels).
pub fn ctrl_c(c: &Ctx, copy: bool) -> CtrlC {
    // 1. A selection anywhere: copy it.
    if c.body_selection {
        return CtrlC::CopyBodySelection;
    }
    if c.input_selection {
        return CtrlC::CopyInputSelection;
    }
    // 4 (an overlay has the focus, so 2 and 3 can't apply).
    if c.focus == Pane::Overlay {
        return match (c.overlay_own_ctrl_c, copy) {
            (true, _) => CtrlC::Overlay,
            (false, true) => CtrlC::NothingToCopy,
            (false, false) => CtrlC::CloseOverlay,
        };
    }
    let on_chat = c.focus == Pane::Body || c.hovered == Pane::Body;
    // 2. Something running, attention on the conversation or the input.
    if !copy && c.in_flight && (c.focus == Pane::Input || on_chat) {
        return CtrlC::CancelRequest;
    }
    // 3. Text in the focused input.
    if c.focus == Pane::Input && c.input_has_text {
        return if copy {
            CtrlC::CopyInput
        } else {
            CtrlC::ClearInput
        };
    }
    // 5. Over the chat, nothing selected.
    if on_chat {
        return CtrlC::BodyHint;
    }
    // 6. Nothing else applies.
    match (copy, c.quit_armed) {
        (true, _) => CtrlC::NothingToCopy,
        (false, true) => CtrlC::Quit,
        (false, false) => CtrlC::ArmQuit,
    }
}

/// What Ctrl-A selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtrlA {
    /// All of the input.
    Input,
    /// The message under the mouse / in view in the chat.
    BodyMessage,
    None,
}

pub fn ctrl_a(c: &Ctx) -> CtrlA {
    match (c.focus, c.hovered) {
        (Pane::Overlay, _) => CtrlA::None,
        (Pane::Body, _) | (_, Pane::Body) if !(c.focus == Pane::Input && c.input_has_text) => {
            CtrlA::BodyMessage
        }
        (Pane::Input, _) if c.input_has_text => CtrlA::Input,
        _ => CtrlA::None,
    }
}

#[cfg(test)]
mod tests {
    use super::{CtrlA, CtrlC, Ctx, Pane, ctrl_a, ctrl_c};

    fn ctx(focus: Pane, hovered: Pane) -> Ctx {
        Ctx {
            focus,
            hovered,
            ..Ctx::default()
        }
    }

    /// The table: focus × selection × in-flight × overlay × input text × armed.
    #[test]
    fn ctrl_c_table() {
        use CtrlC::*;
        use Pane::{Body, Header, Input, None as No, Overlay, Sidebar};
        // focus, hovered, body sel, input sel, in flight, text, armed → Ctrl-C, Ctrl-Shift-C
        type Row = (Pane, Pane, bool, bool, bool, bool, bool, CtrlC, CtrlC);
        let rows: &[Row] = &[
            (
                Input,
                Body,
                true,
                false,
                true,
                true,
                true,
                CopyBodySelection,
                CopyBodySelection,
            ),
            (
                Overlay,
                No,
                true,
                false,
                false,
                false,
                false,
                CopyBodySelection,
                CopyBodySelection,
            ),
            (
                Input,
                No,
                false,
                true,
                true,
                true,
                true,
                CopyInputSelection,
                CopyInputSelection,
            ),
            (
                Overlay,
                Body,
                false,
                false,
                true,
                true,
                true,
                CloseOverlay,
                NothingToCopy,
            ),
            (
                Input,
                No,
                false,
                false,
                true,
                true,
                false,
                CancelRequest,
                CopyInput,
            ),
            (
                Sidebar,
                Body,
                false,
                false,
                true,
                false,
                false,
                CancelRequest,
                BodyHint,
            ),
            (
                Sidebar,
                No,
                false,
                false,
                true,
                false,
                false,
                ArmQuit,
                NothingToCopy,
            ),
            (
                Input, No, false, false, false, true, false, ClearInput, CopyInput,
            ),
            (
                Input, Body, false, false, false, true, true, ClearInput, CopyInput,
            ),
            (
                Sidebar,
                No,
                false,
                false,
                false,
                true,
                false,
                ArmQuit,
                NothingToCopy,
            ),
            (
                Input, Body, false, false, false, false, false, BodyHint, BodyHint,
            ),
            (
                Body, No, false, false, false, false, true, BodyHint, BodyHint,
            ),
            (
                Input,
                No,
                false,
                false,
                false,
                false,
                false,
                ArmQuit,
                NothingToCopy,
            ),
            (
                Input,
                Header,
                false,
                false,
                false,
                false,
                true,
                Quit,
                NothingToCopy,
            ),
            (
                Sidebar,
                Sidebar,
                false,
                false,
                false,
                false,
                true,
                Quit,
                NothingToCopy,
            ),
        ];
        for (i, &(f, h, bs, is, fl, t, armed, want, want_copy)) in rows.iter().enumerate() {
            let c = Ctx {
                focus: f,
                hovered: h,
                body_selection: bs,
                input_selection: is,
                in_flight: fl,
                input_has_text: t,
                quit_armed: armed,
                overlay_own_ctrl_c: false,
            };
            assert_eq!(ctrl_c(&c, false), want, "row {i} Ctrl-C");
            assert_eq!(ctrl_c(&c, true), want_copy, "row {i} Ctrl-Shift-C");
            assert_ne!(
                ctrl_c(&c, true),
                Quit,
                "row {i}: the copy shortcut never quits"
            );
        }
    }

    #[test]
    fn an_overlay_with_its_own_ctrl_c_gets_it() {
        let c = Ctx {
            overlay_own_ctrl_c: true,
            ..ctx(Pane::Overlay, Pane::None)
        };
        assert_eq!(ctrl_c(&c, false), CtrlC::Overlay);
        assert_eq!(ctrl_c(&c, true), CtrlC::Overlay);
    }

    #[test]
    fn ctrl_a_table() {
        let t = |f, h, text| {
            ctrl_a(&Ctx {
                input_has_text: text,
                ..ctx(f, h)
            })
        };
        assert_eq!(t(Pane::Input, Pane::None, true), CtrlA::Input);
        assert_eq!(
            t(Pane::Input, Pane::Body, true),
            CtrlA::Input,
            "typing wins over the hover"
        );
        assert_eq!(t(Pane::Input, Pane::Body, false), CtrlA::BodyMessage);
        assert_eq!(t(Pane::Body, Pane::None, false), CtrlA::BodyMessage);
        assert_eq!(t(Pane::Input, Pane::None, false), CtrlA::None);
        assert_eq!(t(Pane::Overlay, Pane::Body, true), CtrlA::None);
        assert_eq!(t(Pane::Sidebar, Pane::None, true), CtrlA::None);
    }
}
