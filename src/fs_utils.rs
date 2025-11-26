// src/fs_utils.rs - Helper functions for recursive copy and remove
use anyhow::Result;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

pub fn copy_recursively(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let src_path = entry.path();
        let rel = src_path.strip_prefix(src)?;
        let dst_path = dst.join(rel);
        if src_path.is_dir() {
            fs::create_dir_all(&dst_path)?;
        } else {
            fs::copy(src_path, &dst_path)?;
        }
    }
    Ok(())
}

pub fn remove_recursively(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(Into::into)
}