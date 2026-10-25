use super::{IndicatorRegistration, language};
use crate::data_path;
use log::warn;
use std::fs;
use std::path::{Path, PathBuf};

/// Filesystem sources used to build the process-wide indicator catalog.
///
/// Bundled sources are managed by the data crate, while installed paths
/// belong to the user or a development workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndicatorCatalogPaths {
    /// Overrides embedded bundled sources when set.
    pub bundled_directory: Option<PathBuf>,
    pub installed_directory: PathBuf,
    pub additional_files: Vec<PathBuf>,
}

impl IndicatorCatalogPaths {
    fn from_data_directory() -> Self {
        Self {
            bundled_directory: None,
            installed_directory: installed_indicator_directory(),
            additional_files: Vec::new(),
        }
    }
}

fn installed_indicator_directory() -> PathBuf {
    std::env::var_os("FLOWSURFACE_INDICATORS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| data_path(Some("indicators")))
}

fn installed_sources(directory: &Path) -> Result<Vec<language::LoadedIndicatorSource>, String> {
    let (files, diagnostics) = language::indicator_files_in_directory(directory)?;
    if !diagnostics.is_empty() {
        return Err(diagnostics.join("\n"));
    }
    Ok(language::load_from_files_report(&files, language::PackageOrigin::Installed).loaded_sources)
}

pub fn write_indicator_source(id: &super::IndicatorId, source: &[u8]) -> Result<(), String> {
    if super::catalog()
        .get(id)
        .is_some_and(|registration| registration.origin == language::PackageOrigin::Bundled)
    {
        return Err("Bundled indicators cannot be replaced".to_owned());
    }

    let directory = installed_indicator_directory();
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    if installed_sources(&directory)?
        .iter()
        .any(|loaded| loaded.id == id.as_str())
    {
        return Err(
            "An indicator with this ID already exists in the indicators directory".to_owned(),
        );
    }

    let (namespace, name) = id
        .as_str()
        .rsplit_once('.')
        .expect("Validated indicator ID has a namespace");
    let destination_directory = directory.join(namespace);
    let destination = destination_directory.join(format!("{name}.ind"));

    let source_text = std::str::from_utf8(source).map_err(|error| error.to_string())?;
    let report = language::load_compiled_report(
        [(
            destination.clone(),
            indicator_engine::compile(source_text, &destination),
        )],
        language::PackageOrigin::Installed,
    );
    if !report.diagnostics.is_empty() {
        return Err(report.diagnostics.join("\n"));
    }
    if report.registrations.len() != 1 || report.registrations[0].id != *id {
        return Err("Downloaded indicator does not declare the expected ID".to_owned());
    }

    fs::create_dir_all(&destination_directory).map_err(|error| error.to_string())?;
    // A non-source extension keeps interrupted writes out of source discovery on the same filesystem.
    let temporary = destination_directory.join(format!(".install-{}.part", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let result = (|| {
        use std::io::Write;
        file.write_all(source).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);

        // A hard link publishes the complete file atomically and refuses to replace an existing path.
        fs::hard_link(&temporary, &destination).map_err(|error| error.to_string())
    })();
    if let Err(error) = fs::remove_file(&temporary) {
        log::warn!("Could not remove indicator installation staging file: {error}");
    }
    result
}

pub fn remove_indicator_source(id: &super::IndicatorId) -> Result<(), String> {
    if super::catalog()
        .get(id)
        .is_some_and(|registration| registration.origin == language::PackageOrigin::Bundled)
    {
        return Err("Bundled indicators cannot be removed".to_owned());
    }

    let installed = installed_sources(&installed_indicator_directory())?;
    let mut sources = installed.iter().filter(|source| source.id == id.as_str());
    let source = sources.next().ok_or_else(|| {
        "No matching source was found in the configured indicators directory".to_owned()
    })?;
    if sources.next().is_some() {
        return Err(
            "Multiple sources declare this indicator ID; remove the conflicting files manually"
                .to_owned(),
        );
    }
    fs::remove_file(&source.path).map_err(|error| error.to_string())
}

pub fn initialize_indicator_catalog() -> Result<(), String> {
    let additional_files =
        indicator_paths_from_args().map_err(|error| format!("Invalid indicator path: {error}"))?;
    let mut paths = IndicatorCatalogPaths::from_data_directory();
    paths.additional_files.extend(additional_files);

    initialize_indicator_catalog_with_paths(&paths)
        .map_err(|error| format!("Failed to load indicators: {error}"))
}

pub fn initialize_indicator_catalog_with_paths(
    paths: &IndicatorCatalogPaths,
) -> Result<(), String> {
    let sources = load_catalog_sources(paths)?;
    let bundled_ids = sources
        .bundled
        .iter()
        .map(|registration| registration.id.clone())
        .collect::<Vec<_>>();
    super::install_catalog_with_bundled_diagnostics(
        sources.bundled,
        sources.installed,
        sources.diagnostics,
    )
    .map_err(|error| error.to_string())?;
    super::bundled::validate_bundled_catalog(&bundled_ids)
        .map_err(|error| format!("Failed to load bundled indicators: {error}"))
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
    bundled: Vec<IndicatorRegistration>,
    installed: Vec<IndicatorRegistration>,
    diagnostics: Vec<String>,
}

fn load_catalog_sources(paths: &IndicatorCatalogPaths) -> Result<IndicatorCatalogSources, String> {
    let bundled_report = match &paths.bundled_directory {
        Some(directory) => {
            let (files, diagnostics) = language::indicator_files_in_directory(directory)?;
            let mut report =
                language::load_from_files_report(&files, language::PackageOrigin::Bundled);
            report.diagnostics.extend(diagnostics);
            report
        }
        None => super::bundled::load_bundled_sources(),
    };
    if !bundled_report.diagnostics.is_empty() {
        return Err(format!(
            "bundled indicator source validation failed:\n{}",
            bundled_report.diagnostics.join("\n")
        ));
    }
    if bundled_report.registrations.is_empty() {
        return Err("bundled indicator collection contains no valid sources".to_owned());
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
    let installed_report =
        language::load_from_files_report(&installed_files, language::PackageOrigin::Installed);
    for diagnostic in &installed_report.diagnostics {
        warn!("Ignoring invalid installed indicator source: {diagnostic}");
    }
    diagnostics.extend(installed_report.diagnostics);

    Ok(IndicatorCatalogSources {
        bundled: bundled_report.registrations,
        installed: installed_report.registrations,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{catalog, language::PackageOrigin};
    use super::*;

    fn source(id: &str) -> String {
        format!(
            "language(1)\nindicator(\"Example\", id = {id:?})\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close)"
        )
    }

    #[test]
    fn origins_and_full_ids_control_collection_loading() {
        let root =
            std::env::temp_dir().join(format!("flowsurface-host-loading-{}", std::process::id()));
        let bundled = root.join("bundled");
        let installed = root.join("installed");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::create_dir_all(installed.join("alice")).unwrap();
        std::fs::create_dir_all(installed.join("bob")).unwrap();
        std::fs::write(bundled.join("rsi.ind"), source("catalog.rsi")).unwrap();
        std::fs::write(installed.join("alice/rsi.ind"), source("core.rsi")).unwrap();
        std::fs::write(installed.join("bob/rsi.ind"), source("bob.rsi")).unwrap();
        std::fs::write(installed.join("broken.ind"), "language(1)\nindicator(").unwrap();
        let paths = IndicatorCatalogPaths {
            bundled_directory: Some(bundled.clone()),
            installed_directory: installed,
            additional_files: Vec::new(),
        };
        let sources = load_catalog_sources(&paths).unwrap();
        assert_eq!(sources.bundled.len(), 1);
        assert_eq!(sources.installed.len(), 2);
        assert_eq!(sources.bundled[0].origin, PackageOrigin::Bundled);
        assert!(
            sources
                .installed
                .iter()
                .all(|registration| registration.origin == PackageOrigin::Installed)
        );
        assert!(!sources.diagnostics.is_empty());
        std::fs::write(bundled.join("broken.ind"), "language(1)\nindicator(").unwrap();
        assert!(
            load_catalog_sources(&paths)
                .unwrap_err()
                .contains("bundled indicator source validation failed")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn embedded_bundle_initializes_without_a_source_directory() {
        let root =
            std::env::temp_dir().join(format!("flowsurface-host-embedded-{}", std::process::id()));
        let paths = IndicatorCatalogPaths {
            bundled_directory: None,
            installed_directory: root.clone(),
            additional_files: Vec::new(),
        };
        initialize_indicator_catalog_with_paths(&paths).unwrap();
        let report = super::super::bundled::load_bundled_sources();
        assert!(!report.registrations.is_empty());
        assert_eq!(catalog().registrations().len(), report.registrations.len());
        assert!(
            catalog()
                .registrations()
                .all(|registration| registration.origin == PackageOrigin::Bundled)
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
