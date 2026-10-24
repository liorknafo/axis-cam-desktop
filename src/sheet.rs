//! Flat parts cut from sheet: the "Sheet" stock lying on a spoilboard, how
//! far a cut through it may go on into the board, and what holds the sheet
//! down while it is cut: screws through its waste into the board, or
//! double-sided tape. Screw heads are holding boxes (`holding::HoldBox`), so
//! every move is checked against them as against a vise's jaws.
use crate::holding::HoldBox;
use crate::mesh::{Bounds, Mesh};
use crate::model::{Setup, get, num};

/// The stock mode of a flat part cut from a sheet (`stock::MODES`).
pub const SHEET: &str = "Sheet";
/// What holds a sheet down (`holddown`).
pub const HOLDDOWNS: [&str; 3] = ["None", "Screws", "Tape"];
/// How far a cut through the sheet may go on into the spoilboard when the
/// setup does not say, mm.
pub const SPOILBOARD_ALLOWANCE: f32 = 0.5;
/// The deepest spoilboard allowance a setup may ask for, mm.
pub const MAX_ALLOWANCE: f32 = 3.;
/// A hold-down screw's head (or washer) diameter and its height above the
/// sheet when the setup does not say, mm: an M5 pan head on a washer.
pub const SCREW_HEAD: f32 = 12.;
pub const SCREW_HEIGHT: f32 = 5.;
/// How far the default screws sit in from the sheet's corners, at least,
/// mm: their heads' diameter is used when it is larger.
const CORNER_INSET: f32 = 10.;
/// The most hold-down screws a setup lists.
pub const MAX_SCREWS: usize = 16;
/// What a sheet setup's hold-down issue says when it has none.
pub const NO_HOLDDOWN: &str = "Hold the sheet down: choose Screws (through its waste into the spoilboard) or Tape under Hold-down.";

/// Setup 1's field for a DXF part's thickness, as typed; the number the part
/// is extruded to is the project file's (`ModelFile::thickness`).
pub const DXF_THICKNESS_KEY: &str = "dxf_thickness";
/// The thickest a DXF part may be made, mm.
pub const MAX_DXF_THICKNESS: f32 = 500.;
/// Whether `setup`'s stock is a sheet on a spoilboard.
pub fn is_sheet(setup: &Setup) -> bool {
    get(&setup.p, "stock_type") == SHEET
}
/// A number field that may be left empty for `default`.
fn optional(setup: &Setup, key: &str, default: f32) -> Option<f32> {
    match get(&setup.p, key).trim() {
        "" => Some(default),
        text => text.parse::<f32>().ok().filter(|v| v.is_finite()),
    }
}
/// What is wrong with a sheet stock's numbers: its size, its thickness
/// (empty is the part's height) and the spoilboard allowance.
pub fn parameter_issues(setup: &Setup) -> Vec<String> {
    let mut issues: Vec<String> = [("stock_x", "Sheet X"), ("stock_y", "Sheet Y")]
        .iter()
        .filter(|(key, _)| !num(&setup.p, key).is_some_and(|v| v > 0.))
        .map(|(_, label)| format!("{label} must be a finite number greater than zero."))
        .collect();
    if !optional(setup, "sheet_thickness", 1.).is_some_and(|v| v > 0.) {
        issues.push(
            "The sheet thickness must be a number greater than zero, or empty for the part's thickness."
                .into(),
        );
    }
    if !allowance_of(setup).is_some_and(|v| (0. ..=MAX_ALLOWANCE).contains(&v)) {
        issues.push(format!(
            "The spoilboard allowance must be 0 to {MAX_ALLOWANCE} mm, or empty for {SPOILBOARD_ALLOWANCE} mm."
        ));
    }
    issues
}
fn allowance_of(setup: &Setup) -> Option<f32> {
    optional(setup, "spoilboard", SPOILBOARD_ALLOWANCE)
}
/// How far below the stock's bottom a cut may go in `setup`: into the
/// spoilboard under a sheet, by its allowance; 0 for any other stock, which
/// stands on the table or in the jaws.
pub fn below(setup: &Setup) -> f32 {
    if is_sheet(setup) {
        allowance_of(setup).unwrap_or(0.).clamp(0., MAX_ALLOWANCE)
    } else {
        0.
    }
}
/// The sheet of `setup` round the placed `part` (mm, the setup's frame):
/// `stock_x` × `stock_y` centred on X0 Y0 like a fixed box, lying on the
/// spoilboard at the part's bottom and as thick as `sheet_thickness`, or
/// the part when that is empty. A sheet thinner than the part is refused.
pub fn bounds(setup: &Setup, part: Bounds) -> Result<Bounds, String> {
    let height = part.dimensions()[2];
    let thickness = match get(&setup.p, "sheet_thickness").trim() {
        "" => height,
        _ => num(&setup.p, "sheet_thickness").ok_or("Enter a valid sheet thickness.")?,
    };
    if thickness < height - 0.001 {
        return Err(format!(
            "The sheet is {} mm thick but the part is {} mm: use a sheet at least as thick as the part.",
            mm(thickness),
            mm(height)
        ));
    }
    let [x, y] = ["stock_x", "stock_y"].map(|k| num(&setup.p, k).unwrap_or(0.));
    Ok(Bounds {
        min: [-x / 2., -y / 2., part.min[2]],
        max: [x / 2., y / 2., part.min[2] + thickness],
    })
}
/// `v` mm for a message: up to three decimals, without trailing zeros.
fn mm(v: f32) -> String {
    match format!("{v:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
    {
        "-0" => "0".into(),
        text => text.into(),
    }
}
/// The hold-down `setup` chose (`HOLDDOWNS`): "None" when empty.
pub fn holddown(setup: &Setup) -> &str {
    match get(&setup.p, "holddown").trim() {
        "" => "None",
        value => value,
    }
}
/// Whether `setup` holds a sheet down with screws.
pub fn screwed(setup: &Setup) -> bool {
    is_sheet(setup) && holddown(setup) == "Screws"
}
/// The screws' head diameter and height above the sheet, mm.
pub fn screw_size(setup: &Setup) -> Result<(f32, f32), String> {
    let head = optional(setup, "screw_head", SCREW_HEAD)
        .filter(|v| *v > 0.)
        .ok_or("The screw head diameter must be a positive number, or empty for 12 mm.")?;
    let height = optional(setup, "screw_height", SCREW_HEIGHT)
        .filter(|v| *v > 0.)
        .ok_or("The screw head height must be a positive number, or empty for 5 mm.")?;
    Ok((head, height))
}
/// Where `setup`'s hold-down screws go, as X, Y from the sheet's front-left
/// (−X −Y) corner, mm, read from `holddown_screws` ("10,10; 190,10"); when
/// that is empty, one in from each corner of `stock` by the head's diameter
/// (at least 10 mm).
pub fn screw_places(setup: &Setup, stock: Bounds) -> Result<Vec<[f32; 2]>, String> {
    let text = get(&setup.p, "holddown_screws");
    if text.trim().is_empty() {
        let (head, _) = screw_size(setup)?;
        let inset = head.max(CORNER_INSET);
        let [w, d, _] = stock.dimensions();
        return Ok(vec![
            [inset, inset],
            [w - inset, inset],
            [w - inset, d - inset],
            [inset, d - inset],
        ]);
    }
    let mut places = vec![];
    for part in text.split(';').filter(|s| !s.trim().is_empty()) {
        let (x, y) = part
            .split_once(',')
            .ok_or("Write each screw as X,Y from the sheet's front-left corner, for example 10,10; 190,10.")?;
        let read = |v: &str| v.trim().parse::<f32>().ok().filter(|v| v.is_finite());
        let (Some(x), Some(y)) = (read(x), read(y)) else {
            return Err(format!(
                "\"{}\" is not a screw position X,Y in mm.",
                part.trim()
            ));
        };
        places.push([x, y]);
    }
    if places.len() > MAX_SCREWS {
        return Err(format!("List at most {MAX_SCREWS} hold-down screws."));
    }
    Ok(places)
}
/// The screws of `setup` in the stock's frame (mm): their centres.
fn screw_centres(setup: &Setup, stock: Bounds) -> Result<Vec<[f32; 2]>, String> {
    Ok(screw_places(setup, stock)?
        .into_iter()
        .map(|[x, y]| [stock.min[0] + x, stock.min[1] + y])
        .collect())
}
/// What is wrong with how `setup` holds its sheet down, without the part:
/// no hold-down chosen, an unknown one, screws that do not read, a head
/// that is no size, and (with the `stock` known) a screw whose head is not
/// wholly on the sheet. Nothing for a stock that is no sheet.
pub fn holddown_issues(setup: &Setup, stock: Option<Bounds>) -> Vec<String> {
    if !is_sheet(setup) {
        return vec![];
    }
    match holddown(setup) {
        "None" => return vec![NO_HOLDDOWN.into()],
        "Tape" => return vec![],
        "Screws" => {}
        other => {
            return vec![format!(
                "\"{other}\" is not a hold-down: choose Screws or Tape."
            )];
        }
    }
    let size = match screw_size(setup) {
        Ok(size) => size,
        Err(e) => return vec![e],
    };
    let Some(stock) = stock else {
        return match get(&setup.p, "holddown_screws").trim() {
            "" => vec![],
            _ => screw_places(
                setup,
                Bounds {
                    min: [0.; 3],
                    max: [1.; 3],
                },
            )
            .err()
            .into_iter()
            .collect(),
        };
    };
    let places = match screw_places(setup, stock) {
        Ok(places) if places.is_empty() => {
            return vec!["List at least one hold-down screw, or choose Tape.".into()];
        }
        Ok(places) => places,
        Err(e) => return vec![e],
    };
    let r = size.0 / 2.;
    let [w, d, _] = stock.dimensions();
    places
        .iter()
        .enumerate()
        .filter(|(_, [x, y])| *x < r || *y < r || *x > w - r || *y > d - r)
        .map(|(i, [x, y])| {
            format!(
                "Hold-down screw {} at X{} Y{} is not on the {} × {} mm sheet with its Ø{} head: keep it at least {} mm in from every edge.",
                i + 1,
                mm(*x),
                mm(*y),
                mm(w),
                mm(d),
                mm(size.0),
                mm(r)
            )
        })
        .collect()
}
/// The part's outlines seen from above in `setup`'s frame (mm): each outer
/// boundary of its silhouette, holes left out.
fn outlines(setup: &Setup, mesh: &Mesh) -> Vec<Vec<(f64, f64)>> {
    let op = crate::model::Operation::new(crate::model::Strategy::Contour, String::new());
    let Ok(region) = crate::cam::Geometry::new(setup, mesh).silhouette(false, &op) else {
        return vec![];
    };
    let polygons: Vec<crate::cam::Polygon> = region.into();
    polygons
        .into_iter()
        .filter(|p| crate::cam::area(p) > 0.)
        .collect()
}
/// Whether `p` lies inside the closed polygon `poly`.
fn inside(poly: &[(f64, f64)], p: (f64, f64)) -> bool {
    let mut odd = false;
    for (i, a) in poly.iter().enumerate() {
        let b = poly[(i + 1) % poly.len()];
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
            odd = !odd;
        }
    }
    odd
}
/// The distance from `p` to the edges of the closed polygon `poly`.
fn distance(poly: &[(f64, f64)], p: (f64, f64)) -> f64 {
    (0..poly.len())
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy).max(1e-12))
                .clamp(0., 1.);
            (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
        })
        .fold(f64::INFINITY, f64::min)
}
/// The hold-down screws of `setup` that sit on its part (`mesh`), on the
/// `stock` it is cut from: a head inside the part's outline or over its
/// edge, where the screw would go through the part. A screw belongs in the
/// waste outside the outline; whether the cutter reaches it is checked
/// with the toolpaths (`holding::hits`).
pub fn screws_on_part(setup: &Setup, stock: Bounds, mesh: &Mesh) -> Vec<String> {
    if !screwed(setup) {
        return vec![];
    }
    let (Ok((head, _)), Ok(centres)) = (screw_size(setup), screw_centres(setup, stock)) else {
        return vec![];
    };
    let outlines = outlines(setup, mesh);
    let r = f64::from(head) / 2.;
    centres
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let p = (f64::from(c[0]), f64::from(c[1]));
            outlines
                .iter()
                .any(|poly| inside(poly, p) || distance(poly, p) < r)
        })
        .map(|(i, c)| {
            format!(
                "Hold-down screw {} at X{} Y{} from the sheet's front-left corner is on the part: put it in the waste outside the part's outline, its Ø{} head clear of the outline.",
                i + 1,
                mm(c[0] - stock.min[0]),
                mm(c[1] - stock.min[1]),
                mm(head)
            )
        })
        .collect()
}
/// The heads of `setup`'s hold-down screws on `stock`, as holding boxes from
/// the sheet's top up to their height: the 3D view draws them and every
/// move is kept out of them as out of a vise's jaws. None for tape or a
/// stock that is no sheet; a sheet's hold-down refuses with its first issue
/// (`holddown_issues`), a sheet held by nothing too.
pub fn holddown_boxes(setup: &Setup, stock: Bounds) -> Result<Vec<HoldBox>, String> {
    if !is_sheet(setup) {
        return Ok(vec![]);
    }
    if let Some(issue) = holddown_issues(setup, Some(stock)).into_iter().next() {
        return Err(issue);
    }
    if !screwed(setup) {
        return Ok(vec![]);
    }
    let (head, height) = screw_size(setup)?;
    let r = head / 2.;
    Ok(screw_centres(setup, stock)?
        .into_iter()
        .enumerate()
        .map(|(i, c)| HoldBox {
            shape: Default::default(),
            label: format!("hold-down screw {}", i + 1),
            bounds: Bounds {
                min: [c[0] - r, c[1] - r, stock.max[2]],
                max: [c[0] + r, c[1] + r, stock.max[2] + height],
            },
        })
        .collect())
}
/// `setup`'s sheet and hold-down in a sentence, for the setup sheet and the
/// program header; None for a stock that is no sheet. `datum` is the work
/// zero (the setup's frame), which the spoilboard's top is told from.
pub fn sentence(setup: &Setup, stock: Bounds, datum: [f32; 3]) -> Option<String> {
    if !is_sheet(setup) {
        return None;
    }
    let [w, d, t] = stock.dimensions();
    let board = format!(
        "Sheet {} × {} × {} mm on a spoilboard, its top at Z{} (work); cuts go at most {} mm into it.",
        mm(w),
        mm(d),
        mm(t),
        mm(stock.min[2] - datum[2]),
        mm(below(setup))
    );
    let held = match holddown(setup) {
        "Tape" => "Held down by double-sided tape under the sheet.".to_string(),
        "Screws" => match (screw_places(setup, stock), screw_size(setup)) {
            (Ok(places), Ok((head, _))) => format!(
                "Held down by {} screw{} through the waste into the spoilboard (Ø{} heads) at X,Y from the sheet's front-left corner: {}.",
                places.len(),
                if places.len() == 1 { "" } else { "s" },
                mm(head),
                places
                    .iter()
                    .map(|[x, y]| format!("{},{}", mm(*x), mm(*y)))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            _ => "Held down by screws (their places do not read).".into(),
        },
        _ => "No hold-down chosen.".into(),
    };
    Some(format!("{board} {held}"))
}
/// Why `plan`, an operation of `setup`, cuts too deep into the spoilboard
/// under its sheet: deeper than the allowance below the sheet's bottom.
/// None when it does not, and for any other stock.
pub fn floor_issue(setup: &Setup, plan: &crate::simulation::Plan) -> Option<String> {
    if !is_sheet(setup) || plan.is_lathe() {
        return None;
    }
    let lowest = (plan.moves.iter())
        .flat_map(|m| [m.from[2], m.to[2]])
        .fold(f32::INFINITY, f32::min);
    let into = plan.stock.min[2] - lowest;
    (into > below(setup) + 0.001).then(|| {
        format!(
            "This operation goes {} mm into the spoilboard under the sheet, deeper than its {} mm allowance. Reduce the depth or the breakthrough, or raise the allowance.",
            mm(into),
            mm(below(setup))
        )
    })
}
/// Why an outside Contour that cuts through `setup`'s sheet must leave
/// tabs: with screws only the waste is held, so a part cut all the way round
/// comes loose and the cutter can throw it. None for tape, which holds the
/// part itself, and for any other stock.
pub fn loose_part(setup: &Setup) -> Option<&'static str> {
    screwed(setup).then_some(
        "This contour cuts the part free of the sheet all the way round, and the screws hold only the waste: the part comes loose and can be thrown. Turn tabs on, leave a skin with Bottom offset, or hold the sheet with Tape.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Document, Machine, Operation, Strategy, Tool, fields};
    use crate::simulation::{Motion, Plan};

    fn sheet(extra: &[(&str, &str)]) -> Setup {
        let mut s = Setup::new(Machine::new(3), "G54");
        s.p.extend(fields(&[
            ("stock_type", SHEET),
            ("stock_x", "250"),
            ("stock_y", "320"),
        ]));
        s.p.extend(fields(extra));
        s
    }
    fn part() -> Bounds {
        Bounds {
            min: [-98.8, -147.6, 2.],
            max: [98.8, 147.6, 3.5],
        }
    }

    #[test]
    fn a_sheet_lies_on_the_spoilboard_at_the_parts_bottom_and_is_as_thick_as_the_part() {
        let s = sheet(&[]);
        assert!(is_sheet(&s));
        assert!(parameter_issues(&s).is_empty());
        let b = bounds(&s, part()).unwrap();
        assert_eq!(b.min, [-125., -160., 2.]);
        assert_eq!(b.max, [125., 160., 3.5]);
        // A thicker sheet stands above the part; a thinner one is refused.
        let thick = sheet(&[("sheet_thickness", "2")]);
        assert_eq!(bounds(&thick, part()).unwrap().max[2], 4.);
        let thin = sheet(&[("sheet_thickness", "1")]);
        assert!(
            bounds(&thin, part())
                .unwrap_err()
                .contains("at least as thick")
        );
    }

    #[test]
    fn a_cut_may_go_into_the_spoilboard_by_its_allowance_and_no_further() {
        assert_eq!(below(&sheet(&[])), SPOILBOARD_ALLOWANCE);
        assert_eq!(below(&sheet(&[("spoilboard", "0.2")])), 0.2);
        assert_eq!(below(&sheet(&[("spoilboard", "0")])), 0.);
        // Any other stock stands on the table or in the jaws.
        let mut boxed = sheet(&[]);
        boxed.p.insert("stock_type".into(), "Fixed size box".into());
        assert_eq!(below(&boxed), 0.);
        for bad in ["-1", "5", "x"] {
            let s = sheet(&[("spoilboard", bad)]);
            assert_eq!(parameter_issues(&s).len(), 1, "{bad}");
        }
        assert_eq!(parameter_issues(&sheet(&[("stock_x", "0")])).len(), 1);
        assert_eq!(
            parameter_issues(&sheet(&[("sheet_thickness", "-2")])).len(),
            1
        );
    }

    #[test]
    fn a_sheet_needs_a_hold_down_and_screws_must_be_on_the_sheet() {
        let stock = bounds(&sheet(&[]), part()).unwrap();
        assert_eq!(holddown_issues(&sheet(&[]), Some(stock)), vec![NO_HOLDDOWN]);
        assert_eq!(holddown_boxes(&sheet(&[]), stock), Err(NO_HOLDDOWN.into()));
        assert!(holddown_issues(&sheet(&[("holddown", "Tape")]), Some(stock)).is_empty());
        // Tape puts nothing in the cutter's way.
        assert!(
            holddown_boxes(&sheet(&[("holddown", "Tape")]), stock)
                .unwrap()
                .is_empty()
        );
        // Screws default to one in from each corner, by the head's diameter.
        let s = sheet(&[("holddown", "Screws")]);
        assert!(holddown_issues(&s, Some(stock)).is_empty());
        assert_eq!(
            screw_places(&s, stock).unwrap(),
            vec![[12., 12.], [238., 12.], [238., 308.], [12., 308.]]
        );
        let boxes = holddown_boxes(&s, stock).unwrap();
        assert_eq!(boxes.len(), 4);
        assert_eq!(boxes[0].label, "hold-down screw 1");
        assert_eq!(boxes[0].bounds.min, [-119., -154., 3.5]);
        assert_eq!(boxes[0].bounds.max, [-107., -142., 8.5]);
        // Listed screws, one off the sheet's edge.
        let off = sheet(&[
            ("holddown", "Screws"),
            ("holddown_screws", "10,10; 249,100"),
        ]);
        let issues = holddown_issues(&off, Some(stock));
        assert_eq!(issues.len(), 1);
        assert!(
            issues[0]
                .starts_with("Hold-down screw 2 at X249 Y100 is not on the 250 × 320 mm sheet"),
            "{issues:?}"
        );
        assert!(holddown_boxes(&off, stock).is_err());
        let bad = sheet(&[("holddown", "Screws"), ("holddown_screws", "10;10")]);
        assert!(holddown_issues(&bad, None)[0].starts_with("Write each screw as X,Y"));
    }

    #[test]
    fn a_screw_inside_the_parts_outline_or_over_its_edge_is_refused() {
        let square = [(0., 0.), (100., 0.), (100., 50.), (0., 50.)];
        assert!(inside(&square, (50., 25.)));
        assert!(!inside(&square, (150., 25.)));
        assert!((distance(&square, (106., 25.)) - 6.).abs() < 1e-9);
        assert!((distance(&square, (50., 20.)) - 20.).abs() < 1e-9);
    }

    #[test]
    fn screws_alone_leave_a_part_cut_through_without_tabs_loose_and_tape_does_not() {
        assert!(loose_part(&sheet(&[("holddown", "Screws")])).is_some());
        assert!(loose_part(&sheet(&[("holddown", "Tape")])).is_none());
        let s = sheet(&[("holddown", "Screws")]);
        let stock = bounds(&s, part()).unwrap();
        let text = sentence(&s, stock, [0., 0., stock.max[2]]).unwrap();
        assert!(text.starts_with("Sheet 250 × 320 × 1.5 mm on a spoilboard, its top at Z-1.5 (work); cuts go at most 0.5 mm into it. Held down by 4 screws"), "{text}");
    }

    /// The power feed's mains tray, a DXF flat pattern extruded 1.5 mm, on a
    /// 260 × 360 sheet held by `holddown`, with a Ø3 end mill and a Ø4.4
    /// drill: its Ø4.4 holes drilled through, its cut-out and Ø5 holes cut by
    /// a Holes contour, and its outline by an outside contour with 4 tabs,
    /// each 0.3 mm into the spoilboard.
    fn tray(extra: &[(&str, &str)]) -> (Document, Mesh) {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/mains-tray-flat.dxf");
        let mesh = crate::mesh::load_with(&path, Some(1.5)).unwrap();
        let mut doc = Document {
            title: "Mains tray".into(),
            file: Some(crate::model::ModelFile {
                name: "mains-tray-flat.dxf".into(),
                path: path.clone(),
                format: "dxf".into(),
                bytes: 0,
                thickness: Some(1.5),
            }),
            ..Default::default()
        };
        let mut mill = Tool::new("Flat 3", "Flat end mill", "3");
        let mut drill = Tool::new("Drill 4.4", "Drill", "4.4");
        for (n, (t, length, stickout)) in [(&mut mill, "8", "20"), (&mut drill, "24", "35")]
            .into_iter()
            .enumerate()
        {
            t.p.insert("length".into(), length.into());
            t.p.insert("stickout".into(), stickout.into());
            t.p.insert("number".into(), (n + 1).to_string());
        }
        let mut setup = Setup::new(Machine::new(3), "G54");
        setup.p.extend(fields(&[
            ("stock_type", SHEET),
            ("stock_x", "260"),
            ("stock_y", "360"),
            ("holddown", "Screws"),
            ("origin", "Stock top front-left"),
        ]));
        setup.p.extend(fields(extra));
        let top = (mesh.surfaces.iter().enumerate())
            .filter(|(_, s)| s.normal[2] > 0.999)
            .max_by(|a, b| a.1.area.total_cmp(&b.1.area))
            .unwrap()
            .0;
        let op = |strategy, tool: &Tool, keys: &[(&str, &str)]| {
            let mut o = Operation::new(strategy, tool.id.clone());
            o.p.extend(fields(&[
                ("rpm", "12000"),
                ("feed", "480"),
                ("coolant", "Mist"),
            ]));
            o.p.extend(fields(keys));
            o
        };
        let mut holes = op(
            Strategy::Drill,
            &drill,
            &[
                ("name", "Drill"),
                ("cycle", "Drilling"),
                ("through", "Yes"),
                ("breakthrough", "0.4"),
                ("hole_diameter", "4.4"),
            ],
        );
        holes.surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![top],
        });
        let cut = [
            ("stepdown", "0.8"),
            ("bottom", "-0.3"),
            ("finish_passes", "0"),
        ];
        let mut cutouts = op(
            Strategy::Contour,
            &mill,
            &[("name", "Cut-outs"), ("side", "Holes"), ("hole_min", "5")],
        );
        cutouts.p.extend(fields(&cut));
        let mut outline = op(
            Strategy::Contour,
            &mill,
            &[
                ("name", "Outline"),
                ("tabs", "Yes"),
                ("tab_count", "4"),
                ("tab_width", "4"),
                ("tab_height", "0.6"),
            ],
        );
        outline.p.extend(fields(&cut));
        setup.operations = vec![holes, cutouts, outline];
        doc.tools = vec![mill, drill];
        doc.setups = vec![setup];
        (doc, mesh)
    }
    fn plan(doc: &Document, mesh: &Mesh, op: usize) -> Result<Plan, String> {
        let setup = &doc.setups[0];
        let operation = &setup.operations[op];
        let tool = doc.tools.iter().find(|t| t.id == operation.tool).unwrap();
        crate::simulation::generate(setup, operation, tool, mesh)
    }

    #[test]
    fn the_dxf_tray_is_cut_from_a_sheet_with_its_holes_and_a_tabbed_outline_and_exports() {
        let (doc, mesh) = tray(&[]);
        let mm: Vec<f32> = mesh.dimensions.to_vec();
        assert!(
            (mm[0] - 197.535).abs() < 0.01 && (mm[1] - 295.134).abs() < 0.01,
            "{mm:?}"
        );
        assert!((mm[2] - 1.5).abs() < 1e-4);
        let stock = crate::stock::resolve(&doc.setups[0], Some(&mesh))
            .unwrap()
            .stock;
        assert_eq!(
            stock.dimensions().map(|v| (v * 1000.).round() / 1000.),
            [260., 360., 1.5]
        );
        assert!(doc.holding_issues(0, Some(&mesh)).is_empty());
        let lowest = |p: &Plan| {
            (p.moves.iter())
                .flat_map(|m| [m.from[2], m.to[2]])
                .fold(f32::INFINITY, f32::min)
        };
        // The drill and both contours go just into the spoilboard.
        for op in 0..3 {
            let p = plan(&doc, &mesh, op).unwrap();
            let into = stock.min[2] - lowest(&p);
            assert!((0.29..=0.41).contains(&into), "{op}: {into}");
        }
        // The Holes contour goes round the cut-out and the four Ø5 holes:
        // five loops that each start with a plunge, and not the Ø4.4 or Ø3.4
        // holes, narrower than 5 mm.
        let holes = plan(&doc, &mesh, 1).unwrap();
        let xy: std::collections::BTreeSet<(i32, i32)> = (holes.moves.iter())
            .filter(|m| m.cutting_pass)
            .map(|m| ((m.to[0] * 2.).round() as i32, (m.to[1] * 2.).round() as i32))
            .collect();
        assert!(xy.len() > 20);
        let part = crate::stock::resolve(&doc.setups[0], Some(&mesh))
            .unwrap()
            .part
            .unwrap();
        let inside = |c: [f32; 2], r: f32| {
            (holes.moves.iter())
                .filter(|m| m.cutting_pass)
                .any(|m| (m.to[0] - part.min[0] - c[0]).hypot(m.to[1] - part.min[1] - c[1]) < r)
        };
        for c in [[5., 179.], [5., 247.5], [192.535, 179.], [192.535, 247.5]] {
            assert!(inside(c, 2.), "{c:?}");
        }
        // Round the 47.2 × 28.2 cut-out, inside it.
        assert!((holes.moves.iter()).filter(|m| m.cutting_pass).any(|m| {
            let [x, y] = [m.to[0] - part.min[0], m.to[1] - part.min[1]];
            (75.2..122.4).contains(&x) && (260.5..288.8).contains(&y)
        }));
        for c in [[64.77, 5.], [115.27, 87.]] {
            assert!(!inside(c, 2.5), "{c:?}");
        }
        // The outline rises over 4 tabs at each level below their top, 0.6 mm
        // above the sheet's bottom.
        let outline = plan(&doc, &mesh, 2).unwrap();
        let tab_top = stock.min[2] + 0.6;
        let rises: Vec<_> = (outline.moves.iter())
            .filter(|m| m.cutting_pass && m.motion == Motion::Feed)
            .filter(|m| m.from[0] == m.to[0] && m.from[1] == m.to[1] && m.to[2] > m.from[2])
            .collect();
        assert_eq!(rises.len(), 2 * 4, "two levels below the tabs' top");
        assert!(rises.iter().all(|m| (m.to[2] - tab_top).abs() < 1e-4));
        // Exported, the tabs are Z lifts in the program, and its header names
        // the sheet and the screws.
        let job = crate::post::post_job(&doc, &mesh);
        assert!(job.errors.is_empty(), "{:?}", job.errors);
        let programs = job.setups[0].result.as_ref().unwrap();
        let text = &programs[0].text;
        let work_tab = format!("Z{:.3}", tab_top - stock.max[2]);
        assert!(text.contains(&work_tab), "{work_tab}");
        assert!(
            text.contains("Sheet 260 x 360 x 1.5 mm on a spoilboard"),
            "{text}"
        );
        assert!(text.contains("Held down by 4 screws"));
        assert!(text.contains("hold-down screw heads were checked"));
    }

    #[test]
    fn a_hold_down_screw_in_the_cutter_path_or_on_the_part_is_refused() {
        // Beside the tray's left side, 7.2 mm out: off the part, but its Ø12
        // head is in the outline contour's way.
        let (doc, mesh) = tray(&[("holddown_screws", "24,250; 248,348")]);
        assert!(doc.holding_issues(0, Some(&mesh)).is_empty());
        let job = crate::post::post_job(&doc, &mesh);
        let refused = job.setups[0].result.as_ref().unwrap_err();
        assert!(
            refused
                .iter()
                .any(|e| e.contains("hold-down screw 1") && e.starts_with("Outline")),
            "{refused:?}"
        );
        assert!(!refused.iter().any(|e| e.contains("hold-down screw 2")));
        // The UC100 run path checks the same boxes.
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("jy5800.toml");
        std::fs::write(&profile, include_str!("../assets/jy5800.toml")).unwrap();
        let mut uc = doc.clone();
        let machine = &mut uc.setups[0].machine.p;
        machine.insert("controller".into(), "UC100".into());
        machine.insert("uc100_profile".into(), profile.display().to_string());
        // The owner's profile has no mist output.
        for op in &mut uc.setups[0].operations {
            op.p.insert("coolant".into(), "Off".into());
        }
        let mut sims = vec![];
        crate::simulation::sequence(&uc, &mesh, None, |_, oi, r| sims.push((oi, r)));
        let checked =
            crate::post::checked(&uc, &mesh, 0, &sims, crate::post::PostSettings::for_uc100);
        let refused = checked.err().unwrap();
        assert!(
            refused.iter().any(|e| e.contains("hold-down screw 1")),
            "{refused:?}"
        );
        // In the middle of the sheet the screw would go through the part.
        let (doc, mesh) = tray(&[("holddown_screws", "130,180")]);
        let issues = doc.holding_issues(0, Some(&mesh));
        assert!(
            issues[0].starts_with("Hold-down screw 1 at X130 Y180"),
            "{issues:?}"
        );
        assert!(issues[0].contains("is on the part"));
        let job = crate::post::post_job(&doc, &mesh);
        let refused = job.setups[0].result.as_ref().unwrap_err();
        assert!(
            refused.iter().any(|e| e.contains("is on the part")),
            "{refused:?}"
        );
    }

    #[test]
    fn a_cut_deeper_than_the_spoilboard_allowance_is_refused() {
        let (mut doc, mesh) = tray(&[("spoilboard", "0.2")]);
        // 0.3 mm below the sheet, past a 0.2 mm allowance.
        let error = plan(&doc, &mesh, 2).unwrap_err();
        assert!(error.contains("into the spoilboard"), "{error}");
        let error = plan(&doc, &mesh, 0).unwrap_err();
        assert!(
            error.contains("deeper than its 0.2 mm allowance"),
            "{error}"
        );
        doc.setups[0].p.insert("spoilboard".into(), "0.5".into());
        assert!(plan(&doc, &mesh, 2).is_ok());
    }

    #[test]
    fn an_outline_cut_through_without_tabs_is_refused_on_screws_but_not_on_tape() {
        let (mut doc, mesh) = tray(&[]);
        doc.setups[0].operations[2]
            .p
            .insert("tabs".into(), "No".into());
        assert!(
            plan(&doc, &mesh, 2)
                .unwrap_err()
                .contains("the part comes loose")
        );
        doc.setups[0].p.insert("holddown".into(), "Tape".into());
        assert!(plan(&doc, &mesh, 2).is_ok());
        // Tape puts nothing in the way: the export has no boxes to check.
        assert!(crate::post::post_job(&doc, &mesh).setups[0].result.is_ok());
        // A sheet held by nothing is refused.
        doc.setups[0].p.insert("holddown".into(), "None".into());
        let job = crate::post::post_job(&doc, &mesh);
        let refused = job.setups[0].result.as_ref().unwrap_err();
        assert!(
            refused.iter().any(|e| e.ends_with(NO_HOLDDOWN)),
            "{refused:?}"
        );
    }

    #[test]
    fn the_part_report_lists_the_trays_holes_cut_out_sheet_and_screws() {
        let (mut doc, mesh) = tray(&[]);
        crate::proposal::tests::aluminium(&mut doc);
        let report = crate::analysis::report(&doc, &mesh).unwrap();
        let sheet = report.sheet.as_ref().unwrap();
        assert_eq!(
            (sheet.size, sheet.spoilboard_allowance),
            ([260., 360., 1.5], 0.5)
        );
        assert!(sheet.needs_tabs && sheet.holddown == "Screws");
        let holding = report.workholding.as_ref().unwrap();
        assert_eq!((holding.kind, holding.boxes.len()), ("Hold-down screws", 4));
        let top = &report.orientations[0].planes[0];
        assert_eq!(top.kind, "top");
        let mut sizes: Vec<f32> = top.holes.iter().map(|h| h.diameter).collect();
        sizes.sort_by(f32::total_cmp);
        assert_eq!(sizes, [3.4, 3.4, 4.4, 4.4, 5., 5., 5., 5.]);
        assert!(top.holes.iter().all(|h| h.through));
        let cutout = (top.openings.iter())
            .find(|o| o.bbox[2] - o.bbox[0] > 40.)
            .unwrap();
        assert_eq!(
            [
                cutout.bbox[2] - cutout.bbox[0],
                cutout.bbox[3] - cutout.bbox[1]
            ]
            .map(|v| (v * 10.).round() / 10.),
            [47.2, 28.2]
        );
    }

    #[test]
    fn a_plan_for_the_tray_with_tabs_and_holes_contours_passes_the_plan_check() {
        let (mut doc, mesh) = tray(&[]);
        doc.setups[0].operations.clear();
        crate::proposal::tests::aluminium(&mut doc);
        let report = crate::analysis::report(&doc, &mesh).unwrap();
        let plan = |outline: &str| {
            let text = format!(
                r#"{{"summary": "tray", "unmachined": [], "warnings": [], "setups": [{{
                "orientation": "+Z", "name": "Sheet", "origin": "Stock top front-left",
                "workholding": "screws", "holder": null, "operations": [
                {{"strategy": "Drill", "name": "Drill", "tool": 2, "surfaces": [0], "rpm": 12000,
                  "feed": 720, "hole_diameter": 4.4, "through": true, "breakthrough": 0.4,
                  "drill_cycle": "Drilling", "coolant": "Mist", "reason": ""}},
                {{"strategy": "Contour", "name": "Holes", "tool": 1, "surfaces": [], "rpm": 12000,
                  "feed": 480, "contour_mode": "inside_holes", "hole_diameter": 5,
                  "bottom_offset": -0.3, "stepdown": 0.8, "finish_passes": 0,
                  "coolant": "Mist", "reason": ""}},
                {{"strategy": "Contour", "name": "Outline", "tool": 1, "surfaces": [], "rpm": 12000,
                  "feed": 480, "contour_mode": "outside", "stepdown": 0.8, "finish_passes": 0,
                  "coolant": "Mist", "reason": "", {outline}}}]}}]}}"#
            );
            crate::proposal::parse(&text).unwrap()
        };
        let good =
            plan(r#""bottom_offset": -0.3, "tabs": {"count": 4, "width": 4, "height": 0.6}"#);
        let (check, job) = crate::plan_check::check(&doc, &mesh, &report, &good);
        assert!(check.passed, "{:?}", check.problems);
        let job = job.unwrap();
        let ops = &job.setups[0].operations;
        assert_eq!(get(&ops[1].p, "side"), "Holes");
        assert_eq!(get(&ops[1].p, "hole_min"), "5");
        assert_eq!(
            ["tabs", "tab_count", "tab_width", "tab_height"].map(|k| get(&ops[2].p, k)),
            ["Yes", "4", "4", "0.6"]
        );
        // Past the spoilboard's allowance, and without tabs on screws.
        let deep =
            plan(r#""bottom_offset": -0.6, "tabs": {"count": 4, "width": 4, "height": 0.6}"#);
        let (check, _) = crate::plan_check::check(&doc, &mesh, &report, &deep);
        assert!(
            check
                .problems
                .iter()
                .any(|p| p.message.contains("cannot be below -0.5")),
            "{:?}",
            check.problems
        );
        let loose = plan(r#""bottom_offset": -0.3, "tabs": null"#);
        let (check, _) = crate::plan_check::check(&doc, &mesh, &report, &loose);
        assert!(
            check
                .problems
                .iter()
                .any(|p| p.message.contains("comes loose")),
            "{:?}",
            check.problems
        );
    }
}
