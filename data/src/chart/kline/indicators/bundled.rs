use super::{IndicatorId, catalog, language};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
struct CoreSource {
    file_name: &'static str,
    source: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/indicator_core_sources.rs"));

pub(super) fn validate_core_catalog() -> Result<(), String> {
    let catalog = catalog();
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

pub(super) fn seed_core_sources(root: &Path) -> Result<(), String> {
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
        language::indicator_files_in_directory(root).inspect_err(|_| {
            let _ = remove_path(root);
        })?;
    let mut report = language::load_from_files_report(&files, language::PackageOrigin::Core);
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
