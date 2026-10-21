//! First-run setup wizard: `rocas --setup`, offered once when there is no
//! config file yet.

mod answers;
mod console;
mod render;
mod term;
mod toml;
mod wizard;

use std::io::{self, IsTerminal, Write};
use std::path::Path;

use answers::{Answers, Dirs};
use wizard::{Outcome, Wizard};

use crate::args::Args;
use crate::{AppError, config};

/// What the caller does after [`dispatch`].
pub enum Flow {
    /// Go on and start rocas.
    Continue,
    /// The run was only for setup.
    Exit,
}

/// Runs the wizard when asked for (`--setup`) or when a plain interactive
/// start finds no config file.
pub fn dispatch(args: &Args) -> Result<Flow, AppError> {
    if args.setup {
        let opened_console = console::attach();
        let result = run(false);
        if opened_console {
            if let Err(error) = &result {
                eprintln!("rocas: {error}");
            }
            println!("Press Enter to close this window.");
            let _ = io::stdin().read_line(&mut String::new());
        }
        return result.map(|()| Flow::Exit);
    }

    // Only a person typing `rocas` gets asked: not systemd, autostart, pipes,
    // or a run with any option.
    let bare = args.rest.len() == 1 && !args.boot;
    let path = config::config_path();
    if bare && !path.exists() && interactive() {
        if offer(&path)? {
            run(true)?;
        } else {
            println!("Run 'rocas --setup' any time.");
        }
    }
    Ok(Flow::Continue)
}

fn interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn offer(path: &Path) -> io::Result<bool> {
    print!("No configuration found at {}. Run setup now? [Y/n] ", path.display());
    io::stdout().flush()?;
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer)? == 0 {
        return Ok(false);
    }
    Ok(matches!(answer.trim().to_lowercase().as_str(), "" | "y" | "yes"))
}

/// The wizard, then the file it asked for. `starting` means rocas goes on to
/// run right after, so there is no need to say how to start it.
fn run(starting: bool) -> Result<(), AppError> {
    if !interactive() {
        return Err(io::Error::other("setup needs an interactive terminal").into());
    }
    let path = config::config_path();
    let wizard = Wizard::new(Answers::defaults(&Dirs::detect()), path.clone(), path.exists());
    match term::run(wizard)? {
        Outcome::Quit => println!("Nothing was written. Run 'rocas --setup' any time."),
        Outcome::Write(answers) => {
            write_atomically(&path, &toml::render(&answers))?;
            println!("Wrote {}", path.display());
            if answers.start_on_boot {
                match config::ensure_startup_enabled() {
                    Ok(true) => println!("Rocas will start when you log in."),
                    Ok(false) => println!("Rocas already starts when you log in."),
                    Err(error) => eprintln!("Could not turn on starting at login: {error}"),
                }
            }
            if !starting {
                println!("Start rocas with: rocas");
            }
        },
    }
    Ok(())
}

/// Writes next to the target and renames over it, so a reader (or a running
/// rocas reloading it) never sees half a file.
fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("toml.tmp");
    let write = || -> io::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    };
    write().inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_new_file_and_replaces_an_old_one() {
        let dir = std::env::temp_dir().join(format!("rocas-setup-{}", std::process::id()));
        let path = dir.join("nested").join("rocas.toml");

        write_atomically(&path, "first\n").expect("creates the parent directories");
        assert_eq!(std::fs::read_to_string(&path).expect("file exists"), "first\n");
        write_atomically(&path, "second\n").expect("replaces the file");
        assert_eq!(std::fs::read_to_string(&path).expect("file exists"), "second\n");
        assert!(!path.with_extension("toml.tmp").exists());

        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
