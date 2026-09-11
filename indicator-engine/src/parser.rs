mod ast;
mod compiler;
mod lexer;
mod syntax;

use crate::CompileLimits;
use crate::diagnostics::Diagnostic;
use crate::intrinsic::IntrinsicRegistry;
use std::path::Path;

pub(crate) fn compile_with_registry(
    source: &str,
    path: &Path,
    registry: IntrinsicRegistry,
    limits: CompileLimits,
) -> Result<crate::CompiledIndicator, Vec<Diagnostic>> {
    let lexed = lexer::lex(source, path, limits);
    let mut diagnostics = lexed.diagnostics;
    let parsed = syntax::parse(lexed.tokens, path.to_path_buf());
    diagnostics.extend(parsed.diagnostics);
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    ast::validate_ast_limits(&parsed.ast, path, limits, &mut diagnostics);
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let compiled = compiler::compile(parsed.ast, path, registry);
    diagnostics.extend(compiled.diagnostics);
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == crate::Severity::Error)
    {
        Err(diagnostics)
    } else {
        Ok(crate::CompiledIndicator {
            descriptor: compiled.descriptor,
            requirements: compiled.requirements,
            program: compiled.program,
            registry: compiled.registry,
        })
    }
}
