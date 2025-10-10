use std::time::{Instant, SystemTime, UNIX_EPOCH};

use iced::{Element, widget::row};
use rand::Rng;

use crate::widget::chart::comparison::LineComparison;
use crate::widget::chart::{Series, Zoom};

pub struct ComparisonChart {
    zoom: Zoom,
    last_tick: Instant,
    series: Vec<Series>,
    update_interval: u64,
}

impl Default for ComparisonChart {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    ZoomChanged(Zoom),
}

impl ComparisonChart {
    pub fn new() -> Self {
        Self {
            last_tick: Instant::now(),
            zoom: Zoom::all(),
            series: sample_data(),
            update_interval: 1000,
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::ZoomChanged(zoom) => {
                self.zoom = zoom;
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let chart = LineComparison::new(&self.series, self.update_interval)
            .on_zoom(Message::ZoomChanged)
            .with_zoom(self.zoom);

        row![chart].padding(1).into()
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        if let Some(t) = now {
            self.last_tick = t;
        }

        rng_to_sample_data(&mut self.series);

        None
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }
}

/// Generate sample data with UNIX ms timestamps
fn sample_data() -> Vec<Series> {
    use rand::prelude::*;
    let mut rng = rand::rng();
    let n = 120;
    let now = unix_ms_now();

    let mut make_series = |name: &str, start: f32, drift: f32, noise: f32| -> Series {
        let mut y = start;
        let mut points = Vec::with_capacity(n);
        for i in 0..n {
            let step = rng.random_range(-noise..noise) + drift;
            y = (y + step).max(1e-6);
            let x = now - (n - i) as u64 * 1000; // each second back
            points.push((x, y));
        }
        Series {
            name: name.into(),
            points,
            color: None,
        }
    };

    vec![
        make_series("Alpha", 100.0, 0.08, 0.9),
        make_series("Beta", 80.0, 0.18, 1.2),
        make_series("Gamma", 120.0, 0.04, 1.6),
        make_series("Delta", 60.0, 0.22, 1.0),
        make_series("Epsilon", 200.0, -0.02, 2.0),
    ]
}

/// Update rng_to_sample_data to use current UNIX ms
fn rng_to_sample_data(data: &mut Vec<Series>) {
    for s in data {
        if let Some((_last_x, last_y)) = s.points.last().cloned() {
            let mut rng = rand::rng();
            let step = rng.random_range(-1.2..1.2) + 0.1;
            let new_y = (last_y + step).max(1e-6);
            let new_x = unix_ms_now();
            s.points.push((new_x, new_y));
            if s.points.len() > 500 {
                s.points.remove(0);
            }
        }
    }
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
