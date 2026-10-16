use iced::{
    Element, Length, Renderer, Size, advanced,
    widget::{column, container, row, rule},
};

const BODY_VERTICAL_PADDING: f32 = 4.0;
const SECTION_TITLE_SPACING: f32 = 8.0;
pub(crate) const SECTION_SPACING: f32 = 16.0;
pub(crate) const MODAL_SECTION_SPACING: f32 = 8.0;
const MODAL_ROW_SPACING: f32 = 4.0;
const SECTION_PADDING: f32 = 8.0;
const FOOTER_SPACING: f32 = 4.0;

pub(crate) fn modal_container<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .padding(SECTION_PADDING)
        .style(crate::style::modal_container)
        .into()
}

pub(crate) fn modal_row<'a, Message: 'a>(
    children: impl IntoIterator<Item = Element<'a, Message>>,
) -> Element<'a, Message> {
    crate::widget::decorate::decorate(row(children).width(Length::Fill).spacing(MODAL_ROW_SPACING))
        .layout(EqualHeightRowLayout)
        .into()
}

struct EqualHeightRowLayout;

impl<'a, Message: 'a> crate::widget::decorate::Layout<'a, Message, iced::Theme, Renderer, ()>
    for EqualHeightRowLayout
{
    fn layout(
        &mut self,
        _state: &mut (),
        inner: &mut Element<'a, Message>,
        tree: &mut advanced::widget::Tree,
        renderer: &Renderer,
        limits: &advanced::layout::Limits,
    ) -> advanced::layout::Node {
        let layout = inner.as_widget_mut().layout(tree, renderer, limits);
        equalize_row_height(layout)
    }
}

fn equalize_row_height(layout: advanced::layout::Node) -> advanced::layout::Node {
    let row_height = layout
        .children()
        .iter()
        .map(|child| child.size().height)
        .fold(layout.size().height, f32::max);
    let children = layout
        .children()
        .iter()
        .map(|child| {
            let bounds = child.bounds();
            advanced::layout::Node::with_children(
                Size::new(bounds.width, row_height),
                child.children().to_vec(),
            )
            .move_to(bounds.position())
        })
        .collect();

    advanced::layout::Node::with_children(Size::new(layout.size().width, row_height), children)
        .move_to(layout.bounds().position())
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

    pub(crate) fn map<Next>(
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

        for (index, section) in self.sections.into_iter().enumerate() {
            if index > 0 {
                content = content.push(rule::horizontal(1.0).style(crate::style::split_ruler));
            }
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

    pub fn set_footer(&mut self, footer: impl Into<Element<'a, Message>>) {
        self.footer = Some(footer.into());
    }

    pub fn into_element(self) -> Element<'a, Message> {
        InspectorSectionList::new(self.sections).view()
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
    pub fn view<'a, Message: 'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
        Self::container(crate::widget::scrollable_content(content))
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
    pub fn view<'a, Message: 'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
        column![
            rule::horizontal(1.0).style(crate::style::split_ruler),
            container(content)
                .width(Length::Fill)
                .padding([BODY_VERTICAL_PADDING, 0.0]),
        ]
        .spacing(FOOTER_SPACING)
        .into()
    }
}
