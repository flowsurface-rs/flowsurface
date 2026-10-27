mod catalog;
mod installed;

use super::KlineChart;
use data::chart::kline::IndicatorId;
use data::chart::kline::KlineStudy;
use data::chart::kline::indicators::manifest::{self, Message as CatalogMessage};
use iced::Element;

#[derive(Debug, Clone)]
pub enum Message {
    OpenCatalog,
    Back,
    CatalogSearchChanged(String),
    ToggleCatalogEntry(IndicatorId),
    Catalog(CatalogMessage),
    SearchChanged(String),
    ToggleIndicator(IndicatorId),
    ToggleStudy(KlineStudy),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Installed,
    Catalog,
}

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    page: Page,
    installed: installed::ViewState,
    catalog: catalog::ViewState,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            page: Page::Installed,
            installed: installed::ViewState::default(),
            catalog: catalog::ViewState::default(),
        }
    }

    pub(super) fn open_catalog(&mut self) {
        self.page = Page::Catalog;
    }

    pub(super) fn go_back(&mut self) -> bool {
        if self.page == Page::Catalog {
            self.page = Page::Installed;
            return true;
        }
        false
    }

    pub(super) fn set_catalog_query(&mut self, query: String) {
        self.catalog.set_search(query);
    }

    pub(super) fn toggle_catalog_entry(&mut self, id: IndicatorId) {
        self.catalog.toggle_details(id);
    }

    pub(super) fn title(&self) -> &'static str {
        match self.page {
            Page::Installed => "Add indicators or studies",
            Page::Catalog => "Browse indicators",
        }
    }

    pub(super) fn set_search_query(&mut self, query: String) {
        self.installed.set_search(query);
    }

    pub fn search_box_id(&self) -> iced::widget::Id {
        self.installed.search_box_id()
    }

    pub(super) fn view<'a>(
        &'a self,
        chart: &'a KlineChart,
        catalog: &'a manifest::State,
    ) -> Element<'a, Message> {
        let body = match self.page {
            Page::Installed => self.installed.view(chart),
            Page::Catalog => self.catalog.view(catalog),
        };

        let notice = (self.page == Page::Catalog && catalog.restart_pending()).then(|| {
            crate::widget::banner(
                "Restart required.\nChanges will take effect after restart.",
                crate::widget::BannerKind::Warning,
            )
        });

        crate::widget::inspector::panel_page(body, notice)
    }
}
