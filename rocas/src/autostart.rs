pub fn install() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    return windows::install();

    #[cfg(target_os = "macos")]
    return macos::install();

    #[cfg(target_os = "linux")]
    return linux::install();
}

pub fn uninstall() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    return windows::uninstall();

    #[cfg(target_os = "macos")]
    return macos::uninstall();

    #[cfg(target_os = "linux")]
    return linux::uninstall();
}

#[cfg(target_os = "windows")]
mod windows {
    use std::process::Command;

    const APP_NAME: &str = "Rocas";

    pub fn install() -> Result<(), Box<dyn std::error::Error>> {
        let exe = std::env::current_exe()?;
        let exe_str = exe
            .to_str()
            .ok_or("invalid exe path")?;

        // Add to HKCU\Software\Microsoft\Windows\CurrentVersion\Run
        // This starts rocas for the current user on login
        let status = Command::new("reg")
            .args([
                "add",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                APP_NAME,
                "/t",
                "REG_SZ",
                "/d",
                exe_str,
                "/f", // overwrite if exists
            ])
            .status()?;

        if !status.success() {
            return Err("reg add failed".into());
        }

        Ok(())
    }

    pub fn uninstall() -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                APP_NAME,
                "/f",
            ])
            .status()?;

        if !status.success() {
            return Err("reg delete failed".into());
        }

        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::fs;
    use std::path::PathBuf;

    const PLIST_NAME: &str = "com.rocas.agent.plist";

    fn plist_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(PLIST_NAME))
    }

    pub fn install() -> Result<(), Box<dyn std::error::Error>> {
        let exe = std::env::current_exe()?;
        let exe_str = exe
            .to_str()
            .ok_or("invalid exe path")?;
        let plist = plist_path()?;

        fs::create_dir_all(
            plist
                .parent()
                .unwrap(),
        )?;

        // LaunchAgent runs as the current user on login
        let contents = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.rocas.agent</string>

    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>/tmp/rocas.log</string>

    <key>StandardErrorPath</key>
    <string>/tmp/rocas.err</string>
</dict>
</plist>"#,
            exe_str
        );

        fs::write(&plist, contents)?;

        // Load it immediately without needing a reboot
        std::process::Command::new("launchctl")
            .args([
                "load",
                plist
                    .to_str()
                    .unwrap(),
            ])
            .status()?;

        Ok(())
    }

    pub fn uninstall() -> Result<(), Box<dyn std::error::Error>> {
        let plist = plist_path()?;

        // Unload first
        std::process::Command::new("launchctl")
            .args([
                "unload",
                plist
                    .to_str()
                    .unwrap(),
            ])
            .status()?;

        fs::remove_file(&plist)?;

        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    const SERVICE_NAME: &str = "rocas.service";

    fn service_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(home)
            .join(".config/systemd/user")
            .join(SERVICE_NAME))
    }

    pub fn install() -> Result<(), Box<dyn std::error::Error>> {
        let exe = std::env::current_exe()?;
        let exe_str = exe
            .to_str()
            .ok_or("invalid exe path")?;
        let service = service_path()?;

        fs::create_dir_all(
            service
                .parent()
                .unwrap(),
        )?;

        let contents = format!(
            r#"[Unit]
Description=Rocas file watcher
After=network.target

[Service]
ExecStart={}
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=default.target
"#,
            exe_str
        );

        fs::write(&service, contents)?;

        // Reload systemd and enable + start the service
        Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status()?;

        Command::new("systemctl")
            .args(["--user", "enable", "--now", SERVICE_NAME])
            .status()?;

        // Ensure the user's systemd services run even without an active login session
        // (requires sudo, so we just print a hint if this fails)
        let linger = Command::new("loginctl")
            .args(["enable-linger", &std::env::var("USER").unwrap_or_default()])
            .status();

        if linger.is_err() {
            warn!(
                "Tip: run 'loginctl enable-linger $USER' as root to keep rocas running after \
                 logout."
            );
        }

        Ok(())
    }

    pub fn uninstall() -> Result<(), Box<dyn std::error::Error>> {
        let service = service_path()?;

        Command::new("systemctl")
            .args(["--user", "disable", "--now", SERVICE_NAME])
            .status()?;

        fs::remove_file(&service)?;

        Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status()?;

        Ok(())
    }
}
