use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::catalog::is_supported;

const MAX_RECENT_FILES: usize = 12;

#[derive(Debug)]
pub struct RecentFiles {
    entries: Vec<PathBuf>,
    storage_path: Option<PathBuf>,
}

impl RecentFiles {
    pub fn load() -> Self {
        let storage_path = storage_path();
        Self::load_from(storage_path)
    }

    fn load_from(storage_path: Option<PathBuf>) -> Self {
        let entries = storage_path
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok())
            .map(|contents| {
                contents
                    .lines()
                    .map(PathBuf::from)
                    .filter(|path| path.is_file() && is_supported(path))
                    .take(MAX_RECENT_FILES)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            entries,
            storage_path,
        }
    }

    pub fn entries(&self) -> &[PathBuf] {
        &self.entries
    }

    pub fn most_recent(&self) -> Option<&Path> {
        self.entries.first().map(PathBuf::as_path)
    }

    pub fn record(&mut self, path: &Path) {
        if !path.is_file() || !is_supported(path) || path.to_string_lossy().contains(['\n', '\r']) {
            return;
        }
        self.entries.retain(|existing| existing != path);
        self.entries.insert(0, path.to_path_buf());
        self.entries.truncate(MAX_RECENT_FILES);
        self.persist();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.persist();
    }

    fn persist(&self) {
        let Some(path) = &self.storage_path else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if let Err(error) = fs::create_dir_all(parent) {
            eprintln!("could not create recent-files directory: {error}");
            return;
        }
        let contents = self
            .entries
            .iter()
            .map(|entry| entry.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n");
        let temporary = path.with_extension("tmp");
        if let Err(error) =
            fs::write(&temporary, contents).and_then(|()| fs::rename(&temporary, path))
        {
            eprintln!("could not save recent files: {error}");
        }
    }
}

fn storage_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library/Application Support/Eagle Viewer")
            .join("recent-files-v1")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_files_are_unique_newest_first_and_bounded() {
        let temporary =
            std::env::temp_dir().join(format!("eagle-viewer-recents-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temporary);
        fs::create_dir_all(&temporary).unwrap();
        let storage = temporary.join("recents");
        let mut recent = RecentFiles::load_from(Some(storage.clone()));

        for index in 0..14 {
            let path = temporary.join(format!("image-{index}.png"));
            fs::write(&path, b"test").unwrap();
            recent.record(&path);
        }
        let newest = temporary.join("image-13.png");
        recent.record(&newest);
        let unsupported = temporary.join("notes.txt");
        fs::write(&unsupported, b"not an image").unwrap();
        recent.record(&unsupported);

        assert_eq!(recent.entries().len(), MAX_RECENT_FILES);
        assert_eq!(recent.most_recent(), Some(newest.as_path()));
        assert!(!recent.entries().contains(&unsupported));
        assert_eq!(
            RecentFiles::load_from(Some(storage)).entries(),
            recent.entries()
        );
        let _ = fs::remove_dir_all(&temporary);
    }
}
