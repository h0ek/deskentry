use crate::fileio::open_regular;
use crate::model::DesktopDocument;
use crate::xdg;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const FLATPAK_SYSTEM: &str = "/var/lib/flatpak/exports/share/applications";
const SNAP_SYSTEM: &str = "/var/lib/snapd/desktop/applications";
const SYSTEM_LOCAL: &str = "/usr/local/share/applications";
const SYSTEM: &str = "/usr/share/applications";
const MAX_DESKTOP_BYTES: u64 = 1024 * 1024;
const MAX_DESKTOP_BUFFER: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    User,
    System,
    FlatpakUser,
    FlatpakSystem,
    Snap,
    CustomXdg,
}

impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::System => "System",
            Self::FlatpakUser => "Flatpak user",
            Self::FlatpakSystem => "Flatpak system",
            Self::Snap => "Snap",
            Self::CustomXdg => "Custom XDG",
        }
    }
}

#[derive(Clone, Debug)]
pub struct EntryVariant {
    pub path: PathBuf,
    pub source: SourceKind,
    pub priority: usize,
    pub document: DesktopDocument,
}

#[derive(Clone, Debug)]
pub struct EntryGroup {
    pub id: String,
    pub variants: Vec<EntryVariant>,
}

impl EntryGroup {
    pub fn effective(&self) -> Option<&EntryVariant> {
        self.variants.first()
    }

    pub fn display_name(&self) -> String {
        self.effective()
            .and_then(|variant| variant.document.display_name())
            .unwrap_or_else(|| self.id.clone())
    }
}

pub fn scan_entries() -> Vec<EntryGroup> {
    let mut grouped: BTreeMap<String, Vec<EntryVariant>> = BTreeMap::new();

    for (priority, root) in discovery_roots().into_iter().enumerate() {
        if !root.is_dir() {
            continue;
        }

        let source = source_kind(&root);
        let mut files = Vec::new();
        collect_desktop_files(&root, &mut files);
        files.sort();

        for path in files {
            let Ok(relative) = path.strip_prefix(&root) else {
                continue;
            };
            let Some(id) = desktop_file_id(relative) else {
                continue;
            };
            let Some(contents) = read_desktop_file(&path) else {
                continue;
            };
            let document = DesktopDocument::parse(&contents);

            grouped.entry(id).or_default().push(EntryVariant {
                path,
                source,
                priority,
                document,
            });
        }
    }

    let mut groups = grouped
        .into_iter()
        .map(|(id, mut variants)| {
            variants.sort_by_key(|variant| variant.priority);
            EntryGroup { id, variants }
        })
        .collect::<Vec<_>>();

    groups.sort_by_cached_key(|group| group.display_name().to_lowercase());
    groups
}

fn discovery_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(user_applications) = xdg::user_applications_dir() {
        roots.push(user_applications);
    }

    roots.extend(
        xdg::data_dirs()
            .into_iter()
            .map(|directory| directory.join("applications")),
    );

    if let Some(data_home) = xdg::data_home() {
        roots.push(data_home.join("flatpak/exports/share/applications"));
    }

    roots.push(PathBuf::from(FLATPAK_SYSTEM));
    roots.push(PathBuf::from(SNAP_SYSTEM));

    let mut seen = HashSet::new();
    roots
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn source_kind(root: &Path) -> SourceKind {
    if xdg::user_applications_dir().as_deref() == Some(root) {
        return SourceKind::User;
    }

    if xdg::data_home()
        .map(|data_home| data_home.join("flatpak/exports/share/applications"))
        .as_deref()
        == Some(root)
    {
        return SourceKind::FlatpakUser;
    }

    if root == Path::new(FLATPAK_SYSTEM) {
        return SourceKind::FlatpakSystem;
    }

    if root == Path::new(SNAP_SYSTEM) {
        return SourceKind::Snap;
    }

    if root == Path::new(SYSTEM_LOCAL) || root == Path::new(SYSTEM) {
        return SourceKind::System;
    }

    SourceKind::CustomXdg
}

fn collect_desktop_files(root: &Path, output: &mut Vec<PathBuf>) {
    let mut directories = vec![root.to_path_buf()];

    while let Some(directory) = directories.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };

            if file_type.is_dir() {
                directories.push(path);
            } else if has_desktop_extension(&path)
                && (file_type.is_file() || file_type.is_symlink())
            {
                output.push(path);
            }
        }
    }
}

fn has_desktop_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension == "desktop")
}

fn read_desktop_file(path: &Path) -> Option<String> {
    let file = open_regular(path, false).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_DESKTOP_BYTES {
        return None;
    }

    let mut bytes = Vec::new();
    file.take(MAX_DESKTOP_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_DESKTOP_BUFFER {
        return None;
    }
    String::from_utf8(bytes).ok()
}

pub(crate) fn desktop_file_id(relative: &Path) -> Option<String> {
    relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .map(|components| components.join("-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_desktop_file_id_from_relative_path() {
        assert_eq!(
            desktop_file_id(Path::new("vendor/foo.desktop")),
            Some("vendor-foo.desktop".to_string())
        );
    }

    #[test]
    fn skips_fifo_symlinks_but_reads_regular_symlinks() -> Result<(), String> {
        use std::os::unix::fs::symlink;
        use std::process::Command;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let root =
            std::env::temp_dir().join(format!("deskentry-scan-{}-{nanos}", std::process::id()));
        fs::create_dir(&root).map_err(|error| error.to_string())?;
        let fifo = root.join("fifo");
        let status = Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err("mkfifo failed".to_string());
        }
        let fifo_link = root.join("fifo.desktop");
        symlink(&fifo, &fifo_link).map_err(|error| error.to_string())?;
        let normal = root.join("regular");
        let regular_link = root.join("regular.desktop");
        fs::write(&normal, "[Desktop Entry]\nName=Test\n").map_err(|error| error.to_string())?;
        symlink(&normal, &regular_link).map_err(|error| error.to_string())?;
        assert!(read_desktop_file(&fifo_link).is_none());
        assert_eq!(
            read_desktop_file(&regular_link).as_deref(),
            Some("[Desktop Entry]\nName=Test\n")
        );
        fs::remove_dir_all(&root).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn group_uses_id_when_effective_entry_has_no_name() {
        let group = EntryGroup {
            id: "org.example.Hidden.desktop".to_string(),
            variants: vec![EntryVariant {
                path: PathBuf::from("/tmp/org.example.Hidden.desktop"),
                source: SourceKind::User,
                priority: 0,
                document: DesktopDocument::parse("[Desktop Entry]\nHidden=true\n"),
            }],
        };

        assert_eq!(group.display_name(), "org.example.Hidden.desktop");
    }
}
