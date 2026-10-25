//! `burrowd install|uninstall|status`: run the daemon at login so agents keep working
//! without the app. launchd on macOS, a systemd user unit on Linux, a logon task on
//! Windows (a Windows Service would run as another user and miss the user's keychain).
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const LABEL: &str = "dev.burrow.daemon";

fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("running {cmd:?}"))?;
    if !out.status.success() {
        bail!("{cmd:?} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The service sees a minimal PATH; keep the installer's so sandbox runtimes are found.
pub fn launchd_plist(exe: &Path, log: &Path, path_env: &str) -> String {
    format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>{}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>EnvironmentVariables</key><dict><key>PATH</key><string>{}</string></dict>
  <key>StandardErrorPath</key><string>{}</string>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
"#, xml_escape(&exe.to_string_lossy()), xml_escape(path_env), xml_escape(&log.to_string_lossy()))
}

pub fn systemd_unit(exe: &Path, path_env: &str) -> String {
    // systemd needs quotes around paths with spaces
    format!("[Unit]\nDescription=Burrow agent daemon\n\n[Service]\nExecStart=\"{}\"\nEnvironment=\"PATH={}\"\n\
             Restart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n", exe.display(), path_env)
}

fn home() -> Result<PathBuf> {
    dirs::home_dir().context("no home dir")
}

pub fn install(burrow_home: &Path) -> Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let path_env = std::env::var("PATH").unwrap_or_default();
    if cfg!(target_os = "macos") {
        let plist = home()?.join("Library/LaunchAgents").join(format!("{LABEL}.plist"));
        std::fs::create_dir_all(plist.parent().unwrap())?;
        let _ = uninstall(); // replace an older install
        std::fs::write(&plist, launchd_plist(&exe, &burrow_home.join("burrowd.log"), &path_env))?;
        let uid = String::from_utf8(Command::new("id").arg("-u").output()?.stdout)?.trim().to_string();
        run(Command::new("launchctl").args(["bootstrap", &format!("gui/{uid}")]).arg(&plist))?;
    } else if cfg!(target_os = "linux") {
        let unit = home()?.join(".config/systemd/user/burrowd.service");
        std::fs::create_dir_all(unit.parent().unwrap())?;
        std::fs::write(&unit, systemd_unit(&exe, &path_env))?;
        run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        run(Command::new("systemctl").args(["--user", "enable", "--now", "burrowd.service"]))?;
    } else if cfg!(windows) {
        let tr = format!("\"{}\"", exe.display());
        run(Command::new("schtasks").args(["/Create", "/F", "/TN", r"Burrow\burrowd", "/SC", "ONLOGON", "/RL", "LIMITED", "/TR", &tr]))?;
        run(Command::new("schtasks").args(["/Run", "/TN", r"Burrow\burrowd"]))?;
    } else {
        bail!("service install isn't supported on this OS");
    }
    Ok(())
}

pub fn uninstall() -> Result<()> {
    if cfg!(target_os = "macos") {
        let plist = home()?.join("Library/LaunchAgents").join(format!("{LABEL}.plist"));
        let uid = String::from_utf8(Command::new("id").arg("-u").output()?.stdout)?.trim().to_string();
        let _ = Command::new("launchctl").args(["bootout", &format!("gui/{uid}/{LABEL}")]).output();
        let _ = std::fs::remove_file(plist);
    } else if cfg!(target_os = "linux") {
        let _ = Command::new("systemctl").args(["--user", "disable", "--now", "burrowd.service"]).output();
        let _ = std::fs::remove_file(home()?.join(".config/systemd/user/burrowd.service"));
        let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).output();
    } else if cfg!(windows) {
        let _ = Command::new("schtasks").args(["/End", "/TN", r"Burrow\burrowd"]).output();
        let _ = Command::new("schtasks").args(["/Delete", "/F", "/TN", r"Burrow\burrowd"]).output();
    }
    Ok(())
}

pub fn installed() -> Result<bool> {
    Ok(if cfg!(target_os = "macos") {
        home()?.join("Library/LaunchAgents").join(format!("{LABEL}.plist")).exists()
    } else if cfg!(target_os = "linux") {
        home()?.join(".config/systemd/user/burrowd.service").exists()
    } else {
        Command::new("schtasks").args(["/Query", "/TN", r"Burrow\burrowd"]).output().map(|o| o.status.success()).unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_files() {
        let p = launchd_plist(Path::new("/Applications/Burrow & Co.app/burrowd"), Path::new("/tmp/l.log"), "/usr/bin:/opt/homebrew/bin");
        assert!(p.contains("<string>/Applications/Burrow &amp; Co.app/burrowd</string>"));
        assert!(p.contains("/opt/homebrew/bin"));
        let u = systemd_unit(Path::new("/opt/Burrow App/burrowd"), "/usr/bin");
        assert!(u.contains("ExecStart=\"/opt/Burrow App/burrowd\""));
    }
}
