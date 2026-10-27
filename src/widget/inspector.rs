use iced::{
    Element, Length,
    widget::{button, column, container, row, rule},
};

const BODY_VERTICAL_PADDING: f32 = 8.0;
const SECTION_TITLE_SPACING: f32 = 8.0;
pub const SECTION_SPACING: f32 = 16.0;
pub const MODAL_SECTION_SPACING: f32 = 8.0;
const SECTION_PADDING: f32 = 8.0;
const FOOTER_SPACING: f32 = 4.0;
const FOOTER_VERTICAL_PADDING: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorTab {
    Inputs,
    Style,
    Source,
}

pub fn modal_container<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .padding(SECTION_PADDING)
        .style(crate::style::inspector_section)
        .into()
}

pub fn setting_section<'a, Message>(
    label: impl iced::widget::text::IntoFragment<'a>,
    control: impl Into<Element<'a, Message>>,
) -> InspectorSection<'a, Message>
where
    Message: 'a,
{
    InspectorSection::new(
        crate::widget::text::body(label),
        container(control).width(Length::Fill),
    )
}

fn tabs<'a, Message>(
    selected_tab: InspectorTab,
    available_tabs: &[InspectorTab],
    on_select: impl Fn(InspectorTab) -> Message + Copy + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let tab_button = move |label, tab, width| {
        let is_selected = selected_tab == tab;
        let label = crate::widget::text::emphasis(label).align_x(iced::Alignment::Center);
        let label = if is_selected {
            label.style(|theme| iced::widget::text::Style {
                color: Some(theme.extended_palette().primary.strong.color),
            })
        } else {
            label
        };
        let button = button(label)
            .padding([4, 10])
            .width(width)
            .style(move |theme, status| {
                let mut style = crate::style::button::transparent(theme, status, is_selected);
                if is_selected {
                    style.background = None;
                }
                style
            });

        let button = if is_selected {
            button
        } else {
            button.on_press(on_select(tab))
        };

        column![
            button,
            rule::horizontal(2.0).style(move |theme| {
                let mut style = crate::style::tab_ruler(theme);
                if is_selected {
                    style.color = theme.extended_palette().primary.strong.color;
                }
                style
            }),
        ]
        .width(width)
    };

    let mut tab_buttons = row![];
    for tab in available_tabs {
        let (label, width) = match tab {
            InspectorTab::Source => ("i", Length::Shrink),
            InspectorTab::Inputs => ("Inputs", Length::FillPortion(1)),
            InspectorTab::Style => ("Style", Length::FillPortion(1)),
        };
        tab_buttons = tab_buttons.push(tab_button(label, *tab, width));
    }

    column![container(tab_buttons).center_x(Length::Fill)]
        .spacing(4)
        .into()
}

pub fn tabbed_content<'a, Message>(
    selected_tab: InspectorTab,
    on_select: impl Fn(InspectorTab) -> Message + Copy + 'a,
    contents: impl IntoIterator<Item = (InspectorTab, Option<Element<'a, Message>>)>,
) -> (InspectorTab, Element<'a, Message>)
where
    Message: Clone + 'a,
{
    let mut contents = contents
        .into_iter()
        .filter_map(|(tab, content)| content.map(|content| (tab, content)))
        .collect::<Vec<_>>();
    contents.sort_by_key(|(tab, _)| match tab {
        InspectorTab::Source => 0,
        InspectorTab::Inputs => 1,
        InspectorTab::Style => 2,
    });
    let Some((first_tab, _)) = contents.first() else {
        return (selected_tab, column![].into());
    };
    let selected_tab = if contents.iter().any(|(tab, _)| *tab == selected_tab) {
        selected_tab
    } else {
        *first_tab
    };
    let available_tabs = contents.iter().map(|(tab, _)| *tab).collect::<Vec<_>>();
    let tabs = tabs(selected_tab, &available_tabs, on_select);
    let Some((_, content)) = contents.into_iter().find(|(tab, _)| *tab == selected_tab) else {
        return (selected_tab, column![].into());
    };

    (
        selected_tab,
        column![tabs, content].spacing(MODAL_SECTION_SPACING).into(),
    )
}

pub struct InspectorSection<'a, Message> {
    content: Element<'a, Message>,
}

impl<'a, Message: 'a> InspectorSection<'a, Message> {
    pub fn new(
        title: impl Into<Element<'a, Message>>,
        setting: impl Into<Element<'a, Message>>,
    ) -> Self {
        Self {
            content: column![title.into(), setting.into()]
                .width(Length::Fill)
                .spacing(SECTION_TITLE_SPACING)
                .into(),
        }
    }

    pub fn titled(title: &'a str, setting: impl Into<Element<'a, Message>>) -> Self {
        Self::new(crate::widget::text::section(title), setting)
    }

    pub fn into_element(self) -> Element<'a, Message> {
        self.content
    }

    pub fn with_modal_container(self) -> Self {
        Self {
            content: modal_container(self.content),
        }
    }

    pub fn map<Next>(
        self,
        mapper: impl Fn(Message) -> Next + Copy + 'a,
    ) -> InspectorSection<'a, Next>
    where
        Next: 'a,
    {
        InspectorSection {
            content: self.content.map(mapper),
        }
    }
}

pub struct InspectorSectionList<'a, Message> {
    sections: Vec<InspectorSection<'a, Message>>,
}

impl<'a, Message: 'a> InspectorSectionList<'a, Message> {
    pub fn new(sections: Vec<InspectorSection<'a, Message>>) -> Self {
        Self { sections }
    }

    fn view(self) -> Element<'a, Message> {
        let mut content = column![].spacing(SECTION_SPACING).width(Length::Fill);

        for section in self.sections.into_iter() {
            content = content.push(section.content);
        }

        content.into()
    }
}

pub struct InspectorContent<'a, Message> {
    sections: Vec<InspectorSection<'a, Message>>,
    footer: Option<Element<'a, Message>>,
}

impl<'a, Message: 'a> InspectorContent<'a, Message> {
    pub fn new(sections: Vec<InspectorSection<'a, Message>>) -> Self {
        Self {
            sections,
            footer: None,
        }
    }

    pub fn push(&mut self, section: InspectorSection<'a, Message>) {
        self.sections.push(section);
    }

    pub fn set_footer(&mut self, footer: impl Into<Element<'a, Message>>) {
        self.footer = Some(footer.into());
    }

    pub fn into_element(self) -> Element<'a, Message> {
        InspectorSectionList::new(self.sections).view()
    }

    /// Lays the sections and footer out as a page capped at `available_height`.
    pub fn into_page(self, available_height: f32) -> Element<'a, Message> {
        let (sections, footer) = self.into_parts();
        let footer = footer.map(|footer| InspectorFooter::view(footer, InspectorFooter::HEIGHT));
        let reserved = if footer.is_some() {
            InspectorFooter::HEIGHT
        } else {
            0.0
        };

        let mut page = column![InspectorBody::view(
            sections,
            (available_height - reserved).max(0.0)
        )];
        if let Some(footer) = footer {
            page = page.push(footer);
        }
        page.into()
    }

    pub fn map<Next>(
        self,
        mapper: impl Fn(Message) -> Next + Copy + 'a,
    ) -> InspectorContent<'a, Next>
    where
        Next: 'a,
    {
        InspectorContent {
            sections: self
                .sections
                .into_iter()
                .map(|section| section.map(mapper))
                .collect(),
            footer: self.footer.map(|footer| footer.map(mapper)),
        }
    }

    pub fn into_parts(self) -> (Element<'a, Message>, Option<Element<'a, Message>>) {
        let Self { sections, footer } = self;
        (InspectorSectionList::new(sections).view(), footer)
    }
}

pub struct InspectorBody;

impl InspectorBody {
    pub fn view<'a, Message: 'a>(
        content: impl Into<Element<'a, Message>>,
        max_height: f32,
    ) -> Element<'a, Message> {
        container(crate::widget::scrollable_content(content))
            .width(Length::Fill)
            .height(Length::Shrink)
            .max_height(max_height)
            .padding([BODY_VERTICAL_PADDING, 0.0])
            .into()
    }

    pub fn view_without_scroll<'a, Message: 'a>(
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        Self::container(content)
    }

    fn container<'a, Message: 'a>(
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([BODY_VERTICAL_PADDING, 0.0])
            .into()
    }
}

pub struct InspectorFooter;

impl InspectorFooter {
    pub const HEIGHT: f32 = 40.0;

    pub fn view<'a, Message: 'a>(
        content: impl Into<Element<'a, Message>>,
        height: f32,
    ) -> Element<'a, Message> {
        Self::band(content, Length::Fixed(height))
    }

    /// Footer sized by its content, for notices that wrap.
    pub fn view_shrink<'a, Message: 'a>(
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        Self::band(content, Length::Shrink)
    }

    fn band<'a, Message: 'a>(
        content: impl Into<Element<'a, Message>>,
        height: Length,
    ) -> Element<'a, Message> {
        column![
            rule::horizontal(1.0).style(crate::style::split_ruler),
            container(content)
                .width(Length::Fill)
                .padding([FOOTER_VERTICAL_PADDING, 0.0]),
        ]
        .spacing(FOOTER_SPACING)
        .height(height)
        .into()
    }
}

/// A page whose body fills the inspector, with an optional notice footer.
pub fn panel_page<'a, Message: 'a>(
    body: impl Into<Element<'a, Message>>,
    notice: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut page = column![InspectorBody::view_without_scroll(body)];
    if let Some(notice) = notice {
        page = page.push(InspectorFooter::view_shrink(notice));
    }
    page.into()
}
