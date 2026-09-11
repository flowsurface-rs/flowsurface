mod context;
mod expressions;
mod function_index;
mod metadata;
mod outputs;
mod presentation;
mod presentation_defaults;
mod statements;
mod validation;

use self::context::LoweringContext;
use self::function_index::index_functions;
use super::ast::AstProgram;
use crate::DataRequirements;
use crate::diagnostics::Diagnostic;
use crate::hir::HirProgram;
use crate::intrinsic::IntrinsicRegistry;
use crate::scene::PresentationViewKind;
use crate::types::IndicatorDescriptor;
use std::path::Path;

pub(super) struct CompileOutput {
    pub(super) descriptor: IndicatorDescriptor,
    pub(super) program: HirProgram,
    pub(super) requirements: DataRequirements,
    pub(super) registry: IntrinsicRegistry,
    pub(super) diagnostics: Vec<Diagnostic>,
}

pub(super) fn compile(ast: AstProgram, path: &Path, registry: IntrinsicRegistry) -> CompileOutput {
    let AstProgram {
        indicator,
        functions,
        enums,
        statements,
    } = ast;
    let mut ctx = LoweringContext::new(path.to_path_buf(), registry);
    let descriptor = ctx.compile_descriptor(indicator.as_ref());
    ctx.presentation.data = descriptor.presentation.clone();
    ctx.declarations.descriptor = Some(descriptor);

    for item in enums {
        ctx.error(
            item.span,
            "enum declarations are not available; use strings or integers",
        );
    }

    let index = index_functions(&functions, path);
    ctx.diagnostics.extend(index.diagnostics);
    ctx.function.by_name = index.by_name;
    ctx.function.signatures = index.signatures;
    ctx.ir.program.functions = index.stubs;
    ctx.function.call_edges = vec![Vec::new(); ctx.ir.program.functions.len()];

    for statement in statements {
        if let Some(statement) = ctx.compile_statement(statement) {
            ctx.ir.program.blocks[0].statements.push(statement);
        }
    }
    for (function, function_id) in functions.into_iter().zip(index.source_ids) {
        ctx.compile_function(function, function_id);
    }

    ctx.validate_history_sources();
    ctx.reject_recursive_functions();
    let mut descriptor = ctx
        .declarations
        .descriptor
        .take()
        .expect("descriptor is initialized");
    if descriptor.max_bars_back > 50_000 {
        ctx.error(
            crate::diagnostics::Span::new(0, 0),
            "history requirements cannot exceed the host maximum of 50000 bars",
        );
        descriptor.max_bars_back = 50_000;
    }
    descriptor.inputs = std::mem::take(&mut ctx.declarations.inputs);
    descriptor.series_inputs = ctx.declarations.series_inputs.clone();
    ctx.validate_inputs(&descriptor);
    ctx.ensure_default_plot_styles();
    let indicator_overlay = matches!(
        ctx.presentation.data.view,
        PresentationViewKind::PrimaryOverlay
    );
    ctx.ensure_default_presentation_layers(indicator_overlay);
    descriptor.plots = ctx.ir.program.plots.clone();
    descriptor.presentation = ctx.presentation.data.clone();
    ctx.validate_presentation(&descriptor);
    ctx.ir.program.locals = ctx.scope.next_local;
    ctx.ir.program.states = ctx.scope.next_state;
    ctx.ir.program.state_call_sites = ctx.ir.state_call_sites.clone();
    ctx.ir.program.state_functions = ctx.ir.state_functions.clone();
    ctx.ir.program.series = ctx.ir.next_series;
    ctx.ir.program.retained_series = ctx.retained_series();
    ctx.ir.program.call_sites = ctx.ir.next_call_site;
    ctx.ir.program.local_definitions = ctx.scope.local_definitions.clone();

    CompileOutput {
        descriptor,
        program: ctx.ir.program,
        requirements: ctx.declarations.requirements,
        registry: ctx.registry,
        diagnostics: ctx.diagnostics,
    }
}
