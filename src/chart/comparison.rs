use std::collections::HashMap;
use std::time::Instant;

use data::chart::Basis;
use exchange::adapter::StreamKind;
use exchange::fetcher::{FetchRange, RequestHandler};
use exchange::{Kline, TickerInfo, Timeframe};
use iced::{Element, widget::row};

use crate::widget::chart::comparison::LineComparison;
use crate::widget::chart::{Series, Zoom};

const SERIES_MAX_POINTS: usize = 5000;

pub enum Action {
    FetchRequested(uuid::Uuid, FetchRange, TickerInfo, Timeframe),
}

pub struct ComparisonChart {
    zoom: Zoom,
    last_tick: Instant,
    series: Vec<Series>,
    series_index: HashMap<TickerInfo, usize>,
    update_interval: u64,
    pub timeframe: Timeframe,
    request_handler: HashMap<TickerInfo, RequestHandler>,
    selected_tickers: Vec<TickerInfo>,
}

impl Default for ComparisonChart {
    fn default() -> Self {
        Self::new(Basis::Time(Timeframe::M15), &[], &[])
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    ZoomChanged(Zoom),
    DataRequest(FetchRange, TickerInfo),
}

impl ComparisonChart {
    pub fn new(basis: Basis, tickers: &[TickerInfo], _klines_raw: &[Kline]) -> Self {
        let timeframe = match basis {
            Basis::Time(tf) => tf,
            Basis::Tick(_) => todo!("WIP: ComparisonChart does not support tick basis"),
        };

        let mut series = Vec::with_capacity(tickers.len());
        let mut series_index = HashMap::new();
        for (i, t) in tickers.iter().enumerate() {
            series.push(Series {
                name: *t,
                points: Vec::new(),
                color: None,
            });
            series_index.insert(*t, i);
        }

        Self {
            last_tick: Instant::now(),
            zoom: Zoom::points(100),
            series,
            series_index,
            update_interval: 1000,
            timeframe,
            request_handler: tickers
                .iter()
                .map(|t| (*t, RequestHandler::new()))
                .collect(),
            selected_tickers: tickers.to_vec(),
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::ZoomChanged(zoom) => {
                self.zoom = zoom;
                None
            }
            Message::DataRequest(range, ticker_info) => {
                let handler = self.request_handler.entry(ticker_info).or_default();

                match handler.add_request(range) {
                    Ok(Some(req_id)) => Some(Action::FetchRequested(
                        req_id,
                        range,
                        ticker_info,
                        self.timeframe,
                    )),
                    Ok(None) => None,
                    Err(reason) => {
                        log::error!("Failed to request {:?}: {}", range, reason);
                        None
                    }
                }
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let chart = LineComparison::new(&self.series, self.update_interval, self.timeframe)
            .on_zoom(Message::ZoomChanged)
            .on_data_request(Message::DataRequest)
            .with_zoom(self.zoom);

        row![chart].padding(1).into()
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<super::Action> {
        if let Some(t) = now {
            self.last_tick = t;
        }
        None
    }

    pub fn insert_history(
        &mut self,
        req_id: uuid::Uuid,
        ticker_info: TickerInfo,
        klines: &[Kline],
    ) {
        let idx = self.get_or_create_series_idx(&ticker_info);
        let dst = &mut self.series[idx].points;

        let dt = self.timeframe.to_milliseconds().max(1);
        let align = |t: u64| (t / dt) * dt;

        let mut incoming: Vec<(u64, f32)> = klines
            .iter()
            .map(|k| (align(k.time), k.close.to_f32()))
            .collect();

        incoming.sort_by_key(|(x, _)| *x);
        incoming.dedup_by_key(|(x, _)| *x);

        if incoming.is_empty()
            && let Some(handler) = self.request_handler.get_mut(&ticker_info)
        {
            handler.mark_failed(req_id, "No data received".to_string());
            return;
        }

        if dst.is_empty() {
            *dst = incoming;
        } else {
            let mut i = 0usize;
            let mut j = 0usize;
            let mut merged = Vec::with_capacity(dst.len() + incoming.len());

            while i < dst.len() && j < incoming.len() {
                let (x0, y0) = dst[i];
                let (x1, y1) = incoming[j];
                if x0 < x1 {
                    merged.push((x0, y0));
                    i += 1;
                } else if x1 < x0 {
                    merged.push((x1, y1));
                    j += 1;
                } else {
                    // equal timestamp: prefer incoming
                    merged.push((x1, y1));
                    i += 1;
                    j += 1;
                }
            }
            if i < dst.len() {
                merged.extend_from_slice(&dst[i..]);
            }
            if j < incoming.len() {
                merged.extend_from_slice(&incoming[j..]);
            }

            merged.dedup_by_key(|(x, _)| *x);

            *dst = merged;
        }

        if self.series[idx].points.len() > SERIES_MAX_POINTS {
            let drop = self.series[idx].points.len() - SERIES_MAX_POINTS;
            self.series[idx].points.drain(0..drop);
        }

        if let Some(handler) = self.request_handler.get_mut(&ticker_info) {
            handler.mark_completed(req_id);
        }
    }

    pub fn update_latest_kline(&mut self, ticker_info: &TickerInfo, kline: &Kline) {
        let idx = self.get_or_create_series_idx(ticker_info);
        let series = &mut self.series[idx];

        // Align to timeframe grid
        let dt = self.timeframe.to_milliseconds().max(1);
        let t = (kline.time / dt) * dt;
        let new_point = (t, kline.close.to_f32());

        if let Some((last_x, last_y)) = series.points.last_mut() {
            if *last_x == new_point.0 {
                *last_y = new_point.1;
            } else if new_point.0 > *last_x {
                series.points.push(new_point);
            }
        } else {
            series.points.push(new_point);
        }

        // Use same cap as history to avoid churn/backfill loops
        if series.points.len() > SERIES_MAX_POINTS {
            let drop = series.points.len() - SERIES_MAX_POINTS;
            series.points.drain(0..drop);
        }
    }

    fn get_or_create_series_idx(&mut self, ticker_info: &TickerInfo) -> usize {
        if let Some(&i) = self.series_index.get(ticker_info) {
            i
        } else {
            let i = self.series.len();
            self.series.push(Series {
                name: *ticker_info,
                points: Vec::new(),
                color: None,
            });
            self.series_index.insert(*ticker_info, i);
            i
        }
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn add_ticker(&mut self, ticker_info: &TickerInfo) -> Vec<StreamKind> {
        let _ = self.get_or_create_series_idx(ticker_info);
        if !self.selected_tickers.contains(ticker_info) {
            self.selected_tickers.push(*ticker_info);
        }

        let mut new_streams = vec![];

        for ticker in self.selected_tickers.iter() {
            let stream = StreamKind::Kline {
                ticker_info: *ticker,
                timeframe: self.timeframe,
            };
            if !new_streams.contains(&stream) {
                new_streams.push(stream);
            }
        }

        new_streams
    }

    pub fn remove_ticker(&mut self, ticker_info: &TickerInfo) -> Vec<StreamKind> {
        if let Some(idx) = self.series_index.remove(ticker_info) {
            self.series.remove(idx);
            self.series_index.clear();
            for (i, s) in self.series.iter().enumerate() {
                self.series_index.insert(s.name, i);
            }
        }

        self.selected_tickers.retain(|t| t != ticker_info);

        let mut new_streams = vec![];

        for ticker in self.selected_tickers.iter() {
            let stream = StreamKind::Kline {
                ticker_info: *ticker,
                timeframe: self.timeframe,
            };
            if !new_streams.contains(&stream) {
                new_streams.push(stream);
            }
        }
        new_streams
    }

    pub fn selected_tickers(&self) -> &Vec<TickerInfo> {
        &self.selected_tickers
    }
}
