use super::super::ast::{AstCall, constant_value};
use super::context::LoweringContext;
use crate::diagnostics::Span;
use crate::scene::*;
use crate::types::*;

impl LoweringContext {
    pub(super) fn presentation_string(
        &mut self,
        call: &AstCall,
        name: &str,
        positional: Option<usize>,
    ) -> Option<String> {
        if let Some(value) = self.static_named_string(call, name) {
            return Some(value);
        }
        let index = positional?;
        let argument = call
            .arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .nth(index);
        match argument.and_then(|argument| constant_value(&argument.expression)) {
            Some(Value::String(value)) => Some(value),
            Some(_) => {
                self.error(
                    argument.map_or(call.span, |argument| argument.expression.span()),
                    format!("{name} must be a string literal"),
                );
                None
            }
            None => {
                self.error(
                    argument.map_or(call.span, |argument| argument.expression.span()),
                    format!("{name} must be a compile-time string"),
                );
                None
            }
        }
    }

    pub(super) fn required_presentation_string(
        &mut self,
        call: &AstCall,
        names: &[&str],
        positional: Option<usize>,
        description: &str,
    ) -> Option<String> {
        for name in names {
            if call
                .arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some(*name))
            {
                return self.presentation_string(call, name, None);
            }
        }
        let Some(index) = positional else {
            self.error(call.span, format!("{description} is required"));
            return None;
        };
        let has_positional = call
            .arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .nth(index)
            .is_some();
        if !has_positional {
            self.error(call.span, format!("{description} is required"));
            return None;
        }
        self.presentation_string(call, names[0], Some(index))
    }

    pub(super) fn required_presentation_mark(
        &mut self,
        call: &AstCall,
        name: &str,
        description: &str,
    ) -> Option<PresentationMarkKind> {
        if !call
            .arguments
            .iter()
            .any(|argument| argument.name.as_deref() == Some(name))
        {
            self.error(call.span, format!("{description} is required"));
            return None;
        }
        self.parse_presentation_mark(call, name)
    }

    pub(super) fn presentation_shade(&mut self, call: &AstCall) -> Option<String> {
        match self.static_named_value(call, "shade") {
            Some(Value::String(value)) => Some(value),
            Some(_) => {
                self.error(call.span, "shade must be a string literal");
                None
            }
            None => None,
        }
    }

    pub(super) fn presentation_float(&mut self, call: &AstCall, name: &str, default: f32) -> f32 {
        match self.static_named_value(call, name) {
            Some(Value::Int(value)) => value as f32,
            Some(Value::Float(value)) => value as f32,
            Some(_) => {
                self.error(call.span, format!("{name} must be numeric"));
                default
            }
            None => default,
        }
    }

    pub(super) fn presentation_int(&mut self, call: &AstCall, name: &str, default: i32) -> i32 {
        match self.static_named_value(call, name) {
            Some(Value::Int(value)) => match i32::try_from(value) {
                Ok(value) => value,
                Err(_) => {
                    self.error(call.span, format!("{name} must fit in a 32-bit integer"));
                    default
                }
            },
            Some(_) => {
                self.error(call.span, format!("{name} must be an integer"));
                default
            }
            None => default,
        }
    }

    pub(super) fn presentation_color_bindings(
        &mut self,
        call: &AstCall,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let fallback = self.static_named_string(call, "color_slot");
        let (true_slot, false_slot) = self.presentation_conditional_color_bindings(call);
        (fallback, true_slot, false_slot)
    }

    pub(super) fn presentation_conditional_color_bindings(
        &mut self,
        call: &AstCall,
    ) -> (Option<String>, Option<String>) {
        let true_slot = self.static_named_string(call, "true_color_slot");
        let false_slot = self.static_named_string(call, "false_color_slot");
        (true_slot, false_slot)
    }

    pub(super) fn validate_metadata_declaration(
        &mut self,
        call: &AstCall,
        name: &str,
        previous: Option<Span>,
    ) -> bool {
        if !self.is_top_level_declaration() {
            self.error(call.span, format!("{name} declarations must be top-level"));
            return false;
        }
        if !self.ir.program.plots.is_empty() {
            self.error(
                call.span,
                format!("{name} declarations must appear before output declarations"),
            );
            return false;
        }
        if previous.is_some() {
            self.error(call.span, format!("duplicate {name} declaration"));
            return false;
        }
        true
    }

    pub(super) fn compile_display_declaration(
        &mut self,
        call: &AstCall,
    ) -> Option<crate::hir::StatementId> {
        if !self.validate_metadata_declaration(call, "display", self.presentation.spans.display) {
            return None;
        }
        self.validate_positional_arguments(call, 0, "display");
        self.validate_named_arguments(call, &["view", "attachment", "panel_title", "layer_name"]);
        if let Some(view) = self.static_named_string(call, "view") {
            self.presentation.data.view = match view.to_ascii_lowercase().as_str() {
                "plot" => PresentationViewKind::Plot,
                "table" => PresentationViewKind::Table,
                "primary_overlay" => PresentationViewKind::PrimaryOverlay,
                value => {
                    self.error(
                        call.span,
                        format!(
                            "unknown display view {value}; use plot, table, or primary_overlay"
                        ),
                    );
                    self.presentation.data.view
                }
            };
        }
        self.presentation.data.attachment = self.parse_presentation_attachment(call, "attachment");
        self.presentation.data.panel_title = self.static_named_string(call, "panel_title");
        self.presentation.data.layer_name = self.static_named_string(call, "layer_name");
        self.presentation.spans.display = Some(call.span);
        None
    }

    pub(super) fn compile_format_declaration(
        &mut self,
        call: &AstCall,
    ) -> Option<crate::hir::StatementId> {
        if !self.validate_metadata_declaration(call, "format", self.presentation.spans.format) {
            return None;
        }
        self.validate_positional_arguments(call, 0, "format");
        self.validate_named_arguments(
            call,
            &[
                "precision",
                "axis_labels",
                "header_labels",
                "scale",
                "max_decimals",
            ],
        );
        self.presentation.data.value_precision =
            self.parse_presentation_precision(call, "precision");
        self.presentation.data.axis_label_mode =
            self.parse_presentation_label_mode(call, "axis_labels");
        self.presentation.data.header_label_mode =
            self.parse_presentation_label_mode(call, "header_labels");
        self.presentation.data.preferred_scale = self.parse_presentation_scale(call, "scale");
        self.presentation.data.max_decimals = self
            .static_named_value(call, "max_decimals")
            .and_then(|value| match value {
                Value::Int(value) if (0..=18).contains(&value) => Some(value as u8),
                _ => {
                    self.error(call.span, "max_decimals must be an integer from 0 to 18");
                    None
                }
            });
        self.presentation.spans.format = Some(call.span);
        None
    }

    pub(super) fn compile_style_declaration(
        &mut self,
        call: &AstCall,
    ) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "style declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 2, "style");
        self.validate_named_arguments(call, &["id", "title", "role", "shade"]);
        let id = self.presentation_string(call, "id", Some(0))?;
        let label = if call
            .arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .nth(1)
            .is_some()
            || call
                .arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some("title"))
        {
            self.presentation_string(call, "title", Some(1))
                .unwrap_or_else(|| id.clone())
        } else {
            id.clone()
        };
        let role = self
            .presentation_string(call, "role", None)
            .unwrap_or_else(|| "neutral".to_owned());
        let shade = self
            .presentation_shade(call)
            .unwrap_or_else(|| "base".to_owned());
        self.presentation.spans.styles.insert(id.clone(), call.span);
        self.presentation.data.styles.push(PresentationStyle {
            id,
            label,
            role,
            shade,
        });
        None
    }

    pub(super) fn compile_mark_declaration(
        &mut self,
        call: &AstCall,
    ) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "mark declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 1, "mark");
        self.validate_named_arguments(
            call,
            &[
                "title",
                "source",
                "kind",
                "color_slot",
                "true_color_slot",
                "false_color_slot",
                "highlight_by",
                "highlight_color_slot",
                "requires",
                "fallback",
            ],
        );
        let label = self.required_presentation_string(call, &["title"], Some(0), "mark title")?;
        let source_channel =
            self.required_presentation_string(call, &["source"], None, "mark source")?;
        let kind = self.required_presentation_mark(call, "kind", "mark kind")?;
        let required_input = self.parse_presentation_input(call, "requires");
        let fallback_mark = self.parse_presentation_mark(call, "fallback");
        let (color_slot, true_color_slot, false_color_slot) =
            self.presentation_color_bindings(call);
        let highlight_by = self.static_named_string(call, "highlight_by");
        let highlight_color_slot = self.static_named_string(call, "highlight_color_slot");
        if color_slot.is_none() {
            self.error(call.span, "mark requires at least one color_slot");
        }
        self.presentation.spans.marks.push(call.span);
        self.presentation.data.marks.push(PresentationMark {
            label,
            kind,
            source_channel,
            required_input,
            fallback_mark,
            color_slot: color_slot.unwrap_or_default(),
            true_color_slot,
            false_color_slot,
            highlight_by,
            highlight_color_slot,
        });
        None
    }

    pub(super) fn compile_fill_declaration(
        &mut self,
        call: &AstCall,
    ) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "fill declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 2, "fill");
        self.validate_named_arguments(
            call,
            &[
                "upper",
                "lower",
                "color_slot",
                "geometry",
                "style_by",
                "opacity",
                "upper_offset",
                "lower_offset",
                "true_color_slot",
                "false_color_slot",
            ],
        );
        let upper =
            self.required_presentation_string(call, &["upper"], Some(0), "fill upper output")?;
        let lower =
            self.required_presentation_string(call, &["lower"], Some(1), "fill lower output")?;
        let color_slot =
            self.required_presentation_string(call, &["color_slot"], None, "fill color_slot")?;
        let geometry = self
            .presentation_string(call, "geometry", None)
            .unwrap_or_else(|| "panel_band".to_owned());
        let geometry = match geometry.to_ascii_lowercase().as_str() {
            "panel_band" => PresentationGeometry::PanelBand,
            "series_band" => PresentationGeometry::SeriesBand,
            value => {
                self.error(call.span, format!("unknown fill geometry {value}"));
                PresentationGeometry::PanelBand
            }
        };
        let style_by = self.static_named_string(call, "style_by");
        let (true_color_slot, false_color_slot) =
            self.presentation_conditional_color_bindings(call);
        let opacity = self.presentation_float(call, "opacity", 0.2);
        let upper_offset_bars = self.presentation_int(call, "upper_offset", 0);
        let lower_offset_bars = self.presentation_int(call, "lower_offset", 0);
        self.presentation.spans.fills.push(call.span);
        self.presentation.data.fills.push(PresentationFill {
            upper,
            lower,
            upper_offset_bars,
            lower_offset_bars,
            style_by,
            geometry,
            color_slot,
            true_color_slot,
            false_color_slot,
            opacity,
        });
        None
    }

    pub(super) fn compile_readout_declaration(
        &mut self,
        call: &AstCall,
    ) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "readout declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 1, "readout");
        self.validate_named_arguments(call, &["title", "source", "color_slot", "mark"]);
        let label =
            self.required_presentation_string(call, &["title"], Some(0), "readout title")?;
        let channel =
            self.required_presentation_string(call, &["source"], None, "readout source")?;
        let color_slot =
            self.required_presentation_string(call, &["color_slot"], None, "readout color_slot")?;
        let mark = self.parse_presentation_mark(call, "mark");
        self.presentation.spans.readouts.push(call.span);
        self.presentation.data.readouts.push(PresentationReadout {
            label,
            channel,
            color_slot,
            mark,
        });
        None
    }

    pub(super) fn compile_column_declaration(
        &mut self,
        call: &AstCall,
    ) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "column declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 1, "column");
        self.validate_named_arguments(
            call,
            &[
                "title",
                "source",
                "format",
                "color_slot",
                "color_mode",
                "positive_color_slot",
                "negative_color_slot",
            ],
        );
        let label = self.required_presentation_string(call, &["title"], Some(0), "column title")?;
        let channel =
            self.required_presentation_string(call, &["source"], None, "column source")?;
        let format = self
            .presentation_string(call, "format", None)
            .unwrap_or_else(|| "quantity".to_owned());
        let color_mode = self
            .presentation_string(call, "color_mode", None)
            .unwrap_or_else(|| "static".to_owned());
        let has_color_slot = call
            .arguments
            .iter()
            .any(|argument| argument.name.as_deref() == Some("color_slot"));
        let has_positive_color_slot = call
            .arguments
            .iter()
            .any(|argument| argument.name.as_deref() == Some("positive_color_slot"));
        let has_negative_color_slot = call
            .arguments
            .iter()
            .any(|argument| argument.name.as_deref() == Some("negative_color_slot"));
        let color = match color_mode.to_ascii_lowercase().as_str() {
            "static" => {
                if has_positive_color_slot || has_negative_color_slot {
                    self.error(
                        call.span,
                        "static table columns use color_slot and cannot provide signed color slots",
                    );
                }
                PresentationColumnColor::Static {
                    slot: self
                        .presentation_string(call, "color_slot", None)
                        .unwrap_or_else(|| "main".to_owned()),
                }
            }
            "signed" => {
                if has_color_slot {
                    self.error(
                        call.span,
                        "signed table columns cannot provide color_slot; use positive_color_slot and negative_color_slot",
                    );
                }
                if !has_positive_color_slot {
                    self.error(
                        call.span,
                        "signed table columns require positive_color_slot",
                    );
                }
                if !has_negative_color_slot {
                    self.error(
                        call.span,
                        "signed table columns require negative_color_slot",
                    );
                }
                PresentationColumnColor::Signed {
                    positive_slot: self
                        .presentation_string(call, "positive_color_slot", None)
                        .unwrap_or_default(),
                    negative_slot: self
                        .presentation_string(call, "negative_color_slot", None)
                        .unwrap_or_default(),
                }
            }
            _ => {
                self.error(
                    call.span,
                    format!(
                        "unknown table column color_mode '{}'; use static or signed",
                        color_mode
                    ),
                );
                PresentationColumnColor::Static {
                    slot: self
                        .presentation_string(call, "color_slot", None)
                        .unwrap_or_else(|| "main".to_owned()),
                }
            }
        };
        self.presentation.spans.columns.push(call.span);
        self.presentation.data.columns.push(PresentationColumn {
            label,
            channel,
            format,
            color,
        });
        None
    }

    pub(super) fn parse_presentation_data_kind(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationDataKind> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "scalar" => PresentationDataKind::Scalar,
            "histogram" => PresentationDataKind::Histogram,
            "ohlc" => PresentationDataKind::Ohlc,
            "range" => PresentationDataKind::Range,
            value => {
                self.error(call.span, format!("unknown presentation data kind {value}"));
                PresentationDataKind::Scalar
            }
        })
    }

    pub(super) fn parse_presentation_mark(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationMarkKind> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "candle" => PresentationMarkKind::Candle,
            "regular_bar" => PresentationMarkKind::RegularBar,
            "histogram" => PresentationMarkKind::Histogram,
            "histogram_signed_overlay" => PresentationMarkKind::HistogramSignedOverlay,
            "line" => PresentationMarkKind::Line,
            "step" => PresentationMarkKind::Step,
            "area" => PresentationMarkKind::Area,
            "point" => PresentationMarkKind::Point,
            "cross" => PresentationMarkKind::Cross,
            "segment" => PresentationMarkKind::Segment,
            "horizontal_level" => PresentationMarkKind::HorizontalLevel,
            value => {
                self.error(call.span, format!("unknown presentation mark {value}"));
                PresentationMarkKind::Line
            }
        })
    }

    pub(super) fn parse_presentation_input(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationInputSource> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "klines" => PresentationInputSource::Klines,
            "directional_volume" => PresentationInputSource::DirectionalVolume,
            "trades" => PresentationInputSource::Trades,
            "open_interest" => PresentationInputSource::OpenInterest,
            value => {
                self.error(
                    call.span,
                    format!(
                        "unknown presentation input {value}; use klines, directional_volume, trades, or open_interest"
                    ),
                );
                return None;
            }
        })
    }

    pub(super) fn parse_presentation_axis(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationAxis> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "primary" => PresentationAxis::Primary,
            "secondary" => PresentationAxis::Secondary,
            value => {
                self.error(call.span, format!("unknown presentation axis {value}"));
                PresentationAxis::Secondary
            }
        })
    }

    pub(super) fn parse_presentation_precision(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationPrecision> {
        let value = self.static_named_value(call, name)?;
        match value {
            Value::String(value) => match value.to_ascii_lowercase().as_str() {
                "base_ticker_min_tick" => Some(PresentationPrecision::BaseTickerMinTick),
                "base_ticker_min_qty" => Some(PresentationPrecision::BaseTickerMinQty),
                value => value
                    .strip_prefix("fixed_step:")
                    .and_then(|step| step.parse::<f64>().ok())
                    .filter(|step| step.is_finite() && *step > 0.0)
                    .map(PresentationPrecision::FixedStep)
                    .or_else(|| {
                        self.error(call.span, format!("unknown presentation precision {value}"));
                        None
                    }),
            },
            _ => {
                self.error(call.span, format!("{name} must be a precision string"));
                None
            }
        }
    }

    pub(super) fn parse_presentation_label_mode(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationLabelMode> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "compact" => PresentationLabelMode::Compact,
            "commas" => PresentationLabelMode::Commas,
            "abbreviated" => PresentationLabelMode::Abbreviated,
            value => {
                self.error(
                    call.span,
                    format!("unknown presentation label mode {value}"),
                );
                PresentationLabelMode::Compact
            }
        })
    }

    pub(super) fn parse_presentation_scale(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationScale> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "absolute" => PresentationScale::Absolute,
            "fit_visible" => PresentationScale::FitVisible,
            "fit_visible_include_zero" => PresentationScale::FitVisibleIncludeZero,
            "logarithmic" => PresentationScale::Logarithmic,
            "percent_from_base" => PresentationScale::PercentFromBase,
            value => {
                self.error(call.span, format!("unknown presentation scale {value}"));
                PresentationScale::FitVisible
            }
        })
    }

    pub(super) fn parse_presentation_attachment(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationAttachmentMode> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "overlay" => PresentationAttachmentMode::Overlay,
            "panel" => PresentationAttachmentMode::Panel,
            value => {
                self.error(
                    call.span,
                    format!("unknown attachment {value}; use overlay or panel"),
                );
                PresentationAttachmentMode::Panel
            }
        })
    }
}
