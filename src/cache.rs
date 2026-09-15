use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::index::LexicalIndex;
use crate::model::Symbol;
use crate::parser::SymbolExtractor;
use crate::repository::load_source_file;

const CACHE_SCHEMA: u32 = 3;
const CACHE_DIRECTORY: &str = ".flexcontext";
const CACHE_FILE: &str = "index-v3.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FileFingerprint {
    bytes: u64,
    modified_ns: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedFile {
    fingerprint: FileFingerprint,
    symbols: Vec<Symbol>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RepositoryCache {
    schema: u32,
    files: BTreeMap<String, CachedFile>,
    index: LexicalIndex,
}

#[derive(Serialize)]
struct RepositoryCacheRef<'a> {
    schema: u32,
    files: &'a BTreeMap<String, CachedFile>,
    index: &'a LexicalIndex,
}

#[derive(Debug)]
pub struct IndexedRepository {
    pub symbols: Vec<Symbol>,
    pub index: LexicalIndex,
    pub source_bytes: usize,
    pub files_available: usize,
    pub files_reused: usize,
    pub files_reparsed: usize,
    pub index_reused: bool,
    pub cache_load_us: u128,
    pub parse_and_extract_us: u128,
    pub index_us: u128,
    pub cache_write_us: u128,
}

pub fn load_indexed_repository(
    root: &Path,
    paths: &[PathBuf],
    use_cache: bool,
) -> Result<IndexedRepository> {
    let stage = Instant::now();
    let mut old_cache = if use_cache { read_cache(root) } else { None };
    let cache_load_us = stage.elapsed().as_micros();
    let old_cache_valid = old_cache
        .as_ref()
        .is_some_and(|cache| cache.schema == CACHE_SCHEMA);
    let mut old_files = old_cache
        .as_mut()
        .filter(|cache| cache.schema == CACHE_SCHEMA)
        .map(|cache| std::mem::take(&mut cache.files))
        .unwrap_or_default();
    let mut files = BTreeMap::new();
    let mut source_bytes = 0;
    let mut files_reused = 0;
    let mut files_reparsed = 0;
    let parse_stage = Instant::now();

    let mut changed = Vec::new();
    for path in paths {
        let relative = relative_path(root, path);
        let fingerprint = fingerprint(path)?;
        source_bytes += fingerprint.bytes as usize;
        if let Some(cached) = old_files.remove(&relative)
            && cached.fingerprint == fingerprint
        {
            files_reused += 1;
            files.insert(relative, cached);
            continue;
        }
        changed.push((relative, path.clone(), fingerprint));
    }
    let parsed: Result<Vec<_>> = changed
        .par_iter()
        .map_init(
            SymbolExtractor::new,
            |extractor, (relative, path, fingerprint)| {
                let Some(file) = load_source_file(root, path)? else {
                    return Ok(None);
                };
                let mut local_id = 0;
                let symbols = extractor.extract(&file, &mut local_id)?;
                Ok(Some((
                    relative.clone(),
                    CachedFile {
                        fingerprint: fingerprint.clone(),
                        symbols,
                    },
                )))
            },
        )
        .collect();
    for (relative, cached) in parsed?.into_iter().flatten() {
        files_reparsed += 1;
        files.insert(relative, cached);
    }
    let parse_and_extract_us = parse_stage.elapsed().as_micros();
    let removed_files = !old_files.is_empty();
    let mut symbols: Vec<_> = files
        .values()
        .flat_map(|file| file.symbols.iter().cloned())
        .collect();
    for (id, symbol) in symbols.iter_mut().enumerate() {
        symbol.id = id;
    }

    let index_stage = Instant::now();
    let index_reused = use_cache
        && old_cache_valid
        && files_reparsed == 0
        && !removed_files
        && files_reused == files.len();
    let index = if index_reused {
        old_cache.expect("validated cache exists").index
    } else {
        LexicalIndex::build(&symbols)
    };
    let index_us = index_stage.elapsed().as_micros();

    let write_stage = Instant::now();
    if use_cache && !index_reused {
        write_cache(root, &files, &index)?;
    }
    let cache_write_us = write_stage.elapsed().as_micros();

    Ok(IndexedRepository {
        files_available: paths.len(),
        symbols,
        index,
        source_bytes,
        files_reused,
        files_reparsed,
        index_reused,
        cache_load_us,
        parse_and_extract_us,
        index_us,
        cache_write_us,
    })
}

fn read_cache(root: &Path) -> Option<RepositoryCache> {
    let bytes = std::fs::read(cache_path(root)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_cache(
    root: &Path,
    files: &BTreeMap<String, CachedFile>,
    index: &LexicalIndex,
) -> Result<()> {
    let directory = root.join(CACHE_DIRECTORY);
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("cannot create cache directory {}", directory.display()))?;
    let temporary = directory.join(format!(".{CACHE_FILE}.{}.tmp", std::process::id()));
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)
        .with_context(|| format!("cannot write cache {}", temporary.display()))?;
    serde_json::to_writer(
        BufWriter::new(file),
        &RepositoryCacheRef {
            schema: CACHE_SCHEMA,
            files,
            index,
        },
    )
    .with_context(|| format!("cannot serialize cache {}", temporary.display()))?;
    std::fs::rename(&temporary, cache_path(root))
        .with_context(|| format!("cannot install cache in {}", directory.display()))?;
    Ok(())
}

fn cache_path(root: &Path) -> PathBuf {
    root.join(CACHE_DIRECTORY).join(CACHE_FILE)
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn fingerprint(path: &Path) -> Result<FileFingerprint> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("cannot inspect source file {}", path.display()))?;
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    Ok(FileFingerprint {
        bytes: metadata.len(),
        modified_ns,
    })
}
