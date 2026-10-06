//! Locating the bundled ffmpeg / ffprobe / yt-dlp binaries.
//!
//! In a release build Tauri places sidecars next to the app executable
//! (`FillernCut.app/Contents/MacOS/` on macOS, the install folder on Windows). During development we fall back to
//! whatever is on PATH or in Homebrew, because GUI apps launched from Finder
//! don't inherit the shell's PATH.

use std::fs;
use std::path::{Path, PathBuf};

#[cfg(not(windows))]
const FALLBACK_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];
#[cfg(windows)]
const FALLBACK_DIRS: &[&str] = &[];

/// Program names carry `.exe` on Windows.
fn exe_name(name: &str) -> String {
    if cfg!(windows) && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(FALLBACK_DIRS.iter().map(PathBuf::from))
        .map(|d| d.join(name))
        .find(|p| is_runnable(p))
}

/// A regular, non-empty file we're allowed to execute. (Guards against
/// placeholder or half-downloaded binaries shadowing a working one on PATH.)
fn is_runnable(p: &Path) -> bool {
    let Ok(meta) = fs::metadata(p) else { return false };
    if !meta.is_file() || meta.len() == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

/// Path of `name`: bundled copy first, then PATH, else the bare name (which
/// produces a clear "couldn't start" error when spawned).
pub fn locate(name: &str) -> PathBuf {
    let name = &exe_name(name);
    if let Some(dir) = exe_dir() {
        let p = dir.join(name);
        if is_runnable(&p) {
            return p;
        }
    }
    find_on_path(name).unwrap_or_else(|| PathBuf::from(name))
}

pub fn ffmpeg() -> PathBuf {
    locate("ffmpeg")
}

pub fn ffprobe() -> PathBuf {
    locate("ffprobe")
}

/// Directory containing ffmpeg, for yt-dlp's `--ffmpeg-location`.
pub fn ffmpeg_dir() -> Option<PathBuf> {
    let p = ffmpeg();
    p.is_absolute()
        .then(|| p.parent().map(Path::to_path_buf))
        .flatten()
}

/// The app bundle is read-only (and code-signed), so yt-dlp can't update
/// itself in place there. We keep a writable copy in the data directory,
/// seeded from the bundled binary, and update that one.
pub fn managed_ytdlp(data_dir: &Path) -> PathBuf {
    data_dir.join("bin").join(exe_name("yt-dlp"))
}

pub fn ytdlp(data_dir: &Path) -> PathBuf {
    let managed = managed_ytdlp(data_dir);
    if managed.is_file() {
        return managed;
    }
    let bundled = locate("yt-dlp");
    if bundled.is_absolute() && is_runnable(&bundled) && seed_copy(&bundled, &managed).is_ok() {
        return managed;
    }
    bundled
}

fn seed_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(to, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// True when `ytdlp(data_dir)` is the writable managed copy (i.e. `-U` is safe).
pub fn ytdlp_is_managed(data_dir: &Path) -> bool {
    managed_ytdlp(data_dir).is_file()
}

/// Windows pops up a console window for every program a GUI app starts, unless asked not to.
pub fn hide_window(cmd: &mut tokio::process::Command) -> &mut tokio::process::Command {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_names_get_exe_only_on_windows() {
        assert_eq!(
            exe_name("ffmpeg"),
            if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }
        );
        assert_eq!(exe_name("yt-dlp.exe"), "yt-dlp.exe");
        assert!(managed_ytdlp(Path::new("/d")).ends_with(exe_name("yt-dlp")));
    }
}
