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
                    canvas(Illustration { diagram: d, paused })
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
            "Save a portable JSON project containing your settings. Keep the original model file available: the project stores its path, not embedded geometry. Shortcut: ⌘S.",
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
            "The status says how many operations still come within the clearance gap. The setup's operations are generated for this and never changed. Jaws and clamps are checked as boxes with the gap; parallels, bolts, the table and the machine body are not. Choosing Vise or Clamps, or another library entry, does this too; other edits are kept until you press this again.",
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
            "",
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
        ToggleStock => (
            "Show or hide stock. During Facing or Pocket simulation this displays the remaining sampled stock; hide Stock to see the finished reference model. Outside simulation it shows the transparent stock box.",
            "Relative stock extends from each side of the placed part by its offset; fixed stock uses explicit dimensions. The box and part share the same display scale.",
        ),
        ToggleUncut => (
            "Show or hide the faces of the part that the stock still covers after every setup, in magenta. With the simulated stock shown, just those faces are drawn over it, where the stock surface does not hide them. A selected one keeps its magenta and gets a blue outline. Off, the part is drawn as it is without them.",
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
                        "Choose how the drill feeds, pecks, pauses and retracts between holes."
                    }
                    "Passes" => {
                        "Control cutting depth, spacing, finishing allowance and tolerance."
                    }
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
        Flip(_) | AddSetup | DuplicateSetup => h.animated(Diagram::Orientation),
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
            "Open this setup in MACHINE: a setup on the manual mill shows its steps, with the DRO numbers to move to, whether or not the CNC is connected.",
            "",
        ),
        Beat(_) | Built(..) => ("Show the steps of this manual setup.", ""),
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
            None,
        ),
        (Target::Machine, "power") => (
            "The spindle's rated power, in watts. The check of whether a setup's vise or clamps hold the stock against each operation's cut takes no more force than this power drives at the spindle's top speed (the spindle keeps its torque below that speed). Empty: 800 W, a Jingyan CNC4030's spindle.",
            "",
            None,
        ),
        (Target::Machine, "feed") => (
            "Maximum feed rate in the machine profile's units per minute. Use the manufacturer's specification; operation feeds are in mm/min.",
            "",
            None,
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
            None,
        ),
        (Target::Material, "vc_max") => (
            "Highest recommended surface speed for carbide tools, in m/min. Surface speed = π × tool diameter × spindle rpm ÷ 1000. A faster operation is flagged.",
            "",
            None,
        ),
        (Target::Material, k) if k.starts_with("fz") && k.ends_with("_min") => (
            "Lowest recommended chip load per tooth, in mm, for tools in this diameter range. Chip load = feed ÷ (rpm × flutes). A lower value is flagged, unless the machine's top feed is the limit.",
            "",
            None,
        ),
        (Target::Material, k) if k.starts_with("fz") && k.ends_with("_max") => (
            "Highest recommended chip load per tooth, in mm, for tools in this diameter range. Chip load = feed ÷ (rpm × flutes). A higher value is flagged.",
            "",
            None,
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
            None,
        ),
        (Target::Holder, "jaw_height") => (
            "Height of the jaws above the parallels' top, in mm. Kept for reference: the modelled jaws reach from far below the stock up to the setup's grip height.",
            "",
            None,
        ),
        (Target::Holder, "jaw_thickness") => (
            "How thick each jaw is, measured outward from the stock, in mm.",
            "",
            None,
        ),
        (Target::Holder, "max_opening") => (
            "The widest stock the jaws open to, in mm. A setup whose stock is wider across the jaws shows an issue.",
            "",
            None,
        ),
        (Target::Holder, "grip") => (
            "How much of the stock the jaws hold, as a height above the stock's bottom, in mm. A setup starts from it and can change its own grip.",
            "",
            None,
        ),
        (Target::Holder, "length") => (
            "Length of the clamp from its tip outward, in mm; the bite is part of it. The rest of it reaches beyond the stock's edge.",
            "",
            None,
        ),
        (Target::Holder, "width") => (
            "Width of the clamp, in mm, along the side of the stock it sits on.",
            "",
            None,
        ),
        (Target::Holder, "height") => (
            "Height of the clamp above the stock's top, in mm.",
            "",
            None,
        ),
        (Target::Holder, "force") => (
            "How hard the vise clamps the stock, or one clamp holds it down, in newtons. The check of each operation's cutting forces takes it as what the jaws' friction and the clamps' hold work with: measure or estimate yours, and raise it if yours is tightened harder. Empty: 5000 N for a vise, 2000 N for a clamp, typical when tightened by hand.",
            "",
            None,
        ),
        (Target::Holder, "bite") => (
            "How far the clamp reaches onto the stock from its edge, in mm. It must be shorter than the clamp's length.",
            "",
            None,
        ),
        (Target::Setup, "holding") => (
            "The workholding to model for this setup: a Vise from the library, with its jaws on two opposite sides of the stock, or Clamps placed around it. Each is kept as boxes with a clearance gap that every move is checked against, and its hold is checked against each operation's cutting force. None models nothing.",
            "Each is kept as boxes with a clearance gap, and every move is checked against them: the cutter from its tip up to its stickout at its cutting diameter, the tool holder above that at the holder diameter (a wider shank or a reduced neck is not modelled). A move that comes within the gap is warned of in the operation preview, which covers that operation's own toolpath, and refused at export, which checks every program move, the links between operations, approaches and retracts included. Parallels, bolts, the table and the machine body are not modelled. Each operation's cutting force is also set against what the vise's or clamps' clamping force holds (sliding, lifting and tipping), over the contact the earlier setups left the jaws; an operation held with a margin under ×2 is refused at export. In a later setup the jaws or clamps sit at the stock box carried over from setup 1, not at the walls earlier operations cut, and a setup turned from setup 1 by other than a multiple of 90° cannot model them. None models nothing; the Fixture note below is only a description.",
            None,
        ),
        (Target::Setup, "holder") => (
            "The library entry used: a vise for Vise, a clamp for Clamps. Entries are made on the Workholding page; changing one changes every setup that uses it.",
            "",
            None,
        ),
        (Target::Setup, "vise_axis") => (
            "Which sides of the stock the jaws face: X sides put one jaw on the −X side and one on the +X side, Y sides on −Y and +Y. A stock wider across the jaws than the vise's maximum opening shows an issue.",
            "",
            None,
        ),
        (Target::Setup, "vise_offset") => (
            "Shifts the jaws along their length from the middle of the stock, in mm: toward +Y for jaws on the X sides, toward +X for jaws on the Y sides. 0 centres them. The jaws must still overlap the stock along their length: an offset of half the stock's length plus half the jaw width or more shows an issue.",
            "",
            None,
        ),
        (Target::Setup, "vise_grip") => (
            "How much of the stock the jaws hold, as a height above the stock's bottom, in mm. The jaws are modelled from far below the stock up to it; the stock above it is not covered by the modelled jaws. It starts at the vise's own grip.",
            "",
            None,
        ),
        (Target::Setup, "holding_gap") => (
            "The clearance gap the checks keep round the modelled jaws or clamps, in mm: 0 or more; left empty it is 2 mm. The operation preview, the export and the Run checks all use it, with the machine profile's path blending tolerance (G64 P) added, since the controller may leave the programmed path by that much at a corner.",
            "A move whose cutter or holder comes within the two is warned of in the operation preview (that operation's own toolpath) and refused at export and by the Run checks (every program move, the links between operations, approaches and retracts included). It is not the Requested fixture clearance under Setup notes, which nothing checks.",
            None,
        ),
        (Target::Setup, "clamps") => (
            "Where the clamps sit, a row each: the side of the stock, -X, +X, -Y or +Y, and the offset, in mm, that moves the clamp along that side from its middle (toward +Y on the X sides, toward +X on the Y sides), no more than half that side's length. Place 1 to 8 clamps with + Add clamp and Remove.",
            "The positions are saved only when every offset is a number; what is wrong is shown under the rows. The setup keeps them as side:offset separated by semicolons, for example -X:0;+X:20.",
            None,
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
            "",
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
            None,
        ),
        (_, "hole_diameter") => (
            "Filter detected circular openings by nominal diameter in mm (±0.1 mm). Zero includes all detected holes on the selected rim. The drill must fit every included opening. Select the horizontal rim face first.",
            "",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "boundary") => (
            "Defines the region that should contain machining: selected geometry, stock outline or model outline. Facing uses rectangular extents. Pocket follows the selected floor. Contour and finishing use the imported model silhouette.",
            "",
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
            "Additional travel past the exit, in mm: of a hole that goes through, for a drill (never under a blind hole), or of a Pocket's openings cut through, below where their walls end. The real required distance also depends on drill-point geometry.",
            "",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "openings") => (
            "Holes in the selected floors with nothing of the part over or under them, such as switch cut-outs in a plate: Leave clears over them at the floor's height, Cut through also cuts each down to where its walls end, plus Breakthrough.",
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
            None,
        ),
        (_, "cycle") => (
            "Drilling feeds once to depth. Chip breaking retracts slightly between pecks; deep drilling retracts clear of the hole. Counterboring adds a bottom dwell.",
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
            None,
        ),
        (_, "ramp") => (
            "How the cutter enters material: a helix, back-and-forth ramp, straight plunge, or profile ramp. Match the entry to the cutter's cutting capabilities.",
            "",
            Some(Ramp),
        ),
        (_, "ramp_angle") => (
            "Slope of the entry move in degrees relative to the horizontal plane. A smaller angle takes a longer distance to reach cutting depth.",
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
            None,
        ),
        (_, "facing_pattern") => (
            "Both ways alternates travel on neighboring rows, mixing climb and conventional cutting without reversing the spindle. One way enables the Climb / Conventional choice and retracts for return moves. The pass-angle field rotates either pattern in setup XY.",
            "",
            Some(Operation(Strategy::Face)),
        ),
        (_, "facing_depth_order") => (
            "Continue from previous end starts the next depth at the last pass's X/Y, with the row order and travel directions reversed. Return to first start repeats each depth from the original X/Y. Applies to Both ways; One way always returns.",
            "",
            None,
        ),
        (_, "facing_link") => (
            "Keep tool down feeds sideways between adjacent rows, then cuts back across the stock. Continue from previous end also feeds vertically down at the last X/Y for the next depth. Retract each pass lifts to clearance between rows.",
            "Applies to Both ways and the rectangular Facing preview; fixtures are not avoided, and a move within the clearance gap of a modelled vise jaw or clamp is refused at export.",
            None,
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
            "Target radial engagement for adaptive clearing, in mm. It controls how much of the cutter's side is engaged, rather than its full diameter.",
            "",
            Some(Operation(Strategy::Adaptive)),
        ),
        (_, "minimum_radius") => (
            "Smallest allowed radius of the cutting path, in mm. Larger radii can soften abrupt direction changes, where the geometry allows.",
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
            None,
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
            "Requested maximum toolpath approximation deviation, in mm. A smaller value can require more path segments. It is not a guarantee of finished-part accuracy.",
            "",
            None,
        ),
        (_, "rest") => (
            "Target material remaining from an earlier operation, often with a smaller tool. Select the previous operation as the source; remaining-stock calculation is not connected.",
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
            None,
        ),
        (_, "post_retract") => (
            "Where the tool goes before a tool change and at the end: machine Z0 with G53, or the setup's highest clearance height.",
            "",
            None,
        ),
        (_, "post_blend") => (
            "Path blending tolerance for LinuxCNC's trajectory planner. A positive value writes G64 P with this tolerance in mm, so corners may round by up to that much at speed. 0 writes G61 exact path.",
            "The checks allow for it: it is added to the setup's clearance gap around the jaws and clamps and to the cutter's radius when a rapid descent is proven clear of the stock, also for a UC100 run. At a corner the blend can leave the vertical by 2.41 times the tolerance below the height a rapid crosses at, so the clearance plane and every sideways rapid keep 2.5 times the tolerance over the stock top (export and the Run checks refuse a rapid closer than that) and the link between two operations of one tool is raised to that height.",
            None,
        ),
        (_, "post_dwell") => (
            "Seconds to wait after starting or changing the spindle before the first move, and, between two operations of one tool that turn it opposite ways, after the M5 that stops it before it is started the other way. Use it when the spindle has no at-speed feedback. 0 writes no dwell. At a reversal nothing then waits for the spindle to stop.",
            "",
            None,
        ),
        (_, "post_air") => (
            "LinuxCNC output used for operations set to Air: M7 (mist) or M8 (flood), whichever is wired to the air blast.",
            "",
            None,
        ),
        (_, "probe_plate") => (
            "Thickness of the touch plate, in mm: measure it. Probe Z makes work Z at the plate's top this value, so the surface under the plate is Z 0. Used when the project's material is not a metal; on a metal the plate's lead goes on the stock and the tool touches it. Leave it empty without a plate.",
            "",
            None,
        ),
        (_, "pins") => (
            "The fixture plate's dowel pins, as drilled, in the plate's own X Y (mm), pin 1 first: \"0 0; 120 0\" for two pins 120 mm apart along X. Pin 1 is the reference pin, pin 2 gives the turn. Leave it empty without pins.",
            "The plate is taken as square to the machine's X and Y: Record the pins' place on the Control page measures how far off it is. A setup that sits on the pins gets its X and Y work offset from pin 1's recorded place.",
            None,
        ),
        (_, "pin_diameter") => (
            "The pins' diameter, in mm: measure it. The holes a part drops over them by must be this to 0.2 mm over: a wider hole leaves the part free to move. The checks keep tools clear of a pin-sized box per pin.",
            "",
            None,
        ),
        (_, "pin_height") => (
            "How far the pins stand above the plate, in mm: measure it. The part's holes for them must be at least this deep, and Pin centre probes the pins' sides min(3 mm, half of it) below their top.",
            "",
            None,
        ),
        (_, "probe_tip") => (
            "Diameter of the probe's ball or pin, in mm: measure it, or the tool's diameter for a tool with the plate's lead on conductive stock. Probe corner and Probe centre move each side they touch by half of it. Leave it empty without a probe.",
            "",
            None,
        ),
        (_, "probe_feed") => (
            "How fast Z feeds down to the plate, in mm/min. Slow is accurate: the machine stops a little past the contact, and AXIS CAM uses where the contact was.",
            "",
            None,
        ),
        (_, "probe_distance") => (
            "How far down a probe goes at most, in mm, and never below the bottom of Z. Without contact by then it stops and nothing is set.",
            "",
            None,
        ),
        (_, "probe_retract") => (
            "How far the tool lifts after the contact, in mm, so the plate can come away. 0 leaves it on the plate.",
            "",
            None,
        ),
        (Target::Machine, "kind") => (
            "CNC runs G-code. Manual is a hand-operated mill with a DRO, Lathe a hand-operated lathe with dials: no G-code, no Run, and their setups are shown as steps on the MACHINE page.",
            "",
            None,
        ),
        (_, "speeds") => (
            "The spindle speeds the gearbox has, in rpm, comma-separated. A range such as 100-2000 means a variable drive. The rpm of a manual operation must be one of them.",
            "",
            None,
        ),
        (_, "hand_feed") => (
            "The fastest feed you can turn steadily by hand, in mm/min. No manual operation feeds faster.",
            "",
            None,
        ),
        (_, "hand_rapid") => (
            "How fast you move an axis off the cut by hand, in mm/min. It sets the time of the moves between cuts.",
            "",
            None,
        ),
        (_, "turn_x" | "turn_y" | "turn_z") => (
            "How far the axis moves in one handwheel turn, in mm. The steps use it to say how many turns a feed is.",
            "",
            None,
        ),
        (_, "swing") => (
            "The largest diameter the lathe turns over its bed, in mm. A bar or part wider than this is refused.",
            "",
            None,
        ),
        (_, "centres") => (
            "The distance between the centres, in mm: the longest work the lathe takes between chuck and tailstock.",
            "",
            None,
        ),
        (_, "cross_reads") => (
            "Whether a reading on the cross-slide dial takes that much off the diameter or off the radius. Not known yet: the first lathe setup asks you to cut a test and measure, and saves the answer.",
            "",
            None,
        ),
        (_, "cross_turn" | "top_turn" | "carriage_turn" | "quill_turn") => (
            "The numbers the dial goes through in one full turn of the handwheel: what it reads at the index after one turn. The steps use it to say how many turns and what reading.",
            "",
            None,
        ),
        (_, "cross_div" | "top_div" | "carriage_div" | "quill_div") => (
            "How many divisions the dial has round its collar, a whole number. A reading is rounded to one division.",
            "",
            None,
        ),
        (_, "feeds") => (
            "The carriage power feeds the gearbox has, in mm per revolution, comma-separated. Turning passes use the one nearest 0.1.",
            "",
            None,
        ),
        (_, "pitches") => (
            "The thread pitches the gearbox cuts, in mm, comma-separated. A thread operation must use one of them.",
            "",
            None,
        ),
        (_, "dro_comp") => (
            "Yes when the DRO has tool-radius compensation, so it reads the tool's edge instead of its centre.",
            "",
            None,
        ),
        (_, "edge_finder") => (
            "The diameter of the edge finder that picks up the work zero, in mm. The Zero step takes its radius off each face it touches.",
            "",
            None,
        ),
        (_, "pass_time") => (
            "Seconds to reset, set and lock the axes for one pass. It adds to the setup's minutes.",
            "",
            None,
        ),
        (_, "tool_time") => (
            "Minutes to change a tool and pick up the zero again. It adds to the setup's minutes.",
            "",
            None,
        ),
        (_, "move_time") => (
            "Minutes to move the stock between this machine and another. The plan adds it each time the stock changes machine.",
            "",
            None,
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
            "Relative size box adds independent offsets to the six sides of the part's bounding box after setup rotation. Fixed size box uses explicit X, Y and Z dimensions. Values for each mode are kept when switching.",
            "",
            None,
        ),
        (_, "stock_x" | "stock_y" | "stock_z") => (
            "Overall stock box extent along this setup axis, in mm. Fixed stock is centered on X and Y with its bottom on Z=0 in the preview. Use relative stock for extra material measured from each part side.",
            "",
            None,
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
            None,
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
            None,
        ),
        (_, "tcp") => (
            "Tool center point control lets the controller compensate linear axes while rotating, to maintain the tool tip's position. Requires supported kinematics and a matching postprocessor.",
            "",
            Some(Operation(Strategy::FiveAxis)),
        ),
        (_, "type") => (
            "Cutter geometry: flat end mill, ball end mill, drill or face mill. Choose a type suited to the operation.",
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
            "A chamfer mill's included point angle in degrees, 90 when left empty. Match a countersink's angle: an 82° countersink needs an 82° cutter.",
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
    paused: bool,
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
        let time = if self.paused {
            STILL_TIME
        } else {
            animation_time()
        };
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
        if crate::choice_diagram::draw(self.diagram, &mut f, time) {
            return vec![f.into_geometry()];
        }
        if let Diagram::StockOffset(axis, positive) = self.diagram {
            let vertices = |min: [f32; 3], max: [f32; 3]| {
                let [x, y, z] = min;
                let [xx, yy, zz] = max;
                [
                    [x, y, z],
                    [xx, y, z],
                    [xx, yy, z],
                    [x, yy, z],
                    [x, y, zz],
                    [xx, y, zz],
                    [xx, yy, zz],
                    [x, yy, zz],
                ]
            };
            let part_min = [-34., -22., 0.];
            let part_max = [34., 22., 24.];
            let part = vertices(part_min, part_max);
            for (indices, color) in [
                ([0, 1, 5, 4], Color::from_rgb8(88, 115, 135)),
                ([1, 2, 6, 5], Color::from_rgb8(116, 144, 163)),
                ([4, 5, 6, 7], Color::from_rgb8(146, 172, 189)),
            ] {
                polygon(&mut f, &indices.map(|i| part[i]), color);
            }
            let offset = 4. + 9. * (0.5 - 0.5 * (phase * std::f32::consts::TAU).cos());
            let mut min = part_min;
            let mut max = part_max;
            if positive {
                max[axis] += offset;
            } else {
                min[axis] -= offset;
            }
            let stock = vertices(min, max);
            for (a, b) in [
                (0, 1),
                (1, 2),
                (2, 3),
                (3, 0),
                (4, 5),
                (5, 6),
                (6, 7),
                (7, 4),
                (0, 4),
                (1, 5),
                (2, 6),
                (3, 7),
            ] {
                line(&mut f, stock[a], stock[b], gold, 1.2);
            }
            let mut from = [0., 0., 12.];
            from[axis] = if positive {
                part_max[axis]
            } else {
                part_min[axis]
            };
            let mut to = from;
            to[axis] += if positive { offset } else { -offset };
            line(&mut f, from, to, cyan, 3.);
            for v in [from, to] {
                f.fill(&canvas::Path::circle(p(v), 2.4), cyan);
            }
            f.fill_text(canvas::Text {
                content: "EXTRA MATERIAL ON ONE SIDE".into(),
                position: Point::new(8., 7.),
                color: gold,
                size: 10.into(),
                ..Default::default()
            });
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
            f.stroke(
                &canvas::Path::line(Point::new(x - 7., 134.), Point::new(x + 7., 134.)),
                canvas::Stroke::default().with_color(gold).with_width(2.),
            );
            f.fill_text(canvas::Text {
                content: "CUTTING DIAMETER".into(),
                position: Point::new(139., 130.),
                color: gold,
                size: 9.into(),
                ..Default::default()
            });
            return vec![f.into_geometry()];
        }
        let s = match self.diagram {
            Diagram::Operation(s) => s,
            Diagram::Stepover => Strategy::Face,
            Diagram::Stepdown | Diagram::Allowance | Diagram::Ramp => Strategy::Pocket,
            Diagram::Orientation => Strategy::Indexed,
            Diagram::Heights | Diagram::Offsets => Strategy::Drill,
            Diagram::Tool
            | Diagram::StockOffset(_, _)
            | Diagram::MillingDirection
            | Diagram::MillingChoice(..)
            | Diagram::Spindle(_)
            | Diagram::FacingPattern(_)
            | Diagram::FacingLink(_)
            | Diagram::Entry(_)
            | Diagram::DrillCycle(_)
            | Diagram::FixtureClearance
            | Diagram::Containment(_) => Strategy::Face,
        };
        let orientation = matches!(
            self.diagram,
            Diagram::Orientation | Diagram::Operation(Strategy::Indexed)
        );
        let angle = if orientation {
            (phase * 2.5).min(1.) * std::f32::consts::FRAC_PI_2
        } else {
            0.
        };
        let pose = |a: [f32; 3]| {
            if orientation {
                [
                    a[0],
                    a[1] * angle.cos() - (a[2] - 12.) * angle.sin(),
                    12. + a[1] * angle.sin() + (a[2] - 12.) * angle.cos(),
                ]
            } else {
                a
            }
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
                polygon(&mut f, &face.map(pose), c);
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
        if matches!(
            s,
            Strategy::Parallel | Strategy::FiveAxis | Strategy::Scallop | Strategy::Waterline
        ) {
            // A curved surface makes surface-following and changing tilt visible.
            for yi in 0..8 {
                for xi in 0..12 {
                    let x = -36. + xi as f32 * 6.;
                    let y = -20. + yi as f32 * 5.;
                    let surf =
                        |x: f32, y: f32| [x, y, 25. + 8. * (x / 26.).cos() * (y / 28.).cos()];
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
        if matches!(s, Strategy::Pocket | Strategy::Adaptive) {
            polygon(
                &mut f,
                &[
                    [-28., -16., 24.],
                    [28., -16., 24.],
                    [24., -12., 13.],
                    [-24., -12., 13.],
                ],
                Color::from_rgb8(71, 96, 117),
            );
            polygon(
                &mut f,
                &[
                    [-24., -12., 13.],
                    [24., -12., 13.],
                    [24., 12., 13.],
                    [-24., 12., 13.],
                ],
                Color::from_rgb8(102, 128, 146),
            );
        }
        let n = 180usize;
        let mut path = vec![];
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let a = t * std::f32::consts::TAU;
            let q = match s {
                Strategy::Face | Strategy::Parallel | Strategy::Indexed => {
                    let row = (t * 6.).floor().min(5.);
                    let x = if row as i32 % 2 == 0 {
                        -37. + 74. * (t * 6. - row)
                    } else {
                        37. - 74. * (t * 6. - row)
                    };
                    [
                        x,
                        -20. + row * 8.,
                        if s == Strategy::Parallel {
                            26. + 8. * (x / 26.).cos() * ((-20. + row * 8.) / 28.).cos()
                        } else {
                            25.
                        },
                    ]
                }
                Strategy::Adaptive => {
                    let r = 5. + 14. * t;
                    [
                        r * (a * 4.).cos() + 5. * (a * 15.).cos(),
                        r * 0.65 * (a * 4.).sin() + 3. * (a * 15.).sin(),
                        15.,
                    ]
                }
                Strategy::Pocket | Strategy::Scallop => {
                    let r = 5. + 21. * t;
                    [
                        r * (a * 4.).cos(),
                        r * 0.55 * (a * 4.).sin(),
                        if matches!(self.diagram, Diagram::Stepdown) {
                            24. - 3. * (t * 4.).floor().min(3.)
                        } else if matches!(self.diagram, Diagram::Ramp) {
                            24. - 12. * t
                        } else if s == Strategy::Scallop {
                            let x = r * (a * 4.).cos();
                            let y = r * 0.55 * (a * 4.).sin();
                            26. + 8. * (x / 26.).cos() * (y / 28.).cos()
                        } else {
                            15.
                        },
                    ]
                }
                Strategy::Box => {
                    // Passes along X, stepping across by Y: one axis at a time.
                    let row = (t * 4.).floor().min(3.);
                    let k = (t * 4.).fract();
                    let x = if row as i32 % 2 == 0 {
                        2. * k
                    } else {
                        2. - 2. * k
                    };
                    [-30. + 30. * x, -18. + row * 10., 25.]
                }
                Strategy::Contour => [44. * a.cos(), 27. * a.sin(), 21.],
                Strategy::Drill => {
                    let hole = (t * 4.).floor().min(3.) as usize;
                    let xy = [[-23., -12.], [23., -12.], [23., 12.], [-23., 12.]][hole];
                    let k = (t * 4.).fract();
                    [xy[0], xy[1], 40. - 28. * (k * std::f32::consts::PI).sin()]
                }
                Strategy::Waterline => {
                    let level = (t * 4.).floor().min(3.);
                    [42. * (a * 4.).cos(), 26. * (a * 4.).sin(), 23. - level * 5.]
                }
                Strategy::Rotary => {
                    let angle =
                        a * 4. + std::f32::consts::FRAC_PI_2 - phase * std::f32::consts::TAU * 4.;
                    [-32. + 64. * t, 19. * angle.cos(), 20. + 19. * angle.sin()]
                }
                Strategy::FiveAxis => {
                    let x = -34. + 68. * t;
                    let y = 12. * a.sin();
                    [x, y, 26. + 8. * (x / 26.).cos() * (y / 28.).cos()]
                }
            };
            // Indexed cutting only starts after the workpiece is held in its new position.
            path.push(if orientation {
                [q[0], q[1] * 0.5, 38.]
            } else {
                q
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
        }
        if matches!(self.diagram, Diagram::Offsets) {
            for (z, c) in [
                (24., cyan),
                (7., cyan),
                (7. + 5. * (phase * std::f32::consts::TAU).sin(), gold),
            ] {
                line(&mut f, [-40., 24., z], [40., 24., z], c, 2.);
            }
            f.fill_text(canvas::Text {
                content: "REFERENCE / OFFSET PLANE".into(),
                position: Point::new(8., 8.),
                color: gold,
                size: 10.into(),
                ..Default::default()
            });
        }
        let cutting_phase = if orientation {
            ((phase - 0.4) / 0.6).max(0.)
        } else {
            phase
        };
        let end = ((cutting_phase * n as f32) as usize).min(n);
        for (i, pair) in path.windows(2).enumerate() {
            if orientation && phase < 0.4 {
                continue;
            }
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
        if matches!(
            self.diagram,
            Diagram::Stepover | Diagram::Stepdown | Diagram::Allowance
        ) {
            let (a, b, label) = match self.diagram {
                Diagram::Stepover => ([28., -20., 26.], [28., -12., 26.], "SIDEWAYS SPACING"),
                Diagram::Stepdown => ([29., 10., 24.], [29., 10., 21.], "DEPTH PER LAYER"),
                _ => ([27., -16., 25.], [33., -16., 25.], "STOCK TO LEAVE"),
            };
            line(&mut f, a, b, gold, 3.);
            for a in [a, b] {
                f.fill(&canvas::Path::circle(p(a), 2.5), gold)
            }
            f.fill_text(canvas::Text {
                content: label.into(),
                position: Point::new(8., 8.),
                color: gold,
                size: 10.into(),
                ..Default::default()
            });
        }
        let tip = if orientation && phase < 0.4 {
            [-30., -20., 47.]
        } else {
            path[end]
        };
        let tilt = if s == Strategy::FiveAxis {
            0.5 * (phase * std::f32::consts::TAU).sin()
        } else {
            0.
        };
        let endtool = [tip[0] + tilt * 23., tip[1], tip[2] + 23.];
        let mid = [tip[0] + tilt * 11., tip[1], tip[2] + 11.];
        line(&mut f, mid, endtool, Color::from_rgb8(221, 231, 236), 9.);
        line(&mut f, tip, mid, gold, 7.);
        f.fill(&canvas::Path::circle(p(tip), 3.2), gold);
        for z in 0..4 {
            let z = z as f32 * 2.6;
            let a = [tip[0] + tilt * z, tip[1], tip[2] + z];
            let q = p(a);
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
        let caption = if orientation {
            "REPOSITION → HOLD → CUT"
        } else if matches!(self.diagram, Diagram::Heights) {
            "AMBER: CLEARANCE   CYAN: RETRACT"
        } else if matches!(self.diagram, Diagram::Offsets) {
            "CYAN: GEOMETRY   AMBER: OFFSET"
        } else if s == Strategy::FiveAxis {
            "TOOL AXIS CHANGES ALONG THE SURFACE"
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
        assert!(gap.diagram.is_none());
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
            ("holding", vec!["None", "Vise", "Clamps"]),
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
            RunOnMachine(0),
            NewMachine,
            AddSetup,
            AddManualSetup,
            ToolMachine("m".into(), true),
            HolderMachine("m".into(), true),
            StockDetail(Detail::Draft),
            StockDetail(Detail::Standard),
            StockDetail(Detail::Fine),
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
            cards.push((
                format!("dimmed:{s:?}"),
                dimmed_help(s, crate::manual::ONE_AXIS),
            ));
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
