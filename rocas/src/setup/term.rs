//! The terminal side of the wizard: raw mode, drawing, key decoding.

use std::io::{self, Write};
use std::panic;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};

use super::render::{self, Line, Style};
use super::wizard::{Key, Outcome, Wizard};

/// Whether the terminal is in the wizard's mode and needs restoring.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static HOOK: Once = Once::new();

/// Leaves raw mode and the alternate screen, once. Safe to call from a panic
/// hook, a destructor, or both.
fn restore() {
    if ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

/// Puts the terminal in the wizard's mode; dropping it undoes that.
struct Guard;

impl Guard {
    fn enter() -> io::Result<Self> {
        // A release build aborts on panic and skips destructors, so the hook
        // is what restores the terminal there.
        HOOK.call_once(|| {
            let previous = panic::take_hook();
            panic::set_hook(Box::new(move |info| {
                restore();
                previous(info);
            }));
        });
        terminal::enable_raw_mode()?;
        ACTIVE.store(true, Ordering::SeqCst);
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(guard)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

/// Runs the wizard until it ends.
pub fn run(mut wizard: Wizard) -> io::Result<Outcome> {
    let _guard = Guard::enter()?;
    loop {
        let (width, height) = terminal::size()?;
        wizard.resize(usize::from(height));
        draw(&render::render(&wizard, usize::from(width), usize::from(height)))?;
        // A resize event just falls through and redraws.
        if let Event::Key(key) = event::read()?
            && key.kind != KeyEventKind::Release
            && let Some(key) = decode(key)
            && let Some(outcome) = wizard.handle(key)
        {
            return Ok(outcome);
        }
    }
}

fn draw(lines: &[Line]) -> io::Result<()> {
    let mut out = io::stdout().lock();
    for (row, line) in (0u16..).zip(lines) {
        queue!(out, MoveTo(0, row))?;
        match line.style {
            Style::Plain => queue!(out, Print(&line.text))?,
            Style::Title => queue!(
                out,
                SetAttribute(Attribute::Bold),
                Print(&line.text),
                SetAttribute(Attribute::Reset)
            )?,
            Style::Selected => {
                queue!(
                    out,
                    SetAttribute(Attribute::Reverse),
                    Print(&line.text),
                    SetAttribute(Attribute::Reset)
                )?;
            },
            Style::Dim => queue!(
                out,
                SetAttribute(Attribute::Dim),
                Print(&line.text),
                SetAttribute(Attribute::Reset)
            )?,
            Style::Warn => {
                queue!(out, SetForegroundColor(Color::Yellow), Print(&line.text), ResetColor)?;
            },
        }
        queue!(out, Clear(ClearType::UntilNewLine))?;
    }
    queue!(out, Clear(ClearType::FromCursorDown))?;
    out.flush()
}

fn decode(key: KeyEvent) -> Option<Key> {
    let ctrl = key
        .modifiers
        .contains(KeyModifiers::CONTROL);
    Some(match key.code {
        KeyCode::Char('c') if ctrl => Key::Interrupt,
        KeyCode::Char(_) if ctrl => return None,
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_keys_the_wizard_uses() {
        let plain = |code| decode(KeyEvent::new(code, KeyModifiers::NONE));
        assert_eq!(plain(KeyCode::Char('j')), Some(Key::Char('j')));
        assert_eq!(plain(KeyCode::BackTab), Some(Key::BackTab));
        assert_eq!(plain(KeyCode::F(1)), None);
        let ctrl = |c| decode(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
        assert_eq!(ctrl('c'), Some(Key::Interrupt));
        assert_eq!(ctrl('q'), None);
    }
}
