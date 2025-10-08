use iced::advanced::graphics::core::widget;

use iced::advanced::graphics::geometry;
use iced::advanced::{Layout, Widget, layout, renderer};
use iced::mouse::Cursor;
use iced::widget::canvas::{self, Cache, Canvas, Path, Program};
use iced::{Element, Length, Point, Rectangle, Size, Theme, theme};

pub struct WaveLoader<Renderer = iced::Renderer>
where
    Renderer: geometry::Renderer,
{
    pub phase: f32,
    pub bar_count: usize,
    pub cache: Cache<Renderer>,
}

impl<Renderer> WaveLoader<Renderer>
where
    Renderer: geometry::Renderer,
{
    pub fn new(phase: f32) -> Self {
        Self {
            phase,
            bar_count: 7,
            cache: Cache::new(),
        }
    }

    pub fn with_bar_count(mut self, bar_count: usize) -> Self {
        self.bar_count = bar_count;
        self
    }
    pub fn update_phase(&mut self, phase: f32) {
        if (self.phase - phase).abs() > f32::EPSILON {
            self.phase = phase;
            self.cache.clear();
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for WaveLoader<Renderer>
where
    Renderer: geometry::Renderer,
{
    fn size(&self) -> iced::Size<Length> {
        Size {
            width: Length::Shrink,
            height: Length::Shrink,
        }
    }
    fn layout(
        &self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(Size::new(200., 80.))
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
    ) {
        let canvas: Canvas<WaveLoaderProgram<'_, Renderer>, Message, Theme, Renderer> =
            Canvas::new(WaveLoaderProgram {
                phase: self.phase,
                bar_count: self.bar_count,
                cache: &self.cache,
            })
            .width(Length::Fill)
            .height(Length::FillPortion(3));
        // canvas.draw(state,renderer, theme, style, layout, cursor, viewport);
        canvas.draw(tree, renderer, theme, style, layout, cursor, viewport);
    }
}

impl<'a, Message> From<WaveLoader> for Element<'a, Message> {
    fn from(loader: WaveLoader) -> Self {
        Self::new(loader)
    }
}

#[derive(Clone, Copy)]
struct WaveLoaderProgram<'a, Renderer>
where
    Renderer: geometry::Renderer,
{
    phase: f32,
    bar_count: usize,
    cache: &'a Cache<Renderer>,
}

impl<'a, Message, Theme, Renderer> Program<Message, Theme, Renderer>
    for WaveLoaderProgram<'a, Renderer>
where
    Renderer: geometry::Renderer,
{
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let phase = self.phase;
        let bar_count = self.bar_count;
        let theme = theme.clone();

        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            let width = bounds.width;
            let height = bounds.height;
            let loader_width = width * 0.8;
            let loader_height = height * 0.6;
            let x_start = (width - loader_width) / 2.0;
            let y_base = height / 2.0;

            let bar_width = loader_width / (bar_count as f32 * 2.0);
            let bar_spacing = loader_width / (bar_count as f32);

            for i in 0..bar_count {
                let x = x_start + i as f32 * bar_spacing;
                let offset = i as f32 * 0.4;
                let wave = ((phase + offset).sin() + 1.0) / 2.0;
                let bar_h = loader_height * (0.3 + 0.7 * wave);
                let y = y_base - bar_h / 2.0;
                let rect = Rectangle {
                    x,
                    y,
                    width: bar_width,
                    height: bar_h,
                };
                let rounded_rect_path = Path::new(|p| {
                    p.rounded_rectangle(Point::new(x, y / 2.0), rect.size(), 15.0.into());
                });
                let palette = theme.extended_palette();
                let custom_color = palette.success.base.color;
                frame.fill(&rounded_rect_path, custom_color);
            }
        });

        vec![geometry]
    }
}
