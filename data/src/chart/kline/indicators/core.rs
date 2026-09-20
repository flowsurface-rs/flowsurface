pub(super) mod input;
pub(super) mod output;
pub(super) mod runtime;

// Execution modules share the indicator contract and catalog facade defined
// by the parent module. Re-exporting it here keeps their APIs cohesive without
// adding a second chart-facing dispatch layer.
pub(super) use super::*;
