//! The Steps page for the TypeScript shop tablet (LIO-477). The tablet plans
//! nothing, so the computer works out every screen of each hand setup the
//! way this page does and draws it: the pictures half and the numbers half
//! of `parts`, headless (tiny-skia), as PNGs. With them go the words the
//! tablet lays out itself around them, finger-sized: the setup's line, each
//! section's header (drawn too) and the place, the step's caption, and
//! Back and the step's own command.
//!
//! A screen is a step at one item: a hole of a Holes or Bore step, as Next
//! and Back walk them here; other steps have one, its first readout lit.
//! Its numbers are drawn once more for each readout reached in turn, and
//! where each readout and tile sits on them goes along: a tap there shows
//! the next lit, or goes to that pass, as a click does here (LIO-525).
use super::*;
use std::collections::HashMap;

/// The window the screens are laid out for: below 1200 wide, as narrow as
/// the desktop's 1024 × 700 class (`turning::narrow`), so the lathe's numbers
/// take half the card. Its card is about as wide as the shop tablet's
/// (K10C: 960 × 552 dp, the operations list beside it), so the screens show
/// near their size and the drawings' small words stay readable (LIO-477).
/// The height only sizes the lathe's dials (`turning::dial_size`).
const WINDOW: (f32, f32) = (1000., 760.);
/// The width the card leaves the pictures and the numbers together: the
/// window less the setups list beside it (`turning::dial_size`).
const CARD: f32 = WINDOW.0 - 300.;
/// How high the pictures are drawn: the room the tablet's page leaves them.
const PICTURE_HEIGHT: f32 = 330.;
/// Pixels a logical pixel: the K10C has 1.33 pixels a dp. More pixels only
/// make it slower to show a screen.
const SCALE: f32 = 1.5;
/// What a reader of an older `format` would show wrong bumps it.
pub const FORMAT: u32 = 1;

/// What the tablet's Steps page shows: the hand setups' screens.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Steps {
    pub format: u32,
    /// Pixels a logical pixel of the pictures (`SCALE`).
    pub scale: f32,
    pub setups: Vec<SetupSteps>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SetupSteps {
    /// The setup's id.
    pub setup: String,
    /// The line over its steps, as the Steps page's (`setup_title`).
    pub title: String,
    /// Its name in the page's list of setups, "2 Top · by hand".
    pub label: String,
    pub lathe: bool,
    pub minutes: f32,
    /// Why its steps cannot be shown, the Steps page's words; then nothing
    /// else is there.
    pub problem: Option<String>,
    pub sections: Vec<SectionSteps>,
    pub screens: Vec<Screen>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SectionSteps {
    pub name: String,
    /// Its number among the operations, or "•" (Set up, the hand-over).
    pub mark: String,
    /// What it counts, "4 passes", or empty.
    pub count: String,
    /// The header drawn: number, name, tool, count and time (`head`).
    pub head: Png,
    /// Its screens, `[first, end)`: a press on it goes to the first.
    pub screens: [usize; 2],
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Screen {
    /// The step of the guide and its item (hole).
    pub step: usize,
    pub item: usize,
    pub section: usize,
    pub title: String,
    pub sub: String,
    /// Where the job is, "Setup 1 of 4 · operation 2 of 5 · hole 1/4".
    pub place: String,
    /// The place in its section, "hole 1/4", or empty.
    pub progress: String,
    /// The command that ends the screen, "Pass done ▶"; none on the last.
    pub next: Option<String>,
    /// A hand-over's command opens this setup (its id), not the next screen.
    pub opens: Option<String>,
    pub back: bool,
    /// A lathe step that takes a typed measurement or the dial check's
    /// answer, which only the computer works into the steps after it.
    pub asks: bool,
    /// What it asks, which the tablet sends the computer (`tablet::Answer`,
    /// LIO-524). Missing in files before it.
    #[serde(default)]
    pub ask: Option<Ask>,
    /// The pictures, and the numbers, which scroll when they do not fit.
    pub picture: Png,
    pub numbers: Png,
    /// Where a tap on the numbers does something.
    pub taps: Vec<Tap>,
    /// The screen once readouts `0..=n` are reached, `reached[n]`: the next
    /// lit, or all ticked after the last (`GuideAction::Reached`). A
    /// walk's pictures move the tool to the readout lit.
    pub reached: Vec<Reached>,
}
/// A screen drawn with some of its readouts reached.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Reached {
    pub picture: Png,
    pub numbers: Png,
    pub taps: Vec<Tap>,
}
/// A readout or a tile on a numbers PNG: its box there, `[x, y, width,
/// height]` in its pixels, and what a tap on it does.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Tap {
    pub rect: [u32; 4],
    /// Readout `n` reached: shows the screen's `reached[n]`.
    pub reached: Option<usize>,
    /// A pass's tile: goes to this screen of the setup.
    pub screen: Option<usize>,
}
/// What a lathe step asks.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Ask {
    /// A diameter measured with the calipers, mm, kept under `key`
    /// (`guide::measure_key`): `expect` as planned, `measured` once typed.
    Diameter {
        key: String,
        expect: f32,
        measured: Option<f32>,
    },
    /// The dial check: the diameter went down by `reading` (the dial reads
    /// the diameter) or by twice it (the radius).
    Dial { reading: f32 },
}
/// A PNG in the tablet file, by its path there, and its size in pixels.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Png {
    pub path: String,
    pub size: [u32; 2],
}

/// The steps of every hand setup in `doc`, its part `mesh`, and the PNGs
/// they name by path (named by their bytes: a picture many screens share is
/// there once).
pub fn steps(doc: &Document, mesh: Result<&Mesh, &str>) -> (Steps, HashMap<String, Vec<u8>>) {
    let mut files = HashMap::new();
    let setups = (doc.setups.iter().enumerate())
        .filter(|(_, s)| crate::manual::is_hand(&s.machine))
        .map(|(si, setup)| {
            let shown = mesh.map_err(str::to_owned).and_then(|m| shown(doc, m, si));
            let mut out = SetupSteps {
                setup: setup.id.clone(),
                title: setup_title(setup),
                label: crate::ui::job_setup_label(si, setup),
                lathe: crate::lathe::is_lathe(&setup.machine),
                minutes: 0.,
                problem: None,
                sections: vec![],
                screens: vec![],
            };
            match shown {
                Ok(shown) => fill(doc, si, &shown, &mut out, &mut files),
                Err(e) => out.problem = Some(e),
            }
            out
        })
        .collect();
    (
        Steps {
            format: FORMAT,
            scale: SCALE,
            setups,
        },
        files,
    )
}

fn fill(
    doc: &Document,
    si: usize,
    shown: &Shown,
    out: &mut SetupSteps,
    files: &mut HashMap<String, Vec<u8>>,
) {
    let g = &shown.guide;
    let setup = &doc.setups[si];
    out.minutes = g.minutes;
    let page = |at: usize, item: usize, reached: usize| Page {
        doc,
        shown,
        key: 0,
        at,
        item,
        reached,
        typed: None,
        window: WINDOW,
        clock: (std::time::Instant::now(), std::time::Instant::now()),
        setup,
        tablet: true,
    };
    let mut keep = |(png, bytes): (Png, Vec<u8>)| {
        files.entry(png.path.clone()).or_insert(bytes);
        png
    };
    // A screen's pictures and numbers drawn, `reached` readouts reached:
    // a hole's by its own count, another step's by the item, as
    // `App::guide` keeps them.
    let pictures = |at: usize, item: usize, reached: usize| {
        let holes = matches!(
            &g.steps[at],
            Step::Holes { holes, .. } | Step::Bore { holes, .. } if !holes.is_empty()
        );
        let p = match holes {
            true => page(at, item, reached),
            false => page(at, reached, 0),
        };
        let Parts {
            title,
            sub,
            left,
            right,
            next,
            back,
            split: (l, r),
        } = parts(&p);
        let (left_w, right_w) = widths((l, r));
        let (rgba, size, _) = drawn(left, (left_w, PICTURE_HEIGHT));
        let picture = named((rgba, size));
        let (rgba, size, taps) = tall(scrollable(right.spacing(10)).into(), right_w, false);
        let next = next.map(|(label, _)| label);
        (picture, named((rgba, size)), taps, (title, sub, next, back))
    };
    // One screen: its words, and its pictures and numbers drawn, once more
    // for each readout reached.
    let draw = |k: usize, at: usize, item: usize| {
        let (picture, numbers, taps, (title, sub, next, back)) = pictures(at, item, 0);
        let readouts = (taps.iter())
            .filter_map(|t| match t.1 {
                Message::Guide(GuideAction::Reached(n)) => Some(n + 1),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let mut pngs = vec![picture.1, numbers.1];
        let reached: Vec<_> = (1..=readouts)
            .map(|n| {
                let (picture, numbers, taps, _) = pictures(at, item, n);
                pngs.extend([picture.1, numbers.1]);
                (picture.0, numbers.0, taps)
            })
            .collect();
        let opens = match (&g.steps[at], &next) {
            (Step::HandOver { next: Some(n) }, Some(_)) => doc.setups.get(*n).map(|s| s.id.clone()),
            _ => None,
        };
        use crate::guide::LatheStep as L;
        let diameter = |key: &String, expect: &f32, measured: &Option<f32>| Ask::Diameter {
            key: key.clone(),
            expect: *expect,
            measured: *measured,
        };
        let ask = match &g.steps[at] {
            Step::Lathe(L::Measure {
                key,
                expect,
                measured,
                ..
            }) => Some(diameter(key, expect, measured)),
            Step::Lathe(L::Zero {
                measure: true,
                key,
                planned,
                measured,
                ..
            }) => Some(diameter(key, planned, measured)),
            Step::Lathe(L::DialCheck { reading, .. }) => Some(Ask::Dial { reading: *reading }),
            _ => None,
        };
        let screen = Screen {
            step: at,
            item,
            section: k,
            title,
            sub: sub.into(),
            place: place(doc, g, k, at, item),
            progress: progress(g, &g.sections[k], at, item),
            next,
            opens,
            back,
            asks: ask.is_some(),
            ask,
            picture: picture.0,
            numbers: numbers.0,
            taps: vec![],
            reached: vec![],
        };
        (screen, pngs, taps, reached)
    };
    // Every screen, a hole of a Holes or Bore step each, drawn on every
    // core: the tablet waits for them.
    let jobs: Vec<(usize, usize, usize)> = (g.sections.iter().enumerate())
        .flat_map(|(k, s)| s.steps.clone().map(move |at| (k, at)))
        .flat_map(|(k, at)| {
            let items = match &g.steps[at] {
                Step::Holes { holes, .. } | Step::Bore { holes, .. } => holes.len().max(1),
                _ => 1,
            };
            (0..items).map(move |item| (k, at, item))
        })
        .collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut drawn_screens: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let (jobs, draw) = (&jobs, &draw);
                scope.spawn(move || {
                    let drawn = (jobs.iter().enumerate().skip(t).step_by(threads))
                        .map(|(i, &(k, at, item))| (i, draw(k, at, item)))
                        .collect::<Vec<_>>();
                    // The words the drawings wrote on this thread, for the
                    // tests on the caller's (LIO-545).
                    #[cfg(test)]
                    let drawn = (drawn, DRAWN.take());
                    drawn
                })
            })
            .collect();
        (handles.into_iter())
            .flat_map(|h| {
                let drawn = h.join().expect("a screen drawn");
                #[cfg(test)]
                let drawn = {
                    DRAWN.with_borrow_mut(|d| d.extend(drawn.1));
                    drawn.0
                };
                drawn
            })
            .collect()
    });
    drawn_screens.sort_by_key(|(i, _)| *i);
    // A tile goes to its step's first screen, the screens being the jobs.
    let taps = |taps: Taps| -> Vec<Tap> {
        (taps.into_iter())
            .filter_map(|(r, m)| {
                let (reached, screen) = match m {
                    Message::Guide(GuideAction::Reached(n)) => (Some(n), None),
                    Message::Guide(GuideAction::Go(at)) => {
                        (None, Some(jobs.iter().position(|j| (j.1, j.2) == (at, 0))?))
                    }
                    _ => return None,
                };
                // As the PNG's size is: within it.
                let px = |v: f32| (v * SCALE) as u32;
                let rect = [px(r.x), px(r.y), px(r.width), px(r.height)];
                Some(Tap {
                    rect,
                    reached,
                    screen,
                })
            })
            .collect()
    };
    for (_, (mut screen, pngs, first, reached)) in drawn_screens {
        let mut named = vec![screen.picture.clone(), screen.numbers.clone()];
        screen.taps = taps(first);
        for (picture, numbers, t) in reached {
            named.extend([picture.clone(), numbers.clone()]);
            screen.reached.push(Reached {
                picture,
                numbers,
                taps: taps(t),
            });
        }
        for (png, bytes) in named.into_iter().zip(pngs) {
            keep((png, bytes));
        }
        out.screens.push(screen);
    }
    for (k, s) in g.sections.iter().enumerate() {
        let first = out
            .screens
            .iter()
            .position(|x| x.section == k)
            .unwrap_or(out.screens.len());
        let end = out
            .screens
            .iter()
            .rposition(|x| x.section == k)
            .map_or(first, |i| i + 1);
        let (head, size, _) = tall(
            head(&page(s.steps.start, 0, 0), k).into(),
            CARD + 200.,
            true,
        );
        out.sections.push(SectionSteps {
            name: section_name(doc, g, s),
            mark: op_number(g, k).map_or("•".to_string(), |n| n.to_string()),
            count: if s.count > 0 {
                counted(setup, s)
            } else {
                String::new()
            },
            head: keep(named((head, size))),
            screens: [first, end],
        });
    }
}

/// A drawing as a PNG named by its bytes, so that one many screens share is
/// in the tablet file once.
fn named((rgba, size): (Vec<u8>, (u32, u32))) -> (Png, Vec<u8>) {
    use std::hash::{Hash, Hasher};
    let bytes = png(&rgba, size);
    let mut h = std::hash::DefaultHasher::new();
    bytes.hash(&mut h);
    let png = Png {
        path: format!("steps/{:016x}.png", h.finish()),
        size: [size.0, size.1],
    };
    (png, bytes)
}
/// How wide the pictures and the numbers are drawn, the card shared as
/// `parts` splits it.
fn widths((l, r): (u16, u16)) -> (f32, f32) {
    let (l, r) = (f32::from(l), f32::from(r));
    (
        ((CARD - 18.) * l / (l + r)).round(),
        ((CARD - 18.) * r / (l + r)).round(),
    )
}
/// What a press on each of a drawing's controls sends, and where it is.
type Taps = Vec<(Rectangle, Message)>;
/// `element` drawn `size` logical px on white, headless (tiny-skia), at
/// `SCALE`: RGBA by rows, its size in pixels, and its controls.
pub(crate) fn drawn(
    element: Element<'_, Message>,
    size: (f32, f32),
) -> (Vec<u8>, (u32, u32), Taps) {
    let px = ((size.0 * SCALE) as u32, (size.1 * SCALE) as u32);
    let (rgba, taps) = render_taps(
        element,
        Size::new(size.0, size.1),
        SCALE,
        mouse::Cursor::Unavailable,
        true,
    );
    (rgba, px, taps)
}
/// `element` drawn `size` logical px on white at `scale`, headless
/// (tiny-skia), with the pointer at `cursor`: RGBA by rows. With `redraw`,
/// after a frame's redraw, which sets each button's status: without it a
/// button draws as disabled. Only the tests use it (LIO-547).
#[cfg(test)]
pub(crate) fn render(
    element: Element<'_, Message>,
    size: Size,
    scale: f32,
    cursor: mouse::Cursor,
    redraw: bool,
) -> Vec<u8> {
    render_taps(element, size, scale, cursor, redraw).0
}
/// `render`, and where each control that says what it sends is (a
/// `HelpButton`), in logical px.
fn render_taps(
    element: Element<'_, Message>,
    size: Size,
    scale: f32,
    cursor: mouse::Cursor,
    redraw: bool,
) -> (Vec<u8>, Taps) {
    use futures_util::FutureExt;
    use iced::advanced::renderer::{self, Headless};
    let mut renderer =
        <iced::Renderer as Headless>::new(Font::DEFAULT, iced::Pixels(16.), Some("tiny-skia"))
            .now_or_never()
            .flatten()
            .expect("tiny-skia renderer");
    let mut ui = iced_runtime::UserInterface::build(
        element,
        size,
        iced_runtime::user_interface::Cache::new(),
        &mut renderer,
    );
    if redraw {
        let frame = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        let _ = ui.update(
            &[frame],
            cursor,
            &mut renderer,
            &mut iced::advanced::clipboard::Null,
            &mut vec![],
        );
    }
    struct Presses(Taps);
    impl iced::advanced::widget::Operation for Presses {
        fn traverse(
            &mut self,
            operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation),
        ) {
            operate(self);
        }
        fn custom(
            &mut self,
            _: Option<&iced::advanced::widget::Id>,
            bounds: Rectangle,
            state: &mut dyn std::any::Any,
        ) {
            if let Some(m) = state.downcast_ref::<Message>() {
                self.0.push((bounds, m.clone()));
            }
        }
    }
    let mut presses = Presses(vec![]);
    ui.operate(&renderer, &mut presses);
    ui.draw(
        &mut renderer,
        &Theme::Light,
        &renderer::Style {
            text_color: Color::BLACK,
        },
        cursor,
    );
    let physical = Size::new((size.width * scale) as u32, (size.height * scale) as u32);
    (
        renderer.screenshot(physical, scale, Color::WHITE),
        presses.0,
    )
}
/// `element` drawn `width` logical px wide and as high as it is: drawn on a
/// tall page, the white under it cut off, and right of it when `narrow`.
fn tall(element: Element<'_, Message>, width: f32, narrow: bool) -> (Vec<u8>, (u32, u32), Taps) {
    let (mut rgba, (w, h), taps) = drawn(element, (width, 2400.));
    let row = w as usize * 4;
    let margin = (8. * SCALE) as usize;
    let used = (rgba.chunks(row))
        .rposition(|r| r.iter().any(|&b| b != 255))
        .map_or(1, |i| i + 1);
    let h = (used + margin).min(h as usize);
    rgba.truncate(h * row);
    if !narrow {
        return (rgba, (w, h as u32), taps);
    }
    let wide = (rgba.chunks(row))
        .filter_map(|r| r.chunks(4).rposition(|p| p[..3] != [255; 3]))
        .max()
        .map_or(1, |x| x + 1);
    let w2 = (wide + margin).min(w as usize);
    let cut = (rgba.chunks(row))
        .flat_map(|r| r[..w2 * 4].iter().copied())
        .collect();
    (cut, (w2 as u32, h as u32), taps)
}
/// RGBA as a PNG, its alpha dropped: everything is drawn on white.
fn png(rgba: &[u8], (w, h): (u32, u32)) -> Vec<u8> {
    let rgb: Vec<u8> = rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut bytes = vec![];
    let mut encoder = ::png::Encoder::new(&mut bytes, w, h);
    encoder.set_color(::png::ColorType::Rgb);
    encoder.set_depth(::png::BitDepth::Eight);
    // A Steps file carries a numbers PNG per readout reached (LIO-525):
    // Default and Paeth halve it, Fast's size, for a few seconds more here.
    encoder.set_compression(::png::Compression::Default);
    encoder.set_filter(::png::FilterType::Paeth);
    (encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(&rgb)))
    .expect("a PNG in memory");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GuideAction;

    /// The tablet's screens are the desktop's, in the order its Next walks
    /// them, with its commands; every picture is a PNG it carries.
    #[test]
    fn the_tablet_walks_the_screens_the_steps_page_does() {
        let mut a = crate::steps::tests::app("Yes");
        let mesh = a.part_mesh().unwrap().clone();
        let (steps, files) = super::steps(&a.doc, Ok(&mesh));
        assert_eq!(steps.format, FORMAT);
        let hand = (a.doc.setups.iter())
            .filter(|s| crate::manual::is_hand(&s.machine))
            .count();
        assert_eq!(steps.setups.len(), hand);
        let mine = &steps.setups[0];
        assert_eq!(mine.setup, a.doc.setups[0].id);
        assert!(mine.problem.is_none());
        assert!(mine.screens.len() > mine.sections.len(), "{mine:#?}");
        let (mut tiles, mut reached) = (vec![], 0);
        for (k, s) in mine.screens.iter().enumerate() {
            let (shown, at) = a.guide_at().unwrap();
            assert_eq!((s.step, s.item), (at, a.guide_item), "screen {k}");
            let g = &shown.guide;
            let section = section_of(g, at);
            assert_eq!(s.section, section);
            assert_eq!(
                s.place,
                place(&a.doc, g, section, at, a.guide_item),
                "screen {k}"
            );
            if k + 1 < mine.screens.len() {
                assert_eq!(s.next, Some(next_label(g, at, a.guide_item)));
            }
            assert_eq!(s.back, k > 0);
            for png in [&s.picture, &s.numbers] {
                let decoder = ::png::Decoder::new(std::io::Cursor::new(&files[&png.path]));
                let info = decoder.read_info().unwrap().info().clone();
                assert_eq!([info.width, info.height], png.size);
                assert!(info.width > 400 && info.height > 40, "{png:?}");
            }
            // Each readout's tap shows the screen as the page draws it once
            // that readout is reached; the boxes stay where they were.
            let readouts: Vec<usize> = s.taps.iter().filter_map(|t| t.reached).collect();
            assert_eq!(
                readouts,
                (0..s.reached.len()).collect::<Vec<_>>(),
                "screen {k}"
            );
            let holes = s.progress.starts_with("hole ");
            for (n, r) in s.reached.iter().enumerate() {
                let _ = a.update(Message::Guide(GuideAction::Reached(n)));
                let state = (a.guide_item, a.guide_reached);
                assert_eq!(state, if holes { (s.item, n + 1) } else { (n + 1, 0) });
                assert_ne!(r.numbers, s.numbers, "screen {k}, readout {n}");
                assert!(files.contains_key(&r.numbers.path) && files.contains_key(&r.picture.path));
                let boxes = |t: &[Tap]| t.iter().map(|t| t.rect).collect::<Vec<_>>();
                assert_eq!(boxes(&r.taps), boxes(&s.taps), "screen {k}, readout {n}");
            }
            for t in &s.taps {
                let [x, y, w, h] = t.rect;
                assert!(
                    x + w <= s.numbers.size[0] && y + h <= s.numbers.size[1],
                    "{t:?}"
                );
                assert!(h >= 30, "a finger can find it: {t:?}");
                assert!(t.reached.is_some() != t.screen.is_some());
                tiles.extend(t.screen);
            }
            reached += s.reached.len();
            let _ = a.update(Message::Guide(GuideAction::Next));
        }
        // Each section starts where a press on it goes.
        for (i, s) in mine.sections.iter().enumerate() {
            let _ = a.update(Message::Guide(GuideAction::Section(i)));
            let (_, at) = a.guide_at().unwrap();
            let first = &mine.screens[s.screens[0]];
            assert_eq!((first.step, first.item), (at, 0), "{}", s.name);
            assert!(files.contains_key(&s.head.path));
        }
        // Holes have a screen each; the numbers are drawn whole.
        let holes = mine
            .screens
            .iter()
            .filter(|s| s.progress.starts_with("hole "))
            .count();
        assert!(holes > 1, "{mine:#?}");
        assert!(reached > holes, "every hole's readouts are drawn reached");
        // A pass's tile goes to that pass, a step's first screen.
        assert!(!tiles.is_empty());
        for j in tiles {
            let pass = &mine.screens[j];
            assert_eq!(pass.item, 0);
            assert!(pass.title.starts_with("Pass "), "{}", pass.title);
        }
        assert!(mine.screens.iter().all(|s| !s.asks));
        assert!(mine.screens.last().unwrap().opens.is_some());
    }

    /// A lathe setup's screens, its measurement the tablet cannot work in
    /// marked.
    #[test]
    fn a_lathe_setup_goes_to_the_tablet_too() {
        let a = crate::steps::tests::lathe_app("Diameter", |_| {});
        let mesh = a.part_mesh().unwrap().clone();
        let (steps, files) = super::steps(&a.doc, Ok(&mesh));
        let lathe = &steps.setups[0];
        assert!(lathe.lathe && lathe.problem.is_none(), "{lathe:#?}");
        assert!(
            lathe
                .screens
                .iter()
                .any(|s| s.title.starts_with("Pass 1 of "))
        );
        // The bar measured at the Zero, and before each last pass (LIO-524).
        let zero = crate::guide::measure_key(&a.doc.setups[0].id, "zero");
        let asked: Vec<_> = (lathe.screens.iter())
            .filter_map(|s| match &s.ask {
                Some(Ask::Diameter { key, measured, .. }) if s.asks => Some((key, *measured)),
                _ => None,
            })
            .collect();
        assert!(asked.len() > 1 && asked[0] == (&zero, None), "{asked:?}");
        assert!(lathe.screens.iter().all(|s| s.asks == s.ask.is_some()));
        // The dial check while the lathe's dial is not known yet.
        let unknown = crate::steps::tests::lathe_app("", |_| {});
        let (steps, _) = super::steps(&unknown.doc, Ok(&mesh));
        let first = &steps.setups[0].screens[0];
        assert!(
            matches!(first.ask, Some(Ask::Dial { reading }) if reading > 0.),
            "{first:?}"
        );
        assert!(
            lathe
                .screens
                .iter()
                .all(|s| files.contains_key(&s.numbers.path))
        );
        // No part: the setup says why, as the page would.
        let (steps, _) = super::steps(&a.doc, Err(crate::run::NO_PART));
        assert_eq!(
            steps.setups[0].problem.as_deref(),
            Some(crate::run::NO_PART)
        );
    }

    /// A lathe screen that takes a measurement shows it as a plain readout
    /// on the tablet, which types it in its own field: no box to type in
    /// drawn as well (LIO-543). The desktop keeps its box.
    #[test]
    fn the_tablets_measure_screens_hold_no_box_to_type_in() {
        use crate::guide::LatheStep as L;
        let zero = |a: &crate::App| crate::guide::measure_key(&a.doc.setups[0].id, "zero");
        let mut a = crate::steps::tests::lathe_app("Diameter", |_| {});
        let key = zero(&a);
        a.doc.measured.insert(key.clone(), 41.5);
        let mesh = a.part_mesh().unwrap().clone();
        let shown = super::shown(&a.doc, &mesh, 0).unwrap();
        let window = Size::new(WINDOW.0, WINDOW.1);
        let measures: Vec<_> = (shown.guide.steps.iter().enumerate())
            .filter_map(|(at, s)| match s {
                Step::Lathe(
                    L::Measure { key, .. }
                    | L::Zero {
                        measure: true, key, ..
                    },
                ) => Some((at, key)),
                _ => None,
            })
            .collect();
        assert!(measures.len() > 1, "{measures:?}");
        for (at, k) in measures {
            let page = |tablet| Page {
                doc: &a.doc,
                shown: &shown,
                key: 0,
                at,
                item: 0,
                reached: 0,
                typed: None,
                window: WINDOW,
                clock: (std::time::Instant::now(), std::time::Instant::now()),
                setup: &a.doc.setups[0],
                tablet,
            };
            let numbers = |tablet| parts(&page(tablet)).right.into();
            assert_eq!(crate::ui::probe::fields(numbers(false), window).len(), 1);
            assert!(crate::ui::probe::fields(numbers(true), window).is_empty());
            let read = if *k == key { "Ø41.50" } else { "Ø ?" };
            let texts = crate::ui::probe::texts(numbers(true), window);
            assert!(texts.iter().any(|t| t == read), "{read} in {texts:?}");
        }
    }

    /// A pass screen's tiles jump to another pass on the desktop, and on
    /// the tablet too, where they are a finger tall: the tablet finds them by
    /// their taps (LIO-525; LIO-544 had drawn them plain while a tap did
    /// nothing).
    #[test]
    fn the_tablets_pass_tiles_go_where_the_desktops_do() {
        use crate::guide::LatheStep as L;
        let a = crate::steps::tests::lathe_app("Diameter", |_| {});
        let mesh = a.part_mesh().unwrap().clone();
        let shown = super::shown(&a.doc, &mesh, 0).unwrap();
        let window = Size::new(WINDOW.0, WINDOW.1);
        let passes: Vec<_> = (shown.guide.steps.iter().enumerate())
            .filter(|(_, s)| matches!(s, Step::Lathe(L::Pass { .. } | L::Measure { .. })))
            .map(|(at, _)| at)
            .collect();
        assert!(passes.len() > 2, "{passes:?}");
        for at in passes {
            let page = |tablet| Page {
                doc: &a.doc,
                shown: &shown,
                key: 0,
                at,
                item: 0,
                reached: 0,
                typed: None,
                window: WINDOW,
                clock: (std::time::Instant::now(), std::time::Instant::now()),
                setup: &a.doc.setups[0],
                tablet,
            };
            // A click on the middle of each text drawn.
            let goes = |tablet| {
                let placed = crate::ui::probe::placed(parts(&page(tablet)).right.into(), window);
                let mut goes = vec![];
                for (s, b) in &placed {
                    let dot = iced::Rectangle::new(b.center(), Size::new(1., 1.));
                    let clicks = crate::ui::probe::clicks_in(
                        parts(&page(tablet)).right.into(),
                        window,
                        dot,
                        8.,
                    );
                    if clicks
                        .iter()
                        .any(|m| matches!(m, Message::Guide(GuideAction::Go(_))))
                    {
                        goes.push(s.clone());
                    }
                }
                (goes, placed.into_iter().map(|(s, _)| s).collect::<Vec<_>>())
            };
            let (desktop, _) = goes(false);
            let (tablet, _) = goes(true);
            assert!(desktop.iter().any(|s| s == "measure"), "{desktop:?}");
            assert_eq!(tablet, desktop);
            let (_, _, taps) = drawn(parts(&page(true)).right.into(), (WINDOW.0, WINDOW.1));
            let tiles: Vec<_> = (taps.iter())
                .filter(|(_, m)| matches!(m, Message::Guide(GuideAction::Go(_))))
                .collect();
            assert!(!tiles.is_empty());
            for (r, _) in tiles {
                assert!(r.height >= 40., "a finger tall: {r:?}");
            }
        }
    }

    /// Every screen of `doc`'s first setup, each hole of a Holes or Bore
    /// step, laid out as the tablet draws its numbers: their width, and
    /// their texts with where each is.
    fn tablet_numbers(doc: &Document, mesh: &Mesh) -> Vec<(f32, Vec<(String, Rectangle)>)> {
        let shown = super::shown(doc, mesh, 0).unwrap();
        let mut screens = vec![];
        for (at, step) in shown.guide.steps.iter().enumerate() {
            let items = match step {
                Step::Holes { holes, .. } | Step::Bore { holes, .. } => holes.len().max(1),
                _ => 1,
            };
            for item in 0..items {
                let p = Page {
                    doc,
                    shown: &shown,
                    key: 0,
                    at,
                    item,
                    reached: 0,
                    typed: None,
                    window: WINDOW,
                    clock: (std::time::Instant::now(), std::time::Instant::now()),
                    setup: &doc.setups[0],
                    tablet: true,
                };
                let parts = parts(&p);
                let (_, width) = widths(parts.split);
                let numbers = scrollable(parts.right.spacing(10)).into();
                let placed = crate::ui::probe::placed(numbers, Size::new(width, 2400.));
                screens.push((width, placed));
            }
        }
        screens
    }

    /// A readout's tag on the tablet's numbers reads whole (LIO-588): beside
    /// its number on one line, or under it, never squeezed into a column a
    /// few letters wide nor cut off at the page's right edge. A bore with a
    /// 0.001" dial: its "what the last pass left" and "out 1.98 on Ø · 78
    /// divisions" have no room beside the number at the tablet's width.
    /// Every text of every screen of it, of a drilling and of a slot stays
    /// within the page.
    #[test]
    fn the_tablets_numbers_read_every_tag_whole() {
        use crate::steps::tests::{bored_job, drilled_job, slot_job};
        // A tag's one line, at its size.
        let tag = crate::ui::probe::placed(text("out").size(12).into(), Size::new(400., 400.));
        let one_line = tag[0].1.height;
        let (bore, ring) = bored_job(|h| {
            h.p.insert("dial".into(), "0.0254".into());
        });
        // Its passes: each tag after the head's setting and the bore to
        // measure.
        let mut tags = 0;
        for (width, placed) in tablet_numbers(&bore, &ring) {
            let at = |s: &str| placed.iter().position(|(t, _)| t == s);
            let readouts = [at("head").map(|i| i + 1), at("bore").map(|i| i + 1)];
            for value in readouts.into_iter().flatten() {
                let ((v, vb), (tag, tb)) = (&placed[value], &placed[value + 1]);
                let beside = tb.height <= one_line + 0.5 && tb.x >= vb.x + vb.width;
                let under = tb.y >= vb.y + vb.height - 0.5;
                assert!(
                    beside || under,
                    "{tag:?} squeezed beside {v} at {width}: {tb:?}, {v} at {vb:?}"
                );
                assert!(
                    tb.x + tb.width <= width + 0.5,
                    "{tag:?} past {width}: {tb:?}"
                );
                tags += 1;
            }
        }
        assert!(tags >= 20, "{tags} tags");
        // A Tap's Check: the screw to try, wider than the page in one line.
        let screw = readout(
            Readout {
                order: None,
                lead: None,
                axis: String::new(),
                value: "M8×1.25 screw".into(),
                lock: None,
                tags: vec![],
                lit: Lit::Next,
                big: true,
            },
            None,
        );
        // At the numbers' width of a mill's screen (`parts` splits it 11:9).
        let width = widths((11, 9)).1;
        let placed = crate::ui::probe::placed(screw, Size::new(width, 600.));
        let (_, b) = (placed.iter())
            .find(|(s, _)| s == "M8×1.25 screw")
            .unwrap_or_else(|| panic!("the screw in {placed:?}"));
        assert!(
            b.x + b.width <= width + 0.5,
            "the screw past {width}: {b:?}"
        );
        let hole = crate::cam::boring::tests::ring(6.8, 30., 18.5);
        // Through, on parallels clear of the hole, as
        // `a_through_drill_is_checked_against_the_parallels_and_the_table_under_them`.
        let mut drilled = drilled_job(&hole, "6.8", true, "0", "0");
        let s = &mut drilled.setups[0].p;
        for (k, v) in [
            ("stock_bottom", "0"),
            (crate::parallels::RUN, crate::parallels::FRONT_TO_BACK),
            (crate::parallels::AT, "-11; 11"),
            (crate::parallels::HEIGHT, "10"),
            (crate::parallels::WIDTH, "4"),
        ] {
            s.insert(k.into(), v.into());
        }
        let (slot, slotted) = slot_job(false, "-5");
        for (doc, mesh) in [(&bore, &ring), (&drilled, &hole), (&slot, &slotted)] {
            let screens = tablet_numbers(doc, mesh);
            assert!(screens.len() > 3, "{}", screens.len());
            for (width, placed) in screens {
                for (s, b) in &placed {
                    assert!(
                        b.x >= -0.5 && b.x + b.width <= width + 0.5,
                        "{s:?} past {width}: {b:?} in {placed:?}"
                    );
                }
            }
        }
    }

    /// The tablet's numbers of `AXIS_RENDER_PROJECT` (a copy), as
    /// `save_for_tablet` draws them, into `AXIS_RENDER_DIR`: setup
    /// `AXIS_RENDER_SETUP` (an index, else every hand setup), each screen's
    /// numbers `<setup>-<step>-<item>.png` (LIO-588).
    #[test]
    #[ignore]
    fn render_a_projects_tablet_numbers() {
        let dir = std::path::PathBuf::from(std::env::var("AXIS_RENDER_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let path = std::env::var("AXIS_RENDER_PROJECT").unwrap();
        let doc = crate::storage::load(std::path::Path::new(&path)).unwrap();
        let mesh = crate::mesh::load_file(doc.file.as_ref().unwrap()).unwrap();
        let only = std::env::var("AXIS_RENDER_SETUP")
            .ok()
            .map(|s| s.parse::<usize>().unwrap());
        let (steps, files) = super::steps(&doc, Ok(&mesh));
        for s in &steps.setups {
            let si = doc.setups.iter().position(|d| d.id == s.setup).unwrap();
            if only.is_some_and(|o| o != si) {
                continue;
            }
            for screen in &s.screens {
                let name = format!("{si}-{:02}-{}.png", screen.step, screen.item);
                std::fs::write(dir.join(name), &files[&screen.numbers.path]).unwrap();
            }
        }
    }

    /// The lathe setup the planner puts in front of the shaft's CNC setup
    /// (`proposal::tests::turned`), as a tablet file in `AXIS_RENDER_DIR`:
    /// its part is made here, not a file, so it cannot be shared.
    #[test]
    #[ignore]
    fn save_the_planned_lathe_job_for_a_tablet() {
        use crate::proposal::tests::{turned, with_lathe};
        let dir = std::path::PathBuf::from(std::env::var("AXIS_RENDER_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let (doc, mesh, report) = with_lathe();
        let mut job = crate::proposal::build(&doc, &mesh, &report, &turned(&report)).unwrap();
        job.title = "Shaft (lathe)".into();
        job.file = None;
        for s in &mut job.setups {
            if crate::lathe::is_lathe(&s.machine) {
                s.machine.p.insert("cross_reads".into(), "Diameter".into());
            }
        }
        let bytes = crate::bundle::pack_with(&job, Ok(&mesh)).unwrap();
        std::fs::write(dir.join("shaft-lathe.axisjob"), bytes).unwrap();
    }

    /// The computer's own Steps page for the project at
    /// `AXIS_RENDER_PROJECT`, at 1280 × 800, into `AXIS_RENDER_DIR`: each
    /// hand setup at the first screen of each section, `<setup>-<section>.png`,
    /// to set beside the tablet's.
    #[test]
    #[ignore]
    fn render_a_projects_steps_on_the_computer() {
        let dir = std::path::PathBuf::from(std::env::var("AXIS_RENDER_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let path = std::env::var("AXIS_RENDER_PROJECT").unwrap();
        let doc = crate::storage::load(std::path::Path::new(&path)).unwrap();
        let mesh = crate::mesh::load_file(doc.file.as_ref().unwrap()).unwrap();
        let (steps, _) = super::steps(&doc, Ok(&mesh));
        for (k, s) in steps.setups.iter().enumerate() {
            let mut d = doc.clone();
            d.active = s.setup.clone();
            let mut a = crate::steps::tests::app_of(d, mesh.clone());
            (a.window_width, a.window_height) = (1280., 800.);
            for (j, section) in s.sections.iter().enumerate() {
                let _ = a.update(Message::Guide(GuideAction::Go(
                    s.screens[section.screens[0]].step,
                )));
                crate::steps::tests::render(&a, (1280, 800), &dir.join(format!("{k}-{j:02}.png")));
            }
        }
    }

    /// The computer's own Steps page for setup `AXIS_RENDER_SETUP` (an
    /// index) of the project at `AXIS_RENDER_PROJECT`, at the steps
    /// `AXIS_RENDER_STEPS` ("54,55"), each at 1280 × 800 and at the narrow
    /// 1024 × 700, into `AXIS_RENDER_DIR` as `<setup>-<step>-<width>.png`
    /// (LIO-588).
    #[test]
    #[ignore]
    fn render_a_projects_steps_at_both_window_sizes() {
        let var = |k: &str| std::env::var(k).unwrap();
        let dir = std::path::PathBuf::from(var("AXIS_RENDER_DIR"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut doc =
            crate::storage::load(std::path::Path::new(&var("AXIS_RENDER_PROJECT"))).unwrap();
        let mesh = crate::mesh::load_file(doc.file.as_ref().unwrap()).unwrap();
        let si: usize = var("AXIS_RENDER_SETUP").parse().unwrap();
        doc.active = doc.setups[si].id.clone();
        let mut a = crate::steps::tests::app_of(doc, mesh);
        for step in var("AXIS_RENDER_STEPS").split(',') {
            let step: usize = step.trim().parse().unwrap();
            for (w, h) in [(1280, 800), (1024, 700)] {
                (a.window_width, a.window_height) = (w as f32, h as f32);
                let _ = a.update(Message::Guide(GuideAction::Go(step)));
                crate::steps::tests::render(&a, (w, h), &dir.join(format!("{si}-{step}-{w}.png")));
            }
        }
    }
}
