use crossterm::{
    event::{
        self, Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement},
};
use std::io::stdout;

fn main() {
    enable_raw_mode().unwrap();

    let supports = supports_keyboard_enhancement().unwrap_or(false);
    eprintln!("supports_keyboard_enhancement: {}", supports);

    if supports {
        let result = execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
        eprintln!("PushKeyboardEnhancementFlags result: {:?}", result);
    }

    eprintln!("Press keys (Ctrl-C to quit). Try Cmd+1, then ':', then '_':");
    loop {
        if event::poll(std::time::Duration::from_millis(100)).unwrap() {
            let ev = event::read().unwrap();
            match &ev {
                Event::Key(key) => {
                    eprintln!(
                        "Key: code={:?} modifiers={:?} kind={:?} state={:?}",
                        key.code, key.modifiers, key.kind, key.state
                    );
                    if key.code == crossterm::event::KeyCode::Char('c')
                        && key.modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                    {
                        break;
                    }
                }
                _ => {
                    eprintln!("Event: {:?}", ev);
                }
            }
        }
    }

    if supports {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
    disable_raw_mode().unwrap();
}
