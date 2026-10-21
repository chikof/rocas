//! The wizard as a pure state machine: keys in, state out, no terminal.

use std::path::PathBuf;

use super::answers::Answers;
use super::toml;
use crate::config::OnConflict;

/// A key press, already decoded by the terminal layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Tab,
    BackTab,
    Enter,
    Esc,
    Backspace,
    Delete,
    Char(char),
    /// Ctrl-C.
    Interrupt,
}

/// How the wizard ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Write(Answers),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    Watch,
    Rules,
    Conflicts,
    Startup,
    Review,
}

impl Screen {
    pub const ALL: [Self; 6] =
        [Self::Welcome, Self::Watch, Self::Rules, Self::Conflicts, Self::Startup, Self::Review];

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|s| *s == self)
            .unwrap_or_default()
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Welcome => "Welcome",
            Self::Watch => "Watch folders",
            Self::Rules => "Rules",
            Self::Conflicts => "Name clashes",
            Self::Startup => "Startup",
            Self::Review => "Review",
        }
    }
}

/// One row of the watch screen, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchRow {
    Path(usize),
    Add,
    Recursive,
}

/// One checkbox of the startup screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupItem {
    Boot,
    Updates,
}

impl StartupItem {
    /// The update question exists only in builds that can update.
    pub const fn shown() -> &'static [Self] {
        if cfg!(feature = "self-update") { &[Self::Boot, Self::Updates] } else { &[Self::Boot] }
    }
}

/// What a text input edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditTarget {
    /// A watched folder; `None` adds a new one.
    WatchPath(Option<usize>),
    RuleDestination(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub target: EditTarget,
    pub text: String,
}

/// Rows the review screen spends on everything but the config text: title,
/// blank, notice, blank, key hint.
pub const REVIEW_CHROME: usize = 5;

pub struct Wizard {
    pub answers: Answers,
    pub screen: Screen,
    pub config_path: PathBuf,
    /// The config file exists: writing needs an explicit yes.
    pub existing: bool,
    pub edit: Option<Edit>,
    /// Waiting for the answer to "overwrite the existing file?".
    pub confirming: bool,
    /// The text of the review screen.
    pub review: Vec<String>,
    pub scroll: usize,
    review_rows: usize,
    watch_cursor: usize,
    rules_cursor: usize,
    conflict_cursor: usize,
    startup_cursor: usize,
}

const CONFLICTS: [OnConflict; 3] = [OnConflict::Rename, OnConflict::Skip, OnConflict::Overwrite];

impl Wizard {
    pub fn new(answers: Answers, config_path: PathBuf, existing: bool) -> Self {
        let conflict_cursor = CONFLICTS
            .iter()
            .position(|c| *c == answers.on_conflict)
            .unwrap_or_default();
        Self {
            answers,
            screen: Screen::Welcome,
            config_path,
            existing,
            edit: None,
            confirming: false,
            review: Vec::new(),
            scroll: 0,
            review_rows: 1,
            watch_cursor: 0,
            rules_cursor: 0,
            conflict_cursor,
            startup_cursor: 0,
        }
    }

    /// Tells the wizard how tall the terminal is, for review scrolling.
    pub fn resize(&mut self, height: usize) {
        self.review_rows = height
            .saturating_sub(REVIEW_CHROME)
            .max(1);
        self.scroll = self.scroll.min(self.max_scroll());
    }

    pub fn conflict_options() -> [OnConflict; 3] {
        CONFLICTS
    }

    /// The highlighted row of the current list screen.
    pub const fn cursor(&self) -> usize {
        match self.screen {
            Screen::Watch => self.watch_cursor,
            Screen::Rules => self.rules_cursor,
            Screen::Conflicts => self.conflict_cursor,
            Screen::Startup => self.startup_cursor,
            Screen::Welcome | Screen::Review => 0,
        }
    }

    pub fn watch_row(&self, row: usize) -> WatchRow {
        let paths = self.answers.watch_paths.len();
        match row {
            r if r < paths => WatchRow::Path(r),
            r if r == paths => WatchRow::Add,
            _ => WatchRow::Recursive,
        }
    }

    pub fn watch_rows(&self) -> usize {
        self.answers.watch_paths.len() + 2
    }

    /// Feeds one key; `Some` when the wizard is over.
    pub fn handle(&mut self, key: Key) -> Option<Outcome> {
        if key == Key::Interrupt {
            return Some(Outcome::Quit);
        }
        if self.confirming {
            return self.confirm(key);
        }
        if self.edit.is_some() {
            self.edit_key(key);
            return None;
        }
        match key {
            Key::Char('q') => return Some(Outcome::Quit),
            Key::Up | Key::Char('k') => self.step(false),
            Key::Down | Key::Char('j') => self.step(true),
            Key::PageUp => self.scroll_review(-1, self.review_rows),
            Key::PageDown => self.scroll_review(1, self.review_rows),
            Key::Tab | Key::Right => self.go(1),
            Key::BackTab | Key::Left | Key::Backspace => self.go(-1),
            Key::Char(' ') => self.toggle(),
            Key::Char('d') | Key::Delete => self.remove(),
            Key::Enter => return self.enter(),
            Key::Esc | Key::Char(_) | Key::Interrupt => {},
        }
        None
    }

    fn confirm(&mut self, key: Key) -> Option<Outcome> {
        self.confirming = false;
        matches!(key, Key::Char('y' | 'Y')).then(|| Outcome::Write(self.answers.clone()))
    }

    fn go(&mut self, delta: isize) {
        let target = self
            .screen
            .index()
            .checked_add_signed(delta)
            .and_then(|i| Screen::ALL.get(i));
        if let Some(&screen) = target {
            self.screen = screen;
            if screen == Screen::Review {
                self.review = toml::render(&self.answers)
                    .lines()
                    .map(str::to_owned)
                    .collect();
                self.scroll = 0;
            }
        }
    }

    fn rows(&self) -> usize {
        match self.screen {
            Screen::Watch => self.watch_rows(),
            Screen::Rules => self.answers.rules.len(),
            Screen::Conflicts => CONFLICTS.len(),
            Screen::Startup => StartupItem::shown().len(),
            Screen::Welcome | Screen::Review => 0,
        }
    }

    fn cursor_mut(&mut self) -> Option<&mut usize> {
        match self.screen {
            Screen::Watch => Some(&mut self.watch_cursor),
            Screen::Rules => Some(&mut self.rules_cursor),
            Screen::Conflicts => Some(&mut self.conflict_cursor),
            Screen::Startup => Some(&mut self.startup_cursor),
            Screen::Welcome | Screen::Review => None,
        }
    }

    fn step(&mut self, down: bool) {
        if self.screen == Screen::Review {
            self.scroll_review(if down { 1 } else { -1 }, 1);
            return;
        }
        let rows = self.rows();
        if let Some(cursor) = self.cursor_mut()
            && rows > 0
        {
            *cursor = if down { (*cursor + 1) % rows } else { (*cursor + rows - 1) % rows };
        }
    }

    fn max_scroll(&self) -> usize {
        self.review
            .len()
            .saturating_sub(self.review_rows)
    }

    fn scroll_review(&mut self, direction: isize, amount: usize) {
        if self.screen == Screen::Review {
            let moved = if direction < 0 {
                self.scroll.saturating_sub(amount)
            } else {
                self.scroll.saturating_add(amount)
            };
            self.scroll = moved.min(self.max_scroll());
        }
    }

    fn toggle(&mut self) {
        let cursor = self.cursor();
        match self.screen {
            Screen::Watch if self.watch_row(cursor) == WatchRow::Recursive => {
                self.answers.recursive = !self.answers.recursive;
            },
            Screen::Rules => {
                if let Some(rule) = self.answers.rules.get_mut(cursor) {
                    rule.enabled = !rule.enabled;
                }
            },
            Screen::Conflicts => self.answers.on_conflict = CONFLICTS[cursor],
            Screen::Startup => match StartupItem::shown()[cursor] {
                StartupItem::Boot => self.answers.start_on_boot = !self.answers.start_on_boot,
                StartupItem::Updates => {
                    self.answers.check_for_updates = !self.answers.check_for_updates;
                },
            },
            Screen::Welcome | Screen::Watch | Screen::Review => {},
        }
    }

    fn remove(&mut self) {
        if self.screen == Screen::Watch
            && let WatchRow::Path(i) = self.watch_row(self.watch_cursor)
        {
            self.answers.watch_paths.remove(i);
            self.watch_cursor = self
                .watch_cursor
                .min(self.watch_rows() - 1);
        }
    }

    fn enter(&mut self) -> Option<Outcome> {
        let cursor = self.cursor();
        match self.screen {
            Screen::Welcome | Screen::Startup => self.go(1),
            Screen::Watch => match self.watch_row(cursor) {
                WatchRow::Path(i) => {
                    let text = self.answers.watch_paths[i]
                        .to_string_lossy()
                        .into_owned();
                    self.edit = Some(Edit {
                        target: EditTarget::WatchPath(Some(i)),
                        text,
                    });
                },
                WatchRow::Add => {
                    self.edit = Some(Edit {
                        target: EditTarget::WatchPath(None),
                        text: String::new(),
                    });
                },
                WatchRow::Recursive => self.toggle(),
            },
            Screen::Rules => {
                if let Some(rule) = self.answers.rules.get(cursor) {
                    let text = rule
                        .destination
                        .to_string_lossy()
                        .into_owned();
                    self.edit = Some(Edit {
                        target: EditTarget::RuleDestination(cursor),
                        text,
                    });
                }
            },
            Screen::Conflicts => {
                self.toggle();
                self.go(1);
            },
            Screen::Review => {
                if self.existing {
                    self.confirming = true;
                } else {
                    return Some(Outcome::Write(self.answers.clone()));
                }
            },
        }
        None
    }

    fn edit_key(&mut self, key: Key) {
        let Some(edit) = self.edit.as_mut() else { return };
        match key {
            Key::Char(c) => edit.text.push(c),
            Key::Backspace => {
                edit.text.pop();
                if edit.text.is_empty() {
                    self.edit = None;
                }
            },
            Key::Esc => self.edit = None,
            Key::Enter => {
                if let Some(edit) = self.edit.take() {
                    self.commit(&edit);
                }
            },
            _ => {},
        }
    }

    fn commit(&mut self, edit: &Edit) {
        let text = edit.text.trim();
        if text.is_empty() {
            return;
        }
        let path = expand_home(text);
        match edit.target {
            EditTarget::WatchPath(Some(i)) => self.answers.watch_paths[i] = path,
            EditTarget::WatchPath(None) => {
                let at = self
                    .answers
                    .watch_paths
                    .iter()
                    .position(|p| *p == path)
                    .unwrap_or_else(|| {
                        self.answers.watch_paths.push(path);
                        self.answers.watch_paths.len() - 1
                    });
                self.watch_cursor = at;
            },
            EditTarget::RuleDestination(i) => self.answers.rules[i].destination = path,
        }
    }
}

/// `~` and `~/x` mean the home directory, as in a shell.
fn expand_home(text: &str) -> PathBuf {
    let rest = text
        .strip_prefix('~')
        .filter(|rest| rest.is_empty() || rest.starts_with(['/', '\\']));
    match (rest, dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches(['/', '\\'])),
        _ => PathBuf::from(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::answers::test_dirs;

    fn wizard(existing: bool) -> Wizard {
        let mut w =
            Wizard::new(Answers::defaults(&test_dirs()), PathBuf::from("/c/rocas.toml"), existing);
        w.resize(24);
        w
    }

    fn press(w: &mut Wizard, keys: &[Key]) -> Option<Outcome> {
        keys.iter().find_map(|k| w.handle(*k))
    }

    fn typed(text: &str) -> Vec<Key> {
        text.chars().map(Key::Char).collect()
    }

    /// Welcome to Review with Enter/Tab only.
    fn to_review(w: &mut Wizard) {
        assert_eq!(press(w, &[Key::Tab; 5]), None);
        assert_eq!(w.screen, Screen::Review);
    }

    #[test]
    fn default_path_writes_the_default_answers() {
        let mut w = wizard(false);
        to_review(&mut w);
        let outcome = press(&mut w, &[Key::Enter]);
        assert_eq!(outcome, Some(Outcome::Write(Answers::defaults(&test_dirs()))));
    }

    #[test]
    fn enter_walks_forward_through_the_screens() {
        let mut w = wizard(false);
        // Welcome, then Watch needs Tab (Enter edits), Rules likewise.
        press(&mut w, &[Key::Enter, Key::Tab, Key::Tab]);
        assert_eq!(w.screen, Screen::Conflicts);
        press(&mut w, &[Key::Enter]);
        assert_eq!(w.screen, Screen::Startup);
        press(&mut w, &[Key::Enter]);
        assert_eq!(w.screen, Screen::Review);
        assert_eq!(w.answers, Answers::defaults(&test_dirs()));
    }

    #[test]
    fn space_toggles_presets() {
        let mut w = wizard(false);
        press(&mut w, &[Key::Tab, Key::Tab]);
        assert_eq!(w.screen, Screen::Rules);
        press(&mut w, &[Key::Char(' '), Key::Down, Key::Down, Key::Char('j'), Key::Char(' ')]);
        let enabled: Vec<bool> = w
            .answers
            .rules
            .iter()
            .map(|r| r.enabled)
            .collect();
        assert_eq!(enabled, [false, true, true, false, true, true]);
        press(&mut w, &[Key::Char(' ')]);
        assert!(w.answers.rules[3].enabled);
    }

    #[test]
    fn editing_a_destination() {
        let mut w = wizard(false);
        press(&mut w, &[Key::Tab, Key::Tab, Key::Down, Key::Enter]);
        assert_eq!(w.edit.as_ref().map(|e| e.text.as_str()), Some("/home/u/Pictures"));
        // Esc keeps the original; start again and append.
        press(&mut w, &[Key::Esc]);
        assert!(w.edit.is_none());
        assert_eq!(w.answers.rules[1].destination, PathBuf::from("/home/u/Pictures"));

        press(&mut w, &[Key::Enter]);
        press(&mut w, &typed("/x"));
        press(&mut w, &[Key::Enter]);
        assert_eq!(w.answers.rules[1].destination, PathBuf::from("/home/u/Pictures/x"));
    }

    #[test]
    fn q_is_text_while_editing_and_quits_otherwise() {
        let mut w = wizard(false);
        press(&mut w, &[Key::Tab, Key::Tab, Key::Enter]);
        assert_eq!(press(&mut w, &[Key::Char('q')]), None);
        assert!(
            w.edit
                .as_ref()
                .is_some_and(|e| e.text.ends_with('q'))
        );
        press(&mut w, &[Key::Esc]);
        assert_eq!(press(&mut w, &[Key::Char('q')]), Some(Outcome::Quit));
    }

    #[test]
    fn ctrl_c_quits_even_while_editing() {
        let mut w = wizard(false);
        press(&mut w, &[Key::Tab, Key::Tab, Key::Enter]);
        assert_eq!(w.handle(Key::Interrupt), Some(Outcome::Quit));
    }

    #[test]
    fn backspace_on_a_list_goes_back_and_keeps_edits() {
        let mut w = wizard(false);
        press(&mut w, &[Key::Tab, Key::Tab, Key::Char(' ')]);
        assert!(!w.answers.rules[0].enabled);
        press(&mut w, &[Key::Backspace]);
        assert_eq!(w.screen, Screen::Watch);
        press(&mut w, &[Key::BackTab]);
        assert_eq!(w.screen, Screen::Welcome);
        press(&mut w, &[Key::Left]);
        assert_eq!(w.screen, Screen::Welcome);
        press(&mut w, &[Key::Right, Key::Right]);
        assert!(!w.answers.rules[0].enabled);
    }

    #[test]
    fn backspace_in_an_empty_input_cancels_it() {
        let mut w = wizard(false);
        press(&mut w, &[Key::Tab, Key::Down, Key::Enter]);
        press(&mut w, &typed("ab"));
        press(&mut w, &[Key::Backspace, Key::Backspace]);
        assert!(w.edit.is_none());
        assert_eq!(w.screen, Screen::Watch);
    }

    #[test]
    fn watch_folders_add_edit_remove_and_recurse() {
        let mut w = wizard(false);
        press(&mut w, &[Key::Tab]);
        // Down to "Add a folder".
        press(&mut w, &[Key::Down, Key::Enter]);
        press(&mut w, &typed("/data/in"));
        press(&mut w, &[Key::Enter]);
        assert_eq!(
            w.answers.watch_paths,
            [PathBuf::from("/home/u/Downloads"), PathBuf::from("/data/in")]
        );
        assert_eq!(w.cursor(), 1);

        // Adding the same folder again changes nothing.
        press(&mut w, &[Key::Down, Key::Enter]);
        press(&mut w, &typed("/data/in"));
        press(&mut w, &[Key::Enter]);
        assert_eq!(w.answers.watch_paths.len(), 2);

        // Edit the first folder.
        press(&mut w, &[Key::Up, Key::Enter]);
        press(&mut w, &typed("2"));
        press(&mut w, &[Key::Enter]);
        assert_eq!(w.answers.watch_paths[0], PathBuf::from("/home/u/Downloads2"));

        // Remove the second; the cursor stays on a valid row.
        press(&mut w, &[Key::Down, Key::Char('d')]);
        assert_eq!(w.answers.watch_paths, [PathBuf::from("/home/u/Downloads2")]);
        assert_eq!(w.watch_row(w.cursor()), WatchRow::Add);

        press(&mut w, &[Key::Down, Key::Char(' ')]);
        assert!(w.answers.recursive);
        press(&mut w, &[Key::Enter]);
        assert!(!w.answers.recursive);
    }

    #[test]
    fn tilde_expands_to_home() {
        let home = dirs::home_dir().expect("tests run with a home directory");
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("~/Pictures"), home.join("Pictures"));
        assert_eq!(expand_home("~other"), PathBuf::from("~other"));
        assert_eq!(expand_home("/a/~"), PathBuf::from("/a/~"));
    }

    #[test]
    fn conflict_choice_is_selected_with_enter() {
        let mut w = wizard(false);
        press(&mut w, &[Key::Tab, Key::Tab, Key::Tab]);
        assert_eq!(w.screen, Screen::Conflicts);
        press(&mut w, &[Key::Down, Key::Enter]);
        assert_eq!(w.answers.on_conflict, OnConflict::Skip);
        assert_eq!(w.screen, Screen::Startup);
        press(&mut w, &[Key::BackTab, Key::Down, Key::Char(' ')]);
        assert_eq!(w.answers.on_conflict, OnConflict::Overwrite);
    }

    #[test]
    fn startup_toggles() {
        let mut w = wizard(false);
        press(&mut w, &[Key::Tab, Key::Tab, Key::Tab, Key::Tab, Key::Char(' ')]);
        assert_eq!(w.screen, Screen::Startup);
        assert!(w.answers.start_on_boot);
        if cfg!(feature = "self-update") {
            press(&mut w, &[Key::Down, Key::Char(' ')]);
            assert!(!w.answers.check_for_updates);
        }
    }

    #[test]
    fn quitting_writes_nothing() {
        let mut w = wizard(false);
        to_review(&mut w);
        assert_eq!(press(&mut w, &[Key::Char('q')]), Some(Outcome::Quit));
        let mut w = wizard(false);
        assert_eq!(w.handle(Key::Interrupt), Some(Outcome::Quit));
    }

    #[test]
    fn existing_file_needs_a_yes() {
        let mut w = wizard(true);
        to_review(&mut w);
        assert_eq!(press(&mut w, &[Key::Enter]), None);
        assert!(w.confirming);
        assert_eq!(press(&mut w, &[Key::Char('n')]), None);
        assert!(!w.confirming);
        assert_eq!(press(&mut w, &[Key::Enter]), None);
        assert_eq!(press(&mut w, &[Key::Enter]), None, "Enter is not a yes");
        assert!(!w.confirming);
        press(&mut w, &[Key::Enter]);
        assert_eq!(
            press(&mut w, &[Key::Char('y')]),
            Some(Outcome::Write(Answers::defaults(&test_dirs())))
        );
    }

    #[test]
    fn review_scrolls_within_bounds() {
        let mut w = wizard(false);
        w.resize(REVIEW_CHROME + 4);
        to_review(&mut w);
        assert!(w.review.len() > 4);
        press(&mut w, &[Key::Up]);
        assert_eq!(w.scroll, 0);
        press(&mut w, &[Key::Down]);
        assert_eq!(w.scroll, 1);
        press(&mut w, &[Key::PageDown; 100]);
        assert_eq!(w.scroll, w.review.len() - 4);
        press(&mut w, &[Key::PageUp; 100]);
        assert_eq!(w.scroll, 0);
        // Tab does not leave the last screen.
        press(&mut w, &[Key::Tab]);
        assert_eq!(w.screen, Screen::Review);
    }

    #[test]
    fn review_text_follows_the_answers() {
        let mut w = wizard(false);
        press(&mut w, &[Key::Tab, Key::Tab, Key::Char(' ')]);
        to_review_from(&mut w, Screen::Rules);
        assert!(
            !w.review
                .iter()
                .any(|l| l == "# Documents")
        );
        assert!(w.review.iter().any(|l| l == "# Images"));
    }

    fn to_review_from(w: &mut Wizard, from: Screen) {
        assert_eq!(w.screen, from);
        while w.screen != Screen::Review {
            w.handle(Key::Tab);
        }
    }
}
