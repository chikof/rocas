//! Draws a screen as plain lines; the terminal layer only prints them.

use super::wizard::{EditTarget, REVIEW_CHROME, Screen, StartupItem, WatchRow, Wizard};
use crate::config::OnConflict;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Title,
    Selected,
    Dim,
    Warn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub style: Style,
}

impl Line {
    fn new(text: impl Into<String>, style: Style) -> Self {
        Self { text: text.into(), style }
    }

    fn plain(text: impl Into<String>) -> Self {
        Self::new(text, Style::Plain)
    }

    fn dim(text: impl Into<String>) -> Self {
        Self::new(text, Style::Dim)
    }

    fn blank() -> Self {
        Self::plain("")
    }
}

/// The whole screen: exactly `height` lines, each at most `width` characters.
pub fn render(wizard: &Wizard, width: usize, height: usize) -> Vec<Line> {
    let screen = wizard.screen;
    let mut lines = vec![
        Line::new(
            format!(
                "rocas setup: {} ({}/{})",
                screen.title(),
                screen.index() + 1,
                Screen::ALL.len()
            ),
            Style::Title,
        ),
        Line::blank(),
    ];

    let rows = height.saturating_sub(3);
    if screen == Screen::Review {
        lines.extend(review(wizard, rows.saturating_sub(REVIEW_CHROME - 3)));
    } else {
        let (body, selected) = body(wizard);
        lines.extend(window(body, selected, rows));
    }

    lines.truncate(height.saturating_sub(1));
    lines.resize(height.saturating_sub(1), Line::blank());
    lines.push(Line::dim(hint(wizard)));
    for line in &mut lines {
        line.text = truncate(&line.text, width);
    }
    lines.truncate(height);
    lines
}

/// Cuts `text` to `width` characters, ending in an ellipsis when it was cut.
/// Whole characters only; no wide-character width tables.
pub fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut out: String = text
        .chars()
        .take(width.saturating_sub(1))
        .collect();
    if width > 0 {
        out.push('…');
    }
    out
}

/// The `rows` lines that keep `selected` in view.
fn window(mut body: Vec<Line>, selected: Option<usize>, rows: usize) -> Vec<Line> {
    if body.len() <= rows {
        return body;
    }
    let start = selected
        .map_or(0, |s| (s + 1).saturating_sub(rows))
        .min(body.len() - rows);
    body.drain(..start);
    body.truncate(rows);
    body
}

fn mark(on: bool) -> &'static str {
    if on { "[x]" } else { "[ ]" }
}

fn row(selected: bool, text: &str) -> Line {
    if selected {
        Line::new(format!("> {text}"), Style::Selected)
    } else {
        Line::plain(format!("  {text}"))
    }
}

fn body(w: &Wizard) -> (Vec<Line>, Option<usize>) {
    match w.screen {
        Screen::Welcome => (welcome(w), None),
        Screen::Watch => watch(w),
        Screen::Rules => rules(w),
        Screen::Conflicts => conflicts(w),
        Screen::Startup => startup(w),
        Screen::Review => (Vec::new(), None),
    }
}

fn welcome(w: &Wizard) -> Vec<Line> {
    let mut lines = vec![
        Line::plain("Rocas watches folders and moves new files into other folders by rule."),
        Line::plain("A few questions, then it writes its configuration file:"),
        Line::blank(),
        Line::plain(format!("  {}", w.config_path.display())),
        Line::blank(),
        Line::plain("You can edit that file by hand later; rocas reloads it when it changes."),
    ];
    if w.existing {
        lines.push(Line::new(
            "That file already exists. Nothing is replaced without asking.",
            Style::Warn,
        ));
    }
    lines.push(Line::blank());
    lines.push(Line::dim("Steps: watch folders, rules, name clashes, startup, review."));
    lines
}

fn watch(w: &Wizard) -> (Vec<Line>, Option<usize>) {
    let mut lines = vec![
        Line::plain("Folders rocas watches. Files that appear in them get organized."),
        Line::blank(),
    ];
    let first = lines.len();
    let cursor = w.cursor();
    for index in 0..w.watch_rows() {
        let editing = w
            .edit
            .as_ref()
            .filter(|_| index == cursor);
        let label = match w.watch_row(index) {
            WatchRow::Path(i) => w.answers.watch_paths[i]
                .display()
                .to_string(),
            WatchRow::Add => "+ Add a folder".to_owned(),
            WatchRow::Recursive => {
                format!("{} Also watch inside subfolders", mark(w.answers.recursive))
            },
        };
        lines.push(match editing {
            Some(edit) => Line::new(format!("> Folder: {}_", edit.text), Style::Selected),
            None => row(index == cursor, &label),
        });
    }
    if w.answers.watch_paths.is_empty() {
        lines.push(Line::blank());
        lines.push(Line::dim("No folder chosen: rocas will watch your Downloads folder."));
    }
    (lines, Some(first + cursor))
}

fn rules(w: &Wizard) -> (Vec<Line>, Option<usize>) {
    let mut lines = vec![
        Line::plain("What to organize. A file matching a rule moves to that rule's folder."),
        Line::blank(),
    ];
    let first = lines.len();
    let cursor = w.cursor();
    for (index, rule) in w.answers.rules.iter().enumerate() {
        let line = match &w.edit {
            Some(edit)
                if index == cursor && matches!(edit.target, EditTarget::RuleDestination(_)) =>
            {
                Line::new(format!("> {} folder: {}_", rule.name, edit.text), Style::Selected)
            },
            _ => row(
                index == cursor,
                &format!("{} {:<11} {}", mark(rule.enabled), rule.name, rule.destination.display()),
            ),
        };
        lines.push(line);
    }
    if let Some(rule) = w.answers.rules.get(cursor) {
        lines.push(Line::blank());
        lines.push(Line::dim(format!("Matches: {}", rule.patterns.join(" "))));
        lines.push(Line::dim("Patterns are case-sensitive, so both cases are listed."));
    }
    (lines, Some(first + cursor))
}

fn conflicts(w: &Wizard) -> (Vec<Line>, Option<usize>) {
    let mut lines = vec![
        Line::plain("What happens when the destination already has a file with that name?"),
        Line::blank(),
    ];
    let first = lines.len();
    for (index, option) in Wizard::conflict_options()
        .into_iter()
        .enumerate()
    {
        let explanation = match option {
            OnConflict::Rename => "keep both: the new file becomes \"name (1).ext\"",
            OnConflict::Skip => "leave the new file where it is",
            OnConflict::Overwrite => "replace the file that is already there",
        };
        let chosen = if option == w.answers.on_conflict { "(*)" } else { "( )" };
        lines.push(row(
            index == w.cursor(),
            &format!("{chosen} {:<10} {explanation}", option.as_str()),
        ));
    }
    (lines, Some(first + w.cursor()))
}

fn startup(w: &Wizard) -> (Vec<Line>, Option<usize>) {
    let mut lines = vec![Line::plain("Startup options."), Line::blank()];
    let first = lines.len();
    for (index, item) in StartupItem::shown().iter().enumerate() {
        let (on, text) = match item {
            StartupItem::Boot => (w.answers.start_on_boot, "Start rocas when I log in"),
            StartupItem::Updates => {
                (w.answers.check_for_updates, "Look for a newer rocas when it starts")
            },
        };
        lines.push(row(index == w.cursor(), &format!("{} {text}", mark(on))));
    }
    (lines, Some(first + w.cursor()))
}

/// Notice and scrolled config text: `REVIEW_CHROME - 3` rows plus the text.
fn review(w: &Wizard, text_rows: usize) -> Vec<Line> {
    let path = w.config_path.display();
    let notice = if w.confirming {
        Line::new(
            format!("Overwrite {path}? Press y to confirm, any other key to cancel."),
            Style::Warn,
        )
    } else if w.existing {
        Line::new(format!("{path} exists: Enter asks before replacing it."), Style::Warn)
    } else {
        Line::dim(format!("This will be written to {path}"))
    };
    let mut lines = vec![notice, Line::blank()];
    lines.extend(
        w.review
            .iter()
            .skip(w.scroll)
            .take(text_rows)
            .map(Line::plain),
    );
    lines
}

fn hint(w: &Wizard) -> &'static str {
    if w.confirming {
        return "y overwrite · any other key cancel";
    }
    if w.edit.is_some() {
        return "type a path (~ is your home) · Enter save · Esc cancel";
    }
    match w.screen {
        Screen::Welcome => "Enter or Tab start · q quit",
        Screen::Watch => {
            "↑↓ move · Enter edit · Space toggle · d remove · Tab next · ← back · q quit"
        },
        Screen::Rules => {
            "↑↓ move · Space on/off · Enter change folder · Tab next · ← back · q quit"
        },
        Screen::Conflicts => "↑↓ move · Enter choose · Tab next · ← back · q quit",
        Screen::Startup => "↑↓ move · Space toggle · Enter or Tab next · ← back · q quit",
        Screen::Review => "↑↓ PgUp PgDn scroll · Enter write · ← back · q quit",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::setup::answers::{Answers, test_dirs};
    use crate::setup::wizard::Key;

    fn wizard(existing: bool) -> Wizard {
        Wizard::new(Answers::defaults(&test_dirs()), PathBuf::from("/c/rocas.toml"), existing)
    }

    fn texts(lines: &[Line]) -> Vec<&str> {
        lines
            .iter()
            .map(|l| l.text.as_str())
            .collect()
    }

    fn keys(w: &mut Wizard, keys: &[Key]) {
        for key in keys {
            w.handle(*key);
        }
    }

    #[test]
    fn every_screen_fills_the_terminal_exactly() {
        let mut w = wizard(true);
        w.resize(24);
        for _ in Screen::ALL {
            let lines = render(&w, 80, 24);
            assert_eq!(lines.len(), 24);
            assert!(
                lines
                    .iter()
                    .all(|l| l.text.chars().count() <= 80)
            );
            assert_eq!(lines.last().map(|l| l.style), Some(Style::Dim));
            w.handle(Key::Tab);
        }
    }

    #[test]
    fn welcome_names_the_file() {
        let lines = render(&wizard(false), 80, 24);
        assert_eq!(lines[0].text, "rocas setup: Welcome (1/6)");
        assert_eq!(lines[0].style, Style::Title);
        assert!(texts(&lines).contains(&"  /c/rocas.toml"));
    }

    #[test]
    fn watch_screen_marks_the_selection() {
        let mut w = wizard(false);
        keys(&mut w, &[Key::Tab]);
        let lines = render(&w, 80, 24);
        let selected: Vec<&Line> = lines
            .iter()
            .filter(|l| l.style == Style::Selected)
            .collect();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].text, "> /home/u/Downloads");
        assert!(texts(&lines).contains(&"  + Add a folder"));
        assert!(texts(&lines).contains(&"  [ ] Also watch inside subfolders"));
    }

    #[test]
    fn text_input_shows_what_was_typed() {
        let mut w = wizard(false);
        keys(&mut w, &[Key::Tab, Key::Down, Key::Enter, Key::Char('/'), Key::Char('x')]);
        let lines = render(&w, 80, 24);
        assert!(texts(&lines).contains(&"> Folder: /x_"));
        assert_eq!(
            lines.last().map(|l| l.text.as_str()),
            Some("type a path (~ is your home) · Enter save · Esc cancel")
        );
    }

    #[test]
    fn rules_screen_shows_checkboxes_and_patterns() {
        let mut w = wizard(false);
        keys(&mut w, &[Key::Tab, Key::Tab, Key::Char(' '), Key::Down]);
        let lines = render(&w, 100, 24);
        let all = texts(&lines);
        assert!(all.contains(&"  [ ] Documents   /home/u/Documents"));
        assert!(all.contains(&"> [x] Images      /home/u/Pictures"));
        assert!(
            all.iter()
                .any(|l| l.starts_with("Matches: *.jpg *.JPG"))
        );
    }

    #[test]
    fn conflicts_screen_explains_each_choice() {
        let mut w = wizard(false);
        keys(&mut w, &[Key::Tab, Key::Tab, Key::Tab]);
        let lines = render(&w, 100, 24);
        let all = texts(&lines);
        assert!(all.contains(&"> (*) rename     keep both: the new file becomes \"name (1).ext\""));
        assert!(all.contains(&"  ( ) skip       leave the new file where it is"));
        assert!(all.contains(&"  ( ) overwrite  replace the file that is already there"));
    }

    #[test]
    fn review_shows_the_config_text_and_scrolls() {
        let mut w = wizard(false);
        w.resize(10);
        keys(&mut w, &[Key::Tab; 5]);
        let lines = render(&w, 80, 10);
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[2].text, "This will be written to /c/rocas.toml");
        assert_eq!(lines[4].text, w.review[0]);
        keys(&mut w, &[Key::Down, Key::Down]);
        assert_eq!(render(&w, 80, 10)[4].text, w.review[2]);
    }

    #[test]
    fn review_warns_about_an_existing_file() {
        let mut w = wizard(true);
        w.resize(24);
        keys(&mut w, &[Key::Tab; 5]);
        assert!(
            render(&w, 100, 24)[2]
                .text
                .ends_with("exists: Enter asks before replacing it.")
        );
        keys(&mut w, &[Key::Enter]);
        let lines = render(&w, 100, 24);
        assert_eq!(lines[2].style, Style::Warn);
        assert!(
            lines[2]
                .text
                .starts_with("Overwrite /c/rocas.toml?")
        );
        assert_eq!(lines[23].text, "y overwrite · any other key cancel");
    }

    #[test]
    fn long_lines_are_cut_on_character_boundaries() {
        assert_eq!(truncate("héllo wörld", 5), "héll…");
        assert_eq!(truncate("héllo", 5), "héllo");
        assert_eq!(truncate("日本語のパス", 3), "日本…");
        assert_eq!(truncate("abc", 0), "");
        let lines = render(&wizard(false), 20, 24);
        assert!(
            lines
                .iter()
                .all(|l| l.text.chars().count() <= 20)
        );
        assert!(lines[0].text.ends_with('…'));
    }

    #[test]
    fn a_short_terminal_keeps_the_selection_visible() {
        let mut w = wizard(false);
        keys(&mut w, &[Key::Tab, Key::Tab, Key::Up]);
        let lines = render(&w, 80, 7);
        assert_eq!(lines.len(), 7);
        assert!(
            lines
                .iter()
                .any(|l| l.style == Style::Selected)
        );
    }
}
