use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::{DirEntry, WalkBuilder};

use crate::language::detect_language;
use crate::model::SourceFile;

const MAX_SOURCE_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub fn discover_source_paths(root: &Path) -> Result<(Vec<PathBuf>, usize)> {
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot access repository root {}", root.display()))?;
    let mut paths = Vec::new();
    let mut files_scanned = 0;
    let walker = WalkBuilder::new(&root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .filter_entry(|entry| !is_excluded_dir(entry))
        .build();

    for entry in walker {
        let entry = entry.with_context(|| format!("failed while traversing {}", root.display()))?;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        files_scanned += 1;
        if detect_language(entry.path()).is_some()
            && entry
                .metadata()
                .is_ok_and(|meta| meta.len() <= MAX_SOURCE_FILE_BYTES)
        {
            paths.push(entry.into_path());
        }
    }
    paths.sort();
    Ok((paths, files_scanned))
}

pub fn load_source_file(root: &Path, path: &Path) -> Result<Option<SourceFile>> {
    let Some(language) = detect_language(path) else {
        return Ok(None);
    };
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    let relative_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    Ok(Some(SourceFile {
        absolute_path: path.to_owned(),
        relative_path,
        language,
        source,
    }))
}

fn is_excluded_dir(entry: &DirEntry) -> bool {
    if !entry.file_type().is_some_and(|kind| kind.is_dir()) {
        return false;
    }
    matches!(
        entry.file_name().to_str(),
        Some(
            ".git"
                | ".hg"
                | ".svn"
                | "node_modules"
                | "target"
                | "dist"
                | "build"
                | ".next"
                | ".venv"
                | "venv"
                | "__pycache__"
                | "vendor"
                | "coverage"
        )
    )
}
