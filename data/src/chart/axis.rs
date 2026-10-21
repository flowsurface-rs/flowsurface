use super::{
    ticks::{
        Y_LABEL_DENSITY,
        y::{TickValue, YAxisScale},
        y_labels_that_fit,
    },
    value::{PanelValueLabelPolicy, PanelValueScale, YUnit, YUnitRange},
};

const MAX_AUTOSCALE_SPAN_FACTOR: f64 = 1_000.0;
pub const ZOOM_SENSITIVITY: f32 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelYValueRange {
    min: f64,
    max: f64,
}

impl PanelYValueRange {
    fn new(min: f64, max: f64) -> Option<Self> {
        (min.is_finite() && max.is_finite() && min < max).then_some(Self { min, max })
    }

    pub fn from_units(range: YUnitRange, scale: PanelValueScale) -> Option<Self> {
        let (min, max) = scale.values(range);
        Self::new(min, max)
    }

    pub fn to_units(self, scale: PanelValueScale) -> YUnitRange {
        YUnitRange::new(scale.value_to_unit(self.min), scale.value_to_unit(self.max))
    }

    pub fn axis_labels(
        self,
        scale: PanelValueScale,
        policy: PanelValueLabelPolicy,
        top: f32,
        height: f32,
        text_size: f32,
    ) -> Vec<(String, f32)> {
        let ticks = YAxisScale::Quantized {
            step: scale.unit_to_value(YUnit(1)),
        }
        .ticks(
            self.min,
            self.max,
            y_labels_that_fit(height, text_size, Y_LABEL_DENSITY).max(2) as i32,
        );
        let step = ticks.step.unwrap_or(1.0);

        let mut labels: Vec<_> = ticks
            .ticks
            .into_iter()
            .filter_map(|tick| {
                let TickValue::Float(value) = tick.value else {
                    return None;
                };
                let y = f64::from(top) + (1.0 - tick.ratio) * f64::from(height);
                y.is_finite()
                    .then_some(y as f32)
                    .filter(|y| y.is_finite())
                    .map(|y| {
                        (
                            policy.format_axis_value(scale, value, step),
                            y.clamp(top, top + height),
                        )
                    })
            })
            .collect();
        labels.dedup_by(|left, right| left.0 == right.0);
        labels
    }

    pub fn normalized(min_value: f64, max_value: f64) -> Option<Self> {
        if !min_value.is_finite() || !max_value.is_finite() {
            return None;
        }
        let (mut min_value, mut max_value) = if min_value <= max_value {
            (min_value, max_value)
        } else {
            (max_value, min_value)
        };
        if (max_value - min_value).abs() <= 1e-8 {
            let pad = min_value.abs().max(1.0) * 0.01;
            min_value -= pad;
            max_value += pad;
        }
        Self::new(min_value, max_value)
    }

    pub fn min_value(self) -> f64 {
        self.min
    }

    pub fn max_value(self) -> f64 {
        self.max
    }

    pub fn span(self) -> f64 {
        (self.max - self.min).abs()
    }

    pub fn zoom_around_ratio(
        self,
        anchor_ratio: f64,
        zoom_scale: f64,
        zoom_in: bool,
    ) -> Option<Self> {
        if !zoom_scale.is_finite() {
            return None;
        }
        let anchor_ratio = anchor_ratio.clamp(0.0, 1.0);
        let span = self.span().max(1e-8);
        let zoom_scale = zoom_scale.abs().max(1.0);
        let new_span = if zoom_in {
            span / zoom_scale
        } else if span <= f64::MAX / zoom_scale {
            span * zoom_scale
        } else {
            f64::MAX
        };
        if !new_span.is_finite() {
            return None;
        }
        let anchor_value = self.min + anchor_ratio * span;
        let new_min = anchor_value - anchor_ratio * new_span;
        Self::new(new_min, new_min + new_span)
    }

    fn clamp_span(self, min_span: f64, max_span: f64) -> Option<Self> {
        if !self.min.is_finite()
            || !self.max.is_finite()
            || !min_span.is_finite()
            || !max_span.is_finite()
        {
            return None;
        }

        let min_span = min_span.abs().max(1e-8);
        let max_span = max_span.abs().max(min_span);
        let span = self.span();
        let target_span = span.clamp(min_span, max_span);
        if target_span == span {
            return Some(self);
        }
        let center = self.min * 0.5 + self.max * 0.5;
        let min_value = center - target_span * 0.5;
        Self::new(min_value, min_value + target_span)
    }

    fn clamp_to_bounds(self, lower: f64, upper: f64) -> Option<Self> {
        if !lower.is_finite() || !upper.is_finite() || lower >= upper {
            return None;
        }
        if self.min >= lower && self.max <= upper {
            return Some(self);
        }

        let span = self.span().min(upper - lower);
        let mut min_value = self.min.max(lower);
        let mut max_value = min_value + span;
        if max_value > upper {
            max_value = upper;
            min_value = max_value - span;
        }

        Self::new(min_value, max_value)
    }

    pub fn pan_by_ratio(self, delta_ratio: f64) -> Option<Self> {
        if !delta_ratio.is_finite() {
            return None;
        }
        let span = self.span().max(1e-8);
        let shift = delta_ratio * span;
        Self::normalized(self.min + shift, self.max + shift)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PanelYViewportLimits {
    pub min_span: f64,
    pub max_span: f64,
    pub bounds: (f64, f64),
}

impl PanelYViewportLimits {
    pub fn for_scale(
        scale: PanelValueScale,
        autoscale_span: f64,
        physical_height: f64,
    ) -> Option<Self> {
        let (min_span, max_span) = autoscale_span_limits(
            autoscale_span,
            physical_height,
            scale.minimum_span().min(autoscale_span),
        )?;
        let bounds = scale.value_bounds();
        let max_span = max_span.min(bounds.1 - bounds.0);
        (max_span >= min_span).then_some(Self {
            min_span,
            max_span,
            bounds,
        })
    }

    pub fn zoom(
        self,
        viewport: PanelYValueRange,
        anchor_ratio: f64,
        delta: f32,
    ) -> Option<PanelYValueRange> {
        if !delta.is_finite() {
            return None;
        }
        let zoom_in = delta > 0.0;
        let zoom_scale = (f64::from(delta.abs()) / f64::from(ZOOM_SENSITIVITY))
            .min(f64::MAX.ln())
            .exp();
        let scale_limit = if zoom_in {
            viewport.span() / self.min_span
        } else {
            self.max_span / viewport.span()
        };
        let zoom_scale = zoom_scale.min(scale_limit.max(1.0));
        let viewport = viewport.zoom_around_ratio(anchor_ratio, zoom_scale, zoom_in)?;
        self.apply(viewport)
    }

    pub fn apply(self, viewport: PanelYValueRange) -> Option<PanelYValueRange> {
        let viewport = viewport.clamp_span(self.min_span, self.max_span)?;
        viewport.clamp_to_bounds(self.bounds.0, self.bounds.1)
    }
}

pub fn autoscale_span_limits(
    autoscale_span: f64,
    physical_height: f64,
    min_span: f64,
) -> Option<(f64, f64)> {
    if autoscale_span.is_nan()
        || autoscale_span < 0.0
        || !physical_height.is_finite()
        || physical_height <= 0.0
        || !min_span.is_finite()
    {
        return None;
    }

    let min_span = min_span.abs().max(1e-8);
    let factor = physical_height.clamp(1.0, MAX_AUTOSCALE_SPAN_FACTOR);
    let autoscale_span = autoscale_span.min(f64::MAX).max(min_span);
    let max_span = if autoscale_span > f64::MAX / factor {
        f64::MAX
    } else {
        autoscale_span * factor
    };
    Some((min_span, max_span.max(min_span)))
}

pub fn place_non_overlapping_labels(
    desired_centers: &[f32],
    min_y: f32,
    max_y: f32,
    label_height: f32,
    gap: f32,
) -> Vec<Option<f32>> {
    if desired_centers.is_empty() {
        return Vec::new();
    }

    let label_height = label_height.max(1.0);
    let min_center = min_y + label_height * 0.5;
    let max_center = (max_y - label_height * 0.5).max(min_center);
    let spacing = (label_height + gap.max(0.0)).max(label_height);
    let capacity = ((max_center - min_center) / spacing).floor() as usize + 1;
    let visible_count = desired_centers.len().min(capacity.max(1));

    let mut candidates: Vec<(usize, f32)> = desired_centers.iter().copied().enumerate().collect();
    candidates.sort_by(|(_, left), (_, right)| left.total_cmp(right));

    let selected = if visible_count == candidates.len() {
        candidates
    } else {
        (0..visible_count)
            .map(|slot| {
                let source_index = if visible_count == 1 {
                    candidates.len() / 2
                } else {
                    slot * (candidates.len() - 1) / (visible_count - 1)
                };
                candidates[source_index]
            })
            .collect()
    };

    let mut centers: Vec<f32> = selected
        .iter()
        .map(|(_, center)| center.clamp(min_center, max_center))
        .collect();

    if centers.len() > 1 {
        centers[0] = centers[0].max(min_center);

        for index in 1..centers.len() {
            centers[index] = centers[index].max(centers[index - 1] + spacing);
        }

        if centers[centers.len() - 1] > max_center {
            let last = centers.len() - 1;
            centers[last] = max_center;

            for index in (0..last).rev() {
                centers[index] = centers[index].min(centers[index + 1] - spacing);
            }
        }
    }

    let mut placements = vec![None; desired_centers.len()];
    for ((source_index, _), center) in selected.into_iter().zip(centers) {
        placements[source_index] = Some(center);
    }

    placements
}
