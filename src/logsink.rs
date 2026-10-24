//! stderr → log file. The engine `eprintln!`s probe/init noise; once ratatui
//! owns the screen that would corrupt it, so fd 2 is pointed at
//! `<state dir>/omatach.log` before the terminal is taken.
use std::path::PathBuf;

pub struct Dirs {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    /// `~/.local/share/omatach` — recordings live in `recordings/` under it.
    pub data: PathBuf,
}

/// XDG on Linux (`~/.config/omatach`, `~/.cache/omatach`, `~/.local/state/omatach`);
/// the platform equivalents elsewhere.
pub fn dirs() -> Option<Dirs> {
    let pd = directories::ProjectDirs::from("", "", "omatach")?;
    let state = pd.state_dir().map(|p| p.to_path_buf()).unwrap_or_else(|| pd.data_local_dir().to_path_buf());
    Some(Dirs { config: pd.config_dir().to_path_buf(), cache: pd.cache_dir().to_path_buf(), state, data: pd.data_local_dir().to_path_buf() })
}

/// Redirect fd 2 to `<state>/omatach.log` (append). Returns the log path.
pub fn redirect_stderr(state: &std::path::Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(state)?;
    let path = state.join("omatach.log");
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    use std::os::unix::io::AsRawFd;
    let rc = unsafe { libc::dup2(file.as_raw_fd(), 2) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    std::mem::forget(file); // fd 2 now owns it
    eprintln!("--- omatach start {} ---", chrono_free_timestamp());
    Ok(path)
}

fn chrono_free_timestamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("unix {secs}")
}
