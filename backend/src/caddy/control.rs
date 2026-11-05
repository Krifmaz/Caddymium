//! Forcing Caddy down when it does not answer.
//!
//! Asking Caddy to stop goes through its admin API, which a hung process
//! never answers. These controls reach the process itself: in the
//! all-in-one image the panel's supervisor owns the child process; in the
//! two-container layout the caddy container's entrypoint watches a request
//! file in the bin volume both containers share. Either way the process is
//! killed and its supervisor starts Caddy again with the binary it picks.

use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use tokio::sync::watch;

/// Kills the running Caddy process so its supervisor starts a new one.
pub trait Control: Send + Sync {
    /// A number that changes each time a new Caddy process starts, or None
    /// when this layout cannot tell.
    fn generation(&self) -> Option<u64>;
    /// Kills the running Caddy process (not gracefully).
    fn kill(&self) -> Result<()>;
}

/// No way to reach the process (unknown layout): only the admin API works.
pub struct NoControl;

impl Control for NoControl {
    fn generation(&self) -> Option<u64> {
        None
    }
    fn kill(&self) -> Result<()> {
        Err(anyhow!(
            "the panel cannot stop the Caddy process in this layout; restart the Caddy container"
        ))
    }
}

/// All-in-one: shared between the supervisor (which owns the child) and
/// everything that may need to kill it.
#[derive(Debug)]
pub struct ProcessHandle {
    started: watch::Sender<u64>,
    kill: watch::Sender<u64>,
}

impl Default for ProcessHandle {
    fn default() -> Self {
        ProcessHandle {
            started: watch::channel(0).0,
            kill: watch::channel(0).0,
        }
    }
}

impl ProcessHandle {
    /// Called by the supervisor right after it spawned a process; returns
    /// that process's generation.
    pub fn started(&self) -> u64 {
        let mut g = 0;
        self.started.send_modify(|v| {
            *v += 1;
            g = *v;
        });
        g
    }

    /// The supervisor waits on this: it changes to the generation that must
    /// be killed.
    pub fn kill_requests(&self) -> watch::Receiver<u64> {
        self.kill.subscribe()
    }
}

impl Control for ProcessHandle {
    fn generation(&self) -> Option<u64> {
        Some(*self.started.borrow())
    }
    fn kill(&self) -> Result<()> {
        let g = *self.started.borrow();
        self.kill.send_replace(g);
        Ok(())
    }
}

/// Two-container layout: the entrypoint writes `caddy.gen` (a counter) each
/// time it starts Caddy and kills the process when `caddy.kill` appears.
#[derive(Debug, Clone)]
pub struct FileControl {
    pub bin_dir: PathBuf,
}

/// File the entrypoint writes the process generation to.
pub const GEN_FILE: &str = "caddy.gen";
/// File the panel creates to have the entrypoint kill Caddy.
pub const KILL_FILE: &str = "caddy.kill";

impl Control for FileControl {
    fn generation(&self) -> Option<u64> {
        std::fs::read_to_string(self.bin_dir.join(GEN_FILE))
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }
    fn kill(&self) -> Result<()> {
        if self.generation().is_none() {
            return Err(anyhow!(
                "the Caddy container does not support being stopped by the panel (update the caddymium-caddy image), so it was not stopped; restart the Caddy container"
            ));
        }
        let p = self.bin_dir.join(KILL_FILE);
        std::fs::write(&p, b"kill\n").with_context(|| format!("write {}", p.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_handle_kills_the_current_generation() {
        let h = ProcessHandle::default();
        let mut rx = h.kill_requests();
        assert_eq!(h.generation(), Some(0));
        assert_eq!(h.started(), 1);
        h.kill().unwrap();
        assert!(rx.has_changed().unwrap());
        assert_eq!(*rx.borrow_and_update(), 1);
        assert_eq!(h.started(), 2);
        assert_eq!(h.generation(), Some(2));
    }

    #[test]
    fn file_control_needs_a_supporting_entrypoint() {
        let dir = tempfile::tempdir().unwrap();
        let c = FileControl {
            bin_dir: dir.path().to_path_buf(),
        };
        assert_eq!(c.generation(), None);
        let e = c.kill().unwrap_err().to_string();
        assert!(e.contains("restart the Caddy container"), "{e}");
        std::fs::write(dir.path().join(GEN_FILE), "7\n").unwrap();
        assert_eq!(c.generation(), Some(7));
        c.kill().unwrap();
        assert!(dir.path().join(KILL_FILE).exists());
    }
}
