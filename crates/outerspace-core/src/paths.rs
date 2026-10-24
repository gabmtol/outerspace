use std::path::PathBuf;

/// Same as Tauri's app-local-data folder for our bundle identifier, so the uninstaller's "delete
/// app data" option removes exactly this. (`%LOCALAPPDATA%\OuterSpace` is where the per-user
/// installer puts the program itself.)
const DATA_FOLDER: &str = "com.gabmtol.outerspace";
/// Where data lived before the installer existed; its config is carried over once.
const OLD_DATA_FOLDER: &str = "OuterSpace";

/// `%LOCALAPPDATA%\com.gabmtol.outerspace`, created on first use.
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let dir = ensure(base.join(DATA_FOLDER));
    let (old, new) = (base.join(OLD_DATA_FOLDER).join("config.json"), dir.join("config.json"));
    if !new.exists() && old.exists() {
        let _ = std::fs::copy(old, new);
    }
    dir
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
