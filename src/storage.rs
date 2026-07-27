use crate::xdg;
use gio_unix::DesktopAppInfo;
use gtk::gio;
use gtk::gio::prelude::*;
use std::collections::HashMap;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_DESKTOP_BYTES: u64 = 1024 * 1024;
const MAX_DESKTOP_BUFFER: usize = 1024 * 1024;
const MAX_ICON_BYTES: u64 = 16 * 1024 * 1024;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

pub struct SaveResult {
    pub path: PathBuf,
    pub warning: Option<String>,
}

pub struct IconImportResult {
    pub name: String,
    pub warning: Option<String>,
}

pub struct LoadedDesktopFile {
    pub path: PathBuf,
    pub suggested_id: String,
    pub document: crate::model::DesktopDocument,
}

struct IconDirectorySpec {
    size: Option<u32>,
    scale: u32,
    context: Option<String>,
    kind: Option<String>,
}

impl Default for IconDirectorySpec {
    fn default() -> Self {
        Self {
            size: None,
            scale: 1,
            context: None,
            kind: None,
        }
    }
}

pub fn validate_contents(contents: &str) -> Result<String, String> {
    if contents.len() > MAX_DESKTOP_BUFFER {
        return Err("Desktop entry is larger than 1 MiB".to_string());
    }

    let temp = temporary_path(&std::env::temp_dir(), "validate");
    atomic_temp_write(&temp, contents.as_bytes()).map_err(|error| error.to_string())?;

    let output = Command::new("desktop-file-validate")
        .arg(&temp)
        .output()
        .map_err(|error| format!("desktop-file-validate failed to start: {error}"));
    let _ = fs::remove_file(&temp);
    let output = output?;

    if output.status.success() {
        return Ok("Desktop entry is valid".to_string());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let message = if stderr.is_empty() { stdout } else { stderr };

    if message.is_empty() {
        Err("desktop-file-validate rejected the entry".to_string())
    } else {
        Err(message)
    }
}

pub fn load_desktop_file(path: &Path) -> Result<LoadedDesktopFile, String> {
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Selected desktop file has an invalid filename".to_string())?;
    let suggested_id = validate_desktop_id(filename)?;
    let file = File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Selected desktop entry is not a regular file".to_string());
    }
    if metadata.len() > MAX_DESKTOP_BYTES {
        return Err("Desktop entry is larger than 1 MiB".to_string());
    }

    let mut bytes = Vec::new();
    file.take(MAX_DESKTOP_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_DESKTOP_BUFFER {
        return Err("Desktop entry is larger than 1 MiB".to_string());
    }
    let contents =
        String::from_utf8(bytes).map_err(|_| "Desktop entry is not valid UTF-8".to_string())?;
    Ok(LoadedDesktopFile {
        path: path.to_path_buf(),
        suggested_id,
        document: crate::model::DesktopDocument::parse(&contents),
    })
}

pub fn save_user_override(id: &str, contents: &str) -> Result<SaveResult, String> {
    let id = validate_desktop_id(id)?;
    validate_contents(contents)?;

    let dir = xdg::user_applications_dir()
        .ok_or_else(|| "Cannot determine the XDG user data directory".to_string())?;
    create_private_dir_all(&dir).map_err(|error| error.to_string())?;
    let target = dir.join(id);

    backup_existing_file(&target)?;
    atomic_write(&target, contents.as_bytes()).map_err(|error| error.to_string())?;

    Ok(SaveResult {
        path: target,
        warning: refresh_desktop_database(&dir),
    })
}

pub fn remove_user_override(path: &Path) -> Result<Option<String>, String> {
    remove_user_entry(path)
}

pub fn delete_user_entry(path: &Path) -> Result<Option<String>, String> {
    remove_user_entry(path)
}

fn remove_user_entry(path: &Path) -> Result<Option<String>, String> {
    let user_dir = xdg::user_applications_dir()
        .ok_or_else(|| "Cannot determine the XDG user data directory".to_string())?;
    validate_user_entry_path(path, &user_dir)?;

    fs::remove_file(path).map_err(|error| error.to_string())?;
    sync_directory(&user_dir).map_err(|error| error.to_string())?;
    Ok(refresh_desktop_database(&user_dir))
}

fn validate_user_entry_path(path: &Path, user_dir: &Path) -> Result<(), String> {
    if path.parent() != Some(user_dir) {
        return Err(
            "Refusing to remove a file outside the user applications directory".to_string(),
        );
    }

    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "User desktop entry has an invalid filename".to_string())?;
    validate_desktop_id(filename)?;
    Ok(())
}

pub fn import_icon(source: &Path) -> Result<IconImportResult, String> {
    let file = File::open(source).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Selected icon is not a regular file".to_string());
    }
    if metadata.len() > MAX_ICON_BYTES {
        return Err("Icon file is larger than 16 MiB".to_string());
    }
    let mut contents = Vec::new();
    file.take(MAX_ICON_BYTES + 1)
        .read_to_end(&mut contents)
        .map_err(|error| error.to_string())?;
    if contents.len() > 16 * 1024 * 1024 {
        return Err("Icon file is larger than 16 MiB".to_string());
    }

    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "Icon file has no extension".to_string())?;
    if extension != "svg" && extension != "png" {
        return Err("Supported icon import formats are SVG and PNG".to_string());
    }

    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Invalid icon filename".to_string())?;
    let base_name = sanitize_icon_name(stem);
    let base = xdg::user_hicolor_dir()
        .ok_or_else(|| "Cannot determine the XDG user data directory".to_string())?;
    let theme = load_hicolor_theme()?;
    let directory = if extension == "svg" {
        theme.scalable_application_directory()?
    } else {
        let (width, height) = png_dimensions(&contents)?;
        theme.png_application_directory(width, height)?
    };
    let target_dir = base.join(directory);

    create_private_dir_all(&target_dir).map_err(|error| error.to_string())?;
    let name = unique_icon_name(&target_dir, &base_name, &extension, &contents)?;
    let target = target_dir.join(format!("{name}.{extension}"));
    atomic_write(&target, &contents).map_err(|error| error.to_string())?;

    Ok(IconImportResult {
        name,
        warning: refresh_icon_cache(&base),
    })
}

pub fn launch_desktop_file(path: &Path) -> Result<(), String> {
    let app = DesktopAppInfo::from_filename(path)
        .ok_or_else(|| "GIO could not load the selected desktop entry".to_string())?;

    app.launch(&[], None::<&gio::AppLaunchContext>)
        .map_err(|error| format!("Failed to launch desktop entry: {error}"))
}

pub fn open_file_location(path: &Path) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or_else(|| "The selected file has no parent directory".to_string())?;
    let file = gio::File::for_path(directory);

    gio::AppInfo::launch_default_for_uri(&file.uri(), None::<&gio::AppLaunchContext>)
        .map_err(|error| format!("Failed to open file location: {error}"))
}

struct HicolorTheme {
    order: Vec<String>,
    specs: HashMap<String, IconDirectorySpec>,
}

impl HicolorTheme {
    fn scalable_application_directory(&self) -> Result<&str, String> {
        self.order
            .iter()
            .find(|name| {
                self.specs.get(*name).is_some_and(|spec| {
                    spec.context.as_deref() == Some("Applications")
                        && spec.kind.as_deref() == Some("Scalable")
                })
            })
            .map(String::as_str)
            .ok_or_else(|| {
                "The installed hicolor theme has no scalable Applications directory".to_string()
            })
    }

    fn png_application_directory(&self, width: u32, height: u32) -> Result<&str, String> {
        if width == 0 || width != height {
            return Err("PNG application icons must be square".to_string());
        }

        self.order
            .iter()
            .find(|name| {
                self.specs.get(*name).is_some_and(|spec| {
                    let Some(size) = spec.size else {
                        return false;
                    };
                    spec.context.as_deref() == Some("Applications")
                        && spec.kind.as_deref() != Some("Scalable")
                        && size.checked_mul(spec.scale) == Some(width)
                })
            })
            .map(String::as_str)
            .ok_or_else(|| {
                format!(
                    "The installed hicolor theme has no Applications directory for a {width}x{height} PNG; use a supported size or SVG"
                )
            })
    }
}

fn load_hicolor_theme() -> Result<HicolorTheme, String> {
    for path in hicolor_index_candidates() {
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        if let Some(theme) = parse_hicolor_index(&contents) {
            return Ok(theme);
        }
    }

    Err("Could not find a valid hicolor index.theme in the XDG icon paths".to_string())
}

fn hicolor_index_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(data_home) = xdg::data_home() {
        paths.push(data_home.join("icons/hicolor/index.theme"));
    }
    paths.extend(
        xdg::data_dirs()
            .into_iter()
            .map(|directory| directory.join("icons/hicolor/index.theme")),
    );
    paths
}

fn parse_hicolor_index(contents: &str) -> Option<HicolorTheme> {
    let mut section = String::new();
    let mut directory_order = Vec::new();
    let mut specs = HashMap::<String, IconDirectorySpec>::new();

    for source_line in contents.lines() {
        let line = source_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
        {
            section = name.to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        if section == "Icon Theme" && (key == "Directories" || key == "ScaledDirectories") {
            for directory in value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if !directory_order.iter().any(|existing| existing == directory) {
                    directory_order.push(directory.to_string());
                }
            }
            continue;
        }

        if directory_order
            .iter()
            .any(|directory| directory == &section)
        {
            let spec = specs.entry(section.clone()).or_default();
            match key {
                "Size" => spec.size = value.trim().parse().ok(),
                "Scale" => {
                    spec.scale = value
                        .trim()
                        .parse()
                        .ok()
                        .filter(|value| *value > 0)
                        .unwrap_or(1);
                }
                "Context" => spec.context = Some(value.trim().to_string()),
                "Type" => spec.kind = Some(value.trim().to_string()),
                _ => {}
            }
        }
    }

    if directory_order.is_empty() {
        None
    } else {
        Some(HicolorTheme {
            order: directory_order,
            specs,
        })
    }
}

fn png_dimensions(contents: &[u8]) -> Result<(u32, u32), String> {
    let Some(header) = contents.get(..24) else {
        return Err("PNG file is too short to contain a valid IHDR chunk".to_string());
    };

    if header[..8] != PNG_SIGNATURE || &header[12..16] != b"IHDR" {
        return Err("Selected .png file does not have a valid PNG header".to_string());
    }
    let chunk_length = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
    if chunk_length != 13 {
        return Err("Selected .png file has an invalid IHDR chunk".to_string());
    }

    let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
    if width == 0 || height == 0 {
        return Err("PNG dimensions must be greater than zero".to_string());
    }
    Ok((width, height))
}

fn unique_icon_name(
    directory: &Path,
    base_name: &str,
    extension: &str,
    contents: &[u8],
) -> Result<String, String> {
    for suffix in 0..10_000_u32 {
        let candidate = if suffix == 0 {
            base_name.to_string()
        } else {
            format!("{base_name}-{suffix}")
        };
        let target = directory.join(format!("{candidate}.{extension}"));
        match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Err(error) => return Err(error.to_string()),
            Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_ICON_BYTES => {
                if fs::read(&target).is_ok_and(|existing| existing == contents) {
                    return Ok(candidate);
                }
            }
            Ok(_) => {}
        }
    }

    Err("Could not allocate a unique icon name".to_string())
}

fn create_private_dir_all(path: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true).mode(0o700).create(path)
}

fn validate_desktop_id(id: &str) -> Result<String, String> {
    let trimmed = id.trim();

    if trimmed != id {
        return Err("Desktop ID must not contain leading or trailing whitespace".to_string());
    }
    if trimmed.is_empty() || !trimmed.ends_with(".desktop") {
        return Err("Desktop ID must end with .desktop".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed == ".desktop" {
        return Err("Desktop ID must be a filename, not a path".to_string());
    }
    if trimmed.chars().any(char::is_control) {
        return Err("Desktop ID must not contain control characters".to_string());
    }

    Ok(trimmed.to_string())
}

fn backup_existing_file(target: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };

    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Refusing to overwrite a non-regular user desktop entry".to_string());
    }

    if metadata.len() > MAX_DESKTOP_BYTES {
        return Err("Refusing to back up a user desktop entry larger than 1 MiB".to_string());
    }

    let file = File::open(target).map_err(|error| error.to_string())?;
    let mut contents = Vec::new();
    file.take(MAX_DESKTOP_BYTES + 1)
        .read_to_end(&mut contents)
        .map_err(|error| error.to_string())?;
    if contents.len() > MAX_DESKTOP_BUFFER {
        return Err("Refusing to back up a user desktop entry larger than 1 MiB".to_string());
    }

    let backup = target.with_extension("desktop.bak");
    atomic_write(&backup, &contents).map_err(|error| error.to_string())
}

fn atomic_write(target: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "target has no parent"))?;
    let temp = temporary_path(parent, "save");

    if let Err(error) = atomic_temp_write(&temp, contents) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    if let Err(error) = fs::set_permissions(&temp, fs::Permissions::from_mode(0o644)) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temp, target) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }

    sync_directory(parent)
}

fn atomic_temp_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn temporary_path(dir: &Path, purpose: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    dir.join(format!(".deskentry-{purpose}-{pid}-{nanos}.desktop"))
}

fn refresh_desktop_database(dir: &Path) -> Option<String> {
    command_warning(
        Command::new("update-desktop-database").arg(dir),
        "desktop MIME cache",
    )
}

fn refresh_icon_cache(dir: &Path) -> Option<String> {
    command_warning(
        Command::new("gtk4-update-icon-cache")
            .args(["-f", "-t", "-i"])
            .arg(dir),
        "GTK 4 icon cache",
    )
}

fn command_warning(command: &mut Command, description: &str) -> Option<String> {
    match command.status() {
        Ok(status) if status.success() => None,
        Ok(status) => Some(format!(
            "Could not refresh {description}: process exited with {status}"
        )),
        Err(error) => Some(format!("Could not refresh {description}: {error}")),
    }
}

fn sanitize_icon_name(input: &str) -> String {
    let mut output = String::from("deskentry-");

    for character in input.chars() {
        if character.is_ascii_alphanumeric()
            || character == '-'
            || character == '_'
            || character == '.'
        {
            output.push(character.to_ascii_lowercase());
        } else {
            output.push('-');
        }
    }

    while output.contains("--") {
        output = output.replace("--", "-");
    }

    output.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!(
            "deskentry-test-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn rejects_oversized_desktop_buffers_before_validation() {
        let oversized = "x".repeat(MAX_DESKTOP_BUFFER + 1);
        assert!(validate_contents(&oversized).is_err());
    }

    #[test]
    fn validates_safe_desktop_ids() {
        assert_eq!(
            validate_desktop_id("org.example.App.desktop"),
            Ok("org.example.App.desktop".to_string())
        );
        assert!(validate_desktop_id(" ../bad.desktop").is_err());
        assert!(validate_desktop_id("../bad.desktop").is_err());
        assert!(validate_desktop_id("bad\n.desktop").is_err());
    }

    #[test]
    fn sanitizes_icon_names() {
        assert_eq!(sanitize_icon_name("My Icon!"), "deskentry-my-icon");
    }

    #[test]
    fn restricts_user_entry_deletion_to_direct_children() {
        let user_dir = Path::new("/home/test/.local/share/applications");
        assert!(
            validate_user_entry_path(
                Path::new("/home/test/.local/share/applications/org.example.App.desktop"),
                user_dir
            )
            .is_ok()
        );
        assert!(
            validate_user_entry_path(
                Path::new("/usr/share/applications/org.example.App.desktop"),
                user_dir
            )
            .is_err()
        );
        assert!(
            validate_user_entry_path(
                Path::new("/home/test/.local/share/applications/subdir/App.desktop"),
                user_dir
            )
            .is_err()
        );
    }

    #[test]
    fn atomic_write_replaces_contents_with_regular_permissions() -> Result<(), String> {
        let dir = temporary_test_dir("atomic-write");
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let target = dir.join("entry.desktop");
        fs::write(&target, b"old").map_err(|error| error.to_string())?;

        atomic_write(&target, b"new").map_err(|error| error.to_string())?;
        let contents = fs::read(&target).map_err(|error| error.to_string())?;
        let mode = fs::metadata(&target)
            .map_err(|error| error.to_string())?
            .permissions()
            .mode()
            & 0o777;
        fs::remove_dir_all(&dir).map_err(|error| error.to_string())?;

        assert_eq!(contents, b"new");
        assert_eq!(mode, 0o644);
        Ok(())
    }

    #[test]
    fn backup_refuses_symbolic_link_targets() -> Result<(), String> {
        let dir = temporary_test_dir("backup-symlink");
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let target = dir.join("target.desktop");
        let link = dir.join("link.desktop");
        fs::write(&target, b"[Desktop Entry]\nName=Test\n").map_err(|error| error.to_string())?;
        std::os::unix::fs::symlink(&target, &link).map_err(|error| error.to_string())?;

        let result = backup_existing_file(&link);
        fs::remove_dir_all(&dir).map_err(|error| error.to_string())?;

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn parses_hicolor_application_directories() -> Result<(), String> {
        let index = "[Icon Theme]\nDirectories=48x48/apps,scalable/apps\nScaledDirectories=48x48@2/apps\n\n[48x48/apps]\nSize=48\nContext=Applications\nType=Fixed\n\n[scalable/apps]\nSize=48\nContext=Applications\nType=Scalable\nMinSize=1\nMaxSize=512\n\n[48x48@2/apps]\nSize=48\nScale=2\nContext=Applications\nType=Fixed\n";
        let theme = parse_hicolor_index(index)
            .ok_or_else(|| "test hicolor index did not parse".to_string())?;
        assert_eq!(theme.scalable_application_directory(), Ok("scalable/apps"));
        assert_eq!(theme.png_application_directory(48, 48), Ok("48x48/apps"));
        assert_eq!(theme.png_application_directory(96, 96), Ok("48x48@2/apps"));
        assert!(theme.png_application_directory(64, 64).is_err());
        assert!(theme.png_application_directory(48, 32).is_err());
        Ok(())
    }
}
