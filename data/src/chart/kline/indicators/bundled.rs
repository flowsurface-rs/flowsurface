use super::{IndicatorId, catalog, language};
use std::path::PathBuf;

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

pub(super) fn load_core_sources() -> language::IndicatorLoadReport {
    language::load_compiled_report(
        CORE_SOURCES.iter().map(|source| {
            let path = PathBuf::from("core").join(source.file_name);
            let compiled = indicator_engine::compile(source.source, &path);
            (path, compiled)
        }),
        language::PackageOrigin::Core,
    )
}
