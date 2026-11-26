use super::{
    metadata::{
        OriginMetadata, normalize_local_path, origin_from_local_path,
        relative_path_from_working_dir,
    },
    stream::{is_compressed_input, is_unsupported_compressed_input},
};
use clap::ValueEnum;
use eyre::{Result, eyre};
use fluent_uri::UriRef;
use glob::glob;
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum SymlinkMode {
    Follow,
    Fail,
    Skip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum HiddenMode {
    Include,
    Fail,
    Skip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DiscoveryOptions {
    pub(crate) symlinks: SymlinkMode,
    pub(crate) hidden: HiddenMode,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            symlinks: SymlinkMode::Skip,
            hidden: HiddenMode::Skip,
        }
    }
}

pub(super) fn resolve_file_document_paths_with_options(
    values: &[UriRef<String>],
    discovery_options: DiscoveryOptions,
) -> Result<(Vec<PathBuf>, Vec<OriginMetadata>)> {
    let policy = is_batch_import(values).then_some(discovery_options);
    let mut paths = BTreeMap::new();
    for value in values {
        let path = value.path().as_str();
        if has_glob_metachar(path) {
            add_glob_matches(&mut paths, path, policy)?;
        } else {
            add_concrete_path(&mut paths, Path::new(path), policy)?;
        }
    }
    validate_compression(&paths)?;
    if paths.is_empty() {
        return Err(eyre!("No regular files resolved from file inputs"));
    }
    if paths.len() > 1 {
        reject_paths_outside_working_directory(paths.keys(), discovery_options.symlinks)?;
    }
    Ok(paths.into_iter().unzip())
}

pub(super) fn is_batch_import(values: &[UriRef<String>]) -> bool {
    values.len() > 1
        || values
            .iter()
            .any(|value| has_glob_metachar(value.path().as_str()))
}

fn add_glob_matches(
    paths: &mut BTreeMap<PathBuf, OriginMetadata>,
    pattern: &str,
    policy: Option<DiscoveryOptions>,
) -> Result<()> {
    let entries = glob(pattern).map_err(|err| eyre!("Invalid glob pattern {pattern}: {err}"))?;
    let mut matched = false;
    for entry in entries {
        let path = entry.map_err(|err| eyre!("Error expanding glob {pattern}: {err}"))?;
        if should_include_discovery_path(&path, policy)? && path.is_file() {
            add_resolved_path(paths, &path)?;
            matched = true;
        }
    }
    if !matched {
        return Err(eyre!("Glob matched no regular files: {pattern}"));
    }
    Ok(())
}

fn add_concrete_path(
    paths: &mut BTreeMap<PathBuf, OriginMetadata>,
    path: &Path,
    policy: Option<DiscoveryOptions>,
) -> Result<()> {
    if !should_include_discovery_path(path, policy)? {
        return Ok(());
    }
    if !path.exists() {
        return Err(eyre!("File input does not exist: {}", path.display()));
    }
    if !path.is_file() {
        return Err(eyre!(
            "File input is not a regular file: {}",
            path.display()
        ));
    }
    add_resolved_path(paths, path)
}

fn add_resolved_path(paths: &mut BTreeMap<PathBuf, OriginMetadata>, path: &Path) -> Result<()> {
    let path = normalize_local_path(path)?;
    paths
        .entry(path.clone())
        .or_insert_with(|| origin_from_local_path(&path));
    Ok(())
}

fn validate_compression(paths: &BTreeMap<PathBuf, OriginMetadata>) -> Result<()> {
    let multiple_files = paths.len() > 1;
    for path in paths.keys() {
        let path = path.to_string_lossy();
        let unsupported = if multiple_files {
            is_compressed_input(&path)
        } else {
            is_unsupported_compressed_input(&path)
        };
        if unsupported {
            return Err(eyre!("Unsupported compressed input format: {path}"));
        }
    }
    Ok(())
}

fn reject_paths_outside_working_directory<'a>(
    paths: impl IntoIterator<Item = &'a PathBuf>,
    symlink_mode: SymlinkMode,
) -> Result<()> {
    let lexical_working_dir = std::env::current_dir()?;
    let canonical_working_dir = fs::canonicalize(&lexical_working_dir)?;
    for path in paths {
        let canonical_path = fs::canonicalize(path)?;
        let lexical_path_is_inside = path.starts_with(&lexical_working_dir);
        let allowed_external_symlink = symlink_mode == SymlinkMode::Follow
            && lexical_path_is_inside
            && path_contains_symlink(path)?;
        if !canonical_path.starts_with(&canonical_working_dir) && !allowed_external_symlink {
            return Err(eyre!(
                "Multi-source file input is outside the working directory: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn should_include_discovery_path(path: &Path, policy: Option<DiscoveryOptions>) -> Result<bool> {
    let Some(options) = policy else {
        return Ok(true);
    };
    let hidden = path_contains_hidden_component(path);
    match (hidden, options.hidden) {
        (true, HiddenMode::Skip) => return Ok(false),
        (true, HiddenMode::Fail) => {
            return Err(eyre!(
                "Hidden path encountered in multi-source input: {}",
                path.display()
            ));
        }
        _ => {}
    }
    match (path_contains_symlink(path)?, options.symlinks) {
        (true, SymlinkMode::Skip) => Ok(false),
        (true, SymlinkMode::Fail) => Err(eyre!(
            "Symlink path encountered in multi-source input: {}",
            path.display()
        )),
        _ => Ok(true),
    }
}

fn path_contains_hidden_component(path: &Path) -> bool {
    let path = if path.is_absolute() {
        std::env::current_dir()
            .ok()
            .map(|working_dir| relative_path_from_working_dir(path, &working_dir))
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    };
    path.components().any(|component| {
        matches!(component, Component::Normal(value) if value.to_string_lossy().starts_with('.'))
    })
}

fn path_contains_symlink(path: &Path) -> Result<bool> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute
        .components()
        .filter(|component| *component != Component::CurDir)
    {
        current.push(component.as_os_str());
        if is_symlink(&current)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_symlink(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn has_glob_metachar(value: &str) -> bool {
    value.bytes().any(|byte| matches!(byte, b'*' | b'?' | b'['))
}
