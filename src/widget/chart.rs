use std::time::Instant;

use iced::advanced::layout;
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Clipboard, Layout, Shell, Widget};
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

use crate::chart::Action;
use crate::style;

const Y_AXIS_GUTTER: f32 = 66.0; // px
const TEXT_SIZE: f32 = 12.0;

#[derive(Debug, Clone)]
pub struct Series {
    pub name: String,
    /// (x, y) where x is a time-like domain (e.g., timestamp seconds) and y is raw value
    pub points: Vec<(f64, f64)>,
    pub color: Option<Color>,
}

pub struct LineComparison {
    series: Vec<Series>,
    stroke_width: f32,
    zoom: Zoom,
    last_tick: Instant,
}

#[derive(Default)]
struct State {
    plot_cache: canvas::Cache,
    label_cache: canvas::Cache,
}

impl LineComparison {
    pub fn new(series: Vec<Series>) -> Self {
        Self {
            series,
            stroke_width: 2.0,
            zoom: Zoom::all(),
            last_tick: Instant::now(),
        }
    }

    /// Builder to set zoom (0 = all points, otherwise last N points).
    pub fn with_zoom(mut self, zoom: Zoom) -> Self {
        self.zoom = zoom;
        self
    }

    /// Update zoom at runtime (0 = all points, otherwise last N points).
    pub fn set_zoom(&mut self, zoom: Zoom) {
        if self.zoom != zoom {
            self.zoom = zoom;
            // If you retain widget state across frames, consider clearing caches:
            // (requires access to State) — keep as-is if the widget is recreated on change.
        }
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    fn compute_domains(&self) -> Option<((f64, f64), (f32, f32))> {
        if self.series.is_empty() {
            return None;
        }

        // X domain is shared across all series (restricted by zoom)
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;

        // Y domain (percent) over the currently visible X window
        let mut min_pct = f32::INFINITY;
        let mut max_pct = f32::NEG_INFINITY;

        let mut any_point = false;

        for s in &self.series {
            if s.points.is_empty() {
                continue;
            }
            let base = s.points[0].1;
            if base == 0.0 {
                continue; // skip invalid base
            }

            let start_idx = if self.zoom.is_all() {
                0
            } else {
                s.points.len().saturating_sub(self.zoom.0)
            };

            for (x, y) in s.points.iter().skip(start_idx) {
                any_point = true;

                if *x < min_x {
                    min_x = *x;
                }
                if *x > max_x {
                    max_x = *x;
                }

                let pct = ((*y / base) - 1.0) as f32 * 100.0;
                if pct < min_pct {
                    min_pct = pct;
                }
                if pct > max_pct {
                    max_pct = pct;
                }
            }
        }

        if !any_point || !min_x.is_finite() || !max_x.is_finite() {
            return None;
        }

        if (max_x - min_x).abs() < f64::EPSILON {
            max_x = min_x + 1.0;
        }
        if (max_pct - min_pct).abs() < f32::EPSILON {
            min_pct -= 1.0;
            max_pct += 1.0;
        }

        // Apply 5% padding in value space (top & bottom)
        let span = (max_pct - min_pct).max(1e-6);
        let pad = span * 0.05;
        min_pct -= pad;
        max_pct += pad;

        Some(((min_x, max_x), (min_pct, max_pct)))
    }

    pub fn sample() -> Self {
        use rand::prelude::*;
        let mut rng = rand::rng();
        let n = 120;

        let mut make_series = |name: &str, start: f64, drift: f64, noise: f64| -> Series {
            let mut y = start;
            let mut points = Vec::with_capacity(n);
            for i in 0..n {
                let step: f64 = rng.random_range(-noise..noise) + drift;
                y = (y + step).max(1e-6);
                points.push((i as f64, y));
            }
            Series {
                name: name.into(),
                points,
                color: None,
            }
        };

        Self::new(vec![
            make_series("Alpha", 100.0, 0.08, 0.9),
            make_series("Beta", 80.0, 0.18, 1.2),
            make_series("Gamma", 120.0, 0.04, 1.6),
        ])
        // Example: show last 60 points
        // .with_zoom(Zoom::points(60))
    }

    // Compute a "nice" step close to range/target using 1/2/5*10^k
    fn nice_step(range: f32, target: usize) -> f32 {
        let target = target.max(2) as f32;
        let raw = (range / target).max(f32::EPSILON);
        let power = raw.log10().floor();
        let base = 10f32.powf(power);
        let n = raw / base;
        let nice = if n <= 1.0 {
            1.0
        } else if n <= 2.0 {
            2.0
        } else if n <= 5.0 {
            5.0
        } else {
            10.0
        };
        nice * base
    }

    fn ticks(&self, min: f32, max: f32, target: usize) -> (Vec<f32>, f32) {
        let span = (max - min).abs().max(1e-6);
        let step = Self::nice_step(span, target);
        let start = (min / step).floor() * step;
        let end = (max / step).ceil() * step;

        let mut v = Vec::new();
        let mut t = start;
        for _ in 0..100 {
            if t > end + step * 0.5 {
                break;
            }
            v.push(t);
            t += step;
        }
        (v, step)
    }

    fn format_pct(val: f32, step: f32, show_decimals: bool) -> String {
        if show_decimals {
            if step >= 1.0 {
                format!("{:+.1}%", val)
            } else if step >= 0.1 {
                format!("{:+.2}%", val)
            } else {
                format!("{:+.3}%", val)
            }
        } else if step >= 1.0 {
            format!("{:+.0}%", val)
        } else if step >= 0.1 {
            format!("{:+.1}%", val)
        } else {
            format!("{:+.2}%", val)
        }
    }

    fn collect_end_labels<Fy>(
        &self,
        plot: Rectangle,
        map_y: &Fy,
        min_x: f64,
        max_x: f64,
        step: f32,
        line_color_pool: &[Color; 5],
        text_color_pool: &[Color; 5],
        gutter: f32,
    ) -> Vec<EndLabel>
    where
        Fy: Fn(f32) -> f32,
    {
        let mut end_labels: Vec<EndLabel> = Vec::new();

        for (si, s) in self.series.iter().enumerate() {
            if s.points.is_empty() {
                continue;
            }
            let base = s.points[0].1;
            if base == 0.0 {
                continue;
            }
            let text_color = s.color.unwrap_or_else(|| {
                text_color_pool
                    .get(si % text_color_pool.len())
                    .copied()
                    .unwrap_or(Color::BLACK)
            });
            let bg_color = s.color.unwrap_or_else(|| {
                line_color_pool
                    .get(si % line_color_pool.len())
                    .copied()
                    .unwrap_or(Color::BLACK)
            });

            let last = s
                .points
                .iter()
                .rev()
                .find(|(x, _)| (*x >= min_x) && (*x <= max_x))
                .unwrap_or(&s.points[s.points.len() - 1]);

            let pct = ((last.1 / base) - 1.0) as f32 * 100.0;
            let mut py = map_y(pct);
            let half_txt = TEXT_SIZE * 0.5;
            py = py.clamp(plot.y + half_txt, plot.y + plot.height - half_txt);

            let lbl = LineComparison::format_pct(pct, step, true);
            end_labels.push(EndLabel {
                pos: Point::new(plot.x + plot.width + gutter, py),
                text: lbl,
                bg_color,
                text_color,
            });
        }

        end_labels
    }

    fn resolve_label_overlaps(end_labels: &mut [EndLabel], plot: Rectangle) {
        if end_labels.len() <= 1 {
            return;
        }

        let half_h = TEXT_SIZE * 0.5 + 2.0;
        let mut min_y = plot.y + half_h;
        let mut max_y = plot.y + plot.height - half_h;
        if max_y < min_y {
            core::mem::swap(&mut min_y, &mut max_y);
        }

        let mut sep = TEXT_SIZE + 4.0;

        if end_labels.len() > 1 {
            let avail = (max_y - min_y).max(0.0);
            let needed = sep * (end_labels.len() as f32 - 1.0);
            if needed > avail {
                sep = if end_labels.len() > 1 {
                    avail / (end_labels.len() as f32 - 1.0)
                } else {
                    sep
                };
            }
        }

        end_labels.sort_by(|a, b| {
            a.pos
                .y
                .partial_cmp(&b.pos.y)
                .unwrap_or(core::cmp::Ordering::Equal)
        });

        let mut prev_y = f32::NAN;
        for i in 0..end_labels.len() {
            let low = if i == 0 { min_y } else { prev_y + sep };
            let high = max_y - sep * (end_labels.len() as f32 - 1.0 - i as f32);
            let target = end_labels[i].pos.y;
            let y = target.clamp(low, high);
            end_labels[i].pos.y = y;
            prev_y = y;
        }
    }

    fn fill_label_geometry(&self, frame: &mut canvas::Frame, end_labels: &[EndLabel], gutter: f32) {
        for label in end_labels {
            frame.fill_rectangle(
                Point {
                    x: label.pos.x,
                    y: label.pos.y - TEXT_SIZE * 0.5 - 2.0,
                },
                Size {
                    width: -gutter,
                    height: TEXT_SIZE + 4.0,
                },
                label.bg_color,
            );

            frame.fill(
                &canvas::Path::circle(Point::new(label.pos.x, label.pos.y), 4.0),
                label.bg_color,
            );

            frame.fill_text(canvas::Text {
                content: label.text.clone(),
                position: label.pos - Vector::new(4.0, 0.0),
                color: label.text_color,
                size: 12.0.into(),
                font: style::AZERET_MONO,
                align_x: iced::Alignment::End.into(),
                align_y: iced::Alignment::Center.into(),
                ..Default::default()
            });
        }
    }

    fn fill_main_geometry<Fx, Fy>(
        &self,
        frame: &mut canvas::Frame,
        plot: Rectangle,
        gutter: f32,
        ticks: &[f32],
        labels: &[String],
        map_x: &Fx,
        map_y: &Fy,
        line_color_pool: &[Color; 5],
        palette: &Extended,
        min_x: f64,
        max_x: f64,
    ) where
        Fx: Fn(f64) -> f32,
        Fy: Fn(f32) -> f32,
    {
        // splitter / gutter background
        frame.fill_rectangle(
            Point {
                x: plot.x + plot.width,
                y: 0.0,
            },
            Size {
                width: gutter,
                height: plot.height,
            },
            palette.background.weak.color,
        );

        // Y-axis tick labels
        for (i, tick) in ticks.iter().enumerate() {
            let mut y = map_y(*tick);
            let half_txt = TEXT_SIZE * 0.5;
            y = y.clamp(plot.y + half_txt, plot.y + plot.height - half_txt);
            let txt = &labels[i];
            frame.fill_text(canvas::Text {
                content: txt.clone(),
                position: Point::new(plot.x + plot.width + gutter - 4.0, y),
                color: palette.background.base.text,
                size: 12.0.into(),
                font: style::AZERET_MONO,
                align_x: iced::Alignment::End.into(),
                align_y: iced::Alignment::Center.into(),
                ..Default::default()
            });
        }

        for (si, s) in self.series.iter().enumerate() {
            if s.points.len() < 2 {
                continue;
            }
            let base = s.points[0].1;
            if base == 0.0 {
                continue;
            }

            let color = s.color.unwrap_or_else(|| {
                line_color_pool
                    .get(si % line_color_pool.len())
                    .copied()
                    .unwrap_or(Color::BLACK)
            });

            // Build a path only for the visible X-range [min_x, max_x],
            // while preserving continuity by including the point right before the window, if any.
            let mut builder = canvas::path::Builder::new();
            let mut started = false;

            // Find the first index >= min_x
            let mut i0 = s
                .points
                .iter()
                .position(|(x, _)| *x >= min_x)
                .unwrap_or(s.points.len().saturating_sub(1));

            if i0 > 0 {
                i0 -= 1; // include one point before window for continuity
            }

            for (idx, (x, y)) in s.points.iter().enumerate().skip(i0) {
                if *x > max_x && started {
                    break;
                }

                let pct = ((*y / base) - 1.0) as f32 * 100.0;
                let px = map_x(*x);
                let py = map_y(pct);

                if !started {
                    builder.move_to(Point::new(px, py));
                    started = true;
                } else {
                    builder.line_to(Point::new(px, py));
                }

                // If we never found a point >= min_x, ensure we at least draw the last point
                if idx + 1 >= s.points.len() && !started {
                    builder.move_to(Point::new(px, py));
                    started = true;
                }
            }

            if started {
                let path = builder.build();
                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_color(color)
                        .with_width(self.stroke_width),
                );
            }
        }
    }

    pub fn invalidate(&mut self, _now: Option<std::time::Instant>) -> Option<Action> {
        None
    }
}

impl<Message> Widget<Message, Theme, Renderer> for LineComparison
where
    Message: Clone + 'static,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        _event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        _shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        use advanced::Renderer as _;
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();

        let palette = theme.extended_palette();
        let Some(((min_x, max_x), (min_pct, max_pct))) = self.compute_domains() else {
            return;
        };

        let total_ticks = (bounds.height / TEXT_SIZE / 3.).floor() as usize;
        let (all_ticks, step) = self.ticks(min_pct, max_pct, total_ticks);

        let mut ticks: Vec<f32> = all_ticks
            .into_iter()
            .filter(|t| (*t >= min_pct - f32::EPSILON) && (*t <= max_pct + f32::EPSILON))
            .collect();
        if ticks.is_empty() {
            ticks = vec![min_pct, max_pct];
        }
        let labels: Vec<String> = ticks
            .iter()
            .map(|t| LineComparison::format_pct(*t, step, false))
            .collect();

        let gutter = Y_AXIS_GUTTER;

        let plot = Rectangle {
            x: 0.0,
            y: 0.0,
            width: (bounds.width - gutter).max(1.0),
            height: bounds.height.max(1.0),
        };

        // Domain -> screen
        let x_span = (max_x - min_x) as f32;
        let y_span = (max_pct - min_pct).max(1e-6);
        let map_x = |x: f64| -> f32 {
            let t = ((x - min_x) as f32) / x_span;
            plot.x + t.clamp(0.0, 1.0) * plot.width
        };
        let map_y = |pct: f32| -> f32 {
            let t = (pct - min_pct) / y_span;
            plot.y + plot.height - t.clamp(0.0, 1.0) * plot.height
        };

        let line_color_pool = [
            palette.primary.base.color,
            palette.secondary.base.color,
            palette.success.base.color,
            palette.danger.base.color,
            palette.warning.base.color,
        ];
        let text_color_pool = [
            palette.primary.base.text,
            palette.secondary.base.text,
            palette.success.base.text,
            palette.danger.base.text,
            palette.warning.base.text,
        ];

        let mut end_labels = self.collect_end_labels(
            plot,
            &map_y,
            min_x,
            max_x,
            step,
            &line_color_pool,
            &text_color_pool,
            gutter,
        );

        LineComparison::resolve_label_overlaps(&mut end_labels, plot);

        let geometry = state.plot_cache.draw(renderer, bounds.size(), |frame| {
            self.fill_main_geometry(
                frame,
                plot,
                gutter,
                &ticks,
                &labels,
                &map_x,
                &map_y,
                &line_color_pool,
                palette,
                min_x,
                max_x,
            );
        });

        renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
            use iced::advanced::graphics::geometry::Renderer as _;
            renderer.draw_geometry(geometry);
        });

        let labels_geo = state.label_cache.draw(renderer, bounds.size(), |frame| {
            self.fill_label_geometry(frame, &end_labels, gutter);
        });

        renderer.with_layer(bounds, |renderer| {
            renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                use iced::advanced::graphics::geometry::Renderer as _;
                renderer.draw_geometry(labels_geo);
            });
        });
    }
}

impl<'a, Message> From<LineComparison> for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a + 'static,
{
    fn from(chart: LineComparison) -> Self {
        Element::new(chart)
    }
}

struct EndLabel {
    pos: Point,
    text: String,
    bg_color: Color,
    text_color: Color,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct Zoom(pub usize);

impl Zoom {
    pub fn all() -> Self {
        Self(0)
    }
    pub fn points(n: usize) -> Self {
        Self(n)
    }
    pub fn is_all(self) -> bool {
        self.0 == 0
    }
}
