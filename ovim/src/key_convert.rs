//! Conversion layer between crossterm/ratatui types and ovim-core types.
//!
//! All conversions happen at the boundary (event loop) so that the editor
//! internals only deal with ovim-core types.

use crossterm::event as ct;
use ovim_core::key as core;

pub fn convert_key_event(ke: ct::KeyEvent) -> core::KeyEvent {
    core::KeyEvent::new(convert_key_code(ke.code), convert_key_modifiers(ke.modifiers))
}

pub fn convert_key_code(kc: ct::KeyCode) -> core::KeyCode {
    match kc {
        ct::KeyCode::Char(c) => core::KeyCode::Char(c),
        ct::KeyCode::Enter => core::KeyCode::Enter,
        ct::KeyCode::Esc => core::KeyCode::Esc,
        ct::KeyCode::Tab => core::KeyCode::Tab,
        ct::KeyCode::BackTab => core::KeyCode::BackTab,
        ct::KeyCode::Backspace => core::KeyCode::Backspace,
        ct::KeyCode::Delete => core::KeyCode::Delete,
        ct::KeyCode::Left => core::KeyCode::Left,
        ct::KeyCode::Right => core::KeyCode::Right,
        ct::KeyCode::Up => core::KeyCode::Up,
        ct::KeyCode::Down => core::KeyCode::Down,
        ct::KeyCode::Home => core::KeyCode::Home,
        ct::KeyCode::End => core::KeyCode::End,
        ct::KeyCode::PageUp => core::KeyCode::PageUp,
        ct::KeyCode::PageDown => core::KeyCode::PageDown,
        ct::KeyCode::F(n) => core::KeyCode::F(n),
        ct::KeyCode::Null => core::KeyCode::Null,
        _ => core::KeyCode::Null,
    }
}

pub fn convert_key_modifiers(km: ct::KeyModifiers) -> core::Modifiers {
    let mut m = core::Modifiers::NONE;
    if km.contains(ct::KeyModifiers::SHIFT) {
        m |= core::Modifiers::SHIFT;
    }
    if km.contains(ct::KeyModifiers::CONTROL) {
        m |= core::Modifiers::CONTROL;
    }
    if km.contains(ct::KeyModifiers::ALT) {
        m |= core::Modifiers::ALT;
    }
    if km.contains(ct::KeyModifiers::SUPER) {
        m |= core::Modifiers::SUPER;
    }
    m
}

pub fn convert_mouse_event(me: ct::MouseEvent) -> core::MouseEvent {
    core::MouseEvent {
        kind: convert_mouse_event_kind(me.kind),
        column: me.column,
        row: me.row,
    }
}

pub fn convert_mouse_event_kind(mek: ct::MouseEventKind) -> core::MouseEventKind {
    match mek {
        ct::MouseEventKind::Down(b) => core::MouseEventKind::Down(convert_mouse_button(b)),
        ct::MouseEventKind::Up(b) => core::MouseEventKind::Up(convert_mouse_button(b)),
        ct::MouseEventKind::Drag(b) => core::MouseEventKind::Drag(convert_mouse_button(b)),
        ct::MouseEventKind::ScrollUp => core::MouseEventKind::ScrollUp,
        ct::MouseEventKind::ScrollDown => core::MouseEventKind::ScrollDown,
        _ => core::MouseEventKind::ScrollUp,
    }
}

pub fn convert_mouse_button(mb: ct::MouseButton) -> core::MouseButton {
    match mb {
        ct::MouseButton::Left => core::MouseButton::Left,
        ct::MouseButton::Middle => core::MouseButton::Middle,
        ct::MouseButton::Right => core::MouseButton::Right,
    }
}

pub fn convert_event(event: ct::Event) -> Option<core::Event> {
    match event {
        ct::Event::Key(ke) => Some(core::Event::Key(convert_key_event(ke))),
        ct::Event::Mouse(me) => Some(core::Event::Mouse(convert_mouse_event(me))),
        ct::Event::Resize(w, h) => Some(core::Event::Resize(w, h)),
        ct::Event::Paste(text) => Some(core::Event::Paste(text)),
        ct::Event::FocusGained => Some(core::Event::FocusGained),
        _ => None,
    }
}

pub fn convert_ratatui_rect(r: ratatui::layout::Rect) -> ovim_core::Rect {
    ovim_core::Rect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}

pub fn convert_core_rect(r: ovim_core::Rect) -> ratatui::layout::Rect {
    ratatui::layout::Rect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}
