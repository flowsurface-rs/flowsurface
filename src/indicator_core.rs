use std::fs;
use std::path::{Path, PathBuf};

use data::chart::kline::indicators::{self, IndicatorCatalogPaths, IndicatorId};

#[derive(Clone, Copy)]
struct CoreSource {
    file_name: &'static str,
    source: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/indicator_core_sources.rs"));

pub(crate) fn initialize() -> Result<(), String> {
    let (indicator_paths, seed_core) =
        configured_indicator_paths().map_err(|error| format!("Invalid indicator path: {error}"))?;

    if seed_core {
        seed_core_sources(&indicator_paths.core_directory)
            .map_err(|error| format!("Failed to synchronize core indicators: {error}"))?;
    }

    data::chart::kline::indicators::initialize_indicator_catalog_with_paths(&indicator_paths)
        .map_err(|error| format!("Failed to load indicators: {error}"))?;

    validate_core_catalog().map_err(|error| format!("Failed to load core indicators: {error}"))
}

fn indicator_paths_from_args() -> Result<Vec<PathBuf>, String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut paths = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--indicator-path" {
            let path = args
                .get(index + 1)
                .ok_or_else(|| "--indicator-path requires an explicit .ind file".to_owned())?;
            paths.push(validate_indicator_path(PathBuf::from(path))?);
            index += 2;
            continue;
        } else if let Some(path) = args[index].strip_prefix("--indicator-path=") {
            if path.trim().is_empty() {
                return Err("--indicator-path requires an explicit .ind file".to_owned());
            }
            paths.push(validate_indicator_path(PathBuf::from(path))?);
        }
        index += 1;
    }
    Ok(paths)
}

fn validate_indicator_path(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_dir() {
        return Err(format!(
            "{} is a directory; --indicator-path accepts an explicit .ind file",
            path.display()
        ));
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("ind") {
        return Err(format!(
            "{}: --indicator-path accepts only .ind source files",
            path.display()
        ));
    }
    Ok(path)
}

fn configured_indicator_paths() -> Result<(IndicatorCatalogPaths, bool), String> {
    let (mut paths, seed_core) = match std::env::var_os("FLOWSURFACE_INDICATORS_PATH") {
        Some(root) => (IndicatorCatalogPaths::from_indicator_root(root), false),
        None => (IndicatorCatalogPaths::from_data_directory(), true),
    };
    paths.additional_files.extend(indicator_paths_from_args()?);
    Ok((paths, seed_core))
}

fn validate_core_catalog() -> Result<(), String> {
    let catalog = indicators::catalog();
    let missing = core_indicator_ids()
        .filter(|id| {
            IndicatorId::parse(id)
                .ok()
                .is_none_or(|indicator| catalog.get(&indicator).is_none())
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "missing core indicator source(s): {}",
            missing.join(", ")
        ))
    }
}

fn core_indicator_ids() -> impl Iterator<Item = String> {
    CORE_SOURCES.iter().map(|source| {
        let short_name = source
            .file_name
            .strip_suffix(".ind")
            .expect("embedded core source must use the .ind extension");
        format!("core.{short_name}")
    })
}

fn seed_core_sources(root: &Path) -> Result<(), String> {
    let staging = sibling_path(root, "staging");
    if core_tree_matches(root) {
        remove_path(&staging)
            .map_err(|error| format!("could not remove stale core staging directory: {error}"))?;
        return Ok(());
    }

    let parent = root
        .parent()
        .ok_or_else(|| format!("core indicator path has no parent: {}", root.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create core indicator parent: {error}"))?;

    remove_path(&staging)
        .map_err(|error| format!("could not remove stale core staging directory: {error}"))?;
    write_core_tree(&staging)?;
    validate_core_tree(&staging)?;

    replace_core_tree(root, &staging)
}

fn validate_core_tree(root: &Path) -> Result<(), String> {
    let (files, discovery_diagnostics) =
        data::chart::kline::indicators::language::indicator_files_in_directory(root).inspect_err(
            |_| {
                let _ = remove_path(root);
            },
        )?;
    let mut report = data::chart::kline::indicators::language::load_from_files_report(
        &files,
        data::chart::kline::indicators::language::PackageOrigin::Core,
    );
    report.diagnostics.extend(discovery_diagnostics);
    if !report.diagnostics.is_empty() {
        let _ = remove_path(root);
        return Err(format!(
            "new core indicator sources failed validation:\n{}",
            report.diagnostics.join("\n")
        ));
    }
    let missing = core_indicator_ids()
        .filter(|id| {
            !report
                .registrations
                .iter()
                .any(|registration| registration.id.id() == id)
        })
        .collect::<Vec<_>>();
    if missing.is_empty() && report.registrations.len() == CORE_SOURCES.len() {
        Ok(())
    } else {
        let _ = remove_path(root);
        Err(format!(
            "new core indicator sources did not provide exactly the required IDs: {}",
            if missing.is_empty() {
                "unexpected source set".to_owned()
            } else {
                missing.join(", ")
            }
        ))
    }
}

fn write_core_tree(root: &Path) -> Result<(), String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("could not create core indicator directory: {error}"))?;
    for source in CORE_SOURCES {
        let source_path = root.join(source.file_name);
        fs::write(&source_path, source.source).map_err(|error| {
            format!(
                "could not write core indicator source {}: {error}",
                source.file_name
            )
        })?;
    }
    Ok(())
}

fn core_tree_matches(root: &Path) -> bool {
    let Some(entries) = directory_entries(root) else {
        return false;
    };
    let mut expected = CORE_SOURCES
        .iter()
        .map(|source| source.file_name.to_owned())
        .collect::<Vec<_>>();
    expected.sort();
    if entries != expected {
        return false;
    }

    CORE_SOURCES.iter().all(|source| {
        fs::read_to_string(root.join(source.file_name))
            .ok()
            .as_deref()
            == Some(source.source)
    })
}

fn directory_entries(root: &Path) -> Option<Vec<String>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(root).ok()? {
        let entry = entry.ok()?;
        entries.push(entry.file_name().to_string_lossy().into_owned());
    }
    entries.sort();
    Some(entries)
}

fn replace_core_tree(root: &Path, staging: &Path) -> Result<(), String> {
    let backup = sibling_path(root, "backup");
    remove_path(&backup).map_err(|error| format!("could not remove stale core backup: {error}"))?;

    let had_existing = root.exists();
    if had_existing {
        fs::rename(root, &backup)
            .map_err(|error| format!("could not move current core sources aside: {error}"))?;
    }

    if let Err(error) = fs::rename(staging, root) {
        let restore = if had_existing {
            fs::rename(&backup, root).err()
        } else {
            None
        };
        let _ = remove_path(staging);
        return match restore {
            Some(restore_error) => Err(format!(
                "could not activate core sources: {error}; restoring the previous tree also failed: {restore_error}"
            )),
            None => Err(format!("could not activate core sources: {error}")),
        };
    }

    if had_existing {
        let _ = remove_path(&backup);
    }
    Ok(())
}

fn sibling_path(root: &Path, kind: &str) -> PathBuf {
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("core");
    root.with_file_name(format!(".{name}-{kind}-{}", std::process::id()))
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}
