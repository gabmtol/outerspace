use std::path::PathBuf;

/// `%LOCALAPPDATA%\OuterSpace`, created on first use.
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    ensure(base.join("OuterSpace"))
}

pub fn logs_dir() -> PathBuf {
    ensure(data_dir().join("logs"))
}

pub fn driver_dir() -> PathBuf {
    ensure(data_dir().join("driver"))
}

fn ensure(dir: PathBuf) -> PathBuf {
    if let Err(err) = std::fs::create_dir_all(&dir) {
        tracing::warn!(?dir, %err, "could not create directory");
    }
    dir
}
