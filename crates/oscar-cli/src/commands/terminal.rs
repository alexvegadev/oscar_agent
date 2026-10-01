//! A transient prompt editor. It never stores prompts on disk or executes commands.
use crossterm::{
    cursor::{MoveToColumn, MoveUp},
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers,
    },
    queue,
    style::{Color, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use futures_util::StreamExt;
use oscar_core::error::OscarError;
use std::io::{self, Write};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

struct Recommendation {
    command: &'static str,
    description: &'static str,
}
const COMMANDS: &[Recommendation] = &[
    Recommendation {
        command: "/read",
        description: "Load a workspace file · /read PATH",
    },
    Recommendation {
        command: "/reset",
        description: "Clear conversation and file context",
    },
    Recommendation {
        command: "/status",
        description: "Show session usage and limits",
    },
    Recommendation {
        command: "/help",
        description: "Show all commands",
    },
    Recommendation {
        command: "/exit",
        description: "End this session",
    },
];

#[derive(Default)]
struct Editor {
    line: String,
    cursor: usize,
    selected: usize,
    dismissed: bool,
}
enum Action {
    Continue,
    Submit(String),
    Exit,
    Cancel,
}
impl Editor {
    fn recommendations(&self) -> Vec<&'static Recommendation> {
        if self.dismissed || !self.line.starts_with('/') || self.line.contains(char::is_whitespace)
        {
            return vec![];
        }
        COMMANDS
            .iter()
            .filter(|c| c.command.starts_with(&self.line))
            .collect()
    }
    fn insert(&mut self, text: &str, max: usize) -> Result<(), OscarError> {
        // Pasted line breaks become spaces; pasted controls never become commands.
        let clean: String = text
            .chars()
            .filter_map(|c| match c {
                '\n' | '\r' | '\t' => Some(' '),
                c if c.is_control() => None,
                c => Some(c),
            })
            .collect();
        if self.line.len().saturating_add(clean.len()) > max {
            return Err(OscarError::Limit("input line exceeds byte limit".into()));
        }
        self.line.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
        self.selected = 0;
        self.dismissed = false;
        Ok(())
    }
    fn complete(&mut self) -> bool {
        let options = self.recommendations();
        if let Some(option) = options.get(self.selected % options.len().max(1)) {
            self.line = option.command.to_owned();
            if option.command == "/read" {
                self.line.push(' ');
            }
            self.cursor = self.line.len();
            self.dismissed = true;
            return true;
        }
        false
    }
    fn key(&mut self, key: KeyEvent, max: usize) -> Result<Action, OscarError> {
        if key.kind == KeyEventKind::Release {
            return Ok(Action::Continue);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(match key.code {
                KeyCode::Char('c') => Action::Cancel,
                KeyCode::Char('d') if self.line.is_empty() => Action::Exit,
                KeyCode::Char('u') => {
                    *self = Self::default();
                    Action::Continue
                }
                _ => Action::Continue,
            });
        }
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::ALT) => {
                self.insert(&c.to_string(), max)?
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let previous = self.line[..self.cursor]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.line.drain(previous..self.cursor);
                self.cursor = previous;
                self.selected = 0;
                self.dismissed = false;
            }
            KeyCode::Delete if self.cursor < self.line.len() => {
                let next = self.cursor
                    + self.line[self.cursor..]
                        .chars()
                        .next()
                        .map(char::len_utf8)
                        .unwrap_or(0);
                self.line.drain(self.cursor..next);
                self.selected = 0;
                self.dismissed = false;
            }
            KeyCode::Left if self.cursor > 0 => {
                self.cursor = self.line[..self.cursor]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
            }
            KeyCode::Right if self.cursor < self.line.len() => {
                self.cursor += self.line[self.cursor..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0);
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.line.len(),
            KeyCode::Down | KeyCode::Up => {
                let len = self.recommendations().len();
                if len > 0 {
                    self.selected = if key.code == KeyCode::Down {
                        (self.selected + 1) % len
                    } else {
                        (self.selected + len - 1) % len
                    };
                }
            }
            KeyCode::Esc => self.dismissed = true,
            KeyCode::Tab => {
                self.complete();
            }
            KeyCode::Enter => {
                let completed = self.complete();
                if !(completed && self.line == "/read ") {
                    return Ok(Action::Submit(self.line.clone()));
                }
            }
            _ => {}
        }
        Ok(Action::Continue)
    }
}

fn clipped(text: &str, width: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= width
        })
        .collect()
}
fn viewport(line: &str, cursor: usize, width: usize) -> (String, usize) {
    let mut start = 0;
    while line[start..cursor].width() >= width.max(1) {
        start += line[start..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(1);
    }
    (clipped(&line[start..], width), line[start..cursor].width())
}

struct PromptGuard;
impl PromptGuard {
    fn begin() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self;
        crossterm::execute!(io::stdout(), EnableBracketedPaste)?;
        Ok(guard)
    }
}
impl Drop for PromptGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = crossterm::execute!(
            out,
            MoveToColumn(0),
            Clear(ClearType::FromCursorDown),
            ResetColor,
            DisableBracketedPaste
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn draw(editor: &Editor) -> io::Result<()> {
    let (columns, height) = terminal::size()?;
    let width = usize::from(columns.saturating_sub(1));
    let prompt = if width < 12 { "> " } else { "oscar › " };
    let room = width.saturating_sub(prompt.width());
    let (line, column) = viewport(&editor.line, editor.cursor, room);
    let mut out = io::stdout();
    queue!(
        out,
        MoveToColumn(0),
        Clear(ClearType::FromCursorDown),
        SetForegroundColor(Color::Cyan)
    )?;
    write!(out, "{}", clipped(prompt, width))?;
    queue!(out, ResetColor)?;
    write!(out, "{line}")?;
    let options = editor.recommendations();
    let rows = usize::from(height.saturating_sub(2)).min(options.len() + 1);
    let mut drawn = 0;
    if !options.is_empty() && rows > 0 {
        // Keep the selection visible even when the terminal is too short for all commands.
        let count = rows.saturating_sub(1).max(1).min(options.len());
        let start = editor.selected.saturating_sub(count - 1);
        for (index, option) in options.iter().enumerate().skip(start).take(count) {
            let selected = index == editor.selected;
            write!(out, "\r\n")?;
            queue!(
                out,
                SetForegroundColor(if selected {
                    Color::Cyan
                } else {
                    Color::DarkGrey
                })
            )?;
            write!(
                out,
                "{}",
                clipped(
                    &format!(
                        "{} {:8} {}",
                        if selected { "›" } else { " " },
                        option.command,
                        option.description
                    ),
                    width
                )
            )?;
            drawn += 1;
        }
        if drawn < rows {
            write!(out, "\r\n")?;
            queue!(out, SetForegroundColor(Color::DarkGrey))?;
            write!(
                out,
                "{}",
                clipped(
                    "  ↑↓ choose · Tab complete · Enter select · Esc dismiss",
                    width
                )
            )?;
            drawn += 1;
        }
    }
    queue!(out, ResetColor)?;
    if drawn > 0 {
        queue!(out, MoveUp(drawn as u16))?;
    }
    queue!(
        out,
        MoveToColumn((prompt.width().min(width) + column).min(width) as u16)
    )?;
    out.flush()
}

pub(super) async fn read_line(max: usize) -> Result<Option<String>, OscarError> {
    let failure = |_| OscarError::Io("interactive terminal input failed".into());
    let guard = PromptGuard::begin().map_err(failure)?;
    let mut editor = Editor::default();
    let mut events = EventStream::new();
    draw(&editor).map_err(failure)?;
    while let Some(event) = events.next().await {
        let action = match event.map_err(failure)? {
            Event::Key(key) => editor.key(key, max)?,
            Event::Paste(text) => {
                editor.insert(&text, max)?;
                Action::Continue
            }
            Event::Resize(_, _) => Action::Continue,
            _ => continue,
        };
        match action {
            Action::Continue => draw(&editor).map_err(failure)?,
            Action::Submit(line) => {
                drop(guard);
                // Input is sanitized by the editor; restore cooked mode before output.
                println!("oscar › {line}");
                return Ok(Some(line));
            }
            Action::Exit => return Ok(None),
            Action::Cancel => return Err(OscarError::Cancelled),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    #[test]
    fn slash_shows_recommendations_and_filters_as_you_type() {
        let mut e = Editor::default();
        e.key(key(KeyCode::Char('/')), 100).unwrap();
        assert_eq!(e.recommendations().len(), 5);
        e.insert("re", 100).unwrap();
        assert_eq!(
            e.recommendations()
                .iter()
                .map(|c| c.command)
                .collect::<Vec<_>>(),
            ["/read", "/reset"]
        );
        e.key(key(KeyCode::Down), 100).unwrap();
        assert!(
            matches!(e.key(key(KeyCode::Enter), 100).unwrap(), Action::Submit(s) if s == "/reset")
        );
    }
    #[test]
    fn completion_preserves_read_arguments_and_escape_dismisses_menu() {
        let mut e = Editor::default();
        e.insert("/", 100).unwrap();
        assert!(matches!(
            e.key(key(KeyCode::Enter), 100).unwrap(),
            Action::Continue
        ));
        assert_eq!(e.line, "/read ");
        e.insert("src/main.rs", 100).unwrap();
        assert!(e.recommendations().is_empty());
        assert!(
            matches!(e.key(key(KeyCode::Enter), 100).unwrap(), Action::Submit(s) if s == "/read src/main.rs")
        );
        e = Editor::default();
        e.insert("/st", 100).unwrap();
        e.key(key(KeyCode::Tab), 100).unwrap();
        assert_eq!(e.line, "/status");
        e = Editor::default();
        e.insert("/", 100).unwrap();
        e.key(key(KeyCode::Esc), 100).unwrap();
        assert!(e.recommendations().is_empty());
    }
    #[test]
    fn unicode_editing_paste_and_limits_preserve_valid_input() {
        let mut e = Editor::default();
        e.insert("é中", 10).unwrap();
        e.key(key(KeyCode::Left), 10).unwrap();
        e.key(key(KeyCode::Backspace), 10).unwrap();
        assert_eq!(e.line, "中");
        assert_eq!(e.cursor, 0);
        e.key(key(KeyCode::Delete), 10).unwrap();
        e.insert("hi\n\x1bthere", 20).unwrap();
        assert_eq!(e.line, "hi there");
        assert!(e.insert("a long string", 10).is_err());
        assert_eq!(e.line, "hi there");
        let (text, cursor) = viewport("é中abc", "é中abc".len(), 4);
        assert!(text.width() <= 4 && cursor < 4);
    }
    #[test]
    fn released_keys_are_ignored_and_control_keys_are_explicit() {
        let mut e = Editor::default();
        let mut released = key(KeyCode::Char('/'));
        released.kind = KeyEventKind::Release;
        e.key(released, 10).unwrap();
        assert!(e.line.is_empty());
        assert!(matches!(
            e.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), 10)
                .unwrap(),
            Action::Cancel
        ));
        assert!(matches!(
            e.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL), 10)
                .unwrap(),
            Action::Exit
        ));
    }
}
