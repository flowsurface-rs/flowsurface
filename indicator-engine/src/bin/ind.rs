use flowsurface_indicator_engine::{Diagnostic, IndicatorId, Severity, compile_file};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

fn main() {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        println!("{}", usage());
        return;
    }
    if arguments
        .first()
        .is_some_and(|argument| matches!(argument.as_str(), "--version" | "-V"))
    {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let result = match arguments.first().map(String::as_str) {
        Some("new") => command_new(&arguments[1..]),
        Some("check") => command_check(&arguments[1..], "check"),
        Some(command) => Err(format!("unknown subcommand '{command}'")),
        None => Err(usage()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn usage() -> String {
    "usage: flowsurface-indicator <new|check> ...\n\n  flowsurface-indicator new [directory] --id <namespace.name> --title <Title>\n  flowsurface-indicator check <file.ind>".to_owned()
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
        "indicator(\"{}\", id = \"{}\", value_precision = \"base_ticker_min_tick\")",
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
        println!("usage: flowsurface-indicator {command} <file.ind>");
        return Ok(());
    }
    let path = parse_source_argument(arguments, command)?;
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("{}: could not read source: {error}", path.display()))?;
    match compile_file(&path) {
        Ok(compiled) => {
            if let Err(diagnostic) = validate_source_filename(&path, &compiled.descriptor.id) {
                eprintln!("{diagnostic}");
                return Err("1 error".to_owned());
            }
            println!("{command} passed: {}", path.display());
            Ok(())
        }
        Err(diagnostics) => {
            for diagnostic in &diagnostics {
                eprintln!("{}", format_diagnostic(diagnostic, &source));
            }
            let count = diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == Severity::Error)
                .count();
            Err(format!(
                "{} error{}",
                count,
                if count == 1 { "" } else { "s" }
            ))
        }
    }
}

fn validate_source_filename(path: &std::path::Path, id: &IndicatorId) -> Result<(), String> {
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

fn parse_source_argument(arguments: &[String], command: &str) -> Result<PathBuf, String> {
    if arguments.len() != 1 || arguments[0].starts_with('-') {
        return Err(format!("{command} expects exactly one .ind source file"));
    }
    let path = PathBuf::from(&arguments[0]);
    if path.is_dir() {
        return Err(format!(
            "{}: expected one explicit .ind source file, not a directory",
            path.display()
        ));
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("ind") {
        return Err(format!(
            "{}: indicator source files must use the .ind extension",
            path.display()
        ));
    }
    if !path.is_file() {
        return Err(format!("{}: source file does not exist", path.display()));
    }
    Ok(path)
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

        let path = directory.join("community.example.ind");
        fs::write(
            &path,
            "language(1)\nindicator(\"Example\", id = \"community.example\", view = \"plot\", value_precision = \"base_ticker_min_tick\")\nplot(chart.close, id = \"value\")\n",
        )
        .expect("source should exist");

        let check_error = command_check(&[path.to_string_lossy().into_owned()], "check")
            .expect_err("check should reject a nonmatching stem");
        assert_eq!(check_error, "1 error");
        let directory_error =
            parse_source_argument(&[directory.to_string_lossy().into_owned()], "check")
                .expect_err("directories should be rejected");
        assert!(directory_error.contains("not a directory"));

        let _ = fs::remove_dir_all(root);
    }
}
