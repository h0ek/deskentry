use crate::fileio::open_regular;
use crate::{scanner, xdg};
use gio_unix::DesktopAppInfo;
use gtk::gdk_pixbuf::prelude::*;
use gtk::{gdk_pixbuf, gio};
use std::collections::HashMap;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_DESKTOP_BYTES: u64 = 1024 * 1024;
const MAX_DESKTOP_BUFFER: usize = 1024 * 1024;
const MAX_ICON_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ICON_DIMENSION: u32 = 4096;
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

    if crate::model::DesktopDocument::parse(contents).get_main("Type") == Some("Directory") {
        return Err("Directory entries require .directory files; DeskEntry currently saves .desktop files only".to_string());
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
    let file = open_regular(path, false).map_err(|error| error.to_string())?;
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

pub fn save_user_override(
    id: &str,
    contents: &str,
    existing_user_path: Option<&Path>,
) -> Result<SaveResult, String> {
    let id = validate_desktop_id(id)?;
    validate_contents(contents)?;

    let dir = xdg::user_applications_dir()
        .ok_or_else(|| "Cannot determine the XDG user data directory".to_string())?;
    create_private_dir_all(&dir).map_err(|error| error.to_string())?;
    let target = write_user_entry(&dir, &id, contents.as_bytes(), existing_user_path)?;

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
    remove_user_entry_in(path, &user_dir)?;
    Ok(refresh_desktop_database(&user_dir))
}

fn remove_user_entry_in(path: &Path, user_dir: &Path) -> Result<(), String> {
    validate_user_entry_path(path, user_dir)?;
    fs::remove_file(path).map_err(|error| error.to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "Entry has no parent".to_string())?;
    sync_directory(parent).map_err(|error| error.to_string())
}

fn write_user_entry(
    user_dir: &Path,
    id: &str,
    contents: &[u8],
    existing_user_path: Option<&Path>,
) -> Result<PathBuf, String> {
    let target = user_save_target(user_dir, id, existing_user_path)?;
    if existing_user_path.is_some() {
        backup_existing_file(&target)?;
        atomic_write(&target, contents).map_err(|error| error.to_string())?;
    } else {
        atomic_create(&target, contents).map_err(|error| error.to_string())?;
    }
    Ok(target)
}

fn user_save_target(
    user_dir: &Path,
    id: &str,
    existing_user_path: Option<&Path>,
) -> Result<PathBuf, String> {
    let id = validate_desktop_id(id)?;
    if let Some(path) = existing_user_path {
        validate_user_entry_path(path, user_dir)?;
        let relative = path
            .strip_prefix(user_dir)
            .map_err(|error| error.to_string())?;
        if scanner::desktop_file_id(relative).as_deref() != Some(id.as_str()) {
            return Err("Existing user entry does not match the desktop ID".to_string());
        }
        Ok(path.to_path_buf())
    } else {
        let target = user_dir.join(id);
        match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(target),
            Err(error) => Err(error.to_string()),
            Ok(_) => Err(
                "A user entry already exists at the target path; reload entries first".to_string(),
            ),
        }
    }
}

fn validate_user_entry_path(path: &Path, user_dir: &Path) -> Result<(), String> {
    let relative = path.strip_prefix(user_dir).map_err(|_| {
        "Refusing to modify a file outside the user applications directory".to_string()
    })?;
    let mut components = relative.components().peekable();
    let mut parent = user_dir.to_path_buf();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err("Invalid user desktop entry path".to_string());
        };
        if components.peek().is_none() {
            let filename = name
                .to_str()
                .ok_or_else(|| "Invalid desktop filename".to_string())?;
            validate_desktop_id(filename)?;
            let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
            if !metadata.is_file() && !metadata.file_type().is_symlink() {
                return Err("User entry is not a regular file or symbolic link".to_string());
            }
            return Ok(());
        }
        parent.push(name);
        let metadata = fs::symlink_metadata(&parent).map_err(|error| error.to_string())?;
        if !metadata.is_dir() {
            return Err("Refusing to traverse a non-directory or symbolic link".to_string());
        }
    }
    Err("Invalid user desktop entry path".to_string())
}

pub fn import_icon(source: &Path) -> Result<IconImportResult, String> {
    let file = open_regular(source, false).map_err(|error| error.to_string())?;
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

    let extension = detect_icon_format(&contents)?;

    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Invalid icon filename".to_string())?;
    let base_name = sanitize_icon_name(stem);
    let base = xdg::user_hicolor_dir()
        .ok_or_else(|| "Cannot determine the XDG user data directory".to_string())?;
    let theme = load_hicolor_theme()?;
    let directory = if extension == "svg" {
        validate_icon_contents(&contents, extension)?;
        theme.scalable_application_directory()?
    } else {
        let (width, height) = png_dimensions(&contents)?;
        let (directory, size) = theme.png_application_directory(width.max(height))?;
        if width == size && height == size {
            validate_icon_contents(&contents, extension)?;
        } else {
            contents = normalize_png(&contents, size)?;
        }
        directory
    };
    let target_dir = base.join(directory);

    create_private_dir_all(&target_dir).map_err(|error| error.to_string())?;
    let name = unique_icon_name(&target_dir, &base_name, extension, &contents)?;
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

    fn png_application_directory(&self, preferred: u32) -> Result<(&str, u32), String> {
        self.order
            .iter()
            .filter_map(|name| {
                let spec = self.specs.get(name)?;
                let size = spec.size?.checked_mul(spec.scale)?;
                if spec.context.as_deref() != Some("Applications")
                    || spec.kind.as_deref() == Some("Scalable")
                    || size == 0
                    || size > MAX_ICON_DIMENSION
                {
                    return None;
                }
                Some((name.as_str(), size, spec.scale))
            })
            .min_by_key(|&(_, size, scale)| (size > preferred, size.abs_diff(preferred), scale))
            .map(|(name, size, _)| (name, size))
            .ok_or_else(|| {
                "The installed hicolor theme has no suitable PNG Applications directory".to_string()
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

fn detect_icon_format(contents: &[u8]) -> Result<&'static str, String> {
    let (mime, uncertain) = gio::content_type_guess(None::<&Path>, contents);
    if !uncertain {
        match mime.as_str() {
            "image/png" => return Ok("png"),
            "image/svg+xml" => return Ok("svg"),
            _ => {}
        }
    }
    Err("Supported icon import formats are SVG and PNG".to_string())
}

fn validate_icon_contents(contents: &[u8], extension: &str) -> Result<(), String> {
    decode_icon(contents, extension).map(|_| ())
}

fn decode_icon(contents: &[u8], extension: &str) -> Result<gdk_pixbuf::Pixbuf, String> {
    if detect_icon_format(contents)? != extension {
        return Err(format!(
            "Invalid {extension} image: unexpected content type"
        ));
    }

    if extension == "png" {
        let (width, height) = png_dimensions(contents)?;
        if width > MAX_ICON_DIMENSION || height > MAX_ICON_DIMENSION {
            return Err("PNG dimensions exceed 4096 pixels".to_string());
        }
    }
    let loader = gdk_pixbuf::PixbufLoader::with_type(extension)
        .map_err(|error| format!("Cannot initialize {extension} decoder: {error}"))?;
    if extension == "svg" {
        loader.connect_size_prepared(|loader, _, _| loader.set_size(256, 256));
    }
    let written = loader.write(contents);
    let closed = loader.close();
    written
        .and(closed)
        .map_err(|error| format!("Invalid {extension} image: {error}"))?;
    loader
        .pixbuf()
        .ok_or_else(|| format!("Invalid {extension} image: no decoded pixels"))
}

fn normalize_png(contents: &[u8], size: u32) -> Result<Vec<u8>, String> {
    if !(1..=MAX_ICON_DIMENSION).contains(&size) {
        return Err("Invalid target icon size".to_string());
    }
    let pixbuf = decode_icon(contents, "png")?;
    let size = i32::try_from(size).map_err(|error| error.to_string())?;
    let width = pixbuf.width();
    let height = pixbuf.height();
    let longest = width.max(height);
    let limit = i32::try_from(MAX_ICON_DIMENSION).map_err(|error| error.to_string())?;
    if width <= 0 || height <= 0 || longest > limit {
        return Err("Invalid decoded PNG dimensions".to_string());
    }
    let scaled_width = ((width * size + longest / 2) / longest).max(1);
    let scaled_height = ((height * size + longest / 2) / longest).max(1);
    let scaled = pixbuf
        .scale_simple(
            scaled_width,
            scaled_height,
            gdk_pixbuf::InterpType::Bilinear,
        )
        .ok_or_else(|| "Could not scale PNG icon".to_string())?;
    let square = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, size, size)
        .ok_or_else(|| "Could not allocate PNG icon canvas".to_string())?;
    square.fill(0);
    scaled.copy_area(
        0,
        0,
        scaled_width,
        scaled_height,
        &square,
        (size - scaled_width) / 2,
        (size - scaled_height) / 2,
    );
    let encoded = square
        .save_to_bufferv("png", &[])
        .map_err(|error| format!("Could not encode PNG icon: {error}"))?;
    if encoded.len() > 16 * 1024 * 1024 {
        return Err("Resized icon is larger than 16 MiB".to_string());
    }
    Ok(encoded)
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

    let file = open_regular(target, true).map_err(|error| error.to_string())?;
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

fn atomic_create(target: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "target has no parent"))?;
    let temp = temporary_path(parent, "save");
    let result = (|| {
        atomic_temp_write(&temp, contents)?;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o644))?;
        fs::hard_link(&temp, target)
    })();
    let cleanup = fs::remove_file(&temp);
    result?;
    cleanup?;
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
    fn validates_nested_user_paths_without_following_parent_symlinks() -> Result<(), String> {
        let root = temporary_test_dir("nested");
        let user_dir = root.join("applications");
        let nested = user_dir.join("vendor/App.desktop");
        let outside = root.join("outside");
        fs::create_dir_all(user_dir.join("vendor")).map_err(|error| error.to_string())?;
        fs::create_dir_all(&outside).map_err(|error| error.to_string())?;
        fs::write(&nested, b"old").map_err(|error| error.to_string())?;
        fs::write(outside.join("App.desktop"), b"outside").map_err(|error| error.to_string())?;
        std::os::unix::fs::symlink(&outside, user_dir.join("escape"))
            .map_err(|error| error.to_string())?;
        assert!(validate_user_entry_path(&nested, &user_dir).is_ok());
        assert!(validate_user_entry_path(&outside.join("App.desktop"), &user_dir).is_err());
        assert!(
            validate_user_entry_path(&user_dir.join("../outside/App.desktop"), &user_dir).is_err()
        );
        assert!(validate_user_entry_path(&user_dir.join("escape/App.desktop"), &user_dir).is_err());
        let target = user_save_target(&user_dir, "vendor-App.desktop", Some(&nested))?;
        assert_eq!(target, nested);
        assert!(user_save_target(&user_dir, "wrong.desktop", Some(&nested)).is_err());
        assert_eq!(
            write_user_entry(&user_dir, "vendor-App.desktop", b"new", Some(&nested))?,
            target
        );
        assert_eq!(
            fs::read(&target).map_err(|error| error.to_string())?,
            b"new"
        );
        assert_eq!(
            fs::read(target.with_extension("desktop.bak")).map_err(|error| error.to_string())?,
            b"old"
        );
        assert!(!user_dir.join("vendor-App.desktop").exists());
        remove_user_entry_in(&target, &user_dir)?;
        assert!(!target.exists());
        fs::remove_dir_all(&root).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn refuses_to_overwrite_unselected_user_target() -> Result<(), String> {
        let root = temporary_test_dir("collision");
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let target = root.join("App.desktop");
        assert_eq!(user_save_target(&root, "App.desktop", None)?, target);
        assert_eq!(
            write_user_entry(&root, "App.desktop", b"existing", None)?,
            target
        );
        assert!(user_save_target(&root, "App.desktop", None).is_err());
        assert!(atomic_create(&target, b"replacement").is_err());
        assert_eq!(
            fs::read(&target).map_err(|error| error.to_string())?,
            b"existing"
        );
        fs::remove_dir_all(&root).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn rejects_invalid_icon_contents() -> Result<(), String> {
        let png = include_bytes!("../data/icons/hicolor/48x48/apps/io.github.h0ek.DeskEntry.png");
        validate_icon_contents(png, "png")?;
        assert!(validate_icon_contents(&png[..24], "png").is_err());
        assert!(validate_icon_contents(b"not an image", "svg").is_err());
        assert!(validate_icon_contents(png, "svg").is_err());
        validate_icon_contents(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"><rect width="48" height="48"/></svg>"#,
            "svg",
        )?;
        Ok(())
    }

    #[test]
    fn detects_icon_formats_without_filename() {
        let png = include_bytes!("../data/icons/hicolor/48x48/apps/io.github.h0ek.DeskEntry.png");
        assert_eq!(detect_icon_format(png), Ok("png"));
        assert_eq!(
            detect_icon_format(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"/>"#
            ),
            Ok("svg")
        );
        assert!(detect_icon_format(b"not an image").is_err());
    }

    #[test]
    fn resizes_rectangular_png_with_transparent_padding() -> Result<(), String> {
        let source = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 512, 256)
            .ok_or_else(|| "Could not allocate test image".to_string())?;
        source.fill(0xff00_00ff);
        let png = source
            .save_to_bufferv("png", &[])
            .map_err(|error| error.to_string())?;
        let normalized = normalize_png(&png, 256)?;
        assert_eq!(png_dimensions(&normalized)?, (256, 256));
        let decoded = decode_icon(&normalized, "png")?;
        assert!(decoded.has_alpha());
        let pixels = decoded.read_pixel_bytes();
        assert_eq!(&pixels.as_ref()[..4], &[0, 0, 0, 0]);
        let stride = usize::try_from(decoded.rowstride()).map_err(|error| error.to_string())?;
        let center = 128 * stride + 128 * 4;
        assert_eq!(&pixels.as_ref()[center..center + 4], &[255, 0, 0, 255]);
        assert!(normalize_png(&png, 0).is_err());
        assert!(normalize_png(&png, MAX_ICON_DIMENSION + 1).is_err());
        assert!(normalize_png(&png[..24], 256).is_err());
        Ok(())
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
        assert_eq!(theme.png_application_directory(48), Ok(("48x48/apps", 48)));
        assert_eq!(
            theme.png_application_directory(96),
            Ok(("48x48@2/apps", 96))
        );
        assert_eq!(theme.png_application_directory(64), Ok(("48x48/apps", 48)));
        assert_eq!(
            theme.png_application_directory(512),
            Ok(("48x48@2/apps", 96))
        );
        assert_eq!(theme.png_application_directory(12), Ok(("48x48/apps", 48)));
        Ok(())
    }
}
