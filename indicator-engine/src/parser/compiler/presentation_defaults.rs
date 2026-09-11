use super::context::LoweringContext;
use crate::scene::*;
use std::collections::BTreeSet;

impl LoweringContext {
    pub(super) fn ensure_default_plot_styles(&mut self) {
        let existing = self
            .presentation
            .data
            .styles
            .iter()
            .map(|style| style.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut generated = Vec::new();
        for plot in &mut self.ir.program.plots {
            if !matches!(plot.value_type, PlotValueType::Number) {
                continue;
            }
            // An omitted slot receives the deterministic plot-ID style. An
            // explicitly named slot is a source contract and must already
            // be declared with `style`; silently inventing it would hide a
            // typo and defeat missing-style diagnostics.
            let generated_slot = plot.color_slot.is_none();
            let slot = plot
                .color_slot
                .get_or_insert_with(|| plot.id.clone())
                .clone();
            if generated_slot
                && !existing.contains(slot.as_str())
                && !generated
                    .iter()
                    .any(|style: &PresentationStyle| style.id == slot)
            {
                generated.push(PresentationStyle {
                    id: slot,
                    label: plot.label.clone(),
                    role: "neutral".to_owned(),
                    shade: "base".to_owned(),
                });
            }
        }
        // Table columns intentionally default to the neutral `main` slot so a
        // small table indicator does not need boilerplate style metadata. An
        // explicitly named column slot still has to be declared by the source
        // and is validated below.
        if self.presentation.data.columns.iter().any(|column| {
            matches!(
                &column.color,
                PresentationColumnColor::Static { slot } if slot == "main"
            )
        }) && !existing.contains("main")
            && !generated
                .iter()
                .any(|style: &PresentationStyle| style.id == "main")
        {
            generated.push(PresentationStyle {
                id: "main".to_owned(),
                label: "Main".to_owned(),
                role: "neutral".to_owned(),
                shade: "base".to_owned(),
            });
        }
        self.presentation.data.styles.extend(generated);
    }

    /// Materialize the scene implied by numeric plots once, while the source
    /// is being compiled.  The chart host can then map these typed layers
    /// directly without repeating inference or retaining a second set of
    /// presentation defaults.
    pub(super) fn ensure_default_presentation_layers(&mut self, indicator_overlay: bool) {
        if matches!(self.presentation.data.view, PresentationViewKind::Table) {
            return;
        }
        let existing = self
            .presentation
            .data
            .layers
            .iter()
            .map(|layer| layer.channel.as_str())
            .collect::<BTreeSet<_>>();
        let generated = self
            .ir
            .program
            .plots
            .iter()
            .filter(|plot| {
                plot.layer
                    && !matches!(plot.value_type, PlotValueType::Boolean)
                    && !existing.contains(plot.id.as_str())
            })
            .map(|plot| PresentationLayer {
                channel: plot.id.clone(),
                name: plot
                    .layer_title
                    .clone()
                    .unwrap_or_else(|| plot.label.clone()),
                pane: plot.pane,
                data_kind: Some(
                    plot.presentation_data_kind
                        .unwrap_or_else(|| default_data_kind_for_plot(plot)),
                ),
                mark: Some(
                    plot.presentation_mark
                        .unwrap_or_else(|| default_mark_for_plot(plot)),
                ),
                axis: Some(
                    plot.presentation_axis
                        .unwrap_or_else(|| default_axis_for_plot(indicator_overlay, plot)),
                ),
            })
            .collect::<Vec<_>>();
        self.presentation.data.layers.extend(generated);
    }
}

pub(super) fn default_data_kind_for_plot(plot: &PlotDescriptor) -> PresentationDataKind {
    match plot.value_type {
        PlotValueType::Ohlc => PresentationDataKind::Ohlc,
        PlotValueType::Range => PresentationDataKind::Range,
        PlotValueType::Boolean => PresentationDataKind::Scalar,
        PlotValueType::Number => match plot.style {
            PlotStyle::Histogram => PresentationDataKind::Histogram,
            PlotStyle::Candles | PlotStyle::Bars => PresentationDataKind::Ohlc,
            PlotStyle::Cloud | PlotStyle::BrokenCloud => PresentationDataKind::Range,
            PlotStyle::Line | PlotStyle::Step | PlotStyle::Area => PresentationDataKind::Scalar,
        },
    }
}

pub(super) fn default_mark_for_plot(plot: &PlotDescriptor) -> PresentationMarkKind {
    match plot.value_type {
        PlotValueType::Ohlc => PresentationMarkKind::Candle,
        PlotValueType::Range => PresentationMarkKind::Line,
        PlotValueType::Boolean => PresentationMarkKind::Line,
        PlotValueType::Number => match plot.style {
            PlotStyle::Histogram => PresentationMarkKind::Histogram,
            PlotStyle::Candles => PresentationMarkKind::Candle,
            PlotStyle::Bars => PresentationMarkKind::RegularBar,
            PlotStyle::Step => PresentationMarkKind::Step,
            PlotStyle::Area => PresentationMarkKind::Area,
            PlotStyle::Cloud | PlotStyle::BrokenCloud | PlotStyle::Line => {
                PresentationMarkKind::Line
            }
        },
    }
}

pub(super) fn default_axis_for_plot(
    indicator_overlay: bool,
    plot: &PlotDescriptor,
) -> PresentationAxis {
    if (plot.overlay || indicator_overlay)
        && plot.pane == PaneTarget::Main
        && matches!(plot.scale, PlotScale::Price)
    {
        PresentationAxis::Primary
    } else {
        PresentationAxis::Secondary
    }
}

pub(super) fn plot_value_supports_data_kind(
    value_type: PlotValueType,
    data_kind: PresentationDataKind,
) -> bool {
    match value_type {
        PlotValueType::Number => {
            matches!(
                data_kind,
                PresentationDataKind::Scalar | PresentationDataKind::Histogram
            )
        }
        PlotValueType::Boolean => matches!(data_kind, PresentationDataKind::Scalar),
        PlotValueType::Ohlc => matches!(data_kind, PresentationDataKind::Ohlc),
        PlotValueType::Range => matches!(data_kind, PresentationDataKind::Range),
    }
}

pub(super) fn presentation_mark_compatible(
    data_kind: PresentationDataKind,
    mark: PresentationMarkKind,
) -> bool {
    match data_kind {
        PresentationDataKind::Ohlc => {
            matches!(
                mark,
                PresentationMarkKind::Candle | PresentationMarkKind::Line
            )
        }
        PresentationDataKind::Scalar => matches!(
            mark,
            PresentationMarkKind::Line
                | PresentationMarkKind::Step
                | PresentationMarkKind::Area
                | PresentationMarkKind::Point
                | PresentationMarkKind::Cross
                | PresentationMarkKind::Segment
                | PresentationMarkKind::HorizontalLevel
                | PresentationMarkKind::RegularBar
        ),
        PresentationDataKind::Range => matches!(mark, PresentationMarkKind::Line),
        PresentationDataKind::Histogram => matches!(
            mark,
            PresentationMarkKind::Line
                | PresentationMarkKind::Step
                | PresentationMarkKind::Area
                | PresentationMarkKind::Point
                | PresentationMarkKind::Cross
                | PresentationMarkKind::Segment
                | PresentationMarkKind::HorizontalLevel
                | PresentationMarkKind::RegularBar
                | PresentationMarkKind::Histogram
                | PresentationMarkKind::HistogramSignedOverlay
        ),
    }
}

pub(super) fn numeric_mark_compatible(mark: PresentationMarkKind) -> bool {
    !matches!(mark, PresentationMarkKind::Candle)
}

pub(super) fn valid_presentation_role(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "neutral" | "secondary" | "primary" | "success" | "danger" | "warning"
    )
}

pub(super) fn valid_presentation_shade(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "faint" | "soft" | "base" | "strong" | "vivid"
    )
}
