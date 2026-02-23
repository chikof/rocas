use self_update::cargo_crate_version;

pub enum Command {
    Run,
    Setup,
    Unsetup,
}

impl Command {
    pub fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();

        match args
            .get(1)
            .map(|s| s.as_str())
        {
            Some("--setup") => Command::Setup,
            Some("--unsetup") => Command::Unsetup,
            Some("--version") => {
                println!("Rocas version {}", cargo_crate_version!());
                std::process::exit(0);
            },
            _ => Command::Run,
        }
    }
}
