use std::collections::HashMap;

use data::{ScaleFactor, layout::WindowSpec};
use iced::{Point, Size, Subscription, Task, window};

pub use iced::window::{Id, Position, Settings, close, open};

// Windows popout rendering can freeze, enable when upstream rendering is fixed.
pub const MULTI_WINDOW_SUPPORTED: bool = !cfg!(target_os = "windows");

const DEFAULT_SCALE: f32 = 1.0;

#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub id: Id,
    pub position: Option<Point>,
}

impl Window {
    pub fn new(id: Id) -> Self {
        Self { id, position: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WindowScaleFactor(f32);

impl WindowScaleFactor {
    pub(crate) fn new(value: f32) -> Self {
        Self(positive_scale_or_default(value))
    }

    pub(crate) fn value(self) -> f32 {
        self.0
    }
}

impl Default for WindowScaleFactor {
    fn default() -> Self {
        Self(DEFAULT_SCALE)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PixelRatio(f32);

impl PixelRatio {
    fn from_product(value: f32) -> Self {
        Self(positive_scale_or_default(value))
    }

    pub(crate) fn value(self) -> f32 {
        self.0
    }
}

impl Default for PixelRatio {
    fn default() -> Self {
        Self(DEFAULT_SCALE)
    }
}

#[derive(Debug, Default)]
pub(crate) struct WindowScaleFactors {
    factors: HashMap<Id, WindowScaleFactor>,
}

impl WindowScaleFactors {
    pub(crate) fn new(main_window: Id) -> Self {
        Self {
            factors: HashMap::from([(main_window, WindowScaleFactor::default())]),
        }
    }

    pub(crate) fn set(&mut self, window: Id, scale_factor: WindowScaleFactor) {
        self.factors.insert(window, scale_factor);
    }

    pub(crate) fn remove(&mut self, window: Id) {
        self.factors.remove(&window);
    }

    pub(crate) fn pixel_ratio_for_window(
        &self,
        window: Id,
        ui_scale_factor: ScaleFactor,
    ) -> PixelRatio {
        let window_scale_factor = self.factors.get(&window).copied().unwrap_or_default();

        PixelRatio::from_product(f32::from(ui_scale_factor) * window_scale_factor.value())
    }
}

fn positive_scale_or_default(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        DEFAULT_SCALE
    }
}

pub fn default_size() -> Size {
    WindowSpec::default().size()
}

#[derive(Debug, Clone, Copy)]
pub enum Event {
    Opened(window::Id),
    Rescaled(window::Id, WindowScaleFactor),
    CloseRequested(window::Id),
}

pub fn events() -> Subscription<Event> {
    iced::event::listen_with(filtered_events)
}

fn filtered_events(
    event: iced::Event,
    _status: iced::event::Status,
    window: window::Id,
) -> Option<Event> {
    match &event {
        iced::Event::Window(iced::window::Event::Opened { .. }) => Some(Event::Opened(window)),
        iced::Event::Window(iced::window::Event::Rescaled(scale_factor)) => Some(Event::Rescaled(
            window,
            WindowScaleFactor::new(*scale_factor),
        )),
        iced::Event::Window(iced::window::Event::CloseRequested) => {
            Some(Event::CloseRequested(window))
        }
        _ => None,
    }
}

pub fn scale_factor<M, F>(window_id: window::Id, message: F) -> Task<M>
where
    F: Fn(window::Id, WindowScaleFactor) -> M + Send + 'static,
    M: Send + 'static,
{
    iced::window::scale_factor(window_id)
        .map(move |value| message(window_id, WindowScaleFactor::new(value)))
}

pub fn collect_window_specs<M, F>(window_ids: Vec<window::Id>, message: F) -> Task<M>
where
    F: Fn(HashMap<window::Id, WindowSpec>) -> M + Send + 'static,
    M: Send + 'static,
{
    // Create a task that collects specs for each window
    let window_spec_tasks = window_ids
        .into_iter()
        .map(|window_id| {
            // Map both tasks to produce an enum or tuple to distinguish them
            let pos_task: Task<(Option<Point>, Option<Size>)> =
                iced::window::position(window_id).map(|pos| (pos, None));

            let size_task: Task<(Option<Point>, Option<Size>)> =
                iced::window::size(window_id).map(|size| (None, Some(size)));

            Task::batch(vec![pos_task, size_task])
                .collect()
                .map(move |results| {
                    let position = results.iter().find_map(|(pos, _)| *pos);
                    let size = results
                        .iter()
                        .find_map(|(_, size)| *size)
                        .unwrap_or_else(|| Size::new(1024.0, 768.0));

                    (window_id, (position, size))
                })
        })
        .collect::<Vec<_>>();

    // Batch all window tasks together and collect results
    Task::batch(window_spec_tasks)
        .collect()
        .map(move |results| {
            let specs: HashMap<window::Id, WindowSpec> = results
                .into_iter()
                .filter_map(|(id, (pos, size))| {
                    pos.map(|position| (id, WindowSpec::from((&position, &size))))
                })
                .collect();

            message(specs)
        })
}

#[cfg(target_os = "linux")]
pub fn settings() -> Settings {
    Settings {
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: "com.flowsurface.flowsurface".into(),
            ..Default::default()
        },
        min_size: Some(Size::new(800.0, 600.0)),
        ..Default::default()
    }
}

#[cfg(target_os = "macos")]
pub fn settings() -> Settings {
    use iced::window;

    Settings {
        platform_specific: window::settings::PlatformSpecific {
            title_hidden: true,
            titlebar_transparent: true,
            fullsize_content_view: true,
        },
        min_size: Some(Size::new(800.0, 600.0)),
        ..Default::default()
    }
}

#[cfg(target_os = "windows")]
pub fn settings() -> Settings {
    Settings {
        icon: Some(
            window::icon::from_rgba(
                include_bytes!("../assets/app-icon/windows-window.rgba").to_vec(),
                64,
                64,
            )
            .expect("embedded Windows window icon must be valid"),
        ),
        min_size: Some(Size::new(800.0, 600.0)),
        ..Default::default()
    }
}
