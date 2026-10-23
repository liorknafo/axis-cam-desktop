//! Contextual help shared by every command and form field.
use crate::{Message, Target, choice_diagram::SPIN, model::Strategy};
use iced::{
    Border, Color, Element, Event, Font, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, overlay, renderer,
        widget::{Operation, Tree, tree},
    },
    mouse,
    widget::{self, canvas, column, container, row, text, tooltip},
    window,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Diagram {
    Operation(Strategy),
    Heights,
    FixtureClearance,
    Offsets,
    Stepover,
    Stepdown,
    Orientation,
    Ramp,
    Tool,
    Allowance,
    StockOffset(usize, bool),
    MillingDirection,
    MillingChoice(bool, bool),
    Spindle(bool),
    FacingPattern(bool),
    FacingLink(bool),
    Entry(bool),
    DrillCycle(u8),
    Containment(i8),
    // A field's own picture, numbered within its group (`field_art`).
    Vise(u8),
    Clamp(u8),
    Stock(u8),
    Chuck(u8),
    LatheTool(u8),
    TurnOp(u8),
    Probe(u8),
    Post(u8),
    Linking(u8),
    Manual(u8),
    Machine(u8),
    Cutting(u8),
}
#[derive(Clone, Debug)]
pub struct Help {
    pub title: String,
    /// The hover text: what the button or the selected value does and one
    /// caveat, about three sentences and under 350 characters
    /// (`tests::SHORT_LIMIT`).
    pub body: String,
    /// The rest, shown by More… on the card: the procedure, the limits and
    /// the conventions. Empty when the hover text says it all.
    pub more: String,
    pub diagram: Option<Diagram>,
}
impl Help {
    pub fn text(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            more: String::new(),
            diagram: None,
        }
    }
    /// The text More… adds to the card.
    pub fn more(mut self, more: impl Into<String>) -> Self {
        self.more = more.into();
        self
    }
    fn animated(mut self, d: Diagram) -> Self {
        self.diagram = Some(d);
        self
    }
}
pub struct HelpButton<'a> {
    button: widget::Button<'a, Message>,
    help: Help,
}
impl<'a> HelpButton<'a> {
    pub fn new(
        button: widget::Button<'a, Message>,
        message: &Message,
        label: impl Into<String>,
    ) -> Self {
        Self {
            button,
            help: command(message, label.into()),
        }
    }
    pub fn operation(
        button: widget::Button<'a, Message>,
        strategy: Strategy,
        name: &str,
        mark: String,
    ) -> Self {
        Self {
            button,
            help: operation_help(strategy, name, mark),
        }
    }
    /// A ribbon tile for a strategy the open job cannot run: the hover help
    /// leads with the reason, then says where a click goes.
    pub fn unavailable(button: widget::Button<'a, Message>, strategy: Strategy) -> Self {
        Self {
            button,
            help: unavailable_help(strategy),
        }
    }
    /// A ribbon tile for a strategy the active setup's machine cannot run:
    /// the hover help is the machine's `reason`.
    pub fn dimmed(
        button: widget::Button<'a, Message>,
        strategy: Strategy,
        reason: &'static str,
    ) -> Self {
        Self {
            button,
            help: dimmed_help(strategy, reason),
        }
    }
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.button = self.button.width(width);
        self
    }
}
/// The hover help of a dimmed strategy tile: the reason first, then where
/// a click goes, and under More… what the strategy does.
pub fn unavailable_help(strategy: Strategy) -> Help {
    let availability = strategy.availability();
    let reason = availability
        .strip_prefix("Unavailable · ")
        .unwrap_or("this part of the job has no preview yet");
    Help::text(
        format!("{} (unavailable)", strategy.name()),
        format!(
            "Unavailable for imported parts: {reason}.\n\nClick to open Operation support, which lists what each strategy covers."
        ),
    )
    .more(strategy.help())
    .animated(Diagram::Operation(strategy))
}
/// The hover help of a strategy dimmed by the machine: its reason, and under
/// More… what the strategy does.
pub fn dimmed_help(strategy: Strategy, reason: &'static str) -> Help {
    Help::text(format!("{} (unavailable)", strategy.name()), reason)
        .more(strategy.help())
        .animated(Diagram::Operation(strategy))
}
impl<'a> From<HelpButton<'a>> for Element<'a, Message> {
    fn from(b: HelpButton<'a>) -> Self {
        wrap(b.button, b.help, tooltip::Position::Bottom)
    }
}
/// An operation's card in the setup browser: its mark (`ui::mark_help`)
/// and the click, and under More… what its strategy does.
pub fn operation_help(strategy: Strategy, name: &str, mark: String) -> Help {
    Help::text(
        name,
        format!("{mark}\n\nClick to edit this operation's tool, geometry and cutting parameters."),
    )
    .more(strategy.help())
    .animated(Diagram::Operation(strategy))
}
/// The width of a card, and of one expanded by More….
const CARD_WIDTH: f32 = 278.;
const EXPANDED_WIDTH: f32 = 340.;
/// The card's footer toggles.
pub const MORE: &str = "More…";
pub const LESS: &str = "Less";
pub const PAUSE: &str = "Pause";
pub const PLAY: &str = "Play";
/// A help card: its title, its hover text and its animation. `expanded`
/// adds the More… text under the hover text and widens the card; `paused`
/// holds the animation on a still frame. The footer offers More… (then
/// Less) when there is more, and Pause (then Play) when there is an
/// animation.
pub fn card<'a>(h: Help, expanded: bool, paused: bool) -> Element<'a, Message> {
    let has_more = !h.more.is_empty();
    let mut c = column![
        text(h.title).size(14).font(Font {
            weight: iced::font::Weight::Semibold,
            ..Font::DEFAULT
        }),
        text(h.body).size(12)
    ]
    .spacing(9)
    .width(if expanded { EXPANDED_WIDTH } else { CARD_WIDTH });
    if expanded && has_more {
        c = c.push(text(h.more).size(12));
    }
    if let Some(d) = h.diagram {
        c = c
            .push(
                container(
                    canvas(Illustration {
                        diagram: d,
                        still: paused.then_some(STILL_TIME),
                    })
                    .width(CARD_WIDTH)
                    .height(155),
                )
                .center_x(Length::Fill),
            )
            .push(
                text("Concept illustration · not a computed toolpath")
                    .size(10)
                    .color(Color::from_rgb8(159, 189, 209)),
            );
    }
    if has_more || h.diagram.is_some() {
        let mut footer = widget::Row::new().spacing(14);
        if has_more {
            footer = footer.push(link(
                if expanded { LESS } else { MORE },
                Message::HelpMore(!expanded),
            ));
        }
        if h.diagram.is_some() {
            footer = footer.push(link(
                if paused { PLAY } else { PAUSE },
                Message::HelpPause(!paused),
            ));
        }
        c = c.push(footer);
    }
    container(c)
        .padding(13)
        .style(|_| container::Style {
            background: Some(Color::from_rgb8(27, 44, 61).into()),
            text_color: Some(Color::from_rgb8(236, 242, 249)),
            border: Border {
                color: Color::from_rgb8(71, 95, 118),
                width: 1.,
                radius: 7.into(),
            },
            ..Default::default()
        })
        .into()
}
/// A footer toggle: a small link.
fn link<'a>(label: &'static str, m: Message) -> Element<'a, Message> {
    widget::button(text(label).size(11))
        .on_press(m)
        .padding([2, 0])
        .style(|_, status| widget::button::Style {
            background: None,
            text_color: if matches!(status, widget::button::Status::Hovered) {
                Color::WHITE
            } else {
                Color::from_rgb8(159, 189, 209)
            },
            ..Default::default()
        })
        .into()
}
pub fn wrap<'a>(
    content: impl Into<Element<'a, Message>>,
    h: Help,
    position: tooltip::Position,
) -> Element<'a, Message> {
    Element::new(HoverHelp {
        content: content.into(),
        help: h,
        position,
        card: None,
    })
}
const HELP_DELAY: Duration = Duration::from_millis(600);
/// How long an open card stays once the pointer is off both the control
/// and the card: the time to cross the gap between them.
const LINGER: Duration = Duration::from_millis(250);
/// The gap between a control and its card.
const GAP: f32 = 9.;
/// A control and its hover card. The card opens `HELP_DELAY` after the
/// pointer comes to the control, once the pointer has been off the control
/// since it appeared or was last pressed: a click shuts the card, and it
/// stays shut until the pointer leaves the control and comes back, so it
/// never hides the page or result the click opened (LIO-148). Open, the
/// card stays while the pointer is on the control or on the card, and for
/// `LINGER` on the way between them, so its More… and Pause toggles can be
/// clicked (issue #89); More… and Pause are forgotten when it closes.
pub struct HoverHelp<'a> {
    content: Element<'a, Message>,
    help: Help,
    position: tooltip::Position,
    /// The card, built while it is open, for the (expanded, paused) it shows.
    card: Option<(bool, bool, Element<'a, Message>)>,
}
#[derive(Clone, Copy, Debug, Default)]
struct Hover {
    /// The pointer has been off the control since it appeared or was last
    /// pressed, so the card may open.
    armed: bool,
    /// When the pointer came onto the control, while the card waits to open.
    hovered: Option<Instant>,
    open: bool,
    /// The card's last event saw the pointer on it.
    over_card: bool,
    /// When the pointer left both the control and the open card.
    left_at: Option<Instant>,
    expanded: bool,
    paused: bool,
}
impl Hover {
    fn close(&mut self) {
        *self = Self {
            armed: self.armed,
            ..Self::default()
        };
    }
}
impl Widget<Message, Theme, Renderer> for HoverHelp<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Hover>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(Hover::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content), Tree::empty()]
    }
    fn diff(&self, tree: &mut Tree) {
        // The card's tree, the second, is reconciled when the card is built.
        if tree.children.len() == 2 {
            tree.children[0].diff(self.content.as_widget());
        } else {
            tree.children = self.children();
        }
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        )
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        // The card it shows, for the tests that look for a control's help.
        operation.custom(None, layout.bounds(), &mut self.help.clone());
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation)
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if matches!(
            event,
            Event::Mouse(_) | Event::Window(window::Event::RedrawRequested(_))
        ) {
            let hover = tree.state.downcast_mut::<Hover>();
            let now = Instant::now();
            let animated = self.help.diagram.is_some();
            let over_control = cursor.is_over(layout.bounds());
            if !over_control {
                hover.armed = true;
            }
            if over_control && matches!(event, Event::Mouse(mouse::Event::ButtonPressed(_))) {
                hover.armed = false;
                if hover.open && animated {
                    shell.publish(Message::HelpHover(false));
                }
                hover.close();
                shell.invalidate_layout();
                shell.request_redraw();
            }
            if hover.open {
                if over_control || hover.over_card {
                    hover.left_at = None;
                } else {
                    let left = *hover.left_at.get_or_insert(now);
                    if left.elapsed() >= LINGER {
                        if animated {
                            shell.publish(Message::HelpHover(false));
                        }
                        hover.close();
                        shell.invalidate_layout();
                        shell.request_redraw();
                    } else {
                        shell.request_redraw_at(left + LINGER);
                    }
                }
            } else if over_control && hover.armed {
                let since = *hover.hovered.get_or_insert(now);
                if since.elapsed() >= HELP_DELAY {
                    hover.open = true;
                    hover.over_card = false;
                    hover.left_at = None;
                    if animated {
                        shell.publish(Message::HelpHover(true));
                    }
                    shell.invalidate_layout();
                    shell.request_redraw();
                } else {
                    shell.request_redraw_at(since + HELP_DELAY);
                }
            } else {
                hover.hovered = None;
            }
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let Tree {
            state, children, ..
        } = tree;
        let hover = state.downcast_mut::<Hover>();
        let (content_tree, card_tree) = children.split_at_mut(1);
        let content = self.content.as_widget_mut().overlay(
            &mut content_tree[0],
            layout,
            renderer,
            viewport,
            translation,
        );
        let card = if hover.open {
            let shown = (hover.expanded, hover.paused);
            if self.card.as_ref().map(|(e, p, _)| (*e, *p)) != Some(shown) {
                let element = card(self.help.clone(), shown.0, shown.1);
                card_tree[0].diff(element.as_widget());
                self.card = Some((shown.0, shown.1, element));
            }
            let (_, _, element) = self.card.as_mut().expect("built above");
            Some(overlay::Element::new(Box::new(CardOverlay {
                card: element,
                tree: &mut card_tree[0],
                hover,
                control: layout.bounds() + translation,
                position: self.position,
                animated: self.help.diagram.is_some(),
            })))
        } else {
            None
        };
        match (content, card) {
            (None, None) => None,
            (Some(c), None) => Some(c),
            (None, Some(k)) => Some(k),
            (Some(c), Some(k)) => Some(overlay::Group::with_children(vec![c, k]).overlay()),
        }
    }
}
/// The open card, beside its control (`position`, `GAP` away, kept inside
/// the window). It takes the pointer while the pointer is on it, so the
/// controls under it rest and its toggles can be clicked: More… and Pause
/// go to the `Hover` state, not to the app.
struct CardOverlay<'a, 'b> {
    card: &'b mut Element<'a, Message>,
    tree: &'b mut Tree,
    hover: &'b mut Hover,
    /// The control's bounds in the window.
    control: Rectangle,
    position: tooltip::Position,
    animated: bool,
}
impl overlay::Overlay<Message, Theme, Renderer> for CardOverlay<'_, '_> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.card.as_widget_mut().layout(
            self.tree,
            renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );
        let at = place(self.control, node.size(), self.position, bounds);
        node.move_to(at)
    }
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.card.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            &Rectangle::with_size(Size::INFINITE),
        );
    }
    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        if matches!(
            event,
            Event::Mouse(_) | Event::Window(window::Event::RedrawRequested(_))
        ) {
            self.hover.over_card = cursor.is_over(layout.bounds());
        }
        let mut messages = vec![];
        let mut local = Shell::new(&mut messages);
        self.card.as_widget_mut().update(
            self.tree,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            &mut local,
            &layout.bounds(),
        );
        let redraw = local.redraw_request();
        let relayout = local.is_layout_invalid();
        let captured = local.is_event_captured();
        let outdated = local.are_widgets_invalid();
        drop(local);
        for m in messages {
            match m {
                Message::HelpMore(on) => {
                    self.hover.expanded = on;
                    shell.invalidate_layout();
                }
                Message::HelpPause(on) => {
                    self.hover.paused = on;
                    if self.animated {
                        shell.publish(Message::HelpHover(!on));
                    }
                }
                m => shell.publish(m),
            }
            shell.request_redraw();
        }
        shell.request_redraw_at(redraw);
        if relayout {
            shell.invalidate_layout();
        }
        if captured {
            shell.capture_event();
        }
        if outdated {
            shell.invalidate_widgets();
        }
    }
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let interaction = self.card.as_widget().mouse_interaction(
            self.tree,
            layout,
            cursor,
            &layout.bounds(),
            renderer,
        );
        // Anything but None keeps the pointer from the controls under the
        // card. A card that had to be clamped over its own control (issue
        // #184) leaves the pointer to the control instead, except over its
        // More… and Pause links, so the control can still be clicked, which
        // closes the card.
        if interaction == mouse::Interaction::None
            && cursor.is_over(layout.bounds())
            && !layout.bounds().intersects(&self.control)
        {
            mouse::Interaction::Idle
        } else {
            interaction
        }
    }
}
/// Where a card of `size` goes beside `control` in a window of `bounds`:
/// `GAP` away on `position`'s side, centred along it, or on the opposite
/// side when that side lacks the room (issue #184), then kept inside the
/// window. A card too tall or wide for either side is clamped over its
/// control, and `CardOverlay::mouse_interaction` keeps the control live.
fn place(control: Rectangle, size: Size, position: tooltip::Position, bounds: Size) -> Point {
    use tooltip::Position::*;
    let fits = |position: tooltip::Position| match position {
        Top => control.y - size.height - GAP >= 0.,
        Bottom => control.y + control.height + GAP + size.height <= bounds.height,
        Left => control.x - size.width - GAP >= 0.,
        Right | FollowCursor => control.x + control.width + GAP + size.width <= bounds.width,
    };
    let position = match position {
        FollowCursor => Right,
        p => p,
    };
    let opposite = match position {
        Top => Bottom,
        Bottom => Top,
        Left => Right,
        Right | FollowCursor => Left,
    };
    let position = if !fits(position) && fits(opposite) {
        opposite
    } else {
        position
    };
    let centred_x = control.x + (control.width - size.width) / 2.;
    let centred_y = control.y + (control.height - size.height) / 2.;
    let at = match position {
        Top => Point::new(centred_x, control.y - size.height - GAP),
        Bottom => Point::new(centred_x, control.y + control.height + GAP),
        Left => Point::new(control.x - size.width - GAP, centred_y),
        Right | FollowCursor => Point::new(control.x + control.width + GAP, centred_y),
    };
    Point::new(
        at.x.min(bounds.width - size.width).max(0.),
        at.y.min(bounds.height - size.height).max(0.),
    )
}
pub fn field<'a>(
    content: impl Into<Element<'a, Message>>,
    key: &str,
    label: &str,
    target: Target,
) -> Element<'a, Message> {
    Element::new(Field {
        content: wrap(
            content,
            field_help(key, label, target),
            tooltip::Position::Left,
        ),
        tag: FieldTag {
            target,
            key: key.into(),
            label: label.into(),
        },
    })
}
/// A form field as the keyboard finds it: the setting it edits and the name
/// over it, for the help the F1 key pins (`focused_field`) and the field a
/// Tab with nothing focused enters (`control::focus_first`).
#[derive(Debug, Clone, PartialEq)]
pub struct FieldTag {
    pub target: Target,
    pub key: String,
    pub label: String,
}
impl FieldTag {
    /// The field's help, the card its hover shows.
    pub fn help(&self) -> Help {
        field_help(&self.key, &self.label, self.target)
    }
}
/// The widget round a form field and its hover help that offers the field's
/// `FieldTag` to widget operations (`Operation::custom`) before the field.
struct Field<'a> {
    content: Element<'a, Message>,
    tag: FieldTag,
}
impl Widget<Message, Theme, Renderer> for Field<'_> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        )
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let mut tag = self.tag.clone();
        operation.custom(None, layout.bounds(), &mut tag);
        operation.traverse(&mut |operation| {
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], layout, renderer, operation)
        });
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
/// A widget operation that finds the form field with the keyboard focus, if
/// one has it: the F1 key pins its help.
pub fn focused_field() -> impl Operation<Option<FieldTag>> {
    use iced::advanced::widget::{
        Id,
        operation::{Focusable, Outcome},
    };
    #[derive(Default)]
    struct Focused {
        /// The tag a field just offered, for the traverse of its content.
        offered: Option<FieldTag>,
        /// The field whose content is being traversed.
        inside: Option<FieldTag>,
        found: Option<FieldTag>,
    }
    impl Operation<Option<FieldTag>> for Focused {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<FieldTag>>)) {
            match self.offered.take() {
                Some(tag) => {
                    let outer = self.inside.replace(tag);
                    operate(self);
                    self.inside = outer;
                }
                None => operate(self),
            }
        }
        fn custom(&mut self, _: Option<&Id>, _: Rectangle, state: &mut dyn std::any::Any) {
            self.offered = state.downcast_ref::<FieldTag>().cloned();
        }
        fn focusable(&mut self, _: Option<&Id>, _: Rectangle, state: &mut dyn Focusable) {
            if state.is_focused()
                && let Some(tag) = &self.inside
            {
                self.found = Some(tag.clone());
            }
        }
        fn finish(&self) -> Outcome<Option<FieldTag>> {
            Outcome::Some(self.found.clone())
        }
    }
    Focused::default()
}
/// The help the F1 key pinned for a field, for a corner of the window: its
/// card, open to its More… text unless Less was clicked (no hover overlay
/// takes the card's toggles here: `Message::HelpMore` and `HelpPause` reach
/// the app), and how it goes away: Esc, where `esc_closes` (no machine
/// connected and off the Control page, `Message::EscapeKey`); elsewhere Esc
/// is kept for the E-stop and the hint says so (issue #185).
/// The pinned card's hint where Esc puts it away.
pub const PINNED_HINT_ESC_CLOSES: &str = "F1 pins the focused field's help · Esc closes it";
/// The pinned card's hint where Esc is kept for the E-stop: while a machine
/// is connected, and on the Control page.
pub const PINNED_HINT_ESC_IS_ESTOP: &str =
    "F1 pins the focused field's help · Esc is kept for the E-stop · Close puts it away";
pub fn pinned<'a>(
    tag: &FieldTag,
    expanded: bool,
    paused: bool,
    esc_closes: bool,
) -> Element<'a, Message> {
    let close = widget::button(text("Close").size(12))
        .on_press(Message::HelpFor(None))
        .padding([4, 9])
        .style(|_, status| widget::button::Style {
            background: Some(
                if matches!(status, widget::button::Status::Hovered) {
                    Color::from_rgb8(201, 222, 241)
                } else {
                    Color::from_rgb8(223, 237, 249)
                }
                .into(),
            ),
            text_color: Color::from_rgb(0.13, 0.20, 0.28),
            border: Border {
                radius: 4.into(),
                ..Default::default()
            },
            ..Default::default()
        });
    column![
        card(tag.help(), expanded, paused),
        row![
            text(if esc_closes {
                PINNED_HINT_ESC_CLOSES
            } else {
                PINNED_HINT_ESC_IS_ESTOP
            })
            .size(12)
            .color(Color::from_rgb(0.30, 0.37, 0.44))
            .width(Length::Fill),
            close
        ]
        .spacing(9)
        .align_y(iced::Alignment::Center)
    ]
    .spacing(6)
    .width(304)
    .into()
}
pub fn command(m: &Message, label: String) -> Help {
    use Message::*;
    if label == "Cancel" && matches!(m, Page(crate::Page::Machines)) {
        return Help::text(
            label,
            "Leave the machine wizard without saving this draft to the library.",
        );
    }
    if let AddOp(s) = m {
        return Help::text(s.name(), format!("{}\n\n{}", s.help(), s.availability()))
            .more("Configure Tool → Geometry → Heights → Passes/Cycle → Linking. Unavailable strategies open Operation support for imported jobs; their animations are educational.")
            .animated(Diagram::Operation(*s));
    }
    // The work zero step's buttons say what the zero they pick is.
    if let Edit(Target::Setup, "origin", origin) = m {
        return crate::choice_help::selected(
            "origin",
            "Work zero",
            origin,
            Target::Setup,
            &crate::model::Fields::new(),
        );
    }
    // A hole of the part chosen, or no longer chosen, for the fixture pins.
    if let Edit(Target::Setup, "pin_holes", _) = m {
        return Help::text(
            label,
            "Choose or remove this hole of the part for the fixture pins. Choose one hole per pin, as the part's design has them: round holes it opens on the face this setup sits on, the pins' diameter to 0.2 mm over from their opening on (not counterbored), at least the pins' height deep. Which hole takes which pin follows from the plate, and pin 1's hole gives the X and Y work offset with pin 1's recorded machine place. Drill them in an earlier setup with a Drill operation on the face they open on, or before the job.",
        );
    }
    if let Edit(Target::Setup, "pin_place", _) = m {
        return Help::text(
            label,
            "Clear where the pins stand in the stock's leftover, to place them again. A Drill of their holes keeps its place in the job and has no holes until they are placed.",
        );
    }
    if let Edit(Target::Operation, "pin_setup", _) = m {
        return Help::text(
            label,
            "Make the holes for this setup's fixture pins, where its Work zero step places them in the stock's leftover.",
        );
    }
    if let StockDetail(detail) = m {
        return crate::choice_help::selected(
            "stock_detail",
            "Stock surface detail",
            &detail.to_string(),
            Target::Operation,
            &crate::model::Fields::new(),
        );
    }
    let (description, more) = match m {
        NewProject => (
            "Start an empty project with no model, setups or operations. Keep saved machines and tools; archive the current project first. Shortcut: ⌘N.",
            "",
        ),
        ResumeProject => (
            "Open the previous workspace and reload its source model. Nothing is loaded automatically from the start screen.",
            "",
        ),
        OpenSample => (
            "Open the optional bearing-housing example with sample profiles and operations. The current project is archived first.",
            "",
        ),
        RecoverPrevious => (
            "Reopen the project archived before the most recent switch. Your current work is archived before restoring it.",
            "",
        ),
        Page(crate::Page::Audit) => (
            "See the scope of every operation and run generation checks for every enabled operation in every setup.",
            "",
        ),
        Page(crate::Page::Home) => (
            "Return to the start screen to create a blank project, open a file, resume work or explore the optional example.",
            "",
        ),
        Import => (
            "Open STEP/STP, 3MF, STL or OBJ geometry for viewing. STEP and 3MF units are converted to mm. Import runs in the background; machining feature recognition is a future step.",
            "",
        ),
        OpenHistory => (
            "Browse archived projects, including unsaved work preserved before New project or a project switch. Choose an .axis.json file to recover it.",
            "",
        ),
        OpenProject => (
            "Open a saved .axis.json project, including machine profiles, tools, setups and operation settings. The previous workspace is archived first. Shortcut: ⌘O.",
            "",
        ),
        SaveProject => (
            "Save the project to its file. The first time, choose where to keep it; after that Save writes the same file without asking. The file is portable JSON with your settings. Keep the original model file available: the project stores its path, not embedded geometry. Shortcut: ⌘S.",
            "",
        ),
        SaveProjectAs => (
            "Save the project to a new file you choose, in any folder. From then on Save writes to that file. Shortcut: ⇧⌘S.",
            "",
        ),
        Page(crate::Page::Machines) => (
            "Manage reusable machine profiles. Use the wizard to define travel, rotary axes and controller, then assign a saved version to a setup.",
            "",
        ),
        Page(crate::Page::Tools) => (
            "Define cutters and holders, then choose a saved tool inside each operation.",
            "",
        ),
        Page(crate::Page::Materials) => (
            "Manage the material library: each material's group, notes and the surface speed and chip load ranges for carbide tools that speeds and feeds are checked against. Choose the project material in the setup.",
            "",
        ),
        Page(crate::Page::Workholding) => (
            "Manage the library of vises and toe clamps: their sizes and notes. Choose one for a setup under Workholding in the setup editor; that setup's jaws or clamps are then checked as boxes with a clearance gap. The starter entries have typical sizes: measure yours.",
            "",
        ),
        Page(crate::Page::Ai) => (
            "Sign in with ChatGPT and let the model propose a complete job for the imported part. The app checks every proposal before you can apply it; nothing is sent until you click Plan.",
            "",
        ),
        Page(crate::Page::Work) => (
            "Return to the part viewport, setup browser and operation editor. Your current edits remain in the workspace.",
            "",
        ),
        Page(crate::Page::Post) => (
            "Check each setup and export LinuxCNC G-code or a setup sheet. Speeds and feeds outside the project material's cutting data are listed as warnings and never block export. A UC100 machine takes no G-code file: Run on the machine runs its setup from MACHINE.",
            "Checks cover generation, machine limits, rapids into material and holder clearance, and, in a setup that models a vise or clamps, the jaws or clamps as boxes with a clearance gap over every program move, links, approaches and retracts included; parallels, bolts, the table, the machine body and other fixtures are not checked.",
        ),
        RunOnMachine(_) => (
            "Open the MACHINE workspace with this setup picked in the Run panel, once you Connect if the machine is not connected yet. The UC100 runs the setup directly, one tool run at a time, after the same checks as export; no G-code file is written.",
            "",
        ),
        Page(crate::Page::Wizard) | NewMachine => (
            "Create a reusable machine profile in five steps: identity, travel, kinematics, controller and review.",
            "",
        ),
        AddSetup => (
            "Add another machining position with its own machine, stock, work offset and operations. Use a new setup when you reclamp, flip or reorient the part. Without a setup, choose or configure your machine first.",
            "",
        ),
        AddManualSetup => (
            "Add a setup on the manual mill before the CNC setups, for the cuts the big mill makes first. Put first, it takes the stock, orientation, zero and holding of the old first setup, which then starts from the stock it leaves.",
            "",
        ),
        AddLatheSetup => (
            "Add a setup on the lathe before the CNC setups, with a round bar in the 3-jaw chuck. Put first, it takes the orientation of the old first setup, which then starts from the bar it leaves.",
            "",
        ),
        SetupEditor => (
            "Set the machine, work coordinate system, stock, part orientation and workholding for the current setup.",
            "",
        ),
        SetupStep("Machine") => (
            "The machine this setup runs on. ✓ once the machine profile has no issues.",
            "",
        ),
        SetupStep("Work zero") => (
            "Where the program's X0 Y0 Z0 is: on the stock top or bottom at a corner, the middle of an edge or the centre, picked on a grid of the stock seen from above, or Model origin; and the work offset (G54–G59) it is kept in. ✓ once the zero is on the stock this setup starts from.",
            "On the Control page, Probe corner, Probe edge or Probe centre sets this zero; Probe from probes it from another spot when a jaw is in the way.",
        ),
        SetupStep("Stock") => (
            "The project material and the block the part is cut from. ✓ once a material is chosen and the stock box holds the part.",
            "",
        ),
        SetupStep("Position") => (
            "Turn or move the part for this setup: flip it over, stand it on a side or shift it. ✓ while every rotation and translation is a number.",
            "",
        ),
        SetupStep(_) => (
            "The vise or clamps that hold the stock, checked against every toolpath, and the setup sheet's fixture notes. ✓ once a vise or clamps are modelled with nothing wrong.",
            "",
        ),
        SelectSetup(_) => (
            "Make this setup active and show its part orientation and operations. Each setup represents a machining position or reclamping stage. The picture shows the stock as this setup starts, in its jaws or clamps, seen as its 3D view first shows it: its block, or what the earlier setups left it, the cut faces in green.",
            "",
        ),
        SelectOp(_) => (
            "Open this operation's parameters. The tabs guide you through the same tool, geometry, heights, passes and linking workflow used by CAM systems.",
            "",
        ),
        DuplicateOp => (
            "Copy this operation, including its tool, geometry and cutting settings, into the current setup. Adjust the copy for another pass.",
            "",
        ),
        RemoveOp | AskRemoveOp | ConfirmDeleteOp(true) => (
            "Delete this operation from the current setup. References from later rest-machining operations are cleared.",
            "",
        ),
        ConfirmDeleteOp(false) => (
            "Keep this operation and close the deletion confirmation.",
            "",
        ),
        MoveOp(n) if *n < 0 => (
            "Move the operation earlier in the setup. Review rest-machining dependencies after changing the cutting order.",
            "",
        ),
        MoveOp(_) => (
            "Move the operation later in the setup. Review rest-machining dependencies after changing the cutting order.",
            "",
        ),
        ToggleOp => (
            "Exclude or include this operation in generation and review while keeping its parameters saved.",
            "",
        ),
        Feature(_) => (
            "Include this built-in housing feature in the example operation.",
            "",
        ),
        Surface(_) => (
            "Toggle this planar region on your imported part. Blue marks selected surfaces; amber shows the region under the pointer. Selection is saved with this operation and tied to this model's geometry.",
            "",
        ),
        DropHole(_) => (
            "Drop this hole from the operation: its walls are cleared from the selection, but for those another picked hole shares.",
            "",
        ),
        ClearSurfaces => (
            "Remove this operation's saved surface selections so you can choose new surfaces on the current part.",
            "",
        ),
        BoxFromFace => (
            "Fill the six Box fields from the part face selected on this operation: a floor facing up, or a wall facing sideways. A floor is grown to the stock's edge or the next part material above it. A wall is cut from the wall out to the stock's edge. The numbers are in the setup's frame from the work zero.",
            "",
        ),
        PickPinHoles => (
            "Pick the part's holes for the fixture pins in the 3D view: click inside a hole, on its wall or its end, to choose it or remove it; drag to orbit (turn the view to see under the part, where the holes open), and click Finish picking when done. The holes offered are ringed and numbered as listed.",
            "",
        ),
        PlacePins => (
            "Place the fixture pins on the stock's leftover in the 3D view: the plate's pins follow the pointer on the stock's bottom, green where they fit and red, with why, where they do not. Click to put pin 1 there; drag the placed pins to move them; drag elsewhere to orbit. Click Finish placing when done.",
            "",
        ),
        AddThreading => (
            "Add, for each thread size of the setup's threaded holes that open upwards, a Drill of its tap drill and then its thread: Thread milling with a thread mill on the CNC, Tapping by hand on the manual mill. Tools come from the library; the status says which are missing.",
            "",
        ),
        ThreadFromPart => (
            "Fill this thread's side, major diameter, pitch and end from the thread modeled on the part's axis: the one nearest the bar's free end.",
            "",
        ),
        AddPinDrilling => (
            "Add a Drill of the pin holes to the last earlier setup that has the face this setup sits on facing up: holes where the pins are placed, 1 mm deeper than the pins stand, with a drill or flat end mill from the library that fits the pins.",
            "",
        ),
        PickMode => (
            "Select geometry on the current part. Click a surface to toggle it, drag to orbit, and click Finish picking when done. Imported Facing/Pocket selections are connected planar mesh regions, not exact CAD faces.",
            "",
        ),
        Generate => (
            "Generate the selected supported imported-part operation with sampled stock removal. The example project uses illustrative paths. G-code export is on the NC review page.",
            "",
        ),
        GenerateAll => (
            "Generate and check every enabled operation in every setup. The report lists successful previews and specific failures; disabled operations are skipped. Remaining stock carries through earlier operations and setups. Failed upstream operations block downstream stock; playback remains one operation at a time.",
            "",
        ),
        DuplicateSetup => (
            "Copy this setup and its operations to prepare a second machining position. Set its new orientation and work offset; its stock is what the setups before it leave.",
            "",
        ),
        RemoveSetup | ConfirmDeleteSetup(true) => (
            "Remove this setup and its operations. Later stock is rebuilt from the revised setup order.",
            "",
        ),
        ConfirmDeleteSetup(false) => ("Keep this setup and close the deletion confirmation.", ""),
        MoveSetup(n) if *n < 0 => (
            "Move this setup earlier in the job. Stock can only be inherited from a preceding setup.",
            "",
        ),
        MoveSetup(_) => (
            "Move this setup later in the job. Check that the stock and reclamping sequence still matches your intended process.",
            "",
        ),
        Flip("rx") => (
            "Set the part rotation to 180° about X to expose the opposite side. Review the origin and fixture after reclamping; the stock is what the setups before it leave.",
            "",
        ),
        Flip(_) => (
            "Set the part rotation to 90° about Y to expose a side. Review the origin and fixture for this position; the stock is what the setups before it leave.",
            "",
        ),
        TurnStock(..) => (
            "Turn this setup so the stock lies with the chosen face up and turn on the table, as setup 1 has the stock: square to it whatever setup 1's rotation is.",
            "",
        ),
        SquareToStock => (
            "Set this setup's rotation to the quarter turn of the stock nearest it, so the stock is no longer skewed and its stock-based work zero, jaws and clamps can be used.",
            "",
        ),
        AssignMachine(_) => (
            "Use this machine to create the first setup, or copy its saved version into the current setup. Later library edits will not silently change that setup's machine settings.",
            "",
        ),
        EditMachine(_) => (
            "Open this saved profile in the wizard. Saving creates an updated library version; assign it again to update a setup snapshot.",
            "",
        ),
        SaveMachine => (
            "Validate and save the machine profile locally for reuse in future setups and projects.",
            "",
        ),
        DeleteMachine(_) => (
            "Remove this profile from the reusable library. Existing setup snapshots remain available.",
            "",
        ),
        WizardStep(2) if label.ends_with("Manual mill") => (
            "Say if the DRO compensates the tool radius, and the minutes a pass, a tool change and moving the stock cost on this mill.",
            "",
        ),
        WizardStep(2) if label.ends_with("Dials & feeds") => (
            "Say what the cross-slide dial reads, the numbers and divisions of each dial, the power feeds and threads the gearbox cuts, and the minutes a pass, a tool change and moving the stock cost.",
            "",
        ),
        WizardStep(3) if label.ends_with("Review") => (
            "Review the profile and resolve configuration issues before saving it to your local machine library.",
            "",
        ),
        ToolMachine(..) | HolderMachine(..) => (
            "Check the machines that hold this entry. With none checked it fits every CNC machine and no manual machine.",
            "",
        ),
        WizardStep(0) => (
            "Name the machine, choose a CNC, a manual mill or a manual lathe, then 3, 4 or 5 axes for a CNC, and the units used by its specification.",
            "",
        ),
        WizardStep(1) => (
            "Record linear travel, spindle speed and feed limits from the machine specification.",
            "",
        ),
        WizardStep(2) => (
            "Describe rotary axis arrangement, angular limits, direction and pivot positions. These are saved settings; kinematic solving is still planned.",
            "",
        ),
        WizardStep(3) => (
            "Choose the intended controller, tool-change method and tool center point capability.",
            "",
        ),
        WizardStep(_) => (
            "Review the profile and resolve configuration issues before saving it to your local machine library.",
            "",
        ),
        NewTool => (
            "Create a cutter definition with diameter, flute count, cutting length, stickout and holder dimensions.",
            "",
        ),
        EditTool(_) => (
            "Edit this saved cutter. Tool dimensions are stored in millimeters.",
            "",
        ),
        SaveTool => (
            "Validate the cutter and save it to the project's tool library.",
            "",
        ),
        DeleteTool(_) => (
            "Remove this cutter if no operation uses it. Assign another tool to those operations first.",
            "",
        ),
        NewMaterial => (
            "Create a material with a name, group, notes and optional cutting data. It joins the library when you save it.",
            "",
        ),
        EditMaterial(_) => (
            "Edit this library material. Its cutting data are starting points for carbide tools; compare them with your tool maker's data.",
            "",
        ),
        SaveMaterial => (
            "Validate the material and save it to the library. The name must be new and the cutting data complete or empty.",
            "",
        ),
        DeleteMaterial(_) => (
            "Remove this material from the library. The project material cannot be deleted while it is selected: choose another one in the setup first.",
            "",
        ),
        PickMaterial(_) => (
            "Use this library material for the project. Choosing it sets each operation's speed and feed (drilling aside) from its cutting data, for the operation's tool and machine. The data also check speeds and feeds; its name, group, hardness, notes and cutting data are sent to the AI planner.",
            "",
        ),
        NewHolder("Clamp") => (
            "Create a toe clamp with its length, width, height and bite. It joins the library when you save it. Enter the sizes of your own clamp.",
            "",
        ),
        NewHolder("Chuck") => (
            "Create a lathe chuck with its jaw count, the range of diameters it closes on, how deep its jaws reach and its body diameter. It joins the library when you save it. Enter the sizes of your own chuck.",
            "",
        ),
        NewHolder(_) => (
            "Create a vise with its jaw sizes, maximum opening and grip. It joins the library when you save it. Enter the sizes of your own vise.",
            "",
        ),
        EditHolder(_) => (
            "Edit this library entry. Changing it changes every setup that uses it.",
            "",
        ),
        SaveHolder => (
            "Validate the entry and save it to the library. The name must be new and every size a positive number; a clamp's bite must be shorter than its length.",
            "",
        ),
        DeleteHolder(_) => (
            "Remove this entry from the library. It cannot be deleted while a setup uses it: choose other workholding in that setup first.",
            "",
        ),
        SuggestLayout => (
            "Place this setup's vise jaws or clamps. A vise goes on the X or the Y sides, whichever its generated toolpaths leave clearer, at its grip or less. Clamps are chosen among the middle and quarter points of each side: four, or two on a small stock.",
            "The status says how many operations still come within the clearance gap. The setup's operations are generated for this and never changed. Jaws and clamps are checked as boxes with the gap; parallels, bolts, the table and the machine body are not. Choosing Vise or Clamps, or another library entry, does this too; other edits are kept until you press this again. On a lathe setup it chooses the chuck instead: the 3-jaw when the bar in the jaws is round and within its range, else the 4-jaw (change the chuck), never a vise; the grip and the stickout stay yours.",
        ),
        SetClamps(_) if label == crate::ui::ADD_CLAMP => (
            "Add a clamp at the middle of a side of the stock, the sides taken in turn. Change its side and offset in its row.",
            "",
        ),
        SetClamps(_) => (
            "Take this clamp off the setup. The setup keeps at least one.",
            "",
        ),
        SetOpTool(_) => (
            "Use this saved cutter for the current operation. Check diameter, reach, feeds and speeds before generating a preview.",
            "",
        ),
        RestSource(_) => (
            "Choose the earlier operation whose remaining material should guide rest machining. Earlier cuts automatically lower the cutting start when the area above it is fully cleared. Selecting isolated leftover regions for rest machining is not implemented.",
            "Imported Pocket and Adaptive clearing ignore the source: they cut only where earlier operations left material.",
        ),
        Export("sheet") => (
            "Save a Markdown setup sheet with machine, workholding, repositioning instructions and operation parameters.",
            "",
        ),
        ExportGcode => (
            "Choose a folder and write one LinuxCNC .ngc program per setup. Every check and the replay run again first; nothing is written if any setup fails.",
            "",
        ),
        Camera("ISO") => (
            "Reset to an isometric view that shows the top and two sides. Drag to orbit, right-drag to pan, and scroll to zoom.",
            "",
        ),
        Camera("TOP") => (
            "Look down the setup Z axis onto its XY plane. This changes the camera, not the machining setup.",
            "",
        ),
        Camera("FRONT") => (
            "View the setup from the front along Y. This changes the camera only.",
            "",
        ),
        Camera("FIT") => (
            "Fit the scene in the 3D view again from where it looks: the zoom and the pan go back, the direction stays. The part, the stock and the modelled workholding fill the view.",
            "",
        ),
        Camera(_) => (
            "View the setup from its right side along X. This changes the camera only.",
            "",
        ),
        ToggleSidebar(crate::settings::Side::Browser) => (
            "Hide the browser to give the 3D view its width, or show it again from the strip at the left edge. Drag the line between the browser and the 3D view to make it wider or narrower. Both are saved.",
            "",
        ),
        ToggleSidebar(crate::settings::Side::Inspector) => (
            "Hide the inspector to give the 3D view its width, or show it again from the strip at the right edge. Drag the line between the 3D view and the inspector to make it wider or narrower. Both are saved.",
            "",
        ),
        ToggleStrategyHelp => (
            "Show or hide what this strategy does and what it is available for in this build. The choice is saved and holds for every operation.",
            "",
        ),
        Play => (
            "Play or pause cutter motion. For imported Facing or Pocket, generate the preview if needed and show sampled stock removal. Other imported strategies are not supported yet. The example project uses illustrative motion.",
            "",
        ),
        Scrub(_) => (
            "Return to this point in the preview and pause. Simulated stock is recomputed when you rewind; Restart restores the uncut stock.",
            "",
        ),
        ScrubDone => (
            "Let go of the slider: on a long toolpath the stock catches up with where it was moved.",
            "",
        ),
        ToggleStock => (
            "Show or hide stock. During Facing or Pocket simulation this displays the remaining sampled stock; hide Stock to see the finished reference model. Outside simulation it shows the transparent stock box.",
            "Relative stock extends from each side of the placed part by its offset; fixed stock uses explicit dimensions. The box and part share the same display scale.",
        ),
        ToggleUncut => (
            "Show or hide the faces of the part that the stock still covers after every setup, in magenta. With the simulated stock shown, just those faces are drawn over it, where the stock surface does not hide them. A selected one is striped blue and magenta. Off, the part is drawn as it is without them.",
            "",
        ),
        TogglePaths => (
            "Show or hide the cutter path. Pocket shows cuts on the current depth layer; Facing shows the latest cutting pass. Both show the active rapid; the example project shows illustrative paths.",
            "",
        ),
        Plan => (
            "Ask the model for a complete job: setups, operations, tools and cutting parameters. Requests count against your ChatGPT plan. Nothing changes until you apply the plan.",
            "The app checks every proposal with its generators, the stock simulation, the export checks and the speeds and feeds against the project material's cutting data, and sends problems back for correction, up to four requests.",
        ),
        PlanCancel => ("Stop planning. The job is unchanged.", ""),
        ApplyPlan => (
            "Replace the operations of setup 1 and remove later setups, adding the planned ones. Setup 1 keeps its name, machine, stock, work zero and fixture. The previous job is saved to History first and can be undone on this page.",
            "",
        ),
        DiscardPlan => ("Close this result. The job is unchanged.", ""),
        UndoPlan => (
            "Put back the setups and operations from before the plan was applied, and any tool they use that was deleted since (keeping its own number when no tool has it, otherwise with a free one). The applied job, with your edits since, is saved to History first.",
            "Tools, machines, the project title and the material changed since stay. Importing a part or opening or creating a project ends the Undo; the job stays in History.",
        ),
        Tab(t) => {
            return Help::text(
                label,
                match t.as_str() {
                    "Tool" => "Choose a cutter and set spindle speed, feed rate and coolant.",
                    "Geometry" => {
                        "Select the faces, contours or holes to machine, and define the allowed cutting boundary."
                    }
                    "Heights" => {
                        "Set clearance and retract planes, plus the top and bottom offsets that bound the cut."
                    }
                    "Orientation" => {
                        "Set indexed rotary angles or the tool-axis strategy for multi-axis cutting."
                    }
                    "Cycle" => {
                        "Choose how the drill feeds, pecks, pauses and retracts between holes, or mill each hole with an end mill."
                    }
                    "Passes" => {
                        "Control cutting depth, spacing, finishing allowance and tolerance."
                    }
                    "Turn" => {
                        "Choose what to turn and set its sizes: Z 0 is the free end, diameters are diameters."
                    }
                    "Centre" => "Set how deep the drill goes on the lathe's centre line.",
                    "Linking" => {
                        "Control how the tool enters a cut and moves between cutting passes."
                    }
                    _ => "Open this operation parameter group.",
                },
            );
        }
        SignIn => (
            "Open your browser to sign in with ChatGPT. AXIS CAM Studio receives your name, email and profile picture. Planning requests then count against your ChatGPT plan.",
            "",
        ),
        SignInCancel => (
            "Stop waiting for the browser sign-in. Nothing is saved.",
            "",
        ),
        SignOut => (
            "Sign out of ChatGPT on this computer and ask OpenAI to revoke this sign-in. The saved sign-in is deleted.",
            "",
        ),
        CheckConnection => (
            "Send one tiny request to the selected model to confirm that your ChatGPT plan works with this app. It uses a very small amount of your plan's usage.",
            "",
        ),
        LoadModels => (
            "Ask OpenAI again which models your ChatGPT plan can use in this app.",
            "",
        ),
        Machine(a) => machine_help(a),
        Run(a) => run_help(a),
        Guide(a) => guide_help(a),
        ControlSetup(false) => ("Jog, zero and run the machine.", ""),
        ControlSetup(true) => (
            "Edit the machine's UC100 profile, watch its inputs, test its outputs and calibrate steps per mm.",
            "",
        ),
        Setup(crate::setup::Msg::Save) => (
            "Write the profile file. Only the values changed are rewritten; its comments stay. A connected machine is reconnected with it: it starts in E-stop, and its machine position must be set again.",
            "",
        ),
        Setup(crate::setup::Msg::UseCalibration) => (
            "Put this steps per mm in the profile below. Save to use it.",
            "",
        ),
        Setup(crate::setup::Msg::Revert) => (
            "Throw the changes away and read the profile file again.",
            "",
        ),
        PlanNoticeDismissed => ("Close this notice. It is shown once.", ""),
        OpenUrl(_) => (
            "Open your ChatGPT usage settings in the browser, where you can see and manage how much of your plan this app uses.",
            "",
        ),
        _ => ("Change this setting in the current workspace.", ""),
    };
    let h = Help::text(label, description).more(more);
    match m {
        Flip(_) | TurnStock(..) | SquareToStock | AddSetup | DuplicateSetup => {
            h.animated(Diagram::Orientation)
        }
        _ => h,
    }
}

/// What a Control page button does to the machine.
fn machine_help(a: &crate::control::Action) -> (&'static str, &'static str) {
    use crate::control::Action::*;
    match a {
        Home => (
            "Home every axis on its switch, in the profile's order. This sets machine zero, and the soft limits apply from then on.",
            "",
        ),
        MachineZero(None) => (
            "Declare the current position machine zero for X, Y and Z. First jog X to its left end, Y to its front end and Z to the top. The soft limits apply once every limited axis has a machine position.",
            "",
        ),
        MachineZero(Some(_)) => (
            "Declare the current position machine zero for this axis only.",
            "",
        ),
        KeepPosition => (
            "Take the position shown as the machine position. After connecting it is where AXIS CAM last left the machine: use it only if nothing moved the machine since.",
            "",
        ),
        JogPress(..) | JogRelease => (
            "Hold to jog this axis; release to stop. In step mode each press moves one step. The arrow keys and Page Up/Down do the same on this page.",
            "",
        ),
        JogStep(None) => ("Jog for as long as the button or key is held.", ""),
        JogStep(Some(_)) => ("Move exactly this far for each press.", ""),
        JogSpeed(_) => (
            "Jog speed, as a share of each axis's maximum speed in the profile.",
            "",
        ),
        TestOutput(_, Some(_)) => (
            "Force this output pin for a wiring test, while Idle. E-stop releases every forced pin.",
            "",
        ),
        TestOutput(_, None) => ("Release the pin: the machine drives it again.", ""),
        CalAxis(_) => ("Calibrate this axis.", ""),
        CalDistance(_) | CalMeasured(_) => ("Travel in mm.", ""),
        CalMove => (
            "Move the axis by the commanded travel at the jog speed, counting its step pulses. Soft limits apply.",
            "",
        ),
        WorkZero(None) => (
            "Make the current position work zero on every axis, in the active coordinate system (G54–G59).",
            "",
        ),
        WorkZero(Some(_)) => (
            "Make the current position work zero on this axis, in the active coordinate system.",
            "",
        ),
        WorkHalf(_) => (
            "Halve this axis's work position where the tool is, as a milling machine's DRO does. Zero the axis on one side of a part, move to the other side and press 1/2: the middle is now 0. With an edge finder or probe the tip's radius cancels out.",
            "",
        ),
        WorkText(..) => (
            "The work position to give this axis where the tool is, in mm: for example −5 with the edge of a 10 mm edge finder against a side.",
            "",
        ),
        WorkSet(_) => (
            "Make the current position the value beside it on this axis, in the active coordinate system.",
            "",
        ),
        ToWorkXY => (
            "Rapid X and Y to work X0 Y0, Z staying where it is: lift Z clear first. Soft limits apply.",
            "",
        ),
        ProbeFrom(_) => (
            "Where on the stock top the probe finds this setup's work zero from: its own spot, or another corner, edge middle or the centre when a jaw or clamp is in the way. The zero stays where the program has it, placed from the touches by the setup's stock size.",
            "",
        ),
        ProbeFeature(_) => (
            "Probe a feature from where the tip is and set work zero on it in the active coordinate system, without a setup: a surface (Z top), a side (X+, X−, Y+, Y−), an outside corner, or the centre of a bore or of a boss or pin. The tip's radius is allowed for. Anything touched on the way, a miss, Stop or E-stop ends it with nothing set.",
            "",
        ),
        Wcs(_) => (
            "Use this work coordinate system. Each of G54–G59 keeps its own work zero; they change only while Idle.",
            "",
        ),
        Rpm(_) => (
            "Spindle speed for CW and CCW, in revolutions per minute.",
            "",
        ),
        Spindle(uc100::gcode::Spindle::Off) => ("Stop the spindle.", ""),
        Spindle(_) => (
            "Start the spindle at the RPM above. Stop it before changing direction.",
            "",
        ),
        Flood(_) | Mist(_) => ("Switch the coolant output on or off.", ""),
        FeedOverride(_) => ("Scale every programmed feed, from 10 % to 200 %.", ""),
        OverrideLimits(_) => (
            "Ignore the limit switches, to drive off one that tripped. It turns itself off a second after no switch is active.",
            "",
        ),
        MdiText(_) => ("One line of G-code, for example G0 X10 Y10.", ""),
        MdiSend => (
            "Run the line now. Works when Idle, or while a program is paused.",
            "",
        ),
        ProbeZ => (
            "With the touch plate on the stock top under the tool and the clip on the tool, feed Z down until the tool touches the plate, at most the maximum probe distance. Work Z there becomes the stock top's Z plus the plate's thickness; the tool lifts. On a metal project the plate's lead goes on the stock: no thickness. Without contact nothing is set.",
            "The stock top's Z is the program's for the setup picked in the Run panel (Z 0, or the stock's height for a work zero on the bottom), and Z 0 without one.",
        ),
        ProbeEdges => (
            "Probe corner, Probe edge or Probe centre, by the work zero of the setup picked in the Run panel. From above the stock top it touches the top, then the sides at the spots the program header names, and sets the X, Y and Z work zero exactly where the program has it. Anything touched on the way, a miss, Stop or E-stop ends it with nothing set.",
            "With the probe above the stock top where Z0 is touched off, it touches the top, then the sides at the spots the program header names, 1 mm to 10 mm below the top (less on low stock), allowing for the tip's radius, and sets the X, Y and Z work zero exactly where the program has it: on the top or the bottom, and for Model origin from the front-left corner. A side whose first touch was off its spot is touched again on it.",
        ),
        TouchOff => (
            "With the tool tip on the stock top over the spot the steps name, set the work position there to the program's: X0 Y0 Z0 for a zero on the top, the stock's height in Z for one on the bottom, the corner's coordinates for Model origin.",
            "",
        ),
        ToSimZero => (
            "Simulated only: rapid the tool tip up to the top of Z, over where this setup is touched off on the simulator's block, and down onto it. Then the button beside it sets the work zero, as touching off on real stock would.",
            "",
        ),
        RecordPins => (
            "Once, after setting machine zero, and again whenever the plate is moved: with the part off the pins and the probe a few mm above the middle of pin 1, find pin 1's centre and keep its machine X Y in the machine's profile. Then it flies over the top of Z to where the plate puts pin 2, finds it, and says how far it is off the line from pin 1: over 0.05 mm, square the plate.",
            "Pin centre touches the pin's top, then probes its sides from −X, +X, −Y and +Y, min(3 mm, half the pin's height) below the top, starting outside it by the pin's and the tip's diameters and 1 mm. The tip's radius cancels out.",
        ),
        ReferencePins => (
            "After every E-stop or power cycle, or at the start of a session: with the part off the pins and the probe a few mm above the middle of pin 1, find pin 1's centre and declare machine X and Y so that it is where it was recorded. Pin 1 acts as the machine's home for X and Y: every saved G54–G59 offset, the soft limits and the pins' place hold again. Z still gets its machine zero at the top of travel.",
            "X and Y need no machine position first: they are declared where they show, the probe moves only a few mm from where the tip is, and then they are declared again from the pin. Z needs one, as the probe moves it.",
        ),
        PinZero => (
            "Set the X and Y work offset of the active work system to the one the pins give this setup: pin 1's recorded machine place less where pin 1 stands in the setup's work coordinates. Exact, with no probing. Z is touched off as for any setup.",
            "",
        ),
    }
}

/// What a button of a manual setup's Steps page does.
fn guide_help(a: &crate::GuideAction) -> (&'static str, &'static str) {
    use crate::GuideAction::*;
    match a {
        Next => (
            "This step is done: show the next one. The place is saved with the project, so you can stop and come back.",
            "",
        ),
        Back => ("Show the step before.", ""),
        Go(_) => ("Show this pass.", ""),
        Section(_) => (
            "Jump to this section: where you left it, else its first step.",
            "",
        ),
        Reached(_) => (
            "The DRO reads this number: light the next one. The app does not read the DRO.",
            "",
        ),
        Open(_) => (
            "Open this setup in MACHINE: a setup on the manual mill shows its steps with the DRO numbers to move to, one on the lathe with its dial pictures, whether or not the CNC is connected.",
            "",
        ),
        Beat(_) | Built(..) => ("Show the steps of this manual setup.", ""),
        CrossReads(crate::lathe::Reads::Diameter) => (
            "The diameter went down by the dial's reading: the cross-slide dial reads the diameter. Saved to this lathe in the library and in every setup on it; the cross-slide numbers then show.",
            "",
        ),
        CrossReads(crate::lathe::Reads::Radius) => (
            "The diameter went down by twice the dial's reading: the cross-slide dial reads the radius. Saved to this lathe in the library and in every setup on it; the cross-slide numbers then show.",
            "",
        ),
        Typed(..) => ("Type the diameter the calipers read, in mm.", ""),
        Measured(..) => (
            "Keep this measured diameter with the project: the cross-slide numbers that follow are worked out from it, the last pass taking all that is left. The app does not read the calipers.",
            "",
        ),
    }
}

/// What a Run panel button does.
fn run_help(a: &crate::run::Action) -> (&'static str, &'static str) {
    use crate::run::Action::*;
    match a {
        Setup(_) => (
            "Run this setup on the machine: AXIS CAM simulates the job up to it and checks its tool runs as export would, then starts from its first tool run.",
            "",
        ),
        Check => (
            "Check the setup again after a change to the project, keeping which tool runs are done.",
            "",
        ),
        Next(_) => ("Make this tool run the next one Cycle Start runs.", ""),
        From(..) => (
            "Make Cycle Start begin with this operation: those before it are taken as cut, and nothing of them is sent. Its tool run starts as after a tool change, with this operation's spindle speed and coolant, across at the clearance plane and down.",
            "Before the setup's first Cycle Start, set the work zero on the stock the operations before it leave, or keep it: Cycle Start waits for that.",
        ),
        KeepZero => (
            "Keep the work zero as it is for a run that starts partway: it is still the one the operations taken as cut were cut with (the stock has not moved since), or you set Z by hand on a surface they left.",
            "",
        ),
        CycleStart => (
            "Run the next tool run: its moves at the current work zero, checked against the soft limits first. The tool first rises or drops to the clearance plane where it stands, so leave it where a straight vertical move is safe.",
            "",
        ),
        Hold => ("Feed hold: slow to a stop and wait. Resume continues.", ""),
        Resume => ("Continue after a feed hold or a pause.", ""),
        Stop => (
            "Slow to a stop, then end the tool run with the spindle and coolant off.",
            "",
        ),
        RecordReference => (
            "With the tool that is zeroed touching a surface that stays put (a vise jaw, the table, a block), keep that surface's work Z for the next tool's touch-off.",
            "",
        ),
        ZFromReference => (
            "With the new tool touching the recorded reference surface, set work Z here to the reference's Z. X and Y zero stay as they are.",
            "",
        ),
        ProbeReference => (
            "With the touch plate on a surface that stays put and the tool that is zeroed above it, probe down onto the plate and keep the surface's work Z (the contact less the plate's thickness) for the next tool's touch-off.",
            "",
        ),
        ProbeFromReference => (
            "With the touch plate on the recorded reference surface and the new tool above it, probe down onto the plate and set work Z so that the surface is at the reference's Z. X and Y zero stay as they are.",
            "",
        ),
        CoolantOff => (
            "Set the coolant of the operations listed to Off, in every tool run of this setup: they ask for flood or mist coolant the UC100 profile has no output for. Then check the setup again.",
            "",
        ),
    }
}

pub fn field_help(key: &str, label: &str, target: Target) -> Help {
    use Diagram::*;
    let (body, more, diagram) = match (target, key) {
        (Target::Machine, "rpm") => (
            "Maximum spindle speed allowed by this machine, in revolutions per minute. Operations above this limit are flagged.",
            "",
            Some(Machine(0)),
        ),
        (Target::Machine, "power") => (
            "The spindle's rated power, in watts. The check of whether a setup's vise or clamps hold the stock against each operation's cut takes no more force than this power drives at the spindle's top speed (the spindle keeps its torque below that speed). Empty: 800 W, a Jingyan CNC4030's spindle.",
            "",
            Some(Machine(1)),
        ),
        (Target::Machine, "feed") => (
            "Maximum feed rate in the machine profile's units per minute. Use the manufacturer's specification; operation feeds are in mm/min.",
            "",
            Some(Machine(2)),
        ),
        (Target::Machine, "uc100_profile") => (
            "Path of the UC100 driver profile (TOML) this machine runs with. Browse… picks it; MACHINE → Controller configuration on the Control page edits it in place.",
            "",
            None,
        ),
        // The Control page's typed fields say what their commands say.
        (Target::Machine, "control_rpm") => {
            let (body, more) = machine_help(&crate::control::Action::Rpm(String::new()));
            (body, more, None)
        }
        (Target::Machine, "mdi_line") => {
            let (body, more) = machine_help(&crate::control::Action::MdiText(String::new()));
            (body, more, None)
        }
        (Target::Machine, "measured") => {
            let (body, more) = guide_help(&crate::GuideAction::Measured(String::new(), 0.));
            (body, more, None)
        }
        (Target::Machine, "work_set") => {
            let (body, more) = machine_help(&crate::control::Action::WorkText(0, String::new()));
            (body, more, None)
        }
        (Target::Machine, "cal_distance") => {
            let (body, more) = machine_help(&crate::control::Action::CalDistance(String::new()));
            (body, more, None)
        }
        (Target::Machine, "cal_measured") => {
            let (body, more) = machine_help(&crate::control::Action::CalMeasured(String::new()));
            (body, more, None)
        }
        (Target::Machine, "direction" | "direction2") => (
            "Choose the sign convention for this rotary axis from the machine documentation. It affects how a positive angle is interpreted.",
            "",
            Some(Orientation),
        ),
        (Target::Setup, "clearance") => (
            "A minimum gap between the fixture and the outside of the cutter or holder that you ask the operator to keep, in mm: the illustration shows a side gap measured from the wider holder. It is a note. It is printed on the setup sheet and in the program header, it changes no operation height or toolpath, and nothing checks it.",
            "The checks of a modelled vise or clamps use Checked gap to jaws and clamps under Workholding instead; parallels, bolts, the table and the machine body are never checked.",
            Some(FixtureClearance),
        ),
        (Target::Material, "name") => (
            "The material's name in the library and in the project material list. Each material needs its own name of up to 60 characters; capitals do not make a name different. When this is the project material, its name is sent to the AI planner.",
            "",
            None,
        ),
        (Target::Material, "group") => (
            "The kind of material. It is saved with the material, shown in the library list and, when this is the project material, sent to the AI planner.",
            "",
            None,
        ),
        (Target::Material, "hardness") => (
            "Optional hardness of the material, such as 95 HB. It does not change any limit. When this is the project material, it is sent to the AI planner.",
            "",
            None,
        ),
        (Target::Material, "notes") => (
            "Optional notes on the material, up to 500 characters: coolant, chip clearing or anything else to remember. They are saved with the material and, when this is the project material, sent to the AI planner.",
            "",
            None,
        ),
        (Target::Material, "vc_min") => (
            "Lowest recommended surface speed for carbide tools, in m/min. Surface speed = π × tool diameter × spindle rpm ÷ 1000. A slower operation is flagged, unless the machine's top spindle speed is the limit.",
            "",
            Some(Cutting(0)),
        ),
        (Target::Material, "vc_max") => (
            "Highest recommended surface speed for carbide tools, in m/min. Surface speed = π × tool diameter × spindle rpm ÷ 1000. A faster operation is flagged.",
            "",
            Some(Cutting(0)),
        ),
        (Target::Material, k) if k.starts_with("fz") && k.ends_with("_min") => (
            "Lowest recommended chip load per tooth, in mm, for tools in this diameter range. Chip load = feed ÷ (rpm × flutes). A lower value is flagged, unless the machine's top feed is the limit.",
            "",
            Some(Cutting(1)),
        ),
        (Target::Material, k) if k.starts_with("fz") && k.ends_with("_max") => (
            "Highest recommended chip load per tooth, in mm, for tools in this diameter range. Chip load = feed ÷ (rpm × flutes). A higher value is flagged.",
            "",
            Some(Cutting(1)),
        ),
        (Target::Holder, "name") => (
            "The entry's name in the workholding library and in the setup's list. Each entry needs its own name of up to 60 characters; capitals do not make a name different.",
            "",
            None,
        ),
        (Target::Holder, "notes") => (
            "Optional notes on the entry, up to 500 characters, such as where its sizes were measured. The starter entries say to measure yours.",
            "",
            None,
        ),
        (Target::Holder, "jaw_width") => (
            "Length of each jaw along the jaws, in mm. The jaws are centred on the stock, or shifted by the setup's jaw offset.",
            "",
            Some(Vise(0)),
        ),
        (Target::Holder, "jaw_height") => (
            "Height of the jaws above the parallels' top, in mm. Kept for reference: the modelled jaws reach from far below the stock up to the setup's grip height.",
            "",
            Some(Vise(1)),
        ),
        (Target::Holder, "jaw_thickness") => (
            "How thick each jaw is, measured outward from the stock, in mm.",
            "",
            Some(Vise(2)),
        ),
        (Target::Holder, "max_opening") => (
            "The widest stock the jaws open to, in mm. A setup whose stock is wider across the jaws shows an issue.",
            "",
            Some(Vise(3)),
        ),
        (Target::Holder, "grip") => (
            "How much of the stock the jaws hold, as a height above the stock's bottom, in mm. A setup starts from it and can change its own grip.",
            "",
            Some(Vise(4)),
        ),
        (Target::Holder, "length") => (
            "Length of the clamp from its tip outward, in mm; the bite is part of it. The rest of it reaches beyond the stock's edge.",
            "",
            Some(Clamp(0)),
        ),
        (Target::Holder, "width") => (
            "Width of the clamp, in mm, along the side of the stock it sits on.",
            "",
            Some(Clamp(1)),
        ),
        (Target::Holder, "height") => (
            "Height of the clamp above the stock's top, in mm.",
            "",
            Some(Clamp(2)),
        ),
        (Target::Holder, "force") => (
            "How hard the vise clamps the stock, or one clamp holds it down, in newtons. The check of each operation's cutting forces takes it as what the jaws' friction and the clamps' hold work with: measure or estimate yours, and raise it if yours is tightened harder. Empty: 5000 N for a vise, 2000 N for a clamp, typical when tightened by hand.",
            "",
            Some(Vise(5)),
        ),
        (Target::Holder, "bite") => (
            "How far the clamp reaches onto the stock from its edge, in mm. It must be shorter than the clamp's length.",
            "",
            Some(Clamp(3)),
        ),
        (Target::Holder, "jaws") => (
            "How many jaws the chuck has: 3 (self-centring, for round bar) or 4 (independent, for square or off-centre work).",
            "",
            Some(Chuck(0)),
        ),
        (Target::Holder, "min_grip") => (
            "The smallest diameter the jaws close on, in mm, with the jaws in their normal position.",
            "",
            Some(Chuck(1)),
        ),
        (Target::Holder, "max_grip") => (
            "The largest diameter the jaws close on, in mm. It must be above the smallest grip and below the chuck body.",
            "",
            Some(Chuck(2)),
        ),
        (Target::Holder, "jaw_length") => (
            "How deep the jaws reach along the bar, in mm: the most length a setup can hold in them. A setup starts from it.",
            "",
            Some(Chuck(3)),
        ),
        (Target::Holder, "body") => (
            "The chuck's outside diameter, in mm. The 160 mm starters are typical: measure yours.",
            "",
            Some(Chuck(4)),
        ),
        (Target::Setup, "grip") => (
            "How much of the bar is held in the jaws, in mm. It starts at the chuck's jaw length. Hold at least a third of the diameter.",
            "",
            Some(Chuck(5)),
        ),
        (Target::Setup, "stickout") => (
            "How far the bar sticks out of the jaws to its free end, in mm. It must reach the furthest cut from the end.",
            "",
            Some(Chuck(6)),
        ),
        (Target::Tool, "hand") => (
            "Which way the turning tool cuts: Right cuts toward the chuck, Left away from it, Neutral either way.",
            "",
            Some(LatheTool(0)),
        ),
        (Target::Tool, "nose_radius") => (
            "The radius of the insert's tip in mm; 0 is a sharp point. It rounds inside corners and sets the finish.",
            "",
            Some(LatheTool(1)),
        ),
        (Target::Tool, "lead_angle") => (
            "The angle between the cutting edge and the bar's axis, in degrees; 93 is typical for a turning tool that can also face.",
            "",
            Some(LatheTool(2)),
        ),
        (Target::Tool, "min_bore") => (
            "The smallest hole the boring bar goes into, in mm.",
            "",
            Some(LatheTool(3)),
        ),
        (Target::Tool, "reach") => (
            "How far the tool can reach into the work, in mm: a boring bar's depth into the bore, a grooving tool's or parting blade's deepest plunge, a thread mill's neck.",
            "A thread mill's neck is thinner than its cutter so it can go down a drilled hole. The thread cannot be deeper than the neck reaches.",
            Some(LatheTool(4)),
        ),
        (Target::Tool, "width") => (
            "The width of the grooving tool's or parting blade's cutting edge, in mm. A groove is at least this wide.",
            "",
            Some(LatheTool(5)),
        ),
        (Target::Tool, "shank") => (
            "The diameter of the thread mill's shank, the part the collet holds, in mm.",
            "Often wider than the cutter: a Ø4.8 mill on a Ø6 shank. Choose a collet of this size.",
            Some(Tool),
        ),
        (Target::Tool, "teeth") => (
            "How many thread-form teeth the thread mill has stacked along its axis: 1 for a single-tooth mill, 3 for a three-tooth one.",
            "Each turn round the hole, a three-tooth mill cuts three pitches of thread, so it goes round fewer times. Its teeth are spaced at one pitch, so it cuts only that pitch.",
            Some(Tool),
        ),
        (Target::Tool, "thread_angle") => (
            "The included angle of the thread form, in degrees: 60 for metric and unified threads, 55 for Whitworth.",
            "",
            Some(LatheTool(6)),
        ),
        (Target::Tool, "pitches") => (
            "The finest and coarsest pitch the threading tool or thread mill cuts, in mm, written like 0.5-3.",
            "A single-tooth thread mill cuts any pitch in its range: the mill follows the pitch, not the tooth. One with teeth stacked along it cuts only the pitch they are spaced at: enter that one pitch.",
            Some(LatheTool(7)),
        ),
        (Target::Tool, "thread") => (
            "The thread size this tap cuts. Choosing it sets the tool's diameter to the thread's major diameter.",
            "Drill the hole first with the tap drill for this size.",
            Some(Tool),
        ),
        (Target::Tool, "tap_kind") => (
            "Which way the flutes send the chips. Spiral flute taps lift them out: for blind holes. Spiral point taps push them ahead: for through holes. Straight flute taps are hand taps, for either.",
            "",
            Some(Tool),
        ),
        (Target::Operation, "turn_kind") => (
            "What this Turn does: face the end, turn an outside diameter, bore, chamfer an edge, cut a groove, part off or cut a thread. Changing the kind brings that kind's sizes and a fitting tool.",
            "Each kind uses a tool of its own type: a turning tool for Face, Outside and Chamfer, a boring bar, a grooving tool, a parting blade, a threading tool.",
            Some(TurnOp(0)),
        ),
        (Target::Operation, "face_off") => (
            "How much comes off the end of the bar, in mm: the face ends up this far in from the free end.",
            "",
            Some(TurnOp(1)),
        ),
        (Target::Operation, "to_diameter") => (
            "The diameter the facing cut reaches, in mm; 0 faces right to the centre.",
            "",
            Some(TurnOp(2)),
        ),
        (Target::Operation, "diameter") => (
            "The finished diameter in mm: what the turned or bored section measures, a groove's bottom, or a thread's major diameter. Diameters are diameters, not radii.",
            "",
            Some(TurnOp(3)),
        ),
        (Target::Operation, "z_end") => (
            "Where the cut stops along the bar, in mm. Z 0 is the free end and negative is toward the chuck, so -30 turns 30 mm of the bar.",
            "",
            Some(TurnOp(4)),
        ),
        (Target::Operation, "z") => (
            "Where the cut is along the bar, in mm: the groove's side toward the chuck, or the side of a parting cut that stays with the part. Z 0 is the free end and negative is toward the chuck.",
            "",
            Some(TurnOp(5)),
        ),
        (Target::Operation, "depth") => (
            "How much one pass takes off the diameter, in mm; a lathe drill's depth in from the free end. Passes are equal and no larger than this.",
            "",
            Some(TurnOp(6)),
        ),
        (Target::Operation, "finish") => (
            "What the last pass takes off the diameter, in mm, to leave a clean surface at the finished size.",
            "",
            Some(TurnOp(7)),
        ),
        (Target::Operation, "edge") => (
            "Which edge to break: the outside corner of the end, the inside corner of a bore, or a shoulder.",
            "",
            Some(TurnOp(8)),
        ),
        (Target::Operation, "size") => (
            "The length of the 45 degree chamfer's leg, in mm.",
            "",
            Some(TurnOp(9)),
        ),
        (Target::Operation, "width") => (
            "How wide the groove is, in mm. It is at least as wide as the grooving tool.",
            "",
            Some(TurnOp(10)),
        ),
        (Target::Operation, "thread_side") => (
            "Cut the thread on the outside of the bar or inside a bore.",
            "",
            Some(TurnOp(11)),
        ),
        (Target::Operation, "pitch") => (
            "The distance between thread crests, in mm. It must be one the gearbox cuts and one the threading tool's range covers.",
            "",
            Some(TurnOp(12)),
        ),
        (Target::Operation, "passes") => (
            "How many passes cut the thread. Empty takes the usual number for the pitch; the first pass is the deepest and they get lighter.",
            "",
            Some(TurnOp(13)),
        ),
        (Target::Setup, "holding") => (
            "The workholding to model for this setup: a Vise from the library, with its jaws on two opposite sides of the stock, or Clamps placed around it. Each is kept as boxes with a clearance gap that every move is checked against, and its hold is checked against each operation's cutting force. None models nothing.",
            "Each is kept as boxes with a clearance gap, and every move is checked against them: the cutter from its tip up to its stickout at its cutting diameter, the tool holder above that at the holder diameter (a wider shank or a reduced neck is not modelled). A move that comes within the gap is warned of in the operation preview, which covers that operation's own toolpath, and refused at export, which checks every program move, the links between operations, approaches and retracts included. Parallels, bolts, the table and the machine body are not modelled. Each operation's cutting force is also set against what the vise's or clamps' clamping force holds (sliding, lifting and tipping), over the contact the earlier setups left the jaws; an operation held with a margin under ×2 is refused at export. In a later setup the jaws or clamps sit at the stock box carried over from setup 1, not at the walls earlier operations cut, and a setup turned from setup 1 by other than a multiple of 90° cannot model them. None models nothing; the Fixture note below is only a description. A lathe setup has a Chuck instead of a Vise or Clamps, and its checks are the chuck's: the bar's diameter in the jaws must be one the chuck closes on (a 3-jaw holds round work only), the grip at least a third of that diameter and no longer than the jaws, the stickout at least the furthest cut from the free end plus 5 mm, no cut inside the grip, the bar within the lathe's swing and the grip plus the stickout within the bar's length. A stickout over 4 diameters is warned of, not refused. The chuck's body is a box 40 mm deep behind the jaws' end, and a move within the clearance gap of it is warned of in the operation preview. Nothing is exported: a lathe has no G-code.",
            Some(Vise(8)),
        ),
        (Target::Setup, "holder") => (
            "The library entry used: a vise for Vise, a clamp for Clamps. Entries are made on the Workholding page; changing one changes every setup that uses it.",
            "",
            None,
        ),
        (Target::Setup, "vise_axis") => (
            "Which sides of the stock the jaws face: X sides put one jaw on the −X side and one on the +X side, Y sides on −Y and +Y. A stock wider across the jaws than the vise's maximum opening shows an issue.",
            "",
            Some(Vise(6)),
        ),
        (Target::Setup, "vise_offset") => (
            "Shifts the jaws along their length from the middle of the stock, in mm: toward +Y for jaws on the X sides, toward +X for jaws on the Y sides. 0 centres them. The jaws must still overlap the stock along their length: an offset of half the stock's length plus half the jaw width or more shows an issue.",
            "",
            Some(Vise(7)),
        ),
        (Target::Setup, "vise_grip") => (
            "How much of the stock the jaws hold, as a height above the stock's bottom, in mm. The jaws are modelled from far below the stock up to it; the stock above it is not covered by the modelled jaws. It starts at the vise's own grip.",
            "",
            Some(Vise(4)),
        ),
        (Target::Setup, "holding_gap") => (
            "The clearance gap the checks keep round the modelled jaws or clamps, in mm: 0 or more; left empty it is 2 mm. The operation preview, the export and the Run checks all use it, with the machine profile's path blending tolerance (G64 P) added, since the controller may leave the programmed path by that much at a corner.",
            "A move whose cutter or holder comes within the two is warned of in the operation preview (that operation's own toolpath) and refused at export and by the Run checks (every program move, the links between operations, approaches and retracts included). It is not the Requested fixture clearance under Setup notes, which nothing checks.",
            Some(Vise(9)),
        ),
        (Target::Setup, "clamps") => (
            "Where the clamps sit, a row each: the side of the stock, -X, +X, -Y or +Y, and the offset, in mm, that moves the clamp along that side from its middle (toward +Y on the X sides, toward +X on the Y sides), no more than half that side's length. Place 1 to 8 clamps with + Add clamp and Remove.",
            "The positions are saved only when every offset is a number; what is wrong is shown under the rows. The setup keeps them as side:offset separated by semicolons, for example -X:0;+X:20.",
            Some(Clamp(4)),
        ),
        (_, "name") => (
            "A descriptive name shown in the browser, library and setup sheet. Use names that identify the part position, operation or hardware.",
            "",
            None,
        ),
        (_, "rpm") => (
            "Spindle revolutions per minute. LinuxCNC export writes it as S, rounded to a whole number. When the project material has cutting data, a surface speed outside its range is flagged on this tab and on the NC review page; export still goes ahead.",
            "Surface speed = π × tool diameter × rpm ÷ 1000. The app does not calculate a speed for you.",
            Some(Tool),
        ),
        (_, "spindle_rotation") => (
            "CW or CCW, viewed from the spindle toward the workpiece along setup -Z. Match the cutter's cutting hand. Changing rotation reverses Pocket contours and one-way Facing strokes for the same milling method. The illustration uses CW for both examples.",
            "",
            Some(MillingDirection),
        ),
        (_, "facing_milling") => (
            "Climb (down milling) starts with a thicker chip and exits thin; Conventional (up milling) starts thin and exits thicker. With One way, the preview chooses travel relative to the nominal uncut side and saved spindle rotation. Both ways is mixed.",
            "Entry and full-width engagement may include both modes. Illustration: CW spindle in both examples, viewed toward the part.",
            Some(MillingDirection),
        ),
        (_, "pocket_milling") => (
            "Climb or Conventional relative to the remaining material. Reverses contour, waterline and pocket loops while retaining spindle rotation. CW climb runs counterclockwise around an internal wall and clockwise around an outside profile; CCW reverses these directions. Entry and full-width engagement can include both modes.",
            "In Adaptive clearing, which side of the cutter the material is kept on: with a clockwise spindle, Climb keeps it on the right.",
            Some(MillingDirection),
        ),
        (_, "pocket_entry") => (
            "Profile ramp descends along the pocket contour at the ramp angle, then cuts the loop at depth. Plunge feeds straight down and needs a plunge-capable end mill. Both enter at 25% feed. Between loops the tool stays down, stepping at most 1.5 stepovers clear of walls and islands; where it cannot, it retracts and enters again.",
            "A loop too short for its ramp, one the ramp would have to go round more than ten times (twice in a rest pass's corners, where little of the cutter meets material), plunges instead. Below the first level a loop the tool enters again rapids down to 0.5 mm over the level before, where the cutter has already been.",
            Some(Ramp),
        ),
        (_, "feed") => (
            "Cutter travel speed during cutting, in mm/min. Feed per tooth = feed ÷ (rpm × flute count). When the project material has cutting data, a feed per tooth outside the range for the tool's diameter is flagged on this tab and on the NC review page; export still goes ahead. Plunge and ramp feeds are not checked.",
            "",
            Some(Tool),
        ),
        (_, "coolant") => (
            "Coolant for this operation. LinuxCNC export writes M8 for Flood, M7 for Mist, the machine's Air output for Air (refused when it has none) and nothing for Off, then M9 when the coolant changes, at a tool change and at the end of the program. The preview does not simulate coolant.",
            "",
            Some(Linking(0)),
        ),
        (_, "hole_diameter") => (
            "Filter detected circular openings by nominal diameter in mm (±0.1 mm). Zero includes all detected holes on the selected rim. The drill must fit every included opening. Select the horizontal rim face first.",
            "",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "boundary") => (
            "Defines the region that should contain machining: selected geometry, stock outline or model outline. Facing uses rectangles, or the part outline with Model silhouette. Pocket and Adaptive clearing follow the selected floor, and a Pocket the outline of each hole picked by a click on its wall. Contour and finishing use the imported model silhouette.",
            "A Facing with Model silhouette clears over the part's outline seen from above, grown by Clear past outline, and leaves the stock beyond it at its height.",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "containment") => (
            "Places the cutter inside, on, or outside the boundary. Tool-center placement differs from where the cutter's edge reaches.",
            "",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "clearance") => (
            "Height above stock top used for longer non-cutting moves, in mm. It must be at least as high as the retract plane. Where the machine retracts to a clearance plane, export refuses a setup whose plane, the highest height its program reaches, is not above every modelled jaw and clamp and its gap; other fixture clearance still needs verification.",
            "",
            Some(Heights),
        ),
        (_, "retract") => (
            "Height above stock top for lifting between cutting moves, in mm. Keep it below or equal to clearance and above obstacles.",
            "",
            Some(Heights),
        ),
        (_, "top") => (
            "Positive margin above the cutting start. Earlier cuts automatically lower the start when the whole machining area above it is cleared. Otherwise Facing/Pocket/Contour/Drill use stock top and Waterline uses model top.",
            "Parallel uses this offset for approach heights. Clearance/retract keep their original reference; negative values do not force a deeper entry.",
            Some(Offsets),
        ),
        (_, "bottom") => (
            "Offset from the selected bottom geometry, in mm. Negative extends the cut below it; positive leaves the bottom higher.",
            "",
            Some(Offsets),
        ),
        (_, "tab_count") => (
            "How many tabs an outside Contour leaves on each outline it goes around, spaced evenly along it, the first half a spacing from where the cut starts. Two or three hold a small part; use more on a long or thin one, so it cannot bend into the cutter.",
            "",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "tab_width") => (
            "Each tab's width at its narrowest, midway across the cut, in mm. Over each tab the tool's centre rises along this width plus its diameter; its round end leaves the tab wider toward the part's wall and the stock's, by up to that diameter.",
            "",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "tab_height") => (
            "How thick each tab is, in mm: over the tabs the cut stops this far above the stock's bottom, or above the contour's bottom where that is higher. It must be less than the depth the contour cuts.",
            "A tab holds only where stock is left beyond the cut, so leave more stock beside the part than the stock to leave, the tool's diameter and a 2 mm wall. Levels above the tabs' top go round in full. Below it the tool rises straight up at each tab's start and plunges back down at its end at a quarter of the cutting feed, as it does entering the cut. Cut or file the tabs off after the part is unclamped.",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "through") => (
            "Add Breakthrough below the bottom of each hole that goes through the part. A blind hole is refused with Yes, since the part goes on under its bottom. Check the extra depth and the material or fixture underneath.",
            "",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "breakthrough") => (
            "Additional travel past the exit, in mm: of a hole that goes through, for a drill (never under a blind hole), or of a Pocket's or Adaptive clearing's openings cut through, below where their walls end. The real required distance also depends on drill-point geometry.",
            "",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "openings") => (
            "Holes in the selected floors with nothing of the part over or under them, such as switch cut-outs in a plate: Leave clears over them at the floor's height, Cut through also cuts each down to where its walls end, plus Breakthrough. Pocket and Adaptive clearing share this setting.",
            "An opening is a hole in a selected floor whose column holds no part at all. Its walls are followed at the cutter's radius, as a pocket's are, so corners keep that radius; run a smaller cutter after it to sharpen them. Openings narrower than the cutter are left, for a smaller one. Round holes are openings too, but Drilling makes them faster.",
            Some(Operation(Strategy::Pocket)),
        ),
        (_, "angle_a" | "angle_b") => (
            "Indexed position of the primary or secondary rotary axis, in degrees. The axis is positioned before cutting and held fixed during the cut.",
            "",
            Some(Operation(Strategy::Indexed)),
        ),
        (_, "rotary_axis") => (
            "A rotates about X, B about Y, and C about Z. Choose the axis that matches the setup's saved machine arrangement.",
            "",
            Some(Operation(Strategy::Rotary)),
        ),
        (_, "rotary_start" | "rotary_end") => (
            "Start or end of the requested angular sweep, in degrees. Keep the interval within the saved rotary travel limits.",
            "",
            Some(Operation(Strategy::Rotary)),
        ),
        (_, "rotary_style") => (
            "Line, circular or spiral motion around the rotary axis. This records the intended pattern for a future rotary toolpath engine.",
            "",
            Some(Operation(Strategy::Rotary)),
        ),
        (_, "axis_control") => (
            "Align the tool with the surface normal or add lead and lean. Actual multi-axis orientation and collision solving are not connected yet.",
            "",
            Some(Operation(Strategy::FiveAxis)),
        ),
        (_, "tilt_min" | "tilt_max") => (
            "Allowed tool tilt range in degrees. Use it to constrain the intended orientation; it does not replace machine-axis limits or collision checks.",
            "",
            Some(Operation(Strategy::FiveAxis)),
        ),
        (_, "lead_angle") => (
            "Tool tilt forward or backward along the cutting direction, in degrees. It changes where the cutter contacts the surface.",
            "",
            Some(Operation(Strategy::FiveAxis)),
        ),
        (_, "lean_angle") => (
            "Tool tilt sideways across the cutting direction, in degrees. It is independent of forward/backward lead.",
            "",
            Some(Operation(Strategy::FiveAxis)),
        ),
        (_, "collision") => (
            "Preferred response to an obstruction: retract and relink, or tilt within limits. This is a saved preference; no collision solver is active yet.",
            "",
            Some(Linking(1)),
        ),
        (_, "cycle") => (
            "Drilling feeds once to depth. Chip breaking retracts slightly between pecks; deep drilling retracts clear of the hole. Counterboring adds a bottom dwell. Helical milling cuts each hole with a flat end mill, down a helix to a flat bottom.",
            "Thread milling cuts each drilled hole's thread with a thread mill, on a CNC: the mill goes down the hole, arcs in to the wall and climbs one pitch a turn. Tapping is by hand, on the manual mill or the lathe: the spindle (off) or the tailstock holds the tap square. The CNC does not tap.",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "thread") => (
            "The thread to cut or tap, from the standard sizes. From the model reads each hole's size and pitch from the part, where its thread is modeled.",
            "A hole the part does not model as a thread needs a size picked here. The hole must already be drilled: a tap drill for a tap, a bore near the thread's minor diameter for a thread mill.",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "thread_hand") => (
            "Which way the thread turns: right hand screws in clockwise, left hand counterclockwise. From the model reads it from the part.",
            "",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "thread_depth") => (
            "How deep the full thread runs from the hole's rim, in mm. Blank takes the depth the model has.",
            "",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "thread_passes") => (
            "How many times the thread mill goes round each thread, from 1 to 10. Each pass goes further out, the last to the thread's full depth, which eases the load on a small mill or a hard material.",
            "",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "spring_pass") => (
            "Yes goes round the thread once more at full depth. The mill springs back from the wall as it cuts, so the last pass takes the spring out and trues the size.",
            "",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "peck") => (
            "Maximum depth of each drilling increment, in mm. Smaller pecks shorten the chip; retract behavior is selected by the drilling cycle.",
            "",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "dwell") => (
            "Pause duration at the bottom of a counterboring cycle, in seconds, before retracting.",
            "",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "hole_order") => (
            "Optimize travel drills the nearest remaining hole next, a greedy route that is not a global optimum. By X then Y drills the holes in setup X then Y order.",
            "",
            Some(Linking(2)),
        ),
        (_, "ramp") => (
            "How the cutter enters material. Adaptive clearing on an imported part enters with a Helix or a Plunge where it cannot reach the material from the side; the sample also records a back-and-forth or profile ramp. Match the entry to the cutter's cutting capabilities.",
            "",
            Some(Ramp),
        ),
        (_, "ramp_angle") => (
            "Slope of the entry move in degrees relative to the horizontal plane. A smaller angle takes a longer distance to reach cutting depth. Adaptive clearing's helix takes 0.1° to 15°.",
            "",
            Some(Ramp),
        ),
        (_, "lead_in") => (
            "Radius of the curved approach into the cut, in mm. It gives the cutter a gradual entry; available space must be checked.",
            "",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "keep_down") => (
            "Request links close to cutting depth where possible to reduce retracts. This does not authorize crossing uncut stock or fixtures.",
            "",
            Some(Linking(3)),
        ),
        (_, "facing_pattern") => (
            "Both ways alternates travel on neighboring rows, mixing climb and conventional cutting without reversing the spindle. One way enables the Climb / Conventional choice and retracts for return moves. The pass-angle field rotates either pattern in setup XY.",
            "",
            Some(Operation(Strategy::Face)),
        ),
        (_, "facing_depth_order") => (
            "Continue from previous end starts the next depth at the last pass's X/Y, with the row order and travel directions reversed. Return to first start repeats each depth from the original X/Y. Applies to Both ways; One way always returns.",
            "",
            Some(Linking(4)),
        ),
        (_, "facing_link") => (
            "Keep tool down feeds sideways between adjacent rows, then cuts back across the stock. Continue from previous end also feeds vertically down at the last X/Y for the next depth. Retract each pass lifts to clearance between rows.",
            "Applies to Both ways and the rectangular Facing preview; fixtures are not avoided, and a move within the clearance gap of a modelled vise jaw or clamp is refused at export.",
            Some(Linking(5)),
        ),
        (_, "direction") => (
            "Angle of the parallel cutting passes in the setup XY plane, in degrees. Rotates the pass pattern across the face.",
            "",
            Some(Operation(Strategy::Parallel)),
        ),
        (_, "x_min" | "x_max" | "y_min" | "y_max") => (
            "One side of the block a Box removes, in mm in the setup's frame, measured from the work zero. The block must be at least as wide as the tool across its passes.",
            "Passes run along the block's longer side. A side the block shares with open air is cut from outside the stock; a side that touches the part keeps the stock to leave.",
            Some(Operation(Strategy::Box)),
        ),
        (_, "z_top" | "z_bottom") => (
            "The top and the floor of the block a Box removes, in mm in the setup's frame, measured from the work zero. The floor is below the top.",
            "The tool goes down by the stepdown until it reaches the floor.",
            Some(Operation(Strategy::Box)),
        ),
        (_, "stepover") => (
            "Sideways spacing of adjacent passes, as a percentage of cutter diameter. For a 10 mm tool, 40% means 4 mm. Smaller spacing increases overlap and pass count. Pocket accepts up to 50%.",
            "",
            Some(Stepover),
        ),
        (_, "surface_stepover") => (
            "Spacing between adjacent finishing passes, in mm. Smaller spacing usually reduces scallop height but increases machining time.",
            "",
            Some(Stepover),
        ),
        (_, "optimal_load") => (
            "Target radial engagement for adaptive clearing, in mm: how much of the cutter's side is cutting at once, from 2% to 50% of the tool diameter. The path sweeps in open areas and loops into corners and along walls to hold it, so the stepdown can be deeper than for Pocket; a new operation starts at 15% of the diameter and a stepdown of two diameters.",
            "",
            Some(Operation(Strategy::Adaptive)),
        ),
        (_, "silhouette_extend") => (
            "How far past the part's outline seen from above the cutter's edge reaches, in mm, for a Facing with the Model silhouette boundary. A round cutter needs about a third of its radius past a square outer corner to clear it, more for a sharper one; Generate refuses less and says the least that works. Stock beyond it stays at its height.",
            "",
            Some(Operation(Strategy::Face)),
        ),
        (_, "minimum_radius") => (
            "Smallest radius the path may turn in open material, in mm, from 0 to the tool radius. Larger radii soften direction changes. At a wall, or to avoid overloading the cutter, the path turns as sharply as it must.",
            "",
            Some(Operation(Strategy::Adaptive)),
        ),
        (_, "stepdown") => (
            "Maximum depth removed in one Z layer, in mm. A 12 mm pocket with 3 mm stepdown needs at least four depth levels.",
            "",
            Some(Stepdown),
        ),
        (_, "side") => (
            "Choose which side of the selected contour the cutter should follow: inside, outside or centered on the chain.",
            "",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "compensation") => (
            "Choose whether tool-radius offset is calculated in CAM or adjusted at the control. Wear modes need a matching, validated controller/postprocessor convention.",
            "",
            Some(Linking(6)),
        ),
        (_, "finish_passes") => (
            "Number of finishing passes along the boundary after roughing. Extra passes may improve consistency; cutting strategy still needs validation.",
            "",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "slope_min" | "slope_max") => (
            "Surface slope range to include, in degrees from horizontal. 0° is a flat floor; 90° is a vertical wall.",
            "",
            Some(Operation(Strategy::Waterline)),
        ),
        (_, "pattern") => (
            "Choose a parallel or scallop-style finishing pattern. Parallel sweeps in one direction; scallop follows surface spacing.",
            "",
            Some(Operation(Strategy::Scallop)),
        ),
        (_, "allowance") => (
            "Material intentionally left on walls, in mm, for a later finishing operation. Zero requests the nominal wall position.",
            "",
            Some(Allowance),
        ),
        (_, "axial_allowance") => (
            "Material intentionally left on floors, in mm. A finishing operation removes it to reach the nominal depth.",
            "",
            Some(Allowance),
        ),
        (_, "tolerance") => (
            "Requested maximum toolpath approximation deviation, in mm: in Adaptive clearing, how far the path's straight segments may stray from the computed path. A smaller value can require more path segments. It is not a guarantee of finished-part accuracy.",
            "",
            Some(Linking(7)),
        ),
        (_, "rest") => (
            "Target material remaining from an earlier operation, often with a smaller tool. Select the previous operation as the source; remaining-stock calculation is not connected. Adaptive clearing on an imported part needs no setting: like Pocket, it cuts only where earlier operations and setups left material.",
            "",
            Some(Operation(Strategy::Adaptive)),
        ),
        (_, "offset") => (
            "Work coordinate register for this setup, G54–G59. Its origin must correspond to the origin probed on the real machine.",
            "",
            Some(Orientation),
        ),
        (Target::Tool, "number") => (
            "Tool number written as T# and, with a tool table, G43 H#. It must match the entry in LinuxCNC's tool table. Each library tool needs a unique number from 1 to 9999.",
            "",
            None,
        ),
        (_, "post_tool_length") => (
            "How programs apply tool length: from LinuxCNC's tool table with T# M6 and G43 H#, or one program per tool that you touch off before running it.",
            "",
            Some(Post(0)),
        ),
        (_, "post_retract") => (
            "Where the tool goes before a tool change and at the end: machine Z0 with G53, or the setup's highest clearance height.",
            "",
            Some(Post(1)),
        ),
        (_, "post_blend") => (
            "Path blending tolerance for LinuxCNC's trajectory planner. A positive value writes G64 P with this tolerance in mm, so corners may round by up to that much at speed. 0 writes G61 exact path.",
            "The checks allow for it: it is added to the setup's clearance gap around the jaws and clamps and to the cutter's radius when a rapid descent is proven clear of the stock, also for a UC100 run. At a corner the blend can leave the vertical by 2.41 times the tolerance below the height a rapid crosses at, so the clearance plane and every sideways rapid keep 2.5 times the tolerance over the stock top (export and the Run checks refuse a rapid closer than that) and the link between two operations of one tool is raised to that height.",
            Some(Post(2)),
        ),
        (_, "post_dwell") => (
            "Seconds to wait after starting or changing the spindle before the first move, and, between two operations of one tool that turn it opposite ways, after the M5 that stops it before it is started the other way. Use it when the spindle has no at-speed feedback. 0 writes no dwell. At a reversal nothing then waits for the spindle to stop.",
            "",
            Some(Post(3)),
        ),
        (_, "post_air") => (
            "LinuxCNC output used for operations set to Air: M7 (mist) or M8 (flood), whichever is wired to the air blast.",
            "",
            Some(Post(4)),
        ),
        (_, "probe_plate") => (
            "Thickness of the touch plate, in mm: measure it. Probe Z makes work Z at the plate's top this value, so the surface under the plate is Z 0. Used when the project's material is not a metal; on a metal the plate's lead goes on the stock and the tool touches it. Leave it empty without a plate.",
            "",
            Some(Probe(0)),
        ),
        (_, "pins") => (
            "The fixture plate's dowel pins, as drilled, in the plate's own X Y (mm), pin 1 first: \"0 0; 120 0\" for two pins 120 mm apart along X. Pin 1 is the reference pin, pin 2 gives the turn. Leave it empty without pins.",
            "The plate is taken as square to the machine's X and Y: Record the pins' place on the Control page measures how far off it is. A setup that sits on the pins gets its X and Y work offset from pin 1's recorded place.",
            Some(Probe(1)),
        ),
        (_, "pin_diameter") => (
            "The pins' diameter, in mm: measure it. The holes a part drops over them by must be this to 0.2 mm over: a wider hole leaves the part free to move. The checks keep tools clear of a pin-sized box per pin.",
            "",
            Some(Probe(2)),
        ),
        (_, "pin_height") => (
            "How far the pins stand above the plate, in mm: measure it. The part's holes for them must be at least this deep, and Pin centre probes the pins' sides min(3 mm, half of it) below their top.",
            "",
            Some(Probe(3)),
        ),
        (_, "probe_tip") => (
            "Diameter of the probe's ball or pin, in mm: measure it, or the tool's diameter for a tool with the plate's lead on conductive stock. Probe corner and Probe centre move each side they touch by half of it. Leave it empty without a probe.",
            "",
            Some(Probe(4)),
        ),
        (_, "probe_feed") => (
            "How fast Z feeds down to the plate, in mm/min. Slow is accurate: the machine stops a little past the contact, and AXIS CAM uses where the contact was.",
            "",
            Some(Probe(5)),
        ),
        (_, "probe_distance") => (
            "How far down a probe goes at most, in mm, and never below the bottom of Z. Without contact by then it stops and nothing is set.",
            "",
            Some(Probe(6)),
        ),
        (_, "probe_retract") => (
            "How far the tool lifts after the contact, in mm, so the plate can come away. 0 leaves it on the plate.",
            "",
            Some(Probe(7)),
        ),
        (Target::Machine, "kind") => (
            "CNC runs G-code. Manual is a hand-operated mill with a DRO, Lathe a hand-operated lathe with dials: no G-code, no Run, and their setups are shown as steps on the MACHINE page.",
            "",
            Some(Machine(3)),
        ),
        (_, "speeds") => (
            "The spindle speeds the gearbox has, in rpm, comma-separated. A range such as 100-2000 means a variable drive. The rpm of a manual operation must be one of them.",
            "",
            Some(Manual(0)),
        ),
        (_, "hand_feed") => (
            "The fastest feed you can turn steadily by hand, in mm/min. No manual operation feeds faster.",
            "",
            Some(Manual(1)),
        ),
        (_, "hand_rapid") => (
            "How fast you move an axis off the cut by hand, in mm/min. It sets the time of the moves between cuts.",
            "",
            Some(Manual(2)),
        ),
        (_, "turn_x" | "turn_y" | "turn_z") => (
            "How far the axis moves in one handwheel turn, in mm. The steps use it to say how many turns a feed is.",
            "",
            Some(Manual(3)),
        ),
        (_, "swing") => (
            "The largest diameter the lathe turns over its bed, in mm. A bar or part wider than this is refused.",
            "",
            Some(Manual(4)),
        ),
        (_, "centres") => (
            "The distance between the centres, in mm: the longest work the lathe takes between chuck and tailstock.",
            "",
            Some(Manual(5)),
        ),
        (_, "cross_reads") => (
            "Whether a reading on the cross-slide dial takes that much off the diameter or off the radius. Not known yet: the first lathe setup asks you to cut a test and measure, and saves the answer.",
            "",
            Some(Manual(6)),
        ),
        (_, "cross_turn" | "top_turn" | "carriage_turn" | "quill_turn") => (
            "The numbers the dial goes through in one full turn of the handwheel: what it reads at the index after one turn. The steps use it to say how many turns and what reading.",
            "",
            Some(Manual(7)),
        ),
        (_, "cross_div" | "top_div" | "carriage_div" | "quill_div") => (
            "How many divisions the dial has round its collar, a whole number. A reading is rounded to one division.",
            "",
            Some(Manual(8)),
        ),
        (_, "feeds") => (
            "The carriage power feeds the gearbox has, in mm per revolution, comma-separated. Turning passes use the one nearest 0.1.",
            "",
            Some(Manual(9)),
        ),
        (_, "pitches") => (
            "The thread pitches the gearbox cuts, in mm, comma-separated. A thread operation must use one of them.",
            "",
            Some(Manual(10)),
        ),
        (_, "dro_comp") => (
            "Yes when the DRO has tool-radius compensation, so it reads the tool's edge instead of its centre.",
            "",
            Some(Manual(11)),
        ),
        (_, "edge_finder") => (
            "The diameter of the edge finder that picks up the work zero, in mm. The Zero step takes its radius off each face it touches.",
            "",
            Some(Manual(12)),
        ),
        (_, "pass_time") => (
            "Seconds to reset, set and lock the axes for one pass. It adds to the setup's minutes.",
            "",
            Some(Manual(13)),
        ),
        (_, "tool_time") => (
            "Minutes to change a tool and pick up the zero again. It adds to the setup's minutes.",
            "",
            Some(Manual(14)),
        ),
        (_, "move_time") => (
            "Minutes to move the stock between this machine and another. The plan adds it each time the stock changes machine.",
            "",
            Some(Manual(15)),
        ),
        (_, "machines") => (
            "The machines that hold this entry. None checked means every CNC machine and no manual machine.",
            "",
            None,
        ),
        (_, "origin") => (
            "Work zero for this setup's G-code and runs: a spot on the top or bottom of the stock this setup starts from, or the part file's own origin. The probe on the Control page finds exactly this point; the editor says where it is touched off and whether it is still on the stock.",
            "Work zero for this setup's G-code and runs: a corner, the middle of an edge or the centre of the stock's top or bottom, on the stock this setup starts from (what earlier setups leave, turned with it), or the part file's own origin.",
            Some(Orientation),
        ),
        (_, "stock_type") => (
            "Relative size box adds offsets to the six sides of the part's box after setup rotation. Fixed size box uses X, Y and Z sizes. A round bar lies along setup Z on X 0 Y 0, for the lathe. Values are kept when switching.",
            "",
            Some(Stock(0)),
        ),
        (_, "stock_diameter") => (
            "Round bar diameter, mm. The bar lies along the setup Z axis, centred on X 0 Y 0.",
            "",
            Some(Stock(1)),
        ),
        (_, "stock_length") => (
            "Round bar length along the setup Z axis, mm, from Z 0 up.",
            "",
            Some(Stock(2)),
        ),
        (_, "stock_radial") => (
            "Extra material round the part, mm: the bar's radius is the part's largest distance from the setup Z axis plus this. 0 touches the part.",
            "",
            Some(Stock(3)),
        ),
        (_, "stock_x" | "stock_y" | "stock_z") => (
            "Overall stock box extent along this setup axis, in mm. Fixed stock is centered on X and Y with its bottom on Z=0 in the preview. Use relative stock for extra material measured from each part side.",
            "",
            Some(Stock(
                4 + ["stock_x", "stock_y", "stock_z"]
                    .iter()
                    .position(|k| *k == key)
                    .unwrap_or(0) as u8,
            )),
        ),
        (_, "stock_top") => (
            "Extra material above the part's highest point along setup +Z, in mm. 10 adds 10 mm only at the top. Z size = part height + top + bottom. 0 is flush with the part.",
            "",
            Some(StockOffset(2, true)),
        ),
        (_, "stock_bottom") => (
            "Extra material below the part's lowest point along setup −Z, in mm. This is independent of the top offset. 0 is flush with the part; negative values are not allowed.",
            "",
            Some(StockOffset(2, false)),
        ),
        (_, "stock_left") => (
            "Extra material beyond the part's minimum X along setup −X, in mm. X size = part width + left + right. This side follows the setup axis, regardless of the camera view.",
            "",
            Some(StockOffset(0, false)),
        ),
        (_, "stock_right") => (
            "Extra material beyond the part's maximum X along setup +X, in mm. 2 adds 2 mm only on the right. It does not also add material on the left.",
            "",
            Some(StockOffset(0, true)),
        ),
        (_, "stock_front") => (
            "Extra material beyond the part's minimum Y along setup −Y, in mm. Y size = part depth + front + back. The front offset is independent of the back offset.",
            "",
            Some(StockOffset(1, false)),
        ),
        (_, "stock_back") => (
            "Extra material beyond the part's maximum Y along setup +Y, in mm. 0 is flush with the part. Sides are measured after the part's setup rotation, regardless of the camera view.",
            "",
            Some(StockOffset(1, true)),
        ),
        (_, "rx" | "ry" | "rz") => (
            "Rotate the part about this axis, in degrees. Rotation order is X, then Y, then Z. Used to describe a new machining position; this is not a rotary motion command.",
            "",
            Some(Orientation),
        ),
        (_, "tx" | "ty" | "tz") => (
            "Translate the part along this setup axis, in mm. Relative stock follows the part and keeps its offsets; fixed stock stays in place. Exact CAD-to-machine origin placement remains to be implemented.",
            "",
            Some(Orientation),
        ),
        (_, "fixture") => (
            "A note of what holds the stock when nothing is modelled: a vise, soft jaws, clamps, chuck or custom fixture. It is shown in the setup's header as not modelled, printed on the setup sheet and in the program header, and is not checked.",
            "To have a vise's jaws or clamps checked, model them under Modelled workholding above: they then describe the setup everywhere and this note is set aside.",
            None,
        ),
        (_, "reposition") => (
            "Action needed before this setup: initial loading, reclamping and probing, rotary indexing, or changing the fixture. Included in the setup sheet.",
            "",
            Some(Orientation),
        ),
        (_, "notes") => (
            "Instructions for the operator: how to reclamp, which surfaces to locate, and where to probe the new origin.",
            "",
            None,
        ),
        (_, "axes") => (
            "Number of controllable axes: XYZ for 3-axis, one rotary for 4-axis, or two rotaries for 5-axis. Select indexed or simultaneous motion separately.",
            "",
            Some(Orientation),
        ),
        (_, "units") => (
            "Units for machine travel, feed and pivot dimensions. The machining project and tool dimensions remain in millimeters.",
            "",
            None,
        ),
        (_, "x" | "y" | "z") => (
            "Available linear travel along this machine axis, in the selected machine units. Enter the usable stroke from the machine specification.",
            "",
            Some(Machine(
                4 + ["x", "y", "z"].iter().position(|k| *k == key).unwrap_or(0) as u8,
            )),
        ),
        (_, "mode") => (
            "Indexed motion positions the rotary axes before cutting; simultaneous motion changes rotary and linear positions together. This prototype saves the capability but does not solve it.",
            "",
            Some(Operation(Strategy::Indexed)),
        ),
        (_, "arrangement") => (
            "Physical rotary configuration: axes on the table, spindle head, or both. A rotates about X, B about Y, and C about Z.",
            "",
            Some(Orientation),
        ),
        (_, "rotary_min" | "rotary_max" | "rotary2_min" | "rotary2_max") => (
            "Lower or upper travel bound of this rotary axis, in degrees. Enter mechanical limits, including negative angles where supported.",
            "",
            Some(Orientation),
        ),
        (_, "pivot_x" | "pivot_y" | "pivot_z" | "pivot2_x" | "pivot2_y" | "pivot2_z") => (
            "Location of this rotary axis pivot in the machine coordinate frame, using the selected machine units. Accurate measured pivots will be required by the future kinematic solver.",
            "",
            Some(Orientation),
        ),
        (_, "controller") => (
            "Controller family. LinuxCNC has a 3-axis postprocessor; other controllers are saved for planning and cannot export G-code yet.",
            "",
            None,
        ),
        (_, "tool_change") => (
            "Whether the operator changes tools manually or the machine has an automatic tool changer. Used for planning the setup sequence.",
            "",
            Some(Machine(7)),
        ),
        (_, "tcp") => (
            "Tool center point control lets the controller compensate linear axes while rotating, to maintain the tool tip's position. Requires supported kinematics and a matching postprocessor.",
            "",
            Some(Operation(Strategy::FiveAxis)),
        ),
        (_, "type") => (
            "Cutter geometry: flat end mill, ball end mill, drill, face mill, chamfer mill, thread mill or tap, or for a lathe a turning tool, boring bar, grooving tool, parting blade or threading tool. Choose a type suited to the operation.",
            "",
            Some(Tool),
        ),
        (_, "diameter") => (
            "Cutting diameter in mm, not the holder or shank diameter. It determines slot access, radial engagement and stepover in percent.",
            "",
            Some(Tool),
        ),
        (_, "flutes") => (
            "Number of cutting edges. Used with spindle rpm and feed to calculate feed per tooth.",
            "",
            Some(Tool),
        ),
        (_, "length") => (
            "Length of the cutting flutes in mm. The cut must not rely on the non-cutting shank to remove material.",
            "",
            Some(Tool),
        ),
        (_, "stickout") => (
            "Distance from the holder nose to the cutter tip, in mm. More stickout increases reach but generally reduces rigidity.",
            "",
            Some(Tool),
        ),
        (_, "holder") => (
            "Maximum holder diameter in mm. The holder is wider than the cutter and needs its own clearance around walls and fixtures.",
            "",
            Some(Tool),
        ),
        (_, "tip_angle") => (
            "A chamfer mill's included point angle in degrees, 90 when left empty. Match a countersink's angle: an 82° countersink needs an 82° cutter. A Drill finds each countersink the model has with its angle and refuses a chamfer mill of another angle, which would cut into its wall (sharper) or leave its cone (blunter).",
            "Drilling with a chamfer mill plunges its point into each detected opening until the cone is as wide as the opening at its rim, so a countersink comes out at the CAD's size. Bottom offset raises the plunge (a narrower chamfer); a negative one lowers it (a wider one).",
            Some(Tool),
        ),
        _ => (
            "Saved parameter for the current setup or operation. See the field label for its units.",
            "",
            None,
        ),
    };
    Help {
        title: crate::material::label(key)
            .filter(|_| target == Target::Material)
            .unwrap_or_else(|| label.into()),
        body: body.into(),
        more: more.into(),
        diagram,
    }
}

/// The animations repeat every 60 s: a whole number of the 5 s phase, of the
/// spindle's turns (`SPIN`) and of the chips' 41 px travel (`CHIP_SPEED`).
const ANIMATION_PERIOD: f64 = 60.;
/// The chips' speed in px per second: about 10, 15 travels in the period.
const CHIP_SPEED: f32 = 41. * 15. / 60.;
/// Seconds since the first drawing, reduced in f64 to the animations' period
/// so the f32 keeps its precision after days of uptime.
fn animation_time() -> f32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    wrapped(START.get_or_init(Instant::now).elapsed().as_secs_f64())
}
fn wrapped(seconds: f64) -> f32 {
    (seconds % ANIMATION_PERIOD) as f32
}
struct Illustration {
    diagram: Diagram,
    /// The moment to hold, or None to follow the clock.
    still: Option<f32>,
}
/// The moment a paused animation shows: 3 s into the 5 s phase, with the
/// cutter partway along its path and an indexed part already turned.
const STILL_TIME: f32 = 3.;
impl canvas::Program<Message> for Illustration {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut f = canvas::Frame::new(renderer, bounds.size());
        let time = self.still.unwrap_or_else(animation_time);
        let phase = (time / 5.).fract();
        let p = |a: [f32; 3]| {
            Point::new(
                139. + (a[0] - a[1]) * 1.40,
                111. + (a[0] + a[1]) * 0.52 - a[2] * 1.35,
            )
        };
        let line = |f: &mut canvas::Frame, a: [f32; 3], b: [f32; 3], color: Color, width: f32| {
            f.stroke(
                &canvas::Path::line(p(a), p(b)),
                canvas::Stroke::default()
                    .with_color(color)
                    .with_width(width),
            );
        };
        let polygon = |f: &mut canvas::Frame, points: &[[f32; 3]], color: Color| {
            let path = canvas::Path::new(|b| {
                b.move_to(p(points[0]));
                for a in &points[1..] {
                    b.line_to(p(*a))
                }
                b.close()
            });
            f.fill(&path, color);
            f.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(Color::from_rgb8(157, 182, 198))
                    .with_width(0.7),
            );
        };
        let cyan = Color::from_rgb8(66, 211, 234);
        let gold = Color::from_rgb8(255, 191, 85);
        if matches!(
            self.diagram,
            Diagram::MillingDirection | Diagram::MillingChoice(..)
        ) {
            let stroke = canvas::Stroke::default().with_color(cyan).with_width(2.);
            let clockwise = match self.diagram {
                Diagram::MillingChoice(_, cw) => cw,
                _ => true,
            };
            let examples = match self.diagram {
                Diagram::MillingChoice(climb, _) => vec![(
                    75.,
                    if climb { -1. } else { 1. },
                    if climb { "CLIMB" } else { "CONVENTIONAL" },
                )],
                _ => vec![(5., -1., "CLIMB"), (145., 1., "CONVENTIONAL")],
            };
            for (left, sign, label) in examples {
                let sign = if clockwise { sign } else { -sign };
                f.fill_text(canvas::Text {
                    content: label.into(),
                    position: Point::new(left + 7., 3.),
                    color: cyan,
                    size: 10.into(),
                    ..Default::default()
                });
                f.fill_rectangle(
                    Point::new(left, 28.),
                    iced::Size::new(126., 44.),
                    Color::from_rgb8(105, 90, 62),
                );
                f.fill_text(canvas::Text {
                    content: "UNCUT STOCK".into(),
                    position: Point::new(left + 21., 34.),
                    color: gold,
                    size: 9.into(),
                    ..Default::default()
                });
                let center = Point::new(left + 63. + sign * (phase - 0.5) * 42., 72.);
                // The strip the cutter has already taken off behind it.
                let start = left + 63. - sign * 21.;
                f.fill_rectangle(
                    Point::new(start.min(center.x), 52.),
                    iced::Size::new((center.x - start).abs(), 20.),
                    Color::from_rgb8(27, 44, 61),
                );
                f.fill(
                    &canvas::Path::circle(center, 20.),
                    Color::from_rgb8(52, 76, 95),
                );
                f.stroke(
                    &canvas::Path::circle(center, 20.),
                    canvas::Stroke::default().with_color(gold).with_width(1.5),
                );
                // Screen Y points down, so increasing angle shows CW in top view.
                for tooth in 0..6 {
                    let a = time * if clockwise { SPIN } else { -SPIN }
                        + tooth as f32 * std::f32::consts::TAU / 6.;
                    f.stroke(
                        &canvas::Path::line(
                            Point::new(center.x + 11. * a.cos(), center.y + 11. * a.sin()),
                            Point::new(center.x + 20. * a.cos(), center.y + 20. * a.sin()),
                        ),
                        canvas::Stroke::default().with_color(gold).with_width(2.),
                    );
                }
                f.fill_text(canvas::Text {
                    content: if clockwise { "CW" } else { "CCW" }.into(),
                    position: Point::new(center.x - 8., center.y - 6.),
                    color: gold,
                    size: 10.into(),
                    ..Default::default()
                });
                let end = Point::new(left + 63. + sign * 36., 111.);
                f.stroke(
                    &canvas::Path::line(Point::new(left + 63. - sign * 36., 111.), end),
                    stroke,
                );
                for dy in [-4., 4.] {
                    f.stroke(
                        &canvas::Path::line(end, Point::new(end.x - sign * 7., end.y + dy)),
                        stroke,
                    );
                }
                f.fill_text(canvas::Text {
                    content: "CUTTER TRAVEL".into(),
                    position: Point::new(left + 22., 119.),
                    color: cyan,
                    size: 9.into(),
                    ..Default::default()
                });
            }
            f.fill_text(canvas::Text {
                content: if matches!(self.diagram, Diagram::MillingChoice(..)) {
                    "TOP VIEW · SELECTED METHOD AND SPINDLE"
                } else {
                    "TOP VIEW · SAME SPINDLE, OPPOSITE FEED"
                }
                .into(),
                position: Point::new(25., 143.),
                color: Color::from_rgb8(185, 207, 223),
                size: 9.into(),
                ..Default::default()
            });
            return vec![f.into_geometry()];
        }
        if crate::illustration::draw(self.diagram, &mut f, time)
            || crate::field_art::draw(self.diagram, &mut f, time)
            || crate::choice_diagram::draw(self.diagram, &mut f, time)
        {
            return vec![f.into_geometry()];
        }
        if matches!(self.diagram, Diagram::Tool) {
            let x = 119.;
            let tip_y = 126.;
            f.fill_rectangle(
                Point::new(x - 21., 12.),
                iced::Size::new(42., 31.),
                Color::from_rgb8(148, 172, 191),
            );
            f.fill_rectangle(
                Point::new(x - 7., 43.),
                iced::Size::new(14., 83.),
                Color::from_rgb8(226, 234, 240),
            );
            f.fill_rectangle(Point::new(x - 7., 85.), iced::Size::new(14., 41.), gold);
            for i in 0..5 {
                let y = 85. + ((i as f32 * 10. + time * CHIP_SPEED) % 41.);
                f.stroke(
                    &canvas::Path::line(Point::new(x - 7., y), Point::new(x + 7., y - 4.)),
                    canvas::Stroke::default()
                        .with_color(Color::from_rgb8(155, 108, 39))
                        .with_width(2.),
                );
            }
            for (x, a, b, label) in [(160., 43., tip_y, "Stickout"), (74., 85., tip_y, "Flutes")] {
                f.stroke(
                    &canvas::Path::line(Point::new(x, a), Point::new(x, b)),
                    canvas::Stroke::default().with_color(cyan).with_width(1.5),
                );
                for y in [a, b] {
                    f.stroke(
                        &canvas::Path::line(Point::new(x - 4., y), Point::new(x + 4., y)),
                        canvas::Stroke::default().with_color(cyan).with_width(1.5),
                    );
                }
                f.fill_text(canvas::Text {
                    content: label.into(),
                    position: Point::new(if x > 100. { x + 8. } else { 13. }, (a + b) / 2. - 6.),
                    color: cyan,
                    size: 11.into(),
                    ..Default::default()
                });
            }
            f.fill_text(canvas::Text {
                content: "HOLDER".into(),
                position: Point::new(158., 20.),
                color: Color::WHITE,
                size: 10.into(),
                ..Default::default()
            });
            // The diameter as a dimension across the flutes' width, with
            // extension lines down from their edges.
            for dx in [-7., 7.] {
                f.stroke(
                    &canvas::Path::line(Point::new(x + dx, 127.), Point::new(x + dx, 138.)),
                    canvas::Stroke::default().with_color(gold).with_width(1.),
                );
                for dy in [-3., 3.] {
                    f.stroke(
                        &canvas::Path::line(
                            Point::new(x + dx, 134.),
                            Point::new(x + dx * 0.55, 134. + dy),
                        ),
                        canvas::Stroke::default().with_color(gold).with_width(1.5),
                    );
                }
            }
            f.stroke(
                &canvas::Path::line(Point::new(x - 7., 134.), Point::new(x + 7., 134.)),
                canvas::Stroke::default().with_color(gold).with_width(1.5),
            );
            f.fill_text(canvas::Text {
                content: "CUTTING DIAMETER".into(),
                position: Point::new(131., 129.),
                color: gold,
                size: 9.into(),
                ..Default::default()
            });
            return vec![f.into_geometry()];
        }
        // What is left for this generic scene: Parallel, Scallop and
        // Rotary finishing, and the heights over a row of holes. The rest
        // is drawn by `illustration` and `choice_diagram` above.
        let s = match self.diagram {
            Diagram::Operation(s) => s,
            _ => Strategy::Drill,
        };
        if s != Strategy::Rotary {
            for (face, c) in [
                (
                    [
                        [-40., -24., 0.],
                        [40., -24., 0.],
                        [40., -24., 24.],
                        [-40., -24., 24.],
                    ],
                    Color::from_rgb8(88, 115, 135),
                ),
                (
                    [
                        [40., -24., 0.],
                        [40., 24., 0.],
                        [40., 24., 24.],
                        [40., -24., 24.],
                    ],
                    Color::from_rgb8(103, 132, 153),
                ),
                (
                    [
                        [-40., 24., 0.],
                        [40., 24., 0.],
                        [40., 24., 24.],
                        [-40., 24., 24.],
                    ],
                    Color::from_rgb8(116, 144, 163),
                ),
                (
                    [
                        [-40., -24., 24.],
                        [40., -24., 24.],
                        [40., 24., 24.],
                        [-40., 24., 24.],
                    ],
                    Color::from_rgb8(146, 170, 186),
                ),
            ] {
                polygon(&mut f, &face, c);
            }
        }
        if s == Strategy::Rotary {
            let spin = std::f32::consts::FRAC_PI_2 - phase * std::f32::consts::TAU * 4.;
            let cyl = |x: f32, a: f32| [x, 18. * a.cos(), 20. + 18. * a.sin()];
            for i in 0..32 {
                let a = i as f32 * std::f32::consts::TAU / 32.;
                let b = (i + 1) as f32 * std::f32::consts::TAU / 32.;
                let shade = 0.54 + 0.10 * (a + spin).sin();
                polygon(
                    &mut f,
                    &[cyl(-40., a), cyl(40., a), cyl(40., b), cyl(-40., b)],
                    Color::from_rgb(shade, shade + 0.10, shade + 0.16),
                );
            }
            let cap: Vec<_> = (0..32)
                .map(|i| cyl(40., i as f32 * std::f32::consts::TAU / 32.))
                .collect();
            polygon(&mut f, &cap, Color::from_rgb8(124, 155, 177));
            line(&mut f, [40., 0., 20.], cyl(40., spin), gold, 2.);
        }
        let surface = |x: f32, y: f32| 26. + 8. * (x / 26.).cos() * (y / 28.).cos();
        if matches!(s, Strategy::Parallel | Strategy::Scallop) {
            // A curved surface makes surface-following visible.
            for yi in 0..8 {
                for xi in 0..12 {
                    let x = -36. + xi as f32 * 6.;
                    let y = -20. + yi as f32 * 5.;
                    let surf = |x: f32, y: f32| [x, y, surface(x, y) - 1.];
                    polygon(
                        &mut f,
                        &[
                            surf(x, y),
                            surf(x + 6., y),
                            surf(x + 6., y + 5.),
                            surf(x, y + 5.),
                        ],
                        Color::from_rgb8(
                            133 + (xi * 2) as u8,
                            161 + (xi * 2) as u8,
                            181 + (xi * 2) as u8,
                        ),
                    );
                }
            }
        }
        let n = 180usize;
        let mut path = vec![];
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let a = t * std::f32::consts::TAU;
            path.push(match s {
                Strategy::Parallel => {
                    let row = (t * 6.).floor().min(5.);
                    let x = if row as i32 % 2 == 0 {
                        -37. + 74. * (t * 6. - row)
                    } else {
                        37. - 74. * (t * 6. - row)
                    };
                    let y = -20. + row * 8.;
                    [x, y, surface(x, y)]
                }
                Strategy::Scallop => {
                    let r = 5. + 21. * t;
                    let (x, y) = (r * (a * 4.).cos(), r * 0.55 * (a * 4.).sin());
                    [x, y, surface(x, y)]
                }
                Strategy::Rotary => {
                    let angle =
                        a * 4. + std::f32::consts::FRAC_PI_2 - phase * std::f32::consts::TAU * 4.;
                    [-32. + 64. * t, 19. * angle.cos(), 20. + 19. * angle.sin()]
                }
                _ => {
                    let hole = (t * 4.).floor().min(3.) as usize;
                    let xy = [[-23., -12.], [23., -12.], [23., 12.], [-23., 12.]][hole];
                    let k = (t * 4.).fract();
                    [xy[0], xy[1], 40. - 28. * (k * std::f32::consts::PI).sin()]
                }
            });
        }
        if matches!(self.diagram, Diagram::Heights) {
            for (z, c) in [(47., gold), (37., cyan)] {
                for (a, b) in [
                    ([-40., -24., z], [40., -24., z]),
                    ([40., -24., z], [40., 24., z]),
                    ([40., 24., z], [-40., 24., z]),
                ] {
                    line(&mut f, a, b, c, 1.);
                }
            }
            // Each plane named at its right-hand corner.
            for (z, c, name) in [(47., gold, "CLEARANCE"), (37., cyan, "RETRACT")] {
                let q = p([40., -24., z]);
                f.fill_text(canvas::Text {
                    content: name.into(),
                    position: Point::new(q.x - 6., q.y - 13.),
                    color: c,
                    size: 9.into(),
                    ..Default::default()
                });
            }
        }
        let end = ((phase * n as f32) as usize).min(n);
        for (i, pair) in path.windows(2).enumerate() {
            line(
                &mut f,
                pair[0],
                pair[1],
                if i <= end {
                    cyan
                } else {
                    Color::from_rgba8(35, 101, 137, 0.38)
                },
                if i <= end { 1.6 } else { 0.8 },
            );
        }
        let tip = path[end];
        let endtool = [tip[0], tip[1], tip[2] + 23.];
        let mid = [tip[0], tip[1], tip[2] + 11.];
        line(&mut f, mid, endtool, Color::from_rgb8(221, 231, 236), 9.);
        line(&mut f, tip, mid, gold, 7.);
        f.fill(&canvas::Path::circle(p(tip), 3.2), gold);
        for z in 0..4 {
            let q = p([tip[0], tip[1], tip[2] + z as f32 * 2.6]);
            f.stroke(
                &canvas::Path::line(
                    Point::new(q.x - 3., q.y + 1.),
                    Point::new(q.x + 3., q.y - 2.),
                ),
                canvas::Stroke::default()
                    .with_color(Color::from_rgb8(142, 100, 40))
                    .with_width(1.),
            );
        }
        let caption = if matches!(self.diagram, Diagram::Heights) {
            "AMBER: CLEARANCE   CYAN: RETRACT"
        } else if s == Strategy::Rotary {
            "ROTARY SWEEP AROUND THE AXIS"
        } else {
            "CYAN: CUTTING MOTION    AMBER: CUTTER"
        };
        f.fill_text(canvas::Text {
            content: caption.into(),
            position: Point::new(8., 143.),
            color: Color::from_rgb8(185, 207, 223),
            size: 9.into(),
            ..Default::default()
        });
        vec![f.into_geometry()]
    }
}

// Renderer smoke preview uses exactly the same cards as hover overlays.
pub fn gallery<'a>(kind: u8) -> Element<'a, Message> {
    let cards: Vec<Help> = if kind >= 5 {
        crate::choice_help::gallery(kind)
    } else if kind == 4 {
        [
            ("pocket_milling", "Pocket milling direction"),
            ("pocket_entry", "Pocket entry"),
            ("spindle_rotation", "Spindle rotation"),
        ]
        .iter()
        .map(|(k, l)| field_help(k, l, Target::Operation))
        .collect()
    } else if kind == 3 {
        [
            ("facing_milling", "Milling direction"),
            ("spindle_rotation", "Spindle rotation"),
            ("facing_pattern", "Pass pattern"),
        ]
        .iter()
        .map(|(k, l)| field_help(k, l, Target::Operation))
        .collect()
    } else if kind == 2 {
        crate::stock::OFFSETS
            .iter()
            .map(|(key, label)| field_help(key, label, Target::Setup))
            .collect()
    } else if kind == 1 {
        [
            (
                "clearance",
                crate::ui::FIXTURE_CLEARANCE_LABEL,
                Target::Setup,
            ),
            ("clearance", "Clearance above stock · mm", Target::Operation),
            ("stepover", "Stepover · % tool diameter", Target::Operation),
            ("stepdown", "Maximum stepdown · mm", Target::Operation),
            ("stickout", "Stickout · mm", Target::Tool),
            ("bottom", "Bottom offset · mm", Target::Operation),
        ]
        .iter()
        .map(|(k, l, target)| field_help(k, l, *target))
        .collect()
    } else {
        [
            Strategy::Face,
            Strategy::Adaptive,
            Strategy::Drill,
            Strategy::Indexed,
            Strategy::Rotary,
            Strategy::FiveAxis,
        ]
        .iter()
        .map(|s| command(&Message::AddOp(*s), s.name().into()))
        .collect()
    };
    let mut rows = widget::Column::new().spacing(10);
    let mut row = widget::Row::new().spacing(10);
    for (i, h) in cards.into_iter().enumerate() {
        // The field gallery's first card as More… and Pause leave it.
        let expanded = kind == 1 && i == 0;
        row = row.push(card(h, expanded, expanded));
        if i % 3 == 2 {
            rows = rows.push(row);
            row = widget::Row::new().spacing(10)
        }
    }
    container(widget::scrollable(rows))
        .padding(20)
        .center(Length::Fill)
        .into()
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Every illustration as a strip of eight moments through its 5 s phase,
    /// written to `$ANIM_DIR` (default /tmp/anim) as `<name>.png`; `ANIM`
    /// limits it to names containing that text.
    #[test]
    #[ignore]
    fn render_animation_frames() {
        use futures_util::FutureExt;
        use iced::advanced::renderer::{self, Headless};
        let dir = std::path::PathBuf::from(
            std::env::var("ANIM_DIR").unwrap_or_else(|_| "/tmp/anim".into()),
        );
        std::fs::create_dir_all(&dir).unwrap();
        let only = std::env::var("ANIM").unwrap_or_default();
        let mut all: Vec<(String, Diagram)> = [
            Strategy::Face,
            Strategy::Adaptive,
            Strategy::Pocket,
            Strategy::Contour,
            Strategy::Drill,
            Strategy::Parallel,
            Strategy::Waterline,
            Strategy::Scallop,
            Strategy::Indexed,
            Strategy::Rotary,
            Strategy::FiveAxis,
            Strategy::Box,
            Strategy::Turn,
        ]
        .into_iter()
        .map(|s| (format!("op-{s:?}"), Diagram::Operation(s)))
        .collect();
        use Diagram::*;
        for (name, d) in [
            ("heights", Heights),
            ("fixture-clearance", FixtureClearance),
            ("offsets", Offsets),
            ("stepover", Stepover),
            ("stepdown", Stepdown),
            ("orientation", Orientation),
            ("ramp", Ramp),
            ("tool", Tool),
            ("allowance", Allowance),
            ("stock-offset-top", StockOffset(2, true)),
            ("stock-offset-left", StockOffset(0, false)),
            ("milling-direction", MillingDirection),
            ("milling-climb-cw", MillingChoice(true, true)),
            ("milling-conventional-cw", MillingChoice(false, true)),
            ("spindle-cw", Spindle(true)),
            ("spindle-ccw", Spindle(false)),
            ("facing-zigzag", FacingPattern(true)),
            ("facing-one-way", FacingPattern(false)),
            ("facing-link-down", FacingLink(true)),
            ("facing-link-up", FacingLink(false)),
            ("entry-ramp", Entry(true)),
            ("entry-plunge", Entry(false)),
            ("drill-simple", DrillCycle(0)),
            ("drill-peck-short", DrillCycle(1)),
            ("drill-peck-full", DrillCycle(2)),
            ("drill-dwell", DrillCycle(3)),
            ("drill-helix", DrillCycle(4)),
            ("drill-thread-mill", DrillCycle(5)),
            ("drill-tap", DrillCycle(6)),
            ("containment-inside", Containment(-1)),
            ("containment-on", Containment(0)),
            ("containment-outside", Containment(1)),
            ("chuck-0-jaws", Chuck(0)),
            ("chuck-1-min_grip", Chuck(1)),
            ("chuck-2-max_grip", Chuck(2)),
            ("chuck-3-jaw_length", Chuck(3)),
            ("chuck-4-body", Chuck(4)),
            ("chuck-5-grip", Chuck(5)),
            ("chuck-6-stickout", Chuck(6)),
            ("clamp-0-length", Clamp(0)),
            ("clamp-1-width", Clamp(1)),
            ("clamp-2-height", Clamp(2)),
            ("clamp-3-bite", Clamp(3)),
            ("clamp-4-clamps", Clamp(4)),
            ("cutting-0-vc_min", Cutting(0)),
            ("cutting-1-fz", Cutting(1)),
            ("lathetool-0-hand", LatheTool(0)),
            ("lathetool-1-nose_radius", LatheTool(1)),
            ("lathetool-2-lead_angle", LatheTool(2)),
            ("lathetool-3-min_bore", LatheTool(3)),
            ("lathetool-4-reach", LatheTool(4)),
            ("lathetool-5-width", LatheTool(5)),
            ("lathetool-6-thread_angle", LatheTool(6)),
            ("lathetool-7-pitches", LatheTool(7)),
            ("linking-0-coolant", Linking(0)),
            ("linking-1-collision", Linking(1)),
            ("linking-2-hole_order", Linking(2)),
            ("linking-3-keep_down", Linking(3)),
            ("linking-4-facing_depth_order", Linking(4)),
            ("linking-5-facing_link", Linking(5)),
            ("linking-6-compensation", Linking(6)),
            ("linking-7-tolerance", Linking(7)),
            ("machine-0-rpm", Machine(0)),
            ("machine-1-power", Machine(1)),
            ("machine-2-feed", Machine(2)),
            ("machine-3-kind", Machine(3)),
            ("machine-7-tool_change", Machine(7)),
            ("manual-0-speeds", Manual(0)),
            ("manual-1-hand_feed", Manual(1)),
            ("manual-10-pitches", Manual(10)),
            ("manual-11-dro_comp", Manual(11)),
            ("manual-12-edge_finder", Manual(12)),
            ("manual-13-pass_time", Manual(13)),
            ("manual-14-tool_time", Manual(14)),
            ("manual-15-move_time", Manual(15)),
            ("manual-2-hand_rapid", Manual(2)),
            ("manual-3-turn_x", Manual(3)),
            ("manual-4-swing", Manual(4)),
            ("manual-5-centres", Manual(5)),
            ("manual-6-cross_reads", Manual(6)),
            ("manual-7-cross_turn", Manual(7)),
            ("manual-8-cross_div", Manual(8)),
            ("manual-9-feeds", Manual(9)),
            ("post-0-post_tool_length", Post(0)),
            ("post-1-post_retract", Post(1)),
            ("post-2-post_blend", Post(2)),
            ("post-3-post_dwell", Post(3)),
            ("post-4-post_air", Post(4)),
            ("probe-0-probe_plate", Probe(0)),
            ("probe-1-pins", Probe(1)),
            ("probe-2-pin_diameter", Probe(2)),
            ("probe-3-pin_height", Probe(3)),
            ("probe-4-probe_tip", Probe(4)),
            ("probe-5-probe_feed", Probe(5)),
            ("probe-6-probe_distance", Probe(6)),
            ("probe-7-probe_retract", Probe(7)),
            ("stock-4-stock_x", Stock(4)),
            ("stock-5-stock_y", Stock(5)),
            ("stock-6-stock_z", Stock(6)),
            ("stock-0-stock_type", Stock(0)),
            ("stock-1-stock_diameter", Stock(1)),
            ("stock-2-stock_length", Stock(2)),
            ("stock-3-stock_radial", Stock(3)),
            ("turnop-0-turn_kind", TurnOp(0)),
            ("turnop-1-face_off", TurnOp(1)),
            ("turnop-10-width", TurnOp(10)),
            ("turnop-11-thread_side", TurnOp(11)),
            ("turnop-12-pitch", TurnOp(12)),
            ("turnop-13-passes", TurnOp(13)),
            ("turnop-2-to_diameter", TurnOp(2)),
            ("turnop-3-diameter", TurnOp(3)),
            ("turnop-4-z_end", TurnOp(4)),
            ("turnop-5-z", TurnOp(5)),
            ("turnop-6-depth", TurnOp(6)),
            ("turnop-7-finish", TurnOp(7)),
            ("turnop-8-edge", TurnOp(8)),
            ("turnop-9-size", TurnOp(9)),
            ("vise-0-jaw_width", Vise(0)),
            ("vise-1-jaw_height", Vise(1)),
            ("vise-2-jaw_thickness", Vise(2)),
            ("vise-3-max_opening", Vise(3)),
            ("vise-4-grip", Vise(4)),
            ("vise-5-force", Vise(5)),
            ("vise-6-vise_axis", Vise(6)),
            ("vise-7-vise_offset", Vise(7)),
            ("vise-8-holding", Vise(8)),
            ("vise-9-holding_gap", Vise(9)),
            ("machine-4-travel_x", Machine(4)),
            ("machine-5-travel_y", Machine(5)),
            ("machine-6-travel_z", Machine(6)),
        ] {
            all.push((name.into(), d));
        }
        // With `ANIM_SEQ=n`, n single frames through the phase instead, as
        // `<name>/000.png`… for an animated preview.
        let sequence: Option<usize> = std::env::var("ANIM_SEQ").ok().and_then(|n| n.parse().ok());
        let mut jobs: Vec<(Element<Message>, iced::Size, std::path::PathBuf)> = vec![];
        for (name, d) in all.into_iter().filter(|(n, _)| n.contains(&only)) {
            if let Some(n) = sequence {
                std::fs::create_dir_all(dir.join(&name)).unwrap();
                for i in 0..n {
                    let t = 5. * i as f32 / n as f32;
                    // `ANIM_MARGIN=m` leaves m px round the canvas, where
                    // anything drawn past its edge shows.
                    let m: f32 = std::env::var("ANIM_MARGIN")
                        .ok()
                        .and_then(|m| m.parse().ok())
                        .unwrap_or(0.);
                    jobs.push((
                        container(
                            canvas(Illustration {
                                diagram: d,
                                still: Some(t),
                            })
                            .width(CARD_WIDTH)
                            .height(155),
                        )
                        .padding(m)
                        .into(),
                        iced::Size::new(CARD_WIDTH + 2. * m, 155. + 2. * m),
                        dir.join(&name).join(format!("{i:03}.png")),
                    ));
                }
                continue;
            }
            let mut grid = widget::Column::new().spacing(6).padding(5);
            for r in 0..2 {
                let mut row = widget::Row::new().spacing(10);
                for c in 0..4 {
                    let k = r * 4 + c;
                    let t = 5. * (k as f32 + 0.5) / 8.;
                    row = row.push(
                        widget::column![
                            canvas(Illustration {
                                diagram: d,
                                still: Some(t)
                            })
                            .width(CARD_WIDTH)
                            .height(155),
                            text(format!("frame {} · t = {t:.2} s", k + 1))
                                .size(11)
                                .color(Color::from_rgb8(236, 242, 249)),
                        ]
                        .spacing(4),
                    );
                }
                grid = grid.push(row);
            }
            jobs.push((
                grid.into(),
                iced::Size::new(4. * 288. + 10., 2. * 182. + 10.),
                dir.join(format!("{name}.png")),
            ));
        }
        for (content, size, path) in jobs {
            let element: Element<Message> = container(content)
                .style(|_| container::Style {
                    background: Some(Color::from_rgb8(27, 44, 61).into()),
                    ..Default::default()
                })
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
            let mut renderer = <iced::Renderer as Headless>::new(
                iced::Font::DEFAULT,
                iced::Pixels(16.),
                Some("tiny-skia"),
            )
            .now_or_never()
            .flatten()
            .expect("tiny-skia renderer");
            let mut ui = iced_runtime::UserInterface::build(
                element,
                size,
                iced_runtime::user_interface::Cache::new(),
                &mut renderer,
            );
            ui.draw(
                &mut renderer,
                &iced::Theme::Dark,
                &renderer::Style {
                    text_color: Color::WHITE,
                },
                mouse::Cursor::Unavailable,
            );
            let scale = 2.;
            let physical =
                iced::Size::new((size.width * scale) as u32, (size.height * scale) as u32);
            let rgba = renderer.screenshot(physical, scale, Color::from_rgb8(27, 44, 61));
            let mut encoder = png::Encoder::new(
                std::io::BufWriter::new(std::fs::File::create(&path).unwrap()),
                physical.width,
                physical.height,
            );
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&rgba)
                .unwrap();
            eprintln!("Rendered {}", path.display());
        }
    }

    /// The most characters a card's hover text may have (issue #89).
    const SHORT_LIMIT: usize = 350;
    /// A card's hover text and its More… text together.
    fn all(h: &Help) -> String {
        format!("{}\n\n{}", h.body, h.more)
    }
    #[test]
    fn account_commands_have_specific_help() {
        let generic = command(&Message::Noop, "x".into()).body;
        for message in [
            Message::SignIn,
            Message::SignInCancel,
            Message::SignOut,
            Message::CheckConnection,
            Message::LoadModels,
            Message::PlanNoticeDismissed,
            Message::OpenUrl(crate::openai::USAGE_URL),
        ] {
            let help = command(&message, "x".into());
            assert_ne!(help.body, generic, "{message:?}");
        }
        assert!(
            command(&Message::SignIn, "x".into())
                .body
                .contains("name, email and profile picture")
        );
        assert!(
            command(&Message::CheckConnection, "x".into())
                .body
                .contains("ChatGPT plan")
        );
        assert_eq!(
            command(&Message::SignOut, "x".into()).body,
            "Sign out of ChatGPT on this computer and ask OpenAI to revoke this sign-in. The saved sign-in is deleted."
        );
    }
    #[test]
    fn material_commands_and_fields_have_specific_help() {
        use crate::Page::Materials;
        let generic = command(&Message::Noop, "x".into()).body;
        for message in [
            Message::NewMaterial,
            Message::EditMaterial("m".into()),
            Message::SaveMaterial,
            Message::DeleteMaterial("m".into()),
            Message::PickMaterial("m".into()),
            Message::Page(Materials),
        ] {
            let body = command(&message, "x".into()).body;
            assert_ne!(body, generic, "{message:?}");
            for claim in ["collision", "gouge", "cutting load", "cutting-load"] {
                assert!(!body.contains(claim), "{message:?}: {body}");
            }
        }
        let pick = command(&Message::PickMaterial("m".into()), "x".into()).body;
        assert!(
            pick.contains(
                "its name, group, hardness, notes and cutting data are sent to the AI planner"
            ),
            "{pick}"
        );
        // What the planner is sent of the project material is said on every
        // field that goes: the name, group, hardness and notes.
        for key in ["name", "group", "hardness", "notes"] {
            let body = field_help(key, "x", Target::Material).body;
            assert!(
                body.contains("project material") && body.contains("sent to the AI planner"),
                "{key}: {body}"
            );
        }
        assert!(
            field_help("name", "x", Target::Material)
                .body
                .contains("up to 60 characters")
        );
        assert!(
            command(&Message::DeleteMaterial("m".into()), "x".into())
                .body
                .contains("project material")
        );
        let unknown = field_help("zz", "x", Target::Material).body;
        assert!(unknown.starts_with("Saved parameter"));
        let keys = ["name", "group", "hardness", "notes", "vc_min", "vc_max"]
            .into_iter()
            .map(String::from)
            .chain(
                ["fz3", "fz6", "fz10", "fz16", "fzbig"]
                    .into_iter()
                    .flat_map(|b| [format!("{b}_min"), format!("{b}_max")]),
            );
        for key in keys {
            let help = field_help(&key, "Minimum", Target::Material);
            assert_ne!(help.body, unknown, "{key}");
            assert!(!help.body.contains("reclamp"), "{key}: {}", help.body);
        }
        // A cutting-data field names itself in full, whatever its label says.
        assert_eq!(
            field_help("fz6_min", "Minimum", Target::Material).title,
            "Chip load up to 6 mm, minimum"
        );
        assert_eq!(
            field_help("vc_max", "Maximum", Target::Material).title,
            "Surface speed, maximum"
        );
        assert_eq!(
            field_help("name", "Material name", Target::Material).title,
            "Material name"
        );
        assert_eq!(
            field_help("fz6_min", "Minimum", Target::Tool).title,
            "Minimum"
        );
    }
    #[test]
    fn workholding_commands_and_fields_have_specific_help() {
        use crate::Page::Workholding;
        let generic = command(&Message::Noop, "x".into()).body;
        for message in [
            Message::NewHolder("Vise"),
            Message::NewHolder("Clamp"),
            Message::NewHolder("Chuck"),
            Message::EditHolder("h".into()),
            Message::SaveHolder,
            Message::DeleteHolder("h".into()),
            Message::SuggestLayout,
            Message::Page(Workholding),
        ] {
            let body = command(&message, "x".into()).body;
            assert_ne!(body, generic, "{message:?}");
        }
        let suggest = command(&Message::SuggestLayout, "x".into());
        assert!(
            suggest.body.contains("middle and quarter points"),
            "{}",
            suggest.body
        );
        assert!(suggest.more.contains("never changed"), "{}", suggest.more);
        assert!(
            suggest.more.contains("parallels, bolts, the table"),
            "{}",
            suggest.more
        );
        let delete = command(&Message::DeleteHolder("h".into()), "x".into()).body;
        assert!(delete.contains("setup uses"), "{delete}");
        let unknown = field_help("zz", "x", Target::Holder).body;
        assert!(unknown.starts_with("Saved parameter"));
        for key in [
            "name",
            "notes",
            "jaw_width",
            "jaw_height",
            "jaw_thickness",
            "max_opening",
            "grip",
            "length",
            "width",
            "height",
            "bite",
            "force",
        ] {
            let body = field_help(key, "x", Target::Holder).body;
            assert_ne!(body, unknown, "{key}");
            // The tool's and the operator's texts of the same keys are not shown.
            for other in ["flutes", "reclamp", "holder diameter"] {
                assert!(!body.contains(other), "{key}: {body}");
            }
        }
        assert!(
            field_help("name", "x", Target::Holder)
                .body
                .contains("up to 60 characters")
        );
        for key in [
            "holding",
            "holder",
            "vise_axis",
            "vise_offset",
            "vise_grip",
            "holding_gap",
            "clamps",
        ] {
            let body = all(&field_help(key, "x", Target::Setup));
            assert_ne!(body, unknown, "{key}");
            // Only the texts that say what the check covers name the cutter.
            if !matches!(key, "holding" | "holding_gap") {
                assert!(!body.contains("cutter"), "{key}: {body}");
            }
        }
        // Only the texts about the setup's workholding and its gap say that
        // toolpaths are checked against it; none promises what the model
        // leaves free.
        for (target, key) in [
            (Target::Setup, "holding"),
            (Target::Setup, "holder"),
            (Target::Setup, "vise_axis"),
            (Target::Setup, "vise_offset"),
            (Target::Setup, "vise_grip"),
            (Target::Setup, "holding_gap"),
            (Target::Setup, "clamps"),
            (Target::Holder, "jaw_width"),
            (Target::Holder, "jaw_height"),
            (Target::Holder, "jaw_thickness"),
            (Target::Holder, "max_opening"),
            (Target::Holder, "grip"),
            (Target::Holder, "length"),
            (Target::Holder, "bite"),
        ] {
            let body = all(&field_help(key, "x", target));
            for claim in ["free to cut", "safe"] {
                assert!(!body.contains(claim), "{key}: {body}");
            }
            if !matches!(key, "holding" | "holding_gap") {
                for claim in ["check", "refus", "toolpath"] {
                    assert!(!body.contains(claim), "{key}: {body}");
                }
            }
        }
        let gap = field_help("holding_gap", "x", Target::Setup);
        assert_eq!(
            gap.body,
            "The clearance gap the checks keep round the modelled jaws or clamps, in mm: 0 or more; left empty it is 2 mm. The operation preview, the export and the Run checks all use it, with the machine profile's path blending tolerance (G64 P) added, since the controller may leave the programmed path by that much at a corner."
        );
        assert_eq!(
            gap.more,
            "A move whose cutter or holder comes within the two is warned of in the operation preview (that operation's own toolpath) and refused at export and by the Run checks (every program move, the links between operations, approaches and retracts included). It is not the Requested fixture clearance under Setup notes, which nothing checks."
        );
        let blend = field_help("post_blend", "x", Target::Machine).more;
        assert!(
            blend.contains("added to the setup's clearance gap") && blend.contains("UC100 run"),
            "{blend}"
        );
        // The blend's dip at a corner and the margin it costs over the stock
        // top (#161), and the dwell a reversal waits (#160).
        assert!(
            blend.contains("keep 2.5 times the tolerance over the stock top")
                && blend.contains("2.41 times the tolerance"),
            "{blend}"
        );
        let dwell = field_help("post_dwell", "x", Target::Machine).body;
        assert!(
            dwell.contains("after the M5 that stops it before it is started the other way")
                && dwell.ends_with(
                    "0 writes no dwell. At a reversal nothing then waits for the spindle to stop."
                ),
            "{dwell}"
        );
        // What the check is, and what it is not, in the field that chooses it.
        let holding = all(&field_help("holding", "x", Target::Setup));
        for part in [
            "every move is checked against them",
            "the cutter from its tip up to its stickout at its cutting diameter",
            "the tool holder above that at the holder diameter",
            "a wider shank or a reduced neck is not modelled",
            "warned of in the operation preview, which covers that operation's own toolpath, and refused at export, which checks every program move, the links between operations, approaches and retracts included",
            "Parallels, bolts, the table and the machine body are not modelled",
            "In a later setup the jaws or clamps sit at the stock box carried over from setup 1, not at the walls earlier operations cut, and a setup turned from setup 1 by other than a multiple of 90° cannot model them",
            "the Fixture note below is only a description",
        ] {
            assert!(holding.contains(part), "{part}: {holding}");
        }
        let offset = field_help("vise_offset", "x", Target::Setup).body;
        assert!(
            offset.contains("must still overlap the stock along their length"),
            "{offset}"
        );
        let clamps = field_help("clamps", "x", Target::Setup).body;
        assert!(
            clamps.contains("no more than half that side's length"),
            "{clamps}"
        );
        // The old fixture description and clearance are records only; the
        // texts that said fixtures are never checked say what is.
        let fixture = all(&field_help("fixture", "x", Target::Setup));
        assert!(
            fixture.contains("is not checked") && fixture.contains("Modelled workholding"),
            "{fixture}"
        );
        let clearance = all(&field_help("clearance", "x", Target::Setup));
        assert!(
            clearance.contains("nothing checks it")
                && clearance
                    .contains("use Checked gap to jaws and clamps under Workholding instead")
                && !clearance.contains("not connected yet"),
            "{clearance}"
        );
        let heights = field_help("clearance", "x", Target::Operation).body;
        assert!(
            heights.contains("export refuses a setup whose plane")
                && heights.contains("every modelled jaw and clamp and its gap"),
            "{heights}"
        );
        let post = command(&Message::Page(crate::Page::Post), "x".into()).more;
        for part in [
            "in a setup that models a vise or clamps, the jaws or clamps as boxes with a clearance gap over every program move, links, approaches and retracts included",
            "parallels, bolts, the table, the machine body and other fixtures are not checked",
        ] {
            assert!(post.contains(part), "{part}: {post}");
        }
        let page = command(&Message::Page(Workholding), "x".into()).body;
        assert!(
            page.contains("checked as boxes with a clearance gap"),
            "{page}"
        );
        assert!(
            field_help("vise_grip", "x", Target::Setup)
                .body
                .contains("the stock above it is not covered by the modelled jaws")
        );
        let clamps = all(&field_help("clamps", "x", Target::Setup));
        for part in ["side:offset", "-X", "+X", "-Y", "+Y", "8"] {
            assert!(clamps.contains(part), "{part}: {clamps}");
        }
        // The tool's holder diameter and flute length keep their own help.
        assert!(
            field_help("holder", "x", Target::Tool)
                .body
                .contains("Maximum holder diameter")
        );
        assert!(
            field_help("length", "x", Target::Tool)
                .body
                .contains("flutes")
        );
    }
    #[test]
    fn speed_and_feed_help_says_what_the_material_check_does() {
        let rpm = field_help("rpm", "Spindle speed", Target::Operation).body;
        let feed = field_help("feed", "Cutting feed", Target::Operation).body;
        for body in [&rpm, &feed] {
            assert!(
                body.contains("project material has cutting data")
                    && body.contains("flagged on this tab and on the NC review page")
                    && body.contains("export still goes ahead"),
                "{body}"
            );
            assert!(!body.contains("recommended speed"), "{body}");
        }
        let rpm_more = field_help("rpm", "Spindle speed", Target::Operation).more;
        assert!(
            rpm_more.contains("π × tool diameter × rpm ÷ 1000"),
            "{rpm_more}"
        );
        assert!(
            feed.contains("Plunge and ramp feeds are not checked"),
            "{feed}"
        );
        let post = command(&Message::Page(crate::Page::Post), "x".into()).body;
        assert!(
            post.contains("outside the project material's cutting data")
                && post.contains("never block export"),
            "{post}"
        );
        let plan = all(&command(&Message::Plan, "x".into()));
        assert!(
            plan.contains("speeds and feeds against the project material's cutting data"),
            "{plan}"
        );
    }
    #[test]
    fn planner_commands_describe_the_live_planner() {
        let generic = command(&Message::Noop, "x".into()).body;
        for message in [
            Message::Plan,
            Message::PlanCancel,
            Message::ApplyPlan,
            Message::DiscardPlan,
            Message::UndoPlan,
            Message::Page(crate::Page::Ai),
        ] {
            let body = command(&message, "x".into()).body;
            assert_ne!(body, generic, "{message:?}");
            for stale in ["not connected", "example plan", "sample housing"] {
                assert!(!body.contains(stale), "{message:?}: {body}");
            }
        }
        let plan = command(&Message::Plan, "x".into());
        assert!(plan.body.contains("Nothing changes until you apply"));
        assert!(plan.more.contains("four requests"));
        assert!(
            command(&Message::ApplyPlan, "x".into())
                .body
                .contains("History")
        );
        let undo = command(&Message::UndoPlan, "x".into());
        assert!(undo.body.contains("saved to History first"));
        assert!(undo.more.contains("Tools, machines"));
        assert!(
            command(&Message::Page(crate::Page::Ai), "x".into())
                .body
                .contains("Sign in with ChatGPT")
        );
    }
    #[test]
    fn a_click_shuts_hover_help_until_the_pointer_comes_back() {
        use futures_util::FutureExt;
        use iced::{Pixels, window};
        let renderer =
            <Renderer as renderer::Headless>::new(Font::DEFAULT, Pixels(16.), Some("tiny-skia"))
                .now_or_never()
                .flatten()
                .expect("tiny-skia renderer");
        // The Face command's illustrated help, as the ribbon wraps it.
        let mut face = wrap(
            widget::button("Face").on_press(Message::Noop),
            command(&Message::AddOp(Strategy::Face), "Face".into()),
            tooltip::Position::Bottom,
        );
        let mut tree = Tree::new(&face);
        let window = Size::new(800., 600.);
        let node = face.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, window),
        );
        let viewport = Rectangle::with_size(window);
        // Sends an event with the pointer at `at`: the messages, and whether the help shows.
        let mut send = |event: Event, at: Point| {
            let mut messages = vec![];
            face.as_widget_mut().update(
                &mut tree,
                &event,
                Layout::new(&node),
                mouse::Cursor::Available(at),
                &renderer,
                &mut iced::advanced::clipboard::Null,
                &mut Shell::new(&mut messages),
                &viewport,
            );
            let shown = face
                .as_widget_mut()
                .overlay(
                    &mut tree,
                    Layout::new(&node),
                    &renderer,
                    &viewport,
                    Vector::ZERO,
                )
                .is_some();
            (messages, shown)
        };
        let moved = |at| Event::Mouse(mouse::Event::CursorMoved { position: at });
        // Moves the pointer to `at` and leaves it there past the help's delay.
        let rest = |send: &mut dyn FnMut(Event, Point) -> (Vec<Message>, bool), at| {
            send(moved(at), at);
            std::thread::sleep(HELP_DELAY + Duration::from_millis(100));
            send(
                Event::Window(window::Event::RedrawRequested(Instant::now())),
                at,
            )
            .1
        };
        let (inside, outside) = (Point::new(5., 5.), Point::new(700., 500.));
        assert!(
            !rest(&mut send, inside),
            "a control that appears under the resting pointer shows no help"
        );
        send(moved(outside), outside);
        assert!(
            rest(&mut send, inside),
            "hovering the control shows its help"
        );
        let left = mouse::Button::Left;
        assert!(
            !send(Event::Mouse(mouse::Event::ButtonPressed(left)), inside).1,
            "a click shuts it"
        );
        let (clicked, shown) = send(Event::Mouse(mouse::Event::ButtonReleased(left)), inside);
        assert!(
            matches!(clicked[..], [Message::Noop]),
            "the click still clicks"
        );
        assert!(!shown);
        assert!(
            !rest(&mut send, Point::new(8., 6.)),
            "it stays shut while the pointer stays"
        );
        send(moved(outside), outside);
        assert!(
            rest(&mut send, inside),
            "it opens again once the pointer comes back"
        );
    }
    #[test]
    fn the_animations_repeat_every_period_so_the_wrapped_clock_never_jumps() {
        use std::f64::consts::TAU;
        // The clock a long-running session has: days, exact in f64.
        let days = 6. * 86_400.;
        for t in [0., 0.25, 3.5, 17.75, 59.5] {
            let (now, later) = (wrapped(days + t), wrapped(days + t + ANIMATION_PERIOD));
            assert_eq!(now, later, "the clock at t and t + 60 s is the same");
            assert!(now < ANIMATION_PERIOD as f32);
            // What the drawings take of it: the 5 s phase, the spindle's
            // angle and the chips' offset.
            let phase = |time: f32| (time / 5.).fract();
            assert_eq!(phase(now), phase(later));
        }
        // Across the wrap the phase, the spindle and the chips continue.
        assert_eq!((ANIMATION_PERIOD / 5.).fract(), 0.);
        let turns = SPIN as f64 * ANIMATION_PERIOD / TAU;
        assert!((turns - turns.round()).abs() < 1e-5, "{turns} turns");
        let travel = CHIP_SPEED as f64 * ANIMATION_PERIOD / 41.;
        assert!((travel - travel.round()).abs() < 1e-5, "{travel} travels");
        // Both speeds stay what they were drawn at, within a percent.
        assert!((SPIN - 2.).abs() < 0.02 && (CHIP_SPEED - 10.).abs() < 0.3);
    }
    #[test]
    fn the_holding_steps_gap_fields_say_what_each_does() {
        use crate::ui::{FIXTURE_CLEARANCE_LABEL, HOLDING_GAP_LABEL, SETUP_NOTES_SECTION};
        // The checked gap is named for what it does, and its card names
        // every check that uses it and the blend they add (issue #72).
        assert_eq!(HOLDING_GAP_LABEL, "Checked gap to jaws and clamps · mm");
        let gap = field_help("holding_gap", HOLDING_GAP_LABEL, Target::Setup);
        assert_eq!(gap.title, HOLDING_GAP_LABEL);
        for part in [
            "The operation preview, the export and the Run checks all use it",
            "path blending tolerance (G64 P) added",
        ] {
            assert!(gap.body.contains(part), "{part}: {}", gap.body);
        }
        for part in [
            "refused at export and by the Run checks",
            "Requested fixture clearance under Setup notes, which nothing checks",
        ] {
            assert!(gap.more.contains(part), "{part}: {}", gap.more);
        }
        // Its picture: the clearance envelope round a jaw.
        assert_eq!(gap.diagram, Some(Diagram::Vise(9)));
        // The requested clearance says in its label that it is not checked,
        // sits under a heading that says the same, and its card says where
        // it is printed and which field the checks use instead.
        assert_eq!(
            FIXTURE_CLEARANCE_LABEL,
            "Requested fixture clearance · mm · not checked"
        );
        assert_eq!(SETUP_NOTES_SECTION, "Setup notes · recorded, not checked");
        let requested = field_help("clearance", FIXTURE_CLEARANCE_LABEL, Target::Setup);
        assert_eq!(requested.title, FIXTURE_CLEARANCE_LABEL);
        for part in [
            "It is a note.",
            "printed on the setup sheet and in the program header",
            "nothing checks it",
        ] {
            assert!(requested.body.contains(part), "{part}: {}", requested.body);
        }
        assert!(
            requested
                .more
                .contains("use Checked gap to jaws and clamps under Workholding instead"),
            "{}",
            requested.more
        );
        assert!(matches!(requested.diagram, Some(Diagram::FixtureClearance)));
        // Neither card calls a field by its old name.
        for body in [all(&gap), all(&requested)] {
            assert!(
                !body.contains("Clearance gap") && !body.contains("Fixture clearance ·"),
                "{body}"
            );
        }
        // The operation's own clearance keeps its name and its card.
        let heights = field_help("clearance", "Clearance above stock · mm", Target::Operation);
        assert_eq!(heights.title, "Clearance above stock · mm");
        assert!(matches!(heights.diagram, Some(Diagram::Heights)));
    }
    /// The six places a field or a choice belongs to.
    const TARGETS: [Target; 6] = [
        Target::Operation,
        Target::Setup,
        Target::Machine,
        Target::Tool,
        Target::Material,
        Target::Holder,
    ];
    /// The string literals in the patterns of the match arms of `function`
    /// in `source`, one list per arm: `(Target::X, "key" | "key2")` gives
    /// the keys after the target; `("key" | "key2", "Value")` the keys, then
    /// the value last. A guard (`if …`) is left out.
    fn arm_literals(source: &str, function: &str) -> Vec<Vec<String>> {
        let body = &source[source.find(function).expect(function)..];
        let body = &body[..body.find("\n}\n").expect("the function's end")];
        body.split("\n        (")
            .skip(1)
            .filter_map(|arm| {
                let pattern = arm.split(" =>").next()?;
                let pattern = pattern.split(" if ").next()?;
                let literals: Vec<String> = pattern
                    .split('"')
                    .skip(1)
                    .step_by(2)
                    .map(String::from)
                    .collect();
                (!literals.is_empty()).then_some(literals)
            })
            .collect()
    }
    /// The keys `field_help` has an arm for.
    fn field_keys() -> Vec<String> {
        let mut keys: Vec<String> = arm_literals(include_str!("help.rs"), "pub fn field_help(")
            .into_iter()
            .flatten()
            .collect();
        // The cutting-data arms match by prefix.
        for band in ["fz3", "fz6", "fz10", "fz16", "fzbig"] {
            keys.push(format!("{band}_min"));
            keys.push(format!("{band}_max"));
        }
        keys.sort();
        keys.dedup();
        keys
    }
    /// The (key, value) pairs `choice_help::selected` has an arm or a
    /// fallback for.
    fn choice_pairs() -> Vec<(String, String)> {
        let mut pairs = vec![];
        for mut literals in arm_literals(include_str!("choice_help.rs"), "pub fn selected(") {
            let value = literals.pop().unwrap();
            for key in literals {
                pairs.push((key, value.clone()));
            }
        }
        let fallbacks: Vec<(&str, Vec<&str>)> = vec![
            ("offset", vec!["G54", "G55", "G56", "G57", "G58", "G59"]),
            ("origin", crate::post::ORIGINS.to_vec()),
            (
                "controller",
                vec![
                    "LinuxCNC",
                    "UC100",
                    "GRBL",
                    "Mach3 / Mach4",
                    "Fanuc",
                    "Haas",
                    "Siemens",
                    "Heidenhain",
                    "Custom",
                ],
            ),
            ("uc100_connection", vec!["USB", "Simulated"]),
            (
                "arrangement",
                vec![
                    "A table around X",
                    "A/C table-table",
                    "A/B head-head",
                    "B/C head-table",
                ],
            ),
            ("speed", vec!["0.5", "1", "120"]),
            ("holding", vec!["None", "Vise", "Clamps", "Chuck"]),
            ("group", crate::material::GROUPS.to_vec()),
        ];
        for (key, values) in fallbacks {
            for value in values {
                pairs.push((key.into(), value.into()));
            }
        }
        pairs.sort();
        pairs.dedup();
        pairs
    }
    /// The settings a choice's text reads besides its own value: none, a
    /// counterclockwise spindle, a one-way pass pattern, both.
    fn field_variants(key: &str) -> Vec<(&'static str, crate::model::Fields)> {
        let mut variants = vec![("", crate::model::Fields::new())];
        if matches!(
            key,
            "facing_milling" | "pocket_milling" | "spindle_rotation"
        ) {
            for (id, spindle, pattern) in [
                ("|ccw", Some("Counterclockwise (CCW)"), None),
                ("|oneway", None, Some("One way")),
                (
                    "|ccw,oneway",
                    Some("Counterclockwise (CCW)"),
                    Some("One way"),
                ),
            ] {
                let mut p = crate::model::Fields::new();
                if let Some(s) = spindle {
                    p.insert("spindle_rotation".into(), s.into());
                }
                if let Some(s) = pattern {
                    p.insert("facing_pattern".into(), s.into());
                }
                variants.push((id, p));
            }
        }
        variants
    }
    /// Every hover card the app makes from its own texts, each with an id:
    /// the commands, the operation buttons, every field of every target and
    /// every choice. The cards worded from the live job (a browser mark's
    /// reasons) are represented by one mark, and the inline cards of `ui`
    /// are not here.
    fn every_card() -> Vec<(String, Help)> {
        use crate::{Page as P, control::Action as Ctl, run::Action as Run, stock_surface::Detail};
        use Message::*;
        let mut commands: Vec<(String, Message)> = vec![
            ("Cancel".into(), Page(P::Machines)),
            (crate::ui::ADD_CLAMP.into(), SetClamps(String::new())),
            ("Remove".into(), SetClamps(String::new())),
        ];
        for tab in [
            "Tool",
            "Geometry",
            "Heights",
            "Orientation",
            "Cycle",
            "Passes",
            "Linking",
            "Turn",
            "Centre",
            "Other",
        ] {
            commands.push((tab.into(), Tab(tab.into())));
        }
        let mut messages = vec![
            NewProject,
            ResumeProject,
            OpenSample,
            RecoverPrevious,
            Page(P::Audit),
            Page(P::Home),
            Page(P::Machines),
            Page(P::Tools),
            Page(P::Materials),
            Page(P::Workholding),
            Page(P::Ai),
            Page(P::Work),
            Page(P::Post),
            Page(P::Wizard),
            Page(P::Control),
            Import,
            OpenHistory,
            OpenProject,
            SaveProject,
            SaveProjectAs,
            RunOnMachine(0),
            NewMachine,
            AddSetup,
            AddManualSetup,
            AddLatheSetup,
            ToolMachine("m".into(), true),
            HolderMachine("m".into(), true),
            StockDetail(Detail::Draft),
            StockDetail(Detail::Standard),
            StockDetail(Detail::Fine),
            StockDetail(Detail::Ultra),
            SetupEditor,
            SetupStep("Machine"),
            SetupStep("Stock"),
            SetupStep("Position"),
            SetupStep("Holding"),
            SetupStep("Work zero"),
            SelectSetup(0),
            SelectOp(0),
            DuplicateOp,
            RemoveOp,
            AskRemoveOp,
            ConfirmDeleteOp(true),
            ConfirmDeleteOp(false),
            MoveOp(-1),
            MoveOp(1),
            PickMode,
            BoxFromFace,
            PickPinHoles,
            PlacePins,
            AddPinDrilling,
            AddThreading,
            ThreadFromPart,
            Generate,
            GenerateAll,
            DuplicateSetup,
            RemoveSetup,
            ConfirmDeleteSetup(true),
            ConfirmDeleteSetup(false),
            MoveSetup(-1),
            MoveSetup(1),
            Flip("rx"),
            Flip("ry"),
            TurnStock(1, 2),
            SquareToStock,
            AssignMachine("m".into()),
            EditMachine("m".into()),
            SaveMachine,
            DeleteMachine("m".into()),
            NewTool,
            EditTool("t".into()),
            SaveTool,
            DeleteTool("t".into()),
            NewMaterial,
            EditMaterial("m".into()),
            SaveMaterial,
            DeleteMaterial("m".into()),
            PickMaterial("m".into()),
            NewHolder("Clamp"),
            NewHolder("Vise"),
            NewHolder("Chuck"),
            EditHolder("h".into()),
            SaveHolder,
            DeleteHolder("h".into()),
            SuggestLayout,
            SetOpTool("t".into()),
            RestSource("r".into()),
            Export("sheet"),
            ExportGcode,
            Camera("ISO"),
            Camera("TOP"),
            Camera("FRONT"),
            Camera("RIGHT"),
            Camera("FIT"),
            ToggleSidebar(crate::settings::Side::Browser),
            ToggleSidebar(crate::settings::Side::Inspector),
            ToggleStrategyHelp,
            Play,
            Scrub(0.),
            ToggleStock,
            ToggleUncut,
            TogglePaths,
            Plan,
            PlanCancel,
            ApplyPlan,
            DiscardPlan,
            UndoPlan,
            SignIn,
            SignInCancel,
            SignOut,
            CheckConnection,
            LoadModels,
            ControlSetup(false),
            ControlSetup(true),
            Setup(crate::setup::Msg::Save),
            Setup(crate::setup::Msg::UseCalibration),
            Setup(crate::setup::Msg::Revert),
            PlanNoticeDismissed,
            OpenUrl(crate::openai::USAGE_URL),
            Feature(0),
            Noop,
        ];
        for step in 0..5 {
            messages.push(WizardStep(step));
        }
        for s in Strategy::ALL {
            messages.push(AddOp(s));
        }
        for a in [
            Ctl::Home,
            Ctl::MachineZero(None),
            Ctl::MachineZero(Some(0)),
            Ctl::KeepPosition,
            Ctl::JogPress(0, false),
            Ctl::JogRelease,
            Ctl::JogStep(None),
            Ctl::JogStep(Some(1.)),
            Ctl::JogSpeed(50.),
            Ctl::WorkZero(None),
            Ctl::WorkZero(Some(0)),
            Ctl::WorkHalf(0),
            Ctl::WorkText(0, String::new()),
            Ctl::WorkSet(0),
            Ctl::ToWorkXY,
            Ctl::Wcs(0),
            Ctl::Rpm(String::new()),
            Ctl::Spindle(uc100::gcode::Spindle::Off),
            Ctl::Spindle(uc100::gcode::Spindle::Cw),
            Ctl::Flood(true),
            Ctl::Mist(true),
            Ctl::FeedOverride(100.),
            Ctl::OverrideLimits(true),
            Ctl::MdiText(String::new()),
            Ctl::MdiSend,
            Ctl::TestOutput(0, Some(true)),
            Ctl::TestOutput(0, None),
            Ctl::CalAxis(0),
            Ctl::CalDistance(String::new()),
            Ctl::CalMeasured(String::new()),
            Ctl::CalMove,
            Ctl::ProbeZ,
            Ctl::ProbeEdges,
            Ctl::ProbeFrom(None),
            Ctl::ProbeFeature(crate::control::Feature::Bore),
            Ctl::TouchOff,
            Ctl::ToSimZero,
        ] {
            messages.push(Machine(a));
        }
        for a in [
            Run::Setup(0),
            Run::Check,
            Run::Next(0),
            Run::CycleStart,
            Run::Hold,
            Run::Resume,
            Run::Stop,
            Run::RecordReference,
            Run::ZFromReference,
            Run::ProbeReference,
            Run::ProbeFromReference,
            Run::CoolantOff,
        ] {
            messages.push(Message::Run(a));
        }
        for a in [
            crate::GuideAction::Next,
            crate::GuideAction::Back,
            crate::GuideAction::Go(0),
            crate::GuideAction::Section(0),
            crate::GuideAction::Reached(0),
            crate::GuideAction::Open(0),
        ] {
            messages.push(Message::Guide(a));
        }
        commands.extend(messages.into_iter().map(|m| ("x".to_string(), m)));
        let mut cards: Vec<(String, Help)> = commands
            .into_iter()
            .map(|(label, m)| (format!("command:{label}:{m:?}"), command(&m, label)))
            .collect();
        for s in Strategy::ALL {
            cards.push((format!("unavailable:{s:?}"), unavailable_help(s)));
            for reason in [
                crate::manual::ONE_AXIS,
                crate::lathe::ONLY_TURN,
                crate::lathe::TURN_ON_MILL,
            ] {
                cards.push((format!("dimmed:{s:?}:{reason}"), dimmed_help(s, reason)));
            }
            let b = HelpButton::operation(
                widget::button("x"),
                s,
                s.name(),
                "Mark ✓: Ready to run. Export's checks of Top pass.".into(),
            );
            cards.push((format!("operation:{s:?}"), b.help));
        }
        for key in field_keys() {
            for target in TARGETS {
                cards.push((
                    format!("field:{key}:{target:?}"),
                    field_help(&key, "x", target),
                ));
            }
        }
        for (key, value) in choice_pairs() {
            for target in TARGETS {
                for (variant, p) in field_variants(&key) {
                    cards.push((
                        format!("choice:{key}:{value}:{target:?}{variant}"),
                        crate::choice_help::selected(&key, "x", &value, target, &p),
                    ));
                }
            }
        }
        cards
    }
    /// The hover text of every card is short (issue #89): under
    /// `SHORT_LIMIT` characters, including the inline cards of `ui`.
    #[test]
    fn every_hover_text_is_under_the_limit() {
        let cards = every_card();
        assert!(cards.len() > 600, "{} cards", cards.len());
        let mut over = vec![];
        for (id, h) in &cards {
            let n = h.body.chars().count();
            if n >= SHORT_LIMIT {
                over.push(format!("{id}: {n} characters"));
            }
        }
        assert!(over.is_empty(), "{}", over.join("\n"));
        let pictures = crate::ui::PICTURES_HELP;
        assert!(pictures.chars().count() < SHORT_LIMIT, "{pictures}");
        // Each hover text says something, and no More… text repeats it.
        for (id, h) in &cards {
            assert!(!h.body.is_empty(), "{id}");
            assert!(h.more.is_empty() || h.more != h.body, "{id}");
        }
    }
    /// No card names a control the app no longer has (issues #194, #36, #88):
    /// the stock source, the Setup view and the old page and field names.
    #[test]
    fn no_card_names_a_control_that_is_gone() {
        let gone = [
            "stock source",
            "Setup view",
            "Program review",
            "Work page",
            "NC program",
            "Clearance gap",
            "Fixture clearance ·",
            "Selection order",
        ];
        let mut found = vec![];
        for (id, h) in every_card() {
            // "LinuxCNC program" is the current name; only a bare "NC program" is stale.
            let text = all(&h).replace("LinuxCNC", "LinuxCnc");
            for name in gone {
                if text.contains(name) {
                    found.push(format!("{id}: {name}"));
                }
            }
        }
        assert!(found.is_empty(), "{}", found.join("\n"));
    }
    /// The card stays open while the pointer is on it, More… expands it,
    /// Pause holds its animation, and both are forgotten when it closes
    /// (issue #89).
    #[test]
    fn the_card_stays_open_under_the_pointer_and_more_expands_it() {
        use futures_util::FutureExt;
        use iced::{Pixels, window};
        let renderer =
            <Renderer as renderer::Headless>::new(Font::DEFAULT, Pixels(16.), Some("tiny-skia"))
                .now_or_never()
                .flatten()
                .expect("tiny-skia renderer");
        // The requested fixture clearance's help: a More… text and an
        // animation, under its control as the Holding step shows it.
        let help = field_help("clearance", "x", Target::Setup);
        assert!(!help.more.is_empty() && help.diagram.is_some());
        let mut field = wrap(
            widget::button("Gap").on_press(Message::Noop),
            help,
            tooltip::Position::Bottom,
        );
        let mut tree = Tree::new(&field);
        let window = Size::new(800., 600.);
        let node = field.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, window),
        );
        let viewport = Rectangle::with_size(window);
        // Sends an event with the pointer at `at`: to the open card first,
        // then to the control, as the runtime does. The card's layout, if
        // it is open afterwards, and the messages.
        let mut send = |event: Event, at: Point| {
            let mut messages = vec![];
            let cursor = mouse::Cursor::Available(at);
            let mut base_cursor = cursor;
            if let Some(mut overlay) = field.as_widget_mut().overlay(
                &mut tree,
                Layout::new(&node),
                &renderer,
                &viewport,
                Vector::ZERO,
            ) {
                let card = overlay.as_overlay_mut().layout(&renderer, window);
                overlay.as_overlay_mut().update(
                    &event,
                    Layout::new(&card),
                    cursor,
                    &renderer,
                    &mut iced::advanced::clipboard::Null,
                    &mut Shell::new(&mut messages),
                );
                if overlay
                    .as_overlay()
                    .mouse_interaction(Layout::new(&card), cursor, &renderer)
                    != mouse::Interaction::None
                {
                    base_cursor = mouse::Cursor::Unavailable;
                }
            }
            field.as_widget_mut().update(
                &mut tree,
                &event,
                Layout::new(&node),
                base_cursor,
                &renderer,
                &mut iced::advanced::clipboard::Null,
                &mut Shell::new(&mut messages),
                &viewport,
            );
            let card = field
                .as_widget_mut()
                .overlay(
                    &mut tree,
                    Layout::new(&node),
                    &renderer,
                    &viewport,
                    Vector::ZERO,
                )
                .map(|mut o| o.as_overlay_mut().layout(&renderer, window));
            (card, messages, *tree.state.downcast_ref::<Hover>())
        };
        let moved = |at| Event::Mouse(mouse::Event::CursorMoved { position: at });
        let redraw = || Event::Window(window::Event::RedrawRequested(Instant::now()));
        let (press, release) = (
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        );
        // The footer's toggles: the children of the last row of the card's
        // column, More… first and Pause second.
        let toggle = |card: &layout::Node, i: usize| -> Point {
            let column = Layout::new(card).children().next().expect("the column");
            let footer = column.children().last().expect("the footer");
            let b = footer.children().nth(i).expect("the toggle").bounds();
            Point::new(b.x + b.width / 2., b.y + b.height / 2.)
        };
        let (inside, outside) = (Point::new(5., 5.), Point::new(700., 500.));
        send(moved(outside), outside);
        send(moved(inside), inside);
        std::thread::sleep(HELP_DELAY + Duration::from_millis(100));
        let (card, messages, _) = send(redraw(), inside);
        let card = card.expect("hovering the control shows its help");
        assert!(
            matches!(messages[..], [Message::HelpHover(true)]),
            "the animation starts: {messages:?}"
        );
        let bounds = card.bounds();
        assert!(
            bounds.y > GAP && bounds.width == CARD_WIDTH + 26.,
            "{bounds:?}"
        );
        // Onto the card, and resting there past the time that shuts a card
        // the pointer has left: it stays.
        let on_card = Point::new(bounds.x + 20., bounds.y + 20.);
        send(moved(on_card), on_card);
        std::thread::sleep(LINGER + Duration::from_millis(100));
        let (still, ..) = send(redraw(), on_card);
        assert!(still.is_some(), "the card stays while the pointer is on it");
        // More… expands it: the card grows, and the app hears nothing.
        let more = toggle(&card, 0);
        send(moved(more), more);
        send(press.clone(), more);
        let (expanded, messages, hover) = send(release.clone(), more);
        let expanded = expanded.expect("More… keeps the card open");
        assert!(messages.is_empty(), "{messages:?}");
        assert!(hover.expanded && !hover.paused);
        let grown = expanded.bounds();
        assert!(
            grown.height > bounds.height && grown.width == EXPANDED_WIDTH + 26.,
            "{bounds:?} → {grown:?}"
        );
        // Pause holds the animation, and the app stops its ticks.
        let pause = toggle(&expanded, 1);
        send(moved(pause), pause);
        send(press.clone(), pause);
        let (paused, messages, hover) = send(release.clone(), pause);
        assert!(paused.is_some());
        assert!(
            matches!(messages[..], [Message::HelpHover(false)]),
            "{messages:?}"
        );
        assert!(hover.expanded && hover.paused);
        // Off the card and the control: the card closes after the linger,
        // and forgets More… and Pause.
        send(moved(outside), outside);
        let (lingering, ..) = send(redraw(), outside);
        assert!(lingering.is_some(), "the card lingers for the way back");
        std::thread::sleep(LINGER + Duration::from_millis(100));
        let (closed, _, hover) = send(redraw(), outside);
        assert!(closed.is_none(), "the card closes once the pointer is gone");
        assert!(!hover.expanded && !hover.paused);
        send(moved(inside), inside);
        std::thread::sleep(HELP_DELAY + Duration::from_millis(100));
        let (again, ..) = send(redraw(), inside);
        assert_eq!(
            again.expect("it opens again").bounds().size(),
            bounds.size(),
            "opened again, the card is the hover text alone"
        );
    }
    #[test]
    fn the_card_takes_the_side_with_room_and_stays_in_the_window() {
        use tooltip::Position;
        let window = Size::new(1120., 720.);
        let card = Size::new(330., 300.);
        // A browser row low in the window: no room below, so the card goes
        // above it, centred on it.
        let low = Rectangle::new(Point::new(400., 650.), Size::new(120., 24.));
        let at = place(low, card, Position::Bottom, window);
        assert_eq!(at, Point::new(295., 650. - 300. - GAP));
        // The same row with room below keeps the preferred side.
        let mid = Rectangle::new(Point::new(400., 300.), Size::new(120., 24.));
        assert_eq!(
            place(mid, card, Position::Bottom, window),
            Point::new(295., 324. + GAP)
        );
        // A wizard field near the left edge: no room to its left, so the
        // card goes to its right.
        let edge = Rectangle::new(Point::new(60., 300.), Size::new(200., 24.));
        let at = place(edge, card, Position::Left, window);
        assert_eq!(at, Point::new(260. + GAP, 300. + (24. - 300.) / 2.));
        // Top and Right flip the same way.
        let high = Rectangle::new(Point::new(400., 20.), Size::new(120., 24.));
        assert_eq!(place(high, card, Position::Top, window).y, 44. + GAP);
        let right = Rectangle::new(Point::new(900., 300.), Size::new(200., 24.));
        assert_eq!(
            place(right, card, Position::Right, window).x,
            900. - 330. - GAP
        );
        // No room on either side: the card is kept in the window, over
        // the control, and the control stays live (the test below).
        let short = Size::new(1120., 400.);
        let at = place(mid, card, Position::Bottom, short);
        assert_eq!(at, Point::new(295., 100.));
        assert!(Rectangle::new(at, card).intersects(&mid));
    }
    #[test]
    fn a_control_under_a_clamped_card_can_still_be_clicked_to_close_it() {
        use futures_util::FutureExt;
        use iced::{Pixels, window};
        let renderer =
            <Renderer as renderer::Headless>::new(Font::DEFAULT, Pixels(16.), Some("tiny-skia"))
                .now_or_never()
                .flatten()
                .expect("tiny-skia renderer");
        // An animated card, about 300 px tall, under a control in the middle
        // of a window too short for it on either side.
        let help = field_help("clearance", "x", Target::Setup);
        assert!(help.diagram.is_some());
        let mut field = wrap(
            widget::button("Gap").on_press(Message::Noop),
            help,
            tooltip::Position::Bottom,
        );
        let mut tree = Tree::new(&field);
        let window = Size::new(800., 420.);
        let node = field
            .as_widget_mut()
            .layout(
                &mut tree,
                &renderer,
                &layout::Limits::new(Size::ZERO, window),
            )
            .move_to(Point::new(0., 200.));
        let control = node.bounds();
        let viewport = Rectangle::with_size(window);
        // Sends an event with the pointer at `at`, to the open card first and
        // then to the control with the cursor the runtime gives it: the card's
        // layout if it is open afterwards, the messages, and whether the
        // control had the pointer.
        let mut send = |event: Event, at: Point| {
            let mut messages = vec![];
            let cursor = mouse::Cursor::Available(at);
            let mut base_cursor = cursor;
            if let Some(mut overlay) = field.as_widget_mut().overlay(
                &mut tree,
                Layout::new(&node),
                &renderer,
                &viewport,
                Vector::ZERO,
            ) {
                let card = overlay.as_overlay_mut().layout(&renderer, window);
                overlay.as_overlay_mut().update(
                    &event,
                    Layout::new(&card),
                    cursor,
                    &renderer,
                    &mut iced::advanced::clipboard::Null,
                    &mut Shell::new(&mut messages),
                );
                if overlay
                    .as_overlay()
                    .mouse_interaction(Layout::new(&card), cursor, &renderer)
                    != mouse::Interaction::None
                {
                    base_cursor = mouse::Cursor::Unavailable;
                }
            }
            field.as_widget_mut().update(
                &mut tree,
                &event,
                Layout::new(&node),
                base_cursor,
                &renderer,
                &mut iced::advanced::clipboard::Null,
                &mut Shell::new(&mut messages),
                &viewport,
            );
            let card = field
                .as_widget_mut()
                .overlay(
                    &mut tree,
                    Layout::new(&node),
                    &renderer,
                    &viewport,
                    Vector::ZERO,
                )
                .map(|mut o| o.as_overlay_mut().layout(&renderer, window));
            (card, messages, base_cursor != mouse::Cursor::Unavailable)
        };
        let moved = |at| Event::Mouse(mouse::Event::CursorMoved { position: at });
        let redraw = || Event::Window(window::Event::RedrawRequested(Instant::now()));
        let inside = Point::new(control.x + 5., control.y + 5.);
        let outside = Point::new(700., 400.);
        send(moved(outside), outside);
        send(moved(inside), inside);
        std::thread::sleep(HELP_DELAY + Duration::from_millis(100));
        let (card, _, live) = send(redraw(), inside);
        let card = card.expect("hovering the control shows its help").bounds();
        assert!(
            card.intersects(&control) && card.y >= 0. && card.y + card.height <= window.height,
            "the card fits on neither side, so it is kept in the window over the control: {card:?} over {control:?}"
        );
        assert!(
            live,
            "the pointer on the control under the card stays the control's"
        );
        // Pressing the control shuts the card, and the release still clicks.
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let (shut, ..) = send(press, inside);
        assert!(shut.is_none(), "the press shuts the card");
        let (_, messages, _) = send(release, inside);
        assert!(
            matches!(messages[..], [Message::Noop]),
            "the click reaches the control: {messages:?}"
        );
    }
}
