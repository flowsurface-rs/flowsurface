use super::{IndicatorRegistration, language};
use crate::data_path;
use log::warn;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// Filesystem sources used to build the process-wide indicator catalog.
///
/// Core sources are managed by the data crate, while installed paths
/// belong to the user or a development workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndicatorCatalogPaths {
    /// Overrides embedded core sources when set.
    pub core_directory: Option<PathBuf>,
    pub installed_directory: PathBuf,
    pub additional_files: Vec<PathBuf>,
}

impl IndicatorCatalogPaths {
    pub fn from_data_directory() -> Self {
        let indicators = data_path(Some("indicators"));
        Self {
            core_directory: None,
            installed_directory: indicators.join("installed"),
            additional_files: Vec::new(),
        }
    }

    /// Use a repository-style source tree with `core/` and `catalog/sources/` collections.
    pub fn from_indicator_root(root: impl Into<PathBuf>) -> Self {
        let indicators = root.into();
        Self {
            core_directory: Some(indicators.join("core")),
            installed_directory: indicators.join("catalog").join("sources"),
            additional_files: Vec::new(),
        }
    }
}

pub fn initialize_indicator_catalog() -> Result<(), String> {
    let additional_files =
        indicator_paths_from_args().map_err(|error| format!("Invalid indicator path: {error}"))?;
    let mut paths = match std::env::var_os("FLOWSURFACE_INDICATORS_PATH") {
        Some(root) => IndicatorCatalogPaths::from_indicator_root(root),
        None => IndicatorCatalogPaths::from_data_directory(),
    };
    paths.additional_files.extend(additional_files);

    initialize_indicator_catalog_with_paths(&paths)
        .map_err(|error| format!("Failed to load indicators: {error}"))?;

    super::bundled::validate_core_catalog()
        .map_err(|error| format!("Failed to load core indicators: {error}"))
}

pub fn initialize_indicator_catalog_with_paths(
    paths: &IndicatorCatalogPaths,
) -> Result<(), String> {
    let sources = load_catalog_sources(paths)?;
    super::install_catalog_with_core_diagnostics(
        sources.core,
        sources.installed,
        sources.diagnostics,
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
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

#[derive(Debug)]
struct IndicatorCatalogSources {
    core: Vec<IndicatorRegistration>,
    installed: Vec<IndicatorRegistration>,
    diagnostics: Vec<String>,
}

fn load_catalog_sources(paths: &IndicatorCatalogPaths) -> Result<IndicatorCatalogSources, String> {
    let core_report = match &paths.core_directory {
        Some(directory) => {
            let (files, diagnostics) = language::indicator_files_in_directory(directory)?;
            let mut report =
                language::load_from_files_report(&files, language::PackageOrigin::Core);
            report.diagnostics.extend(diagnostics);
            report
        }
        None => super::bundled::load_core_sources(),
    };
    if !core_report.diagnostics.is_empty() {
        return Err(format!(
            "core indicator source validation failed:\n{}",
            core_report.diagnostics.join("\n")
        ));
    }
    if core_report.registrations.is_empty() {
        return Err("core indicator collection contains no valid sources".to_owned());
    }

    let mut diagnostics = Vec::new();
    let mut installed_files = if let Err(error) = fs::create_dir_all(&paths.installed_directory) {
        diagnostics.push(format!(
            "could not create installed indicator directory {}: {error}",
            paths.installed_directory.display()
        ));
        Vec::new()
    } else {
        match language::indicator_files_in_directory(&paths.installed_directory) {
            Ok((files, discovery_diagnostics)) => {
                diagnostics.extend(discovery_diagnostics);
                files
            }
            Err(error) => {
                diagnostics.push(error);
                Vec::new()
            }
        }
    };
    installed_files.extend(paths.additional_files.iter().cloned());
    let mut installed_report =
        language::load_from_files_report(&installed_files, language::PackageOrigin::Installed);
    for diagnostic in &installed_report.diagnostics {
        warn!("Ignoring invalid installed indicator source: {diagnostic}");
    }
    diagnostics.extend(installed_report.diagnostics);

    let mut short_names = core_report
        .loaded_sources
        .iter()
        .map(|source| (source.short_name.as_str(), source))
        .collect::<BTreeMap<_, _>>();
    let mut installed_sources = BTreeMap::new();
    for source in &installed_report.loaded_sources {
        installed_sources
            .entry(source.id.as_str())
            .or_insert(source);
    }
    installed_report.registrations.retain(|registration| {
        let source = installed_sources[registration.id.id()];
        if let Some(previous) = short_names.get(source.short_name.as_str()) {
            diagnostics.push(format!(
                "{}: installed indicator '{}' ignored: duplicate indicator short name '{}' already provided by '{}' in {}",
                source.path.display(),
                source.id,
                source.short_name,
                previous.id,
                previous.path.display()
            ));
            false
        } else {
            short_names.insert(source.short_name.as_str(), source);
            true
        }
    });

    Ok(IndicatorCatalogSources {
        core: core_report.registrations,
        installed: installed_report.registrations,
        diagnostics,
    })
}
