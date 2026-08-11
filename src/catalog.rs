use std::{
    fs,
    path::{Path, PathBuf},
};

const SUPPORTED_EXTENSIONS: &[&str] = &[
    // Standard raster and HDR formats.
    "bmp", "exr", "gif", "hdr", "ico", "jpeg", "jpg", "png", "pnm", "ppm", "pgm", "qoi", "tga",
    "tif", "tiff", "webp", // Camera RAW families supported by rawler.
    "3fr", "ari", "arw", "cr2", "cr3", "crw", "dcr", "dcs", "dng", "erf", "fff", "iiq", "kdc",
    "mef", "mos", "mrw", "nef", "nrw", "orf", "pef", "raf", "raw", "rw2", "sr2", "srf", "srw",
    "x3f", // Formats delegated to the native macOS image/Quick Look stack.
    "avif", "cur", "dds", "heic", "heif", "icns", "j2k", "jp2", "jpf", "jpx", "pct", "pict", "psb",
    "psd", "sgi", "svg", "xbm",
];

pub fn is_raw(path: &Path) -> bool {
    const RAW_EXTENSIONS: &[&str] = &[
        "3fr", "ari", "arw", "cr2", "cr3", "crw", "dcr", "dcs", "dng", "erf", "fff", "iiq", "kdc",
        "mef", "mos", "mrw", "nef", "nrw", "orf", "pef", "raf", "raw", "rw2", "sr2", "srf", "srw",
        "x3f",
    ];
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            RAW_EXTENSIONS
                .iter()
                .any(|raw| extension.eq_ignore_ascii_case(raw))
        })
}

pub fn uses_native_fallback(path: &Path) -> bool {
    const NATIVE_EXTENSIONS: &[&str] = &[
        "avif", "cur", "dds", "heic", "heif", "icns", "j2k", "jp2", "jpf", "jpx", "pct", "pict",
        "psb", "psd", "sgi", "svg", "xbm",
    ];
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            NATIVE_EXTENSIONS
                .iter()
                .any(|native| extension.eq_ignore_ascii_case(native))
        })
}

#[derive(Debug, Default)]
pub struct Catalog {
    files: Vec<PathBuf>,
    index: usize,
}

impl Catalog {
    pub fn around(path: &Path) -> Self {
        let path = path.to_path_buf();
        let mut files = path
            .parent()
            .and_then(|parent| fs::read_dir(parent).ok())
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|candidate| candidate.is_file() && is_supported(candidate))
            .collect::<Vec<_>>();

        files.sort_by(|left, right| {
            left.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
                .cmp(
                    &right
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase(),
                )
        });

        if !files.iter().any(|candidate| candidate == &path) {
            files.push(path.clone());
        }
        let index = files
            .iter()
            .position(|candidate| candidate == &path)
            .unwrap_or(0);

        Self { files, index }
    }

    pub fn current(&self) -> Option<&Path> {
        self.files.get(self.index).map(PathBuf::as_path)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn position(&self) -> usize {
        self.index
    }

    pub fn set_position(&mut self, index: usize) -> bool {
        if index >= self.files.len() || index == self.index {
            return false;
        }
        self.index = index;
        true
    }

    pub fn window(&self, radius: usize) -> Vec<(usize, PathBuf)> {
        let start = self.index.saturating_sub(radius);
        let end = (self.index + radius + 1).min(self.files.len());
        self.files[start..end]
            .iter()
            .cloned()
            .enumerate()
            .map(|(offset, path)| (start + offset, path))
            .collect()
    }

    pub fn move_by(&mut self, delta: isize) -> Option<&Path> {
        if self.files.is_empty() {
            return None;
        }
        self.index = self
            .index
            .saturating_add_signed(delta)
            .min(self.files.len() - 1);
        self.current()
    }

    pub fn neighbors(&self) -> impl Iterator<Item = &Path> {
        let before = self.index.checked_sub(1).and_then(|i| self.files.get(i));
        let after = self.files.get(self.index + 1);
        [before, after].into_iter().flatten().map(PathBuf::as_path)
    }
}

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_check_is_case_insensitive() {
        assert!(is_supported(Path::new("photo.JPEG")));
        assert!(is_supported(Path::new("scan.TIFF")));
        assert!(is_supported(Path::new("sony.ARW")));
        assert!(is_raw(Path::new("nikon.NEF")));
        assert!(!is_supported(Path::new("notes.txt")));
    }
}
