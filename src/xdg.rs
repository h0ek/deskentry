use std::ffi::OsString;
use std::path::PathBuf;

const DEFAULT_DATA_DIRS: [&str; 2] = ["/usr/local/share", "/usr/share"];

pub fn data_home() -> Option<PathBuf> {
    match nonempty_env("XDG_DATA_HOME") {
        Some(value) => absolute_path(value),
        None => home_dir().map(|home| home.join(".local/share")),
    }
}

pub fn data_dirs() -> Vec<PathBuf> {
    let Some(value) = nonempty_env("XDG_DATA_DIRS") else {
        return DEFAULT_DATA_DIRS.into_iter().map(PathBuf::from).collect();
    };

    std::env::split_paths(&value)
        .filter(|path| path.is_absolute())
        .collect()
}

pub fn user_applications_dir() -> Option<PathBuf> {
    data_home().map(|path| path.join("applications"))
}

pub fn user_hicolor_dir() -> Option<PathBuf> {
    data_home().map(|path| path.join("icons/hicolor"))
}

fn nonempty_env(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

fn absolute_path(value: OsString) -> Option<PathBuf> {
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

fn home_dir() -> Option<PathBuf> {
    nonempty_env("HOME").and_then(absolute_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_data_dirs_are_absolute() {
        assert!(
            DEFAULT_DATA_DIRS
                .iter()
                .all(|path| PathBuf::from(path).is_absolute())
        );
    }

    #[test]
    fn absolute_path_rejects_relative_values() {
        assert_eq!(absolute_path(OsString::from("relative/path")), None);
        assert_eq!(
            absolute_path(OsString::from("/absolute/path")),
            Some(PathBuf::from("/absolute/path"))
        );
    }
}
