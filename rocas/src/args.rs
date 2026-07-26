//! Pre-parsing of the actions that are not settings.
//!
//! `--boot` and `--version` are actions, not configuration: they are taken
//! out of argv before forgeconf sees it, so a file or environment value can
//! never toggle autostart on every launch. A token that is the value of a
//! preceding value-taking flag (`--watcher.watch-path -b`) is left alone.

use std::ffi::OsString;

use forgeconf::{FromNode, Shape};

use crate::config::Config;

/// The result of [`parse`].
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    /// `--boot` / `-b` was given.
    pub boot: bool,
    /// `--version` / `-V` was given.
    pub version: bool,
    /// Everything else, program name first, for the config loader.
    pub rest: Vec<OsString>,
}

/// One command-line flag generated from the config shape.
struct Flag {
    long: String,
    short: Option<char>,
    is_bool: bool,
}

/// Splits the actions out of `argv` (program name first, kept in `rest`).
pub fn parse(argv: Vec<OsString>) -> Args {
    let flags = flags();
    let mut out = Args {
        boot: false,
        version: false,
        rest: Vec::with_capacity(argv.len()),
    };
    let mut tokens = argv.into_iter();
    out.rest.extend(tokens.next());

    while let Some(token) = tokens.next() {
        let consumes = match token.to_str() {
            Some("--boot" | "-b") => {
                out.boot = true;
                continue;
            },
            Some("--version" | "-V") => {
                out.version = true;
                continue;
            },
            Some(text) => takes_value(&flags, text),
            // forgeconf reports a non-UTF-8 argument itself.
            None => false,
        };
        out.rest.push(token);
        if consumes {
            out.rest.extend(tokens.next());
        }
    }
    out
}

/// Whether `token` is a flag followed by a separate value token.
fn takes_value(flags: &[Flag], token: &str) -> bool {
    if let Some(long) = token.strip_prefix("--") {
        if long.contains('=') || long == "help" {
            return false;
        }
        if let Some(flag) = flags
            .iter()
            .find(|flag| flag.long == long)
        {
            return !flag.is_bool;
        }
        let negated_bool = long
            .strip_prefix("no-")
            .is_some_and(|name| {
                flags
                    .iter()
                    .any(|flag| flag.is_bool && flag.long == name)
            });
        return !negated_bool;
    }

    let mut chars = token
        .strip_prefix('-')
        .unwrap_or_default()
        .chars();
    match (chars.next(), chars.next()) {
        (Some('h'), None) => false,
        (Some(short), None) => flags
            .iter()
            .find(|flag| flag.short == Some(short))
            .is_none_or(|flag| !flag.is_bool),
        _ => false,
    }
}

/// Every flag forgeconf generates for [`Config`].
fn flags() -> Vec<Flag> {
    let mut out = Vec::new();
    collect(<Config as FromNode>::shape(), &mut Vec::new(), &mut out);
    out
}

fn collect(shape: Shape, path: &mut Vec<&'static str>, out: &mut Vec<Flag>) {
    let Shape::Struct(info) = shape else { return };
    for field in info.fields() {
        let mut shape = field.shape();
        while let Shape::Optional(inner) = shape {
            shape = inner();
        }
        if field.is_flatten() {
            collect(shape, path, out);
        } else if matches!(shape, Shape::Struct(_)) {
            path.push(field.key());
            collect(shape, path, out);
            path.pop();
        } else if !field.is_skip_cli() {
            let long = field.flag_name().map_or_else(
                || {
                    let mut name = path
                        .iter()
                        .map(|segment| segment.replace('_', "-"))
                        .collect::<Vec<_>>();
                    name.push(field.key().replace('_', "-"));
                    name.join(".")
                },
                str::to_owned,
            );
            out.push(Flag {
                long,
                short: field.short_flag(),
                is_bool: matches!(shape, Shape::Bool),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(tokens: &[&str]) -> (bool, bool, Vec<String>) {
        let args = parse(
            tokens
                .iter()
                .map(OsString::from)
                .collect(),
        );
        let rest = args
            .rest
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        (args.boot, args.version, rest)
    }

    #[test]
    fn long_boot() {
        assert_eq!(parsed(&["rocas", "--boot"]), (true, false, vec!["rocas".into()]));
    }

    #[test]
    fn short_boot() {
        assert_eq!(parsed(&["rocas", "-b"]), (true, false, vec!["rocas".into()]));
    }

    #[test]
    fn long_version() {
        assert_eq!(parsed(&["rocas", "--version"]), (false, true, vec!["rocas".into()]));
    }

    #[test]
    fn short_version() {
        assert_eq!(parsed(&["rocas", "-V"]), (false, true, vec!["rocas".into()]));
    }

    #[test]
    fn no_actions_leaves_argv_untouched() {
        let (boot, version, rest) = parsed(&["rocas", "--misc.log-level", "debug"]);
        assert_eq!((boot, version), (false, false));
        assert_eq!(rest, ["rocas", "--misc.log-level", "debug"]);
    }

    #[test]
    fn value_of_a_value_flag_is_kept() {
        let (boot, _, rest) = parsed(&["rocas", "--watcher.watch-path", "-b"]);
        assert!(!boot);
        assert_eq!(rest, ["rocas", "--watcher.watch-path", "-b"]);
    }

    #[test]
    fn version_as_a_value_is_kept() {
        let (_, version, rest) = parsed(&["rocas", "--misc.log-file", "--version"]);
        assert!(!version);
        assert_eq!(rest, ["rocas", "--misc.log-file", "--version"]);
    }

    #[test]
    fn bool_flag_does_not_consume_boot() {
        let (boot, _, rest) = parsed(&["rocas", "--watcher.recursive", "-b"]);
        assert!(boot);
        assert_eq!(rest, ["rocas", "--watcher.recursive"]);
    }

    #[test]
    fn negated_bool_flag_does_not_consume_boot() {
        let (boot, _, rest) = parsed(&["rocas", "--no-watcher.recursive", "-b"]);
        assert!(boot);
        assert_eq!(rest, ["rocas", "--no-watcher.recursive"]);
    }

    #[test]
    fn inline_value_does_not_consume_boot() {
        let (boot, _, rest) = parsed(&["rocas", "--watcher.watch-path=x", "-b"]);
        assert!(boot);
        assert_eq!(rest, ["rocas", "--watcher.watch-path=x"]);
    }

    #[test]
    fn list_flag_consumes_one_value() {
        let (boot, _, rest) = parsed(&["rocas", "--watcher.watch-paths", "a", "-b"]);
        assert!(boot);
        assert_eq!(rest, ["rocas", "--watcher.watch-paths", "a"]);
    }

    #[test]
    fn order_is_preserved_around_actions() {
        let (boot, version, rest) = parsed(&[
            "/usr/bin/rocas",
            "--misc.log-level",
            "debug",
            "-b",
            "--watcher.recursive",
            "-V",
            "--watcher.max-depth",
            "2",
        ]);
        assert!(boot && version);
        assert_eq!(
            rest,
            [
                "/usr/bin/rocas",
                "--misc.log-level",
                "debug",
                "--watcher.recursive",
                "--watcher.max-depth",
                "2"
            ]
        );
    }

    #[test]
    fn empty_argv_stays_empty() {
        assert_eq!(parsed(&[]), (false, false, vec![]));
    }

    #[test]
    fn help_does_not_consume() {
        let (boot, _, rest) = parsed(&["rocas", "--help", "-b"]);
        assert!(boot);
        assert_eq!(rest, ["rocas", "--help"]);
    }
}
