use flowsurface_indicator_engine::{
    ChartFrame, CompileLimits, CompiledIndicator, Diagnostic, FrameOutput, IndicatorConfig,
    IndicatorFrame, IndicatorId, InstrumentContext, RuntimeLimits, Severity, TimeOrTick,
    TradeAggregate, compile,
};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

fn main() {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if let Some(output) = global_output(&arguments) {
        println!("{output}");
        return;
    }
    let result = match arguments.first().map(String::as_str) {
        Some("new") => command_new(&arguments[1..]),
        Some("check") => command_check(&arguments[1..], "check"),
        Some("test") => command_test(&arguments[1..]),
        Some(command) => Err(format!("unknown subcommand '{command}'")),
        None => Err(usage()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn usage() -> String {
    "usage: flowsurface-indicator <new|check|test> ...\n\n  flowsurface-indicator new [directory] --id <namespace.name> --title <Title>\n  flowsurface-indicator check <file.ind|directory> [--namespace <prefix>]\n  flowsurface-indicator test <file.ind|directory> [--namespace <prefix>]"
        .to_owned()
}

fn global_output(arguments: &[String]) -> Option<String> {
    match arguments.first().map(String::as_str) {
        Some("--help" | "-h") => Some(usage()),
        Some("--version" | "-V") => Some(env!("CARGO_PKG_VERSION").to_owned()),
        _ => None,
    }
}

fn command_new(arguments: &[String]) -> Result<(), String> {
    if wants_help(arguments) {
        println!(
            "usage: flowsurface-indicator new [directory] --id <namespace.name> --title <Title>"
        );
        return Ok(());
    }
    let options = parse_new_arguments(arguments)?;
    let id = IndicatorId::new(options.id.clone())
        .map_err(|error| format!("invalid indicator ID '{}': {error}", options.id))?;
    if options.title.trim().is_empty() {
        return Err("new requires a non-empty title".to_owned());
    }
    if options.directory.exists() && !options.directory.is_dir() {
        return Err(format!(
            "new destination {} is not a directory",
            options.directory.display()
        ));
    }
    let source_path = options.directory.join(id.source_file_name());
    match fs::symlink_metadata(&source_path) {
        Ok(_) => {
            return Err(format!(
                "refusing to overwrite existing target {}",
                source_path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "cannot inspect scaffold target {}: {error}",
                source_path.display()
            ));
        }
    }
    fs::create_dir_all(&options.directory)
        .map_err(|error| format!("cannot create {}: {error}", options.directory.display()))?;
    let indicator_line = format!(
        "indicator(\"{}\", id = \"{}\")\nformat(precision = \"base_ticker_min_tick\")",
        escape_string(&options.title),
        id.as_str()
    );
    let source = format!(
        "language(1)\n{indicator_line}\nperiod = input.int(14, \"Period\", min = 1, max = 200)\nvalue = ta.sma(chart.close, period)\nplot(value, id = \"value\", title = \"Value\")\n"
    );
    fs::write(&source_path, source)
        .map_err(|error| format!("cannot write scaffold {}: {error}", source_path.display()))?;
    println!("created {}", source_path.display());
    Ok(())
}

fn command_check(arguments: &[String], command: &str) -> Result<(), String> {
    if wants_help(arguments) {
        println!(
            "usage: flowsurface-indicator {command} <file.ind|directory> [--namespace <prefix>]"
        );
        return Ok(());
    }
    let (path, namespace) = parse_source_arguments(arguments, command)?;
    let paths = source_paths(&path)?;
    let report = load_sources(&paths, namespace.as_deref());
    print_diagnostics(&report.diagnostics);
    if !report.diagnostics.is_empty() {
        return Err(failure_summary(command, report.diagnostics.len()));
    }
    println!("{command} passed: {}", source_summary(report.sources.len()));
    Ok(())
}

fn command_test(arguments: &[String]) -> Result<(), String> {
    if wants_help(arguments) {
        println!("usage: flowsurface-indicator test <file.ind|directory> [--namespace <prefix>]");
        return Ok(());
    }
    let (path, namespace) = parse_source_arguments(arguments, "test")?;
    let paths = source_paths(&path)?;
    let report = load_sources(&paths, namespace.as_deref());
    print_diagnostics(&report.diagnostics);

    let mut failures = report.diagnostics.len();
    for source in &report.sources {
        match smoke_test(source) {
            Ok(()) => println!("test passed: {}", source.path.display()),
            Err(error) => {
                eprintln!("{}:1:1: error: {error}", source.path.display());
                failures += 1;
            }
        }
    }

    if failures == 0 {
        println!("test passed: {}", source_summary(report.sources.len()));
        Ok(())
    } else {
        Err(failure_summary("test", failures))
    }
}

fn validate_source_filename(path: &Path, id: &IndicatorId) -> Result<(), String> {
    let expected_file_name = id.source_file_name();
    let actual_file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<non-UTF-8 filename>");
    if actual_file_name == expected_file_name {
        return Ok(());
    }

    Err(format!(
        "{}:1:1: error: source filename '{}' must be '{}' for indicator ID '{}'; rename the file before installing",
        path.display(),
        actual_file_name,
        expected_file_name,
        id.as_str()
    ))
}

struct CompiledSource {
    path: PathBuf,
    indicator: CompiledIndicator,
}

struct SourceReport {
    sources: Vec<CompiledSource>,
    diagnostics: Vec<String>,
}

fn load_sources(paths: &[PathBuf], namespace: Option<&str>) -> SourceReport {
    let mut report = SourceReport {
        sources: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut ids = BTreeMap::<String, PathBuf>::new();
    let mut short_names = BTreeMap::<String, PathBuf>::new();
    let limits = CompileLimits::default();

    for path in paths {
        match fs::metadata(path) {
            Ok(metadata) if metadata.len() > limits.max_source_bytes as u64 => {
                report.diagnostics.push(format!(
                    "{}: source exceeds compile limit of {} bytes",
                    path.display(),
                    limits.max_source_bytes
                ));
                continue;
            }
            Err(error) => {
                report.diagnostics.push(format!(
                    "{}: could not read source: {error}",
                    path.display()
                ));
                continue;
            }
            Ok(_) => {}
        }
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                report.diagnostics.push(format!(
                    "{}: could not read source: {error}",
                    path.display()
                ));
                continue;
            }
        };
        let indicator = match compile(&source, path) {
            Ok(indicator) => indicator,
            Err(diagnostics) => {
                report.diagnostics.extend(
                    diagnostics
                        .iter()
                        .map(|diagnostic| format_diagnostic(diagnostic, &source)),
                );
                continue;
            }
        };

        let id = indicator.descriptor.id.as_str().to_owned();
        let short_name = indicator.descriptor.id.short_name().to_owned();
        let mut invalid = false;
        if let Some(namespace) = namespace {
            let has_namespace = id
                .strip_prefix(namespace)
                .is_some_and(|suffix| suffix.starts_with('.'));
            if !has_namespace {
                report.diagnostics.push(format!(
                    "{}:1:1: error: indicator ID '{}' must use namespace '{}'",
                    path.display(),
                    id,
                    namespace
                ));
                invalid = true;
            }
        }
        if let Err(diagnostic) = validate_source_filename(path, &indicator.descriptor.id) {
            report.diagnostics.push(diagnostic);
            invalid = true;
        }
        if let Some(previous) = ids.get(&id) {
            report.diagnostics.push(format!(
                "{}:1:1: error: duplicate indicator ID '{}' (already defined in {})",
                path.display(),
                id,
                previous.display()
            ));
            invalid = true;
        } else {
            ids.insert(id, path.clone());
        }
        if let Some(previous) = short_names.get(&short_name) {
            report.diagnostics.push(format!(
                "{}:1:1: error: duplicate indicator short name '{}' (already defined in {})",
                path.display(),
                short_name,
                previous.display()
            ));
            invalid = true;
        } else {
            short_names.insert(short_name, path.clone());
        }
        if invalid {
            continue;
        }
        report.sources.push(CompiledSource {
            path: path.clone(),
            indicator,
        });
    }

    report
}

fn print_diagnostics(diagnostics: &[String]) {
    for diagnostic in diagnostics {
        eprintln!("{diagnostic}");
    }
}

fn failure_summary(command: &str, failures: usize) -> String {
    format!(
        "{command} failed: {failures} diagnostic{}",
        if failures == 1 { "" } else { "s" }
    )
}

fn source_summary(count: usize) -> String {
    format!("{count} source{}", if count == 1 { "" } else { "s" })
}

fn smoke_test(source: &CompiledSource) -> Result<(), String> {
    let config = IndicatorConfig::default();
    let limits = RuntimeLimits::untrusted();
    let warmup = source
        .indicator
        .warmup_bars_with_limits(&config, limits)
        .unwrap_or(source.indicator.descriptor.max_bars_back);
    let frame_count = warmup.saturating_add(2).max(32);
    let mut instance = source
        .indicator
        .instantiate_with_limits(config, limits)
        .map_err(|error| format!("could not instantiate indicator: {error}"))?;

    for index in 0..frame_count {
        let frame = smoke_frame(index);
        let output = instance
            .evaluate_closed(frame.clone())
            .map_err(|error| format!("closed frame {index} failed: {error}"))?;
        validate_output(&source.indicator, &frame, &output)?;
    }

    let frame = smoke_frame(frame_count);
    let output = instance
        .preview_open(frame.clone())
        .map_err(|error| format!("open preview at frame {frame_count} failed: {error}"))?;
    validate_output(&source.indicator, &frame, &output)
}

fn smoke_frame(index: usize) -> IndicatorFrame {
    let step = index as f64;
    let close = 100.0 + step * 0.25 + (index % 7) as f64;
    IndicatorFrame {
        index,
        time: (index as i64).saturating_mul(60_000),
        basis: TimeOrTick::Time,
        instrument: InstrumentContext {
            min_tick: Some(0.1),
        },
        chart: ChartFrame {
            open: Some(close - 0.5),
            high: Some(close + 1.0),
            low: Some(close - 1.0),
            close: Some(close),
            volume: Some(1_000.0 + step),
            directional_volume_available: true,
            buy_qty: 40.0 + (index % 11) as f64,
            sell_qty: 30.0 + (index % 9) as f64,
            trade: Some(TradeAggregate {
                buy_qty: 12.0 + (index % 5) as f64,
                sell_qty: 8.0 + (index % 4) as f64,
                buy_count: 3 + (index % 6) as u64,
                sell_count: 2 + (index % 5) as u64,
            }),
        },
        open_interest: Some(10_000.0 + step * 2.0),
        series_inputs: Vec::new().into(),
    }
}

fn validate_output(
    indicator: &CompiledIndicator,
    frame: &IndicatorFrame,
    output: &FrameOutput,
) -> Result<(), String> {
    if output.index != frame.index || output.time != frame.time {
        return Err(format!(
            "output identity mismatch: expected index {} at {}, got index {} at {}",
            frame.index, frame.time, output.index, output.time
        ));
    }

    let output_ids = output
        .plots
        .iter()
        .map(|plot| plot.id.as_str())
        .collect::<BTreeSet<_>>();
    for plot in &indicator.descriptor.plots {
        if !output_ids.contains(plot.id.as_str()) {
            return Err(format!("declared plot '{}' was not emitted", plot.id));
        }
    }
    Ok(())
}

struct NewArguments {
    directory: PathBuf,
    id: String,
    title: String,
}

fn parse_new_arguments(arguments: &[String]) -> Result<NewArguments, String> {
    let mut directory = PathBuf::from(".");
    let mut id = None;
    let mut title = None;
    let mut index = 0;
    if arguments
        .first()
        .is_some_and(|argument| !argument.starts_with('-'))
    {
        directory = PathBuf::from(&arguments[0]);
        index = 1;
    }
    while index < arguments.len() {
        let option = arguments[index].as_str();
        match option {
            "--id" | "--title" => {
                let Some(value) = arguments
                    .get(index + 1)
                    .filter(|value| !value.starts_with('-'))
                else {
                    return Err(format!("new requires {option} <value>"));
                };
                let target = if option == "--id" {
                    &mut id
                } else {
                    &mut title
                };
                if target.replace(value.clone()).is_some() {
                    return Err(format!("new received duplicate {option}"));
                }
                index += 2;
            }
            _ => return Err(format!("new received unknown argument '{option}'")),
        }
    }
    Ok(NewArguments {
        directory,
        id: id.ok_or_else(|| "new requires --id <namespace.name>".to_owned())?,
        title: title.ok_or_else(|| "new requires --title <Title>".to_owned())?,
    })
}

fn parse_source_arguments(
    arguments: &[String],
    command: &str,
) -> Result<(PathBuf, Option<String>), String> {
    let mut path = None;
    let mut namespace = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--namespace" => {
                let Some(value) = arguments
                    .get(index + 1)
                    .filter(|value| !value.starts_with('-'))
                else {
                    return Err(format!("{command} requires --namespace <prefix>"));
                };
                if namespace.replace(value.clone()).is_some() {
                    return Err(format!("{command} received duplicate --namespace"));
                }
                validate_namespace_prefix(value)?;
                index += 2;
            }
            argument if argument.starts_with('-') => {
                return Err(format!("{command} received unknown argument '{argument}'"));
            }
            _ => {
                if path.replace(PathBuf::from(&arguments[index])).is_some() {
                    return Err(format!(
                        "{command} expects exactly one .ind source file or directory"
                    ));
                }
                index += 1;
            }
        }
    }
    let path =
        path.ok_or_else(|| format!("{command} expects exactly one .ind source file or directory"))?;
    if !path.exists() {
        return Err(format!("{}: source path does not exist", path.display()));
    }
    if path.is_file() && path.extension().and_then(|extension| extension.to_str()) != Some("ind") {
        return Err(format!(
            "{}: indicator source files must use the .ind extension",
            path.display()
        ));
    }
    if !path.is_file() && !path.is_dir() {
        return Err(format!(
            "{}: source path is not a file or directory",
            path.display()
        ));
    }
    Ok((path, namespace))
}

fn validate_namespace_prefix(namespace: &str) -> Result<(), String> {
    IndicatorId::new(format!("{namespace}.indicator"))
        .map(|_| ())
        .map_err(|error| format!("invalid namespace prefix '{namespace}': {error}"))
}

fn source_paths(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_owned()]);
    }

    let mut paths = fs::read_dir(path)
        .map_err(|error| format!("{}: could not read directory: {error}", path.display()))?
        .map(|entry| {
            entry
                .map_err(|error| {
                    format!(
                        "{}: could not read directory entry: {error}",
                        path.display()
                    )
                })
                .map(|entry| entry.path())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|entry| {
        entry.is_file() && entry.extension().and_then(|extension| extension.to_str()) == Some("ind")
    });
    paths.sort();
    if paths.is_empty() {
        return Err(format!(
            "{}: directory contains no .ind sources",
            path.display()
        ));
    }
    Ok(paths)
}

fn format_diagnostic(diagnostic: &Diagnostic, source: &str) -> String {
    let (line, column) = line_column(source, diagnostic.span.start);
    format!(
        "{}:{line}:{column}: {}: {}",
        diagnostic.path.display(),
        match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        },
        diagnostic.message
    )
}

fn line_column(source: &str, character_offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for character in source.chars().take(character_offset) {
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn wants_help(arguments: &[String]) -> bool {
    arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
}

fn escape_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => escaped.push(' '),
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn cli_commands_validate_sources_and_create_without_overwriting() {
        let parsed = parse_new_arguments(&arguments(&[
            "--id",
            "community.example",
            "--title",
            "Example",
        ]))
        .expect("new arguments should parse");
        assert_eq!(parsed.directory, PathBuf::from("."));
        assert_eq!(parsed.id, "community.example");

        let root = std::env::temp_dir().join(format!(
            "flowsurface-indicator-cli-new-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let directory = root.join("collection");
        command_new(&arguments(&[
            directory.to_str().expect("temporary path should be UTF-8"),
            "--id",
            "community.example",
            "--title",
            "Example",
        ]))
        .expect("new should create a source");

        assert!(directory.join("example.ind").is_file());
        assert!(!directory.join("community.example.ind").exists());
        let overwrite = command_new(&arguments(&[
            directory.to_str().expect("temporary path should be UTF-8"),
            "--id",
            "community.example",
            "--title",
            "Example",
        ]))
        .expect_err("new should not overwrite an existing source");
        assert!(overwrite.contains("refusing to overwrite"));

        command_check(&[directory.to_string_lossy().into_owned()], "check")
            .expect("directory should validate");

        let mismatch = root.join("mismatch");
        fs::create_dir_all(&mismatch).expect("mismatch directory should exist");
        let path = mismatch.join("community.example.ind");
        fs::write(
            &path,
            "language(1)\nindicator(\"Example\", id = \"community.example\")\ndisplay(view = \"plot\")\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\n",
        )
        .expect("source should exist");
        let check_error = command_check(&[path.to_string_lossy().into_owned()], "check")
            .expect_err("check should reject a nonmatching stem");
        assert!(check_error.contains("1 diagnostic"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn directory_validation_reports_invalid_sources_and_duplicates() {
        let root = std::env::temp_dir().join(format!(
            "flowsurface-indicator-cli-directory-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary directory should exist");
        fs::write(
            root.join("first.ind"),
            "language(1)\nindicator(\"First\", id = \"test.first\")\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\n",
        )
        .expect("first source should be written");
        fs::write(
            root.join("second.ind"),
            "language(1)\nindicator(\"Second\", id = \"test.second\")\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\n",
        )
        .expect("second source should be written");

        fs::write(root.join("broken.ind"), "language(1)\nindicator(\n")
            .expect("broken source should be written");

        let error = command_check(&[root.to_string_lossy().into_owned()], "check")
            .expect_err("invalid sources should fail");
        assert!(error.contains("1 diagnostic"));

        let empty = root.join("empty");
        fs::create_dir_all(&empty).expect("empty directory should exist");
        let error = command_check(&[empty.to_string_lossy().into_owned()], "check")
            .expect_err("empty directories should fail");
        assert!(error.contains("contains no .ind sources"));

        let first = root.join("one");
        let second = root.join("two");
        fs::create_dir_all(&first).expect("first duplicate directory should exist");
        fs::create_dir_all(&second).expect("second duplicate directory should exist");
        let first_path = first.join("same.ind");
        let second_path = second.join("same.ind");
        let source = "language(1)\nindicator(\"Same\", id = \"test.same\")\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\n";
        fs::write(&first_path, source).expect("first duplicate source should be written");
        fs::write(&second_path, source).expect("second duplicate source should be written");
        let report = load_sources(&[first_path, second_path], None);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("duplicate indicator ID"))
        );

        let short_first = root.join("short-one");
        let short_second = root.join("short-two");
        fs::create_dir_all(&short_first).expect("first short-name directory should exist");
        fs::create_dir_all(&short_second).expect("second short-name directory should exist");
        let short_first_path = short_first.join("shared.ind");
        let short_second_path = short_second.join("shared.ind");
        fs::write(
            &short_first_path,
            "language(1)\nindicator(\"One\", id = \"test.one\")\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\n",
        )
        .expect("first short-name source should be written");
        fs::write(
            &short_second_path,
            "language(1)\nindicator(\"Two\", id = \"other.one\")\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\n",
        )
        .expect("second short-name source should be written");
        let report = load_sources(&[short_first_path, short_second_path], None);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("duplicate indicator short name"))
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_formats_compile_diagnostics() {
        let root = std::env::temp_dir().join(format!(
            "flowsurface-indicator-cli-diagnostics-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary directory should exist");
        let path = root.join("broken.ind");
        fs::write(
            &path,
            "language(1)\nindicator(\"Broken\", id = \"test.broken\")\nplot(\n",
        )
        .expect("broken source should be written");

        let error = command_check(&[path.to_string_lossy().into_owned()], "check")
            .expect_err("broken source should fail");
        assert!(error.contains("diagnostic"));

        let type_path = root.join("type-error.ind");
        fs::write(
            &type_path,
            "language(1)\nindicator(\"Type error\", id = \"test.type-error\")\nformat(precision = \"base_ticker_min_tick\")\nvalue = chart.close + true\nplot(value, id = \"value\")\n",
        )
        .expect("type-error source should be written");
        let report = load_sources(&[type_path], None);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("invalid binary operands"))
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn test_runs_ta_smoke_and_reports_runtime_failures() {
        let root = std::env::temp_dir().join(format!(
            "flowsurface-indicator-cli-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary directory should exist");

        fs::write(
            root.join("smoke.ind"),
            "language(1)\nindicator(\"Smoke\", id = \"test.smoke\")\nformat(precision = \"base_ticker_min_tick\")\nlength = input.int(2, \"Length\", min = 1, max = 10)\nplot(ta.sma(chart.close, length), id = \"value\")\n",
        )
        .expect("smoke source should be written");
        command_test(&[root.to_string_lossy().into_owned()]).expect("TA smoke source should pass");

        fs::write(
            root.join("smoke.ind"),
            "language(1)\nindicator(\"Smoke\", id = \"test.smoke\")\nformat(precision = \"base_ticker_min_tick\")\nwhile true {}\nplot(chart.close, id = \"value\")\n",
        )
        .expect("runtime failure source should be written");
        let error = command_test(&[root.to_string_lossy().into_owned()])
            .expect_err("runtime failure should fail");
        assert!(error.contains("diagnostic"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn namespace_filter_accepts_matching_ids_and_rejects_other_ids() {
        let root = std::env::temp_dir().join(format!(
            "flowsurface-indicator-cli-namespace-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary directory should exist");
        let path = root.join("valid.ind");
        fs::write(
            &path,
            "language(1)\nindicator(\"Valid\", id = \"catalog.valid\")\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\n",
        )
        .expect("source should be written");
        command_check(
            &arguments(&[
                path.to_str().expect("temporary path should be UTF-8"),
                "--namespace",
                "catalog",
            ]),
            "check",
        )
        .expect("matching namespace should pass");

        fs::write(
            &path,
            "language(1)\nindicator(\"Valid\", id = \"core.valid\")\nformat(precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\n",
        )
        .expect("source should be rewritten");
        let report = load_sources(&[path], Some("catalog"));
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("must use namespace 'catalog'"))
        );

        let _ = fs::remove_dir_all(root);
    }
}
