use super::super::ast::AstFunction;
use crate::diagnostics::Diagnostic;
use crate::hir::{FunctionId, HirFunction};
use crate::types::{Type, ValueType};
use std::collections::BTreeMap;
use std::path::Path;

pub(super) struct FunctionIndex {
    pub(super) by_name: BTreeMap<String, FunctionId>,
    pub(super) signatures: BTreeMap<FunctionId, (Vec<String>, usize)>,
    pub(super) source_ids: Vec<FunctionId>,
    pub(super) stubs: Vec<HirFunction>,
    pub(super) diagnostics: Vec<Diagnostic>,
}

pub(super) fn index_functions(functions: &[AstFunction], path: &Path) -> FunctionIndex {
    let mut index = FunctionIndex {
        by_name: BTreeMap::new(),
        signatures: BTreeMap::new(),
        source_ids: Vec::with_capacity(functions.len()),
        stubs: Vec::new(),
        diagnostics: Vec::new(),
    };
    for function in functions {
        if let Some(id) = index.by_name.get(&function.name).copied() {
            index.diagnostics.push(Diagnostic::error(
                path.to_path_buf(),
                function.span,
                format!("function {} is already declared", function.name),
            ));
            index.source_ids.push(id);
            continue;
        }
        let id = FunctionId(index.stubs.len());
        index.by_name.insert(function.name.clone(), id);
        index.signatures.insert(
            id,
            (
                function
                    .parameters
                    .iter()
                    .map(|parameter| parameter.name.clone())
                    .collect(),
                function
                    .parameters
                    .iter()
                    .filter(|parameter| parameter.default.is_none())
                    .count(),
            ),
        );
        index.stubs.push(HirFunction {
            id,
            name: function.name.clone(),
            parameters: Vec::new(),
            parameter_names: Vec::new(),
            parameter_defaults: Vec::new(),
            local_slots: Vec::new(),
            body: Vec::new(),
            return_type: Type::series(ValueType::Float),
        });
        index.source_ids.push(id);
    }
    index
}
