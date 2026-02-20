pub enum Command {
    Run,
    Setup,
    Unsetup,
    PostUpdate(String), // holds the old exe path
}

impl Command {
    pub fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();

        if let Some(pos) = args
            .iter()
            .position(|a| a == "--post-update")
        {
            let old_exe = args
                .get(pos + 1)
                .cloned()
                .unwrap_or_else(|| {
                    error!("--post-update requires a path argument");
                    std::process::exit(1);
                });
            return Command::PostUpdate(old_exe);
        }

        match args
            .get(1)
            .map(|s| s.as_str())
        {
            Some("--setup") => Command::Setup,
            Some("--unsetup") => Command::Unsetup,
            _ => Command::Run,
        }
    }
}
