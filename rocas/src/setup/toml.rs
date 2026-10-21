//! Turns the answers into the text of `rocas.toml`.
//!
//! Only what differs from the defaults is written, plus the watched folders
//! and the rules, each section with a comment saying what it does.

use std::fmt::Write as _;

use super::answers::Answers;
use crate::config::{MiscConfig, WatcherConfig};

/// Longest line the pattern lists are wrapped to.
const WRAP: usize = 76;

pub fn render(answers: &Answers) -> String {
    let watcher_defaults = WatcherConfig::default();
    let misc_defaults = MiscConfig::default();

    let mut out = String::from(concat!(
        "# rocas configuration, written by `rocas --setup`.\n",
        "# Edit it any time: rocas reloads this file when it changes.\n",
        "# Every option and its default is in the README and in `rocas --help`.\n",
    ));

    let mut watcher = String::new();
    if !answers.watch_paths.is_empty() {
        watcher.push_str("# Folders to watch. New files that appear in them are organized.\n");
        let paths: Vec<String> = answers
            .watch_paths
            .iter()
            .map(|p| quote(&p.to_string_lossy()))
            .collect();
        watcher.push_str(&array("watch_paths", &paths));
    }
    if answers.recursive != watcher_defaults.recursive {
        watcher.push_str("# Also look inside subfolders of the watched folders.\n");
        let _ = writeln!(watcher, "recursive = {}", answers.recursive);
    }
    section(&mut out, "[watcher]", &watcher);

    let mut misc = String::new();
    if answers.on_conflict != misc_defaults.on_conflict {
        misc.push_str(concat!(
            "# What to do when the destination already has a file with that name:\n",
            "# rename keeps both, skip leaves the new file where it is, overwrite replaces.\n",
        ));
        let _ = writeln!(misc, "on_conflict = {}", quote(answers.on_conflict.as_str()));
    }
    if answers.check_for_updates != misc_defaults.check_for_updates {
        misc.push_str("# Look for a newer release when rocas starts.\n");
        let _ = writeln!(misc, "check_for_updates = {}", answers.check_for_updates);
    }
    section(&mut out, "[misc]", &misc);

    let mut rules = answers
        .rules
        .iter()
        .filter(|rule| rule.enabled)
        .peekable();
    if rules.peek().is_some() {
        out.push_str(concat!(
            "\n# Rules: a file whose name matches any pattern is moved to the destination.\n",
            "# The first matching rule wins. Patterns are case-sensitive, so each\n",
            "# extension is listed in lower and upper case.\n",
        ));
    }
    for rule in rules {
        let patterns: Vec<String> = rule
            .patterns
            .iter()
            .map(|p| quote(p))
            .collect();
        let _ = write!(out, "\n[[rules]]\n# {}\n", rule.name);
        out.push_str(&array("patterns", &patterns));
        let _ = writeln!(out, "destination = {}", quote(&rule.destination.to_string_lossy()));
    }
    out
}

fn section(out: &mut String, header: &str, body: &str) {
    if !body.is_empty() {
        let _ = write!(out, "\n{header}\n{body}");
    }
}

/// `key = [..]` on one line when it fits, else one wrapped block.
fn array(key: &str, items: &[String]) -> String {
    let one_line = format!("{key} = [{}]\n", items.join(", "));
    if one_line.len() <= WRAP {
        return one_line;
    }
    let mut out = format!("{key} = [\n");
    let mut line = String::new();
    for item in items {
        if !line.is_empty() && line.len() + item.len() + 3 > WRAP {
            let _ = writeln!(out, "    {line}");
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(item);
        line.push(',');
    }
    let _ = writeln!(out, "    {line}\n]");
    out
}

/// A TOML basic string.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            },
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use forgeconf::FileFormat;

    use super::*;
    use crate::config::{Config, OnConflict};
    use crate::setup::answers::test_dirs;

    fn parse(text: &str) -> Config {
        forgeconf::from_str(text, FileFormat::Toml).expect("rendered config parses")
    }

    #[test]
    fn defaults_round_trip() {
        let answers = Answers::defaults(&test_dirs());
        let text = render(&answers);
        let config = parse(&text);

        assert_eq!(config.watcher.watch_paths, answers.watch_paths);
        assert!(!config.watcher.recursive);
        assert_eq!(config.misc.on_conflict, OnConflict::Rename);
        assert_eq!(config.rules.len(), answers.rules.len());
        for (rule, choice) in config.rules.iter().zip(&answers.rules) {
            assert_eq!(rule.patterns, choice.patterns);
            assert_eq!(rule.destination, choice.destination);
        }
        assert!(!text.contains("[misc]"), "defaults are not written:\n{text}");
    }

    #[test]
    fn changed_answers_round_trip() {
        let mut answers = Answers::defaults(&test_dirs());
        answers.recursive = true;
        answers.on_conflict = OnConflict::Skip;
        answers.check_for_updates = false;
        answers.rules[0].enabled = false;
        answers.rules[1].destination = PathBuf::from("/data/pics");
        answers
            .watch_paths
            .push(PathBuf::from("/home/u/Desktop"));

        let config = parse(&render(&answers));
        assert!(config.watcher.recursive);
        assert_eq!(config.misc.on_conflict, OnConflict::Skip);
        assert!(!config.misc.check_for_updates);
        assert_eq!(config.watcher.watch_paths, answers.watch_paths);
        assert_eq!(config.rules.len(), 5);
        assert_eq!(config.rules[0].destination, PathBuf::from("/data/pics"));
    }

    #[test]
    fn no_rules_and_no_folders_write_no_sections() {
        let mut answers = Answers::defaults(&test_dirs());
        answers.watch_paths.clear();
        answers
            .rules
            .iter_mut()
            .for_each(|r| r.enabled = false);
        let text = render(&answers);
        assert!(!text.contains('['), "{text}");
        let config = parse(&text);
        assert_eq!(config.rules.len(), 0);
        assert_eq!(config.watcher.watch_paths.len(), 0);
    }

    #[test]
    fn windows_paths_and_quotes_are_escaped() {
        let mut answers = Answers::defaults(&test_dirs());
        let tricky = [r"C:\Users\me\Down loads", r#"D:\say "hi"\tab	end"#];
        answers.watch_paths = tricky.map(PathBuf::from).to_vec();
        answers.rules[0].destination = PathBuf::from(r"C:\Users\me\Documents");

        let text = render(&answers);
        assert!(text.contains(r#""C:\\Users\\me\\Down loads""#), "{text}");
        let config = parse(&text);
        assert_eq!(config.watcher.watch_paths, answers.watch_paths);
        assert_eq!(config.rules[0].destination, answers.rules[0].destination);
    }

    #[test]
    fn quote_escapes_control_characters() {
        assert_eq!(quote("a\"b\\c\n\u{1}"), r#""a\"b\\c\n\u0001""#);
    }

    #[test]
    fn long_pattern_lists_wrap() {
        let text = render(&Answers::defaults(&test_dirs()));
        assert!(
            text.lines()
                .all(|line| line.len() <= 100),
            "{text}"
        );
    }
}
