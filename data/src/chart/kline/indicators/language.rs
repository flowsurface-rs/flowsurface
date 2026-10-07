use super::registry::IndicatorRegistration;
use super::scripting::registration;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Identifies how a source file entered the host catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageOrigin {
    /// A source bundled with the application or overridden for development.
    Core,
    /// A source discovered in a user or development directory.
    Installed,
}

/// Load results for a set of explicit indicator source files.
#[derive(Debug, Default)]
pub struct IndicatorLoadReport {
    pub registrations: Vec<IndicatorRegistration>,
    pub diagnostics: Vec<String>,
    pub loaded_sources: Vec<LoadedIndicatorSource>,
}

/// Metadata for a source that compiled and passed host registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedIndicatorSource {
    pub path: PathBuf,
    pub id: String,
    pub short_name: String,
}

/// Return the immediate `.ind` files in a collection directory.
pub fn indicator_files_in_directory(
    path: impl AsRef<Path>,
) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    let root = path.as_ref();
    let metadata = fs::metadata(root).map_err(|error| {
        format!(
            "{}: could not inspect indicator directory: {error}",
            root.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!("{} is not an indicator directory", root.display()));
    }

    let mut entries = fs::read_dir(root)
        .map_err(|error| {
            format!(
                "{}: could not read indicator directory: {error}",
                root.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "{}: could not read indicator directory: {error}",
                root.display()
            )
        })?;
    entries.sort_by_key(|entry| entry.path());

    let mut files = Vec::new();
    let diagnostics = Vec::new();
    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("{}: could not inspect entry: {error}", path.display()))?;
        if file_type.is_file() {
            collect_source_file(path, &mut files);
        }
    }
    Ok((files, diagnostics))
}

/// Load explicit source files and retain per-file diagnostics.
pub fn load_from_files(
    paths: &[PathBuf],
    origin: PackageOrigin,
) -> Result<Vec<IndicatorRegistration>, Vec<String>> {
    let report = load_from_files_report(paths, origin);
    if report.diagnostics.is_empty() {
        Ok(report.registrations)
    } else {
        Err(report.diagnostics)
    }
}

/// Load valid source files and retain diagnostics for files that could not
/// be activated. The application uses this form so one broken file does not
/// disable unrelated indicators.
pub fn load_from_files_report(paths: &[PathBuf], origin: PackageOrigin) -> IndicatorLoadReport {
    load_compiled_report(
        paths
            .iter()
            .map(|path| (path.clone(), indicator_engine::compile_file(path))),
        origin,
    )
}

pub(super) fn load_compiled_report(
    sources: impl IntoIterator<
        Item = (
            PathBuf,
            Result<indicator_engine::CompiledIndicator, Vec<indicator_engine::Diagnostic>>,
        ),
    >,
    origin: PackageOrigin,
) -> IndicatorLoadReport {
    let mut report = IndicatorLoadReport::default();
    let mut ids = BTreeMap::<String, PathBuf>::new();
    for (path, compiled) in sources {
        if path.extension().and_then(|extension| extension.to_str()) != Some("ind") {
            report.diagnostics.push(format!(
                "{}: indicator source files must use the .ind extension",
                path.display()
            ));
            continue;
        }
        let compiled = match compiled {
            Ok(compiled) => compiled,
            Err(diagnostics) => {
                report.diagnostics.extend(
                    diagnostics
                        .into_iter()
                        .map(|diagnostic| diagnostic.to_string()),
                );
                continue;
            }
        };
        let id = compiled.descriptor.id.as_str().to_owned();
        let short_name = compiled.descriptor.id.short_name().to_owned();
        let expected_file_name = compiled.descriptor.id.source_file_name();
        if let Err(diagnostic) = validate_source_filename(&path, &id, &expected_file_name) {
            report.diagnostics.push(diagnostic);
            continue;
        }
        match registration(compiled, origin) {
            Ok(registration) => {
                report.loaded_sources.push(LoadedIndicatorSource {
                    path: path.clone(),
                    id: id.clone(),
                    short_name,
                });
                if let Some(previous) = ids.get(&id) {
                    report.diagnostics.push(format!(
                        "{}: duplicate indicator ID '{}' (already defined in {})",
                        path.display(),
                        id,
                        previous.display()
                    ));
                    continue;
                }
                ids.insert(id, path.clone());
                report.registrations.push(registration);
            }
            Err(errors) => report.diagnostics.extend(errors),
        }
    }
    report
}

fn validate_source_filename(path: &Path, id: &str, expected_file_name: &str) -> Result<(), String> {
    let actual_file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<non-UTF-8 filename>");
    if actual_file_name == expected_file_name {
        return Ok(());
    }

    Err(format!(
        "{}: source filename '{}' must be '{}' for indicator ID '{}'; rename the file before installing",
        path.display(),
        actual_file_name,
        expected_file_name,
        id
    ))
}

/// Return deterministic diagnostics for duplicate short names in one catalog.
pub fn duplicate_short_name_diagnostics(sources: &[LoadedIndicatorSource]) -> Vec<String> {
    let mut grouped = BTreeMap::<&str, Vec<&LoadedIndicatorSource>>::new();
    for source in sources {
        grouped
            .entry(source.short_name.as_str())
            .or_default()
            .push(source);
    }

    grouped
        .into_iter()
        .filter_map(|(short_name, entries)| {
            (entries.len() > 1).then(|| {
                let mut diagnostic = format!("duplicate indicator short name '{short_name}':");
                for source in entries {
                    diagnostic.push_str(&format!("\n  {} ({})", source.path.display(), source.id));
                }
                diagnostic
            })
        })
        .collect()
}

fn collect_source_file(path: PathBuf, files: &mut Vec<PathBuf>) {
    if path.extension().and_then(|extension| extension.to_str()) == Some("ind") {
        files.push(path);
    }
}

#[cfg(test)]
pub(crate) fn repository_registrations() -> Vec<super::registry::IndicatorRegistration> {
    let report = super::bundled::load_core_sources();
    assert!(
        report.diagnostics.is_empty(),
        "repository indicator sources should load without diagnostics: {:?}",
        report.diagnostics
    );
    report.registrations
}
