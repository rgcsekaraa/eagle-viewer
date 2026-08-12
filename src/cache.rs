use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

use eframe::egui::TextureHandle;

use crate::loader::DecodedImage;

#[derive(Clone)]
pub struct CachedPreview {
    pub image: Arc<DecodedImage>,
    pub texture: TextureHandle,
}

impl CachedPreview {
    fn memory_bytes(&self) -> usize {
        // Approximate one CPU copy and one GPU RGBA8 texture.
        self.image.memory_bytes().saturating_mul(2)
    }
}

/// A byte-budgeted LRU. Image count is the wrong limit: one preview can be 64 MiB.
pub struct PreviewCache {
    entries: HashMap<PathBuf, CachedPreview>,
    order: VecDeque<PathBuf>,
    used_bytes: usize,
    budget_bytes: usize,
}

impl PreviewCache {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            used_bytes: 0,
            budget_bytes,
        }
    }

    pub fn get(&mut self, path: &Path) -> Option<CachedPreview> {
        let value = self.entries.get(path).cloned()?;
        self.touch(path);
        Some(value)
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }

    pub fn peek(&self, path: &Path) -> Option<CachedPreview> {
        self.entries.get(path).cloned()
    }

    pub fn remove(&mut self, path: &Path) {
        if let Some(preview) = self.entries.remove(path) {
            self.used_bytes = self.used_bytes.saturating_sub(preview.memory_bytes());
        }
        self.order.retain(|candidate| candidate != path);
    }

    pub fn insert(&mut self, preview: CachedPreview, protected: Option<&Path>) {
        let path = preview.image.path.clone();
        if let Some(old) = self.entries.insert(path.clone(), preview.clone()) {
            self.used_bytes = self.used_bytes.saturating_sub(old.memory_bytes());
        }
        self.used_bytes = self.used_bytes.saturating_add(preview.memory_bytes());
        self.touch(&path);

        while self.used_bytes > self.budget_bytes && self.entries.len() > 1 {
            let Some(candidate) = self.order.pop_front() else {
                break;
            };
            if protected.is_some_and(|protected| protected == candidate) {
                self.order.push_back(candidate);
                continue;
            }
            if let Some(evicted) = self.entries.remove(&candidate) {
                self.used_bytes = self.used_bytes.saturating_sub(evicted.memory_bytes());
            }
        }
    }

    fn touch(&mut self, path: &Path) {
        if let Some(position) = self.order.iter().position(|candidate| candidate == path) {
            self.order.remove(position);
        }
        self.order.push_back(path.to_path_buf());
    }
}
