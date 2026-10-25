//! Keep the machine awake while an agent runs on AC power (Night Shift §6.4). On battery we let
//! it sleep: a paused run is better than a flat battery.
use crate::agent::State;
use std::sync::Arc;
use std::time::Duration;

fn on_ac() -> bool {
    let Ok(m) = starship_battery::Manager::new() else { return true };
    let Ok(bs) = m.batteries() else { return true };
    // no battery (desktop) counts as AC
    !bs.flatten().any(|b| b.state() == starship_battery::State::Discharging)
}

/// The OS sleep assertion, released on drop.
#[cfg(not(windows))]
struct Hold(std::process::Child);

#[cfg(not(windows))]
impl Hold {
    fn take() -> Option<Hold> {
        let pid = std::process::id().to_string();
        // both helpers exit with the daemon, so a crash can't leave the machine awake forever
        let mut cmd = if cfg!(target_os = "macos") {
            let mut c = std::process::Command::new("caffeinate");
            c.args(["-i", "-w", &pid]);
            c
        } else {
            let mut c = std::process::Command::new("systemd-inhibit");
            c.args(["--what=idle:sleep", "--who=Burrow", "--why=An agent is working", "--mode=block",
                "tail", &format!("--pid={pid}"), "-f", "/dev/null"]);
            c
        };
        cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        cmd.spawn().map(Hold).map_err(|e| eprintln!("keep-awake unavailable: {e}")).ok()
    }
}

#[cfg(not(windows))]
impl Drop for Hold {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// SetThreadExecutionState is per thread, so a parked thread holds it until told to stop.
#[cfg(windows)]
struct Hold(#[allow(dead_code)] std::sync::mpsc::Sender<()>);

#[cfg(windows)]
impl Hold {
    fn take() -> Option<Hold> {
        use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
            let _ = rx.recv(); // returns when the Hold is dropped
            SetThreadExecutionState(ES_CONTINUOUS);
        });
        Some(Hold(tx))
    }
}

pub async fn run(st: Arc<State>) {
    let mut hold: Option<Hold> = None;
    loop {
        let want = !st.busy().is_empty() && tokio::task::spawn_blocking(on_ac).await.unwrap_or(false);
        if want && hold.is_none() {
            hold = Hold::take();
        } else if !want {
            hold = None;
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}
