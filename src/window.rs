use std::collections::HashMap;

use iced::{window, Point, Size, Subscription, Task};

pub use iced::window::{close, open, Id, Position, Settings};
use iced_futures::MaybeSend;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub id: Id,
    pub position: Option<Point>,
    pub size: Size,
    pub focused: bool,
}

#[allow(dead_code)]
impl Window {
    pub fn new(id: Id) -> Self {
        Self {
            id,
            position: None,
            size: Size::default(),
            focused: false,
        }
    }

    pub fn opened(&mut self, position: Option<Point>, size: Size) {
        self.position = position;
        self.size = size;
        self.focused = true;
    }

    pub fn resized(&mut self, size: Size) {
        self.size = size;
    }

    pub fn moved(&mut self, position: Point) {
        self.position = Some(position);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum WindowEvent {
    CloseRequested(window::Id),
}

pub fn window_events() -> Subscription<WindowEvent> {
    iced::event::listen_with(filtered_events)
}

fn filtered_events(
    event: iced::Event,
    _status: iced::event::Status,
    window: window::Id,
) -> Option<WindowEvent> {
    match &event {
        iced::Event::Window(iced::window::Event::CloseRequested) => {
            Some(WindowEvent::CloseRequested(window))
        }
        _ => None,
    }
}

pub fn collect_window_specs<M, F>(window_ids: Vec<window::Id>, message: F) -> Task<M>
where
    F: Fn(HashMap<window::Id, (Point, Size)>) -> M + Send + 'static,
    M: MaybeSend + 'static,
{
    // Create a task that collects specs for each window
    let window_spec_tasks: Vec<Task<(window::Id, (Option<Point>, Size))>> = window_ids
        .into_iter()
        .map(|window_id| {
            // Map both tasks to produce an enum or tuple to distinguish them
            let pos_task: Task<(Option<Point>, Option<Size>)> =
                iced::window::get_position(window_id).map(|pos| (pos, None));

            let size_task: Task<(Option<Point>, Option<Size>)> =
                iced::window::get_size(window_id).map(|size| (None, Some(size)));

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
        .collect();

    // Batch all window tasks together and collect results
    Task::batch(window_spec_tasks)
        .collect()
        .map(move |results| {
            let specs: HashMap<window::Id, (Point, Size)> = results
                .into_iter()
                .filter_map(|(id, (pos, size))| pos.map(|position| (id, (position, size))))
                .collect();

            message(specs)
        })
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
        ..Default::default()
    }
}