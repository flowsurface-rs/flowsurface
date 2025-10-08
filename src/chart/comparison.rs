use std::time::Instant;

use iced::{Element, widget::row};
use rand::Rng;

use crate::widget::chart::{LineComparison, Series, Zoom};

pub struct ComparisonChart {
    zoom: Zoom,
    last_tick: Instant,
    series: Vec<Series>,
    update_interval: u64,
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
            update_interval: 100,
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
        let chart = LineComparison::new(self.series.clone(), self.update_interval)
            .on_zoom(|z| Message::ZoomChanged(z))
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

/// Modifies the data in place by adding a new point to each series.
fn rng_to_sample_data(data: &mut Vec<Series>) {
    for s in data {
        if let Some((last_x, last_y)) = s.points.last().cloned() {
            let mut rng = rand::rng();
            let step: f64 = rng.random_range(-1.2..1.2) + 0.1;
            let new_y = (last_y + step).max(1e-6);
            s.points.push((last_x + 1.0, new_y));
            if s.points.len() > 500 {
                s.points.remove(0);
            }
        }
    }
}

fn sample_data() -> Vec<Series> {
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

    vec![
        make_series("Alpha", 100.0, 0.08, 0.9),
        make_series("Beta", 80.0, 0.18, 1.2),
        make_series("Gamma", 120.0, 0.04, 1.6),
    ]
}
