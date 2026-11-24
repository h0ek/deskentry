use gtk::glib;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

pub const RELEASES_URL: &str = "https://github.com/h0ek/deskentry/releases/latest";
const TAG_PREFIX: &str = "https://github.com/h0ek/deskentry/releases/tag/v";
static STARTUP_CHECKED: AtomicBool = AtomicBool::new(false);

pub struct Release {
    pub version: String,
    pub newer: bool,
}

fn preference_path() -> PathBuf {
    glib::user_config_dir().join("deskentry/check-updates-on-startup")
}

pub fn enabled() -> bool {
    fs::symlink_metadata(preference_path()).is_ok_and(|metadata| metadata.is_dir())
}

pub fn set_enabled(value: bool) -> io::Result<()> {
    let path = preference_path();
    if value {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        match fs::create_dir(&path) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && enabled() => Ok(()),
            result => result,
        }
    } else {
        match fs::remove_dir(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

pub fn should_check_at_startup() -> bool {
    !STARTUP_CHECKED.swap(true, Ordering::Relaxed) && enabled()
}

fn parse_version(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn release_from_redirect(redirect: &str, current: &str) -> Option<Release> {
    let version = redirect.trim().strip_prefix(TAG_PREFIX)?;
    if version.is_empty()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return None;
    }
    let newer = parse_version(version)? > parse_version(current)?;
    Some(Release {
        version: version.to_string(),
        newer,
    })
}

pub fn check() -> Option<Release> {
    let output = Command::new("curl")
        .args([
            "--disable",
            "--silent",
            "--fail",
            "--head",
            "--proto",
            "=https",
            "--connect-timeout",
            "3",
            "--max-time",
            "8",
            "--output",
            "/dev/null",
            "--write-out",
            "%{redirect_url}",
            RELEASES_URL,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    release_from_redirect(
        std::str::from_utf8(&output.stdout).ok()?,
        env!("CARGO_PKG_VERSION"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_release_versions_numerically() {
        let release = release_from_redirect(&format!("{TAG_PREFIX}1.10.0"), "1.9.9");
        assert!(release.is_some_and(|release| release.newer && release.version == "1.10.0"));
        let release = release_from_redirect(&format!("{TAG_PREFIX}1.0.1"), "1.0.1");
        assert!(release.is_some_and(|release| !release.newer));
        let release = release_from_redirect(&format!("{TAG_PREFIX}1.0.0"), "1.0.1");
        assert!(release.is_some_and(|release| !release.newer));
    }

    #[test]
    fn rejects_unexpected_release_redirects() {
        assert!(
            release_from_redirect("https://example.org/releases/tag/v9.0.0", "1.0.1").is_none()
        );
        assert!(release_from_redirect(&format!("{TAG_PREFIX}2.0.0-rc.1"), "1.0.1").is_none());
        assert!(release_from_redirect(&format!("{TAG_PREFIX}2.0.0/other"), "1.0.1").is_none());
        assert!(release_from_redirect(&format!("{TAG_PREFIX}2.0"), "1.0.1").is_none());
        assert!(release_from_redirect("", "1.0.1").is_none());
    }
}
