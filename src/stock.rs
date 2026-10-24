//! Stock dimensions in millimetres, derived from the same placed mesh as the viewport.
use crate::{
    mesh::{Bounds, Mesh, rotation},
    model::{Setup, get, num},
};

// Negative and positive side of each setup axis; front is -Y.
pub const OFFSETS: [(&str, &str); 6] = [
    ("stock_left", "Left (−X)"),
    ("stock_right", "Right (+X)"),
    ("stock_front", "Front (−Y)"),
    ("stock_back", "Back (+Y)"),
    ("stock_bottom", "Bottom (−Z)"),
    ("stock_top", "Top (+Z)"),
];
/// The offsets of a relative round bar: radial, then along the setup's Z.
pub const ROUND_OFFSETS: [(&str, &str); 3] = [
    ("stock_radial", "Radial"),
    ("stock_bottom", "Bottom (−Z)"),
    ("stock_top", "Top (+Z)"),
];
/// The stock modes of the first setup: a round bar along Z, or round the
/// part, on most machines (`is_round`), or a sheet on a spoilboard
/// (`sheet`)…
pub const MODES: [&str; 5] = [
    "Relative size box",
    "Fixed size box",
    "Round bar",
    "Relative round bar",
    crate::sheet::SHEET,
];
/// …and a round bar along X on a machine with a rotary A axis about X
/// (`round_bar`), to hold in the chuck on A.
pub const ROTARY_MODES: [&str; 4] = [
    "Relative size box",
    "Fixed size box",
    "Round bar",
    crate::sheet::SHEET,
];
/// The stock modes `setup`'s machine offers.
pub fn modes(setup: &Setup) -> &'static [&'static str] {
    if setup.machine.has_a() {
        &ROTARY_MODES
    } else {
        &MODES
    }
}
pub fn relative(setup: &Setup) -> bool {
    get(&setup.p, "stock_type") == "Relative size box"
}
/// Whether the setup's stock is a round bar along its Z axis, X = Y = 0:
/// "Round bar" (`stock_diameter` × `stock_length`) or "Relative round bar"
/// (`stock_radial` round the part, `stock_top` and `stock_bottom` along
/// the axis). Its bounds are the bar's box (`bar`). A "Round bar" on a
/// machine with a rotary A axis lies along X instead (`round_bar`).
pub fn is_round(setup: &Setup) -> bool {
    match get(&setup.p, "stock_type") {
        "Round bar" => !setup.machine.has_a(),
        "Relative round bar" => true,
        _ => false,
    }
}
/// How far below the stock's bottom a cut may go in `setup`: into a
/// sheet's spoilboard (`sheet::below`), else nowhere.
pub fn below(setup: &Setup) -> f32 {
    crate::sheet::below(setup)
}
fn relative_round(setup: &Setup) -> bool {
    get(&setup.p, "stock_type") == "Relative round bar"
}
/// The axis (X, Y) and radius of the round bar whose box is `bounds`: the
/// cylinder along Z in the middle of the box, as wide as its narrower side.
pub fn bar(bounds: Bounds) -> ([f32; 2], f32) {
    let d = bounds.dimensions();
    (
        [
            (bounds.min[0] + bounds.max[0]) / 2.,
            (bounds.min[1] + bounds.max[1]) / 2.,
        ],
        d[0].min(d[1]) / 2.,
    )
}
/// The part's largest distance from `setup`'s Z axis (X = Y = 0), in mm.
pub fn part_radius(setup: &Setup, mesh: &Mesh) -> f32 {
    let placement = Placement::new(mesh, Some(setup));
    let scale = mesh.display_scale();
    mesh.faces
        .iter()
        .flat_map(|f| &f.points)
        .map(|&p| {
            let q = placement.transform(p);
            q[0].hypot(q[1]) / scale
        })
        .fold(0., f32::max)
}
/// Whether the setup's stock is a round bar along X, on a machine with a
/// rotary A axis about X (`Machine::has_a`), to hold in the chuck on A: a
/// cylinder of `stock_diameter` and `stock_length`, centred on X and Y and
/// standing on Z0 like a fixed box, the box round it being the stock's box
/// everywhere. Elsewhere a "Round bar" stands along Z (`is_round`).
pub fn round_bar(setup: &Setup) -> bool {
    get(&setup.p, "stock_type") == "Round bar" && setup.machine.has_a()
}
/// The round bar a first setup's `stock` box holds, in its frame: its axis
/// (Y, Z) and radius. None for a box.
pub fn round(setup: &Setup, stock: Bounds) -> Option<Round> {
    round_bar(setup).then(|| Round {
        axis: [
            (stock.min[1] + stock.max[1]) / 2.,
            (stock.min[2] + stock.max[2]) / 2.,
        ],
        radius: (stock.max[1] - stock.min[1]).min(stock.max[2] - stock.min[2]) / 2.,
    })
}
/// A round bar along X: its axis's Y and Z and its radius, mm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Round {
    pub axis: [f32; 2],
    pub radius: f32,
}
impl Round {
    /// Whether `p` (the bar's frame) lies inside it, ignoring its ends.
    pub fn contains(self, p: [f32; 3]) -> bool {
        (p[1] - self.axis[0]).hypot(p[2] - self.axis[1]) <= self.radius
    }
}
/// Why a "Relative round bar" is refused on a machine with a rotary A axis,
/// which does not offer it (`modes`): it stands along the setup's Z, which
/// the chuck on A (along X) would hold as its box.
pub const NO_RELATIVE_BAR_ON_A: &str = "A Relative round bar stands along the setup's Z, which the chuck on this machine's rotary A axis does not hold: choose Round bar (along X), or a box.";
pub fn parameter_issues(setup: &Setup) -> Vec<String> {
    if relative_round(setup) && setup.machine.has_a() {
        vec![NO_RELATIVE_BAR_ON_A.into()]
    } else if relative_round(setup) {
        ROUND_OFFSETS
            .iter()
            .filter(|&(key, _label)| !num(&setup.p, key).is_some_and(|v| v >= 0.))
            .map(|(_key, label)| {
                format!("{label} stock offset must be a finite number of 0 mm or more.")
            })
            .collect()
    } else if get(&setup.p, "stock_type") == "Round bar" {
        [
            ("stock_diameter", "Bar diameter"),
            ("stock_length", "Bar length"),
        ]
        .iter()
        .filter(|&(key, _label)| !num(&setup.p, key).is_some_and(|v| v > 0.))
        .map(|(_key, label)| format!("{label} must be a finite number greater than zero."))
        .collect()
    } else if relative(setup) {
        OFFSETS
            .iter()
            .filter(|&(key, _label)| !num(&setup.p, key).is_some_and(|v| v >= 0.))
            .map(|(_key, label)| {
                format!("{label} stock offset must be a finite number of 0 mm or more.")
            })
            .collect()
    } else if crate::sheet::is_sheet(setup) {
        crate::sheet::parameter_issues(setup)
    } else if get(&setup.p, "stock_type") == "Fixed size box" {
        ["stock_x", "stock_y", "stock_z"]
            .iter()
            .filter(|&key| !num(&setup.p, key).is_some_and(|v| v > 0.))
            .map(|key| {
                format!(
                    "{} must be a finite number greater than zero.",
                    key.replace('_', " ")
                )
            })
            .collect()
    } else {
        vec!["Choose a stock mode.".into()]
    }
}

pub struct Placement {
    angles: [f32; 3],
    shift: [f32; 3],
    /// Bounds after part placement, in display coordinates.
    pub bounds: Bounds,
}
impl Placement {
    /// The part placed as `setup` places it, turned by A when the setup is
    /// indexed (`Setup::turn`): its base's placement turned about the A
    /// centreline.
    pub fn new(mesh: &Mesh, setup: Option<&Setup>) -> Self {
        let placed = Self::unturned(mesh, setup);
        let Some(t) = setup.and_then(|s| s.turn) else {
            return placed;
        };
        let axis = t.axis.map(|v| v * mesh.display_scale());
        let angles = crate::indexed::turned_angles(placed.angles, t.turn);
        let shift = crate::indexed::turn_point(placed.shift, axis, t.turn);
        Self {
            angles,
            shift,
            bounds: mesh.rotated_bounds(angles).translated(shift),
        }
    }
    /// The part placed by `setup`'s own rotation and translation: an
    /// indexed setup's base's placement, before A turns it.
    pub fn unturned(mesh: &Mesh, setup: Option<&Setup>) -> Self {
        let angles = ["rx", "ry", "rz"].map(|k| setup.and_then(|s| num(&s.p, k)).unwrap_or(0.));
        let t = ["tx", "ty", "tz"]
            .map(|k| setup.and_then(|s| num(&s.p, k)).unwrap_or(0.) * mesh.display_scale());
        let bounds = mesh.rotated_bounds(angles);
        let shift = [t[0], t[1], 4. - bounds.min[2] + t[2]];
        Self {
            angles,
            shift,
            bounds: bounds.translated(shift),
        }
    }
    /// The rotation angles and then the shift `transform` applies.
    pub fn parts(&self) -> ([f32; 3], [f32; 3]) {
        (self.angles, self.shift)
    }
    /// The part's rotation (`mesh::rotation`'s angles), A's turn included.
    pub fn rotation(&self) -> [f32; 3] {
        self.angles
    }
    pub fn transform(&self, point: [f32; 3]) -> [f32; 3] {
        let r = rotation(point, self.angles);
        std::array::from_fn(|i| r[i] + self.shift[i])
    }
}

pub struct ResolvedStock {
    pub part: Option<Bounds>,
    pub stock: Bounds,
    pub display_scale: f32,
    /// For a round bar, the part's largest distance from the bar's axis
    /// (`part_radius`), when the part is known.
    pub part_radius: Option<f32>,
}
pub fn resolve(setup: &Setup, mesh: Option<&Mesh>) -> Result<ResolvedStock, String> {
    if let Some(error) = parameter_issues(setup).into_iter().next() {
        return Err(error);
    }
    if ["rx", "ry", "rz", "tx", "ty", "tz"]
        .iter()
        .any(|k| num(&setup.p, k).is_none())
    {
        return Err("Enter valid part rotation and translation values to size the stock.".into());
    }
    let scale = mesh.map(Mesh::display_scale).unwrap_or(1.);
    let part = mesh.map(|m| Placement::new(m, Some(setup)).bounds.scaled(1. / scale));
    let part_radius = mesh
        .filter(|_| is_round(setup))
        .map(|m| part_radius(setup, m));
    let stock = if let Some(bounds) = setup.preview_stock {
        bounds
    } else if relative_round(setup) {
        let part = part.ok_or("Load the part to calculate relative stock dimensions.")?;
        let r = part_radius.unwrap() + num(&setup.p, "stock_radial").unwrap();
        Bounds {
            min: [-r, -r, part.min[2] - num(&setup.p, "stock_bottom").unwrap()],
            max: [r, r, part.max[2] + num(&setup.p, "stock_top").unwrap()],
        }
    } else if round_bar(setup) {
        let [d, l] = ["stock_diameter", "stock_length"].map(|k| num(&setup.p, k).unwrap());
        Bounds {
            min: [-l / 2., -d / 2., 0.],
            max: [l / 2., d / 2., d],
        }
    } else if get(&setup.p, "stock_type") == "Round bar" {
        let [d, l] = ["stock_diameter", "stock_length"].map(|k| num(&setup.p, k).unwrap());
        Bounds {
            min: [-d / 2., -d / 2., 0.],
            max: [d / 2., d / 2., l],
        }
    } else if crate::sheet::is_sheet(setup) {
        let part = part.ok_or("Load the part to place it on the sheet.")?;
        crate::sheet::bounds(setup, part)?
    } else if relative(setup) {
        let part = part.ok_or("Load the part to calculate relative stock dimensions.")?;
        let mut stock = part;
        for axis in 0..3 {
            stock.min[axis] -= num(&setup.p, OFFSETS[axis * 2].0).unwrap();
            stock.max[axis] += num(&setup.p, OFFSETS[axis * 2 + 1].0).unwrap();
        }
        stock
    } else {
        let [x, y, z] = ["stock_x", "stock_y", "stock_z"].map(|k| num(&setup.p, k).unwrap());
        Bounds {
            min: [-x / 2., -y / 2., 0.],
            max: [x / 2., y / 2., z],
        }
    };
    if stock.min.iter().chain(&stock.max).any(|v| !v.is_finite())
        || stock
            .dimensions()
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.)
    {
        return Err("Stock must have finite, positive dimensions on all three axes.".into());
    }
    Ok(ResolvedStock {
        part,
        stock,
        display_scale: scale,
        part_radius,
    })
}
pub fn dimensions_text(d: [f32; 3]) -> String {
    format!("{:.3} × {:.3} × {:.3} mm", d[0], d[1], d[2])
}
/// A round bar's size from its box: "Ø50.000 × 100.000 mm".
pub fn bar_text(stock: Bounds) -> String {
    format!(
        "Ø{:.3} × {:.3} mm",
        2. * bar(stock).1,
        stock.dimensions()[2]
    )
}
pub fn sheet_description(setup: &Setup, mesh: Option<&Mesh>) -> String {
    let mut text = format!("Stock mode: {}\n\n", get(&setup.p, "stock_type"));
    if relative(setup) {
        text += "Stock offsets from the part along setup axes:\n\n";
        for (key, label) in OFFSETS {
            text += &format!("- {label}: {} mm\n", get(&setup.p, key));
        }
        text += "\n";
    } else if relative_round(setup) {
        text += "Round bar offsets from the part, round the setup Z axis and along it:\n\n";
        for (key, label) in ROUND_OFFSETS {
            text += &format!("- {label}: {} mm\n", get(&setup.p, key));
        }
        text += "\n";
    }
    if crate::sheet::is_sheet(setup) {
        text += &format!(
            "Sheet on a spoilboard: cuts go at most {} mm into the board. Hold-down: {}.\n\n",
            crate::sheet::below(setup),
            crate::sheet::holddown(setup)
        );
    }
    if round_bar(setup) {
        text += &format!(
            "Round bar along X: Ø{} × {} mm\n\n",
            get(&setup.p, "stock_diameter"),
            get(&setup.p, "stock_length")
        );
    }
    match resolve(setup, mesh) {
        Ok(result) if is_round(setup) && setup.preview_stock.is_none() => {
            text += &format!("Round bar Ø × length: {}", bar_text(result.stock))
        }
        Ok(result) => {
            text += &format!(
                "Stock X × Y × Z: {}",
                dimensions_text(result.stock.dimensions())
            )
        }
        Err(error) => text += &format!("Stock dimensions unavailable: {error}"),
    }
    if mesh.is_some_and(|m| m.units != "mm") {
        text += "\n\nSTL/OBJ source coordinates are treated as mm for stock sizing.";
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Document, Machine};
    fn fixture() -> Mesh {
        crate::mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/four-setup-block.3mf"),
        )
        .unwrap()
    }
    fn close(a: [f32; 3], b: [f32; 3]) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 0.001, "{a:?} != {b:?}");
        }
    }
    #[test]
    fn independent_offsets_expand_only_the_requested_part_side() {
        let mesh = fixture();
        let mut s = Setup::new(Machine::new(3), "G54");
        let part = resolve(&s, Some(&mesh)).unwrap().part.unwrap();
        close(part.dimensions(), [120., 80., 40.]);
        for (index, (key, _)) in OFFSETS.iter().enumerate() {
            s.p.insert((*key).into(), "2".into());
            let actual = resolve(&s, Some(&mesh)).unwrap().stock;
            let mut expected = part;
            if index % 2 == 0 {
                expected.min[index / 2] -= 2.;
            } else {
                expected.max[index / 2] += 2.;
            }
            close(actual.min, expected.min);
            close(actual.max, expected.max);
            s.p.insert((*key).into(), "0".into());
        }
        s.p.insert("stock_top".into(), "10".into());
        s.p.insert("stock_right".into(), "2".into());
        let result = resolve(&s, Some(&mesh)).unwrap();
        close(result.stock.dimensions(), [122., 80., 50.]);
        close(result.stock.min, part.min);
    }
    #[test]
    fn rotated_and_translated_stock_matches_the_preview_scale() {
        let mesh = fixture();
        let mut s = Setup::new(Machine::new(3), "G54");
        for (key, val) in [
            ("ry", "90"),
            ("tx", "200"),
            ("tz", "-3"),
            ("stock_top", "10"),
            ("stock_right", "2"),
        ] {
            s.p.insert(key.into(), val.into());
        }
        let resolved = resolve(&s, Some(&mesh)).unwrap();
        close(resolved.stock.dimensions(), [42., 80., 130.]);
        let placed = Placement::new(&mesh, Some(&s));
        let displayed_stock = resolved.stock.scaled(resolved.display_scale);
        close(displayed_stock.min, placed.bounds.min);
        close(
            displayed_stock.max,
            [
                placed.bounds.max[0] + 2. * resolved.display_scale,
                placed.bounds.max[1],
                placed.bounds.max[2] + 10. * resolved.display_scale,
            ],
        );
        for face in &mesh.faces {
            for point in &face.points {
                let point = placed.transform(*point);
                for (axis, &v) in point.iter().enumerate() {
                    assert!(v >= displayed_stock.min[axis] - 0.001);
                    assert!(v <= displayed_stock.max[axis] + 0.001);
                }
            }
        }
        s.p.insert("tx".into(), "0".into());
        let unshifted = resolve(&s, Some(&mesh)).unwrap();
        assert!((resolved.stock.min[0] - unshifted.stock.min[0] - 200.).abs() < 0.001);
    }
    #[test]
    fn invalid_offsets_and_unavailable_geometry_never_use_fallback_stock() {
        let mesh = fixture();
        let mut s = Setup::new(Machine::new(3), "G54");
        assert!(resolve(&s, None).err().unwrap().contains("Load the part"));
        for value in ["", "-1", "NaN", "inf", "abc"] {
            s.p.insert("stock_top".into(), value.into());
            assert!(resolve(&s, Some(&mesh)).is_err());
            assert!(!s.issues().is_empty());
        }
        s.p.insert("stock_top".into(), "3e38".into());
        s.p.insert("stock_bottom".into(), "3e38".into());
        assert!(resolve(&s, Some(&mesh)).is_err());
        s.p.insert("stock_type".into(), "Fixed size box".into());
        close(
            resolve(&s, None).unwrap().stock.dimensions(),
            [150., 104., 40.],
        );
        s.p.insert("stock_x".into(), "".into());
        assert!(resolve(&s, Some(&mesh)).is_err());
    }
    #[test]
    fn a_round_bar_resolves_to_its_box_on_the_setup_axis() {
        let mesh = fixture();
        let mut s = Setup::new(Machine::new(3), "G54");
        assert!(!is_round(&s));
        assert_eq!(resolve(&s, Some(&mesh)).unwrap().part_radius, None);
        s.p.insert("stock_type".into(), "Round bar".into());
        assert!(is_round(&s));
        // Ø50 × 100 by default, from Z 0 up, on X 0 Y 0.
        let bar_stock = resolve(&s, None).unwrap().stock;
        close(bar_stock.min, [-25., -25., 0.]);
        close(bar_stock.max, [25., 25., 100.]);
        assert_eq!(bar(bar_stock), ([0., 0.], 25.));
        assert_eq!(bar_text(bar_stock), "Ø50.000 × 100.000 mm");
        assert!(parameter_issues(&s).is_empty());
        for (key, bad) in [("stock_diameter", "0"), ("stock_length", "-1")] {
            let mut wrong = s.clone();
            wrong.p.insert(key.into(), bad.into());
            assert!(resolve(&wrong, None).is_err());
            assert_eq!(parameter_issues(&wrong).len(), 1);
        }
        // The fixture is a 120 × 80 × 40 block, its corners rounded, centred
        // on the axis: no point of it lies further than √(60² + 40²) from it.
        let resolved = resolve(&s, Some(&mesh)).unwrap();
        let corner = resolved.part_radius.unwrap();
        assert!(
            corner > 60. && corner < 60f32.hypot(40.) + 0.001,
            "{corner}"
        );
        let placed = Placement::new(&mesh, Some(&s));
        let farthest = (mesh.faces.iter().flat_map(|f| &f.points))
            .map(|p| {
                let q = placed.transform(*p);
                q[0].hypot(q[1]) / mesh.display_scale()
            })
            .fold(0., f32::max);
        assert!((farthest - corner).abs() < 0.001);
        s.p.insert("stock_type".into(), "Relative round bar".into());
        assert!(is_round(&s));
        for (key, value) in [
            ("stock_radial", "2"),
            ("stock_top", "5"),
            ("stock_bottom", "1"),
        ] {
            s.p.insert(key.into(), value.into());
        }
        let resolved = resolve(&s, Some(&mesh)).unwrap();
        let part = resolved.part.unwrap();
        let r = corner + 2.;
        close(resolved.stock.min, [-r, -r, part.min[2] - 1.]);
        close(resolved.stock.max, [r, r, part.max[2] + 5.]);
        assert!(resolve(&s, None).err().unwrap().contains("Load the part"));
        s.p.insert("stock_radial".into(), "-1".into());
        assert!(parameter_issues(&s)[0].starts_with("Radial"));
        s.p.insert("stock_radial".into(), "2".into());
        let text = sheet_description(&s, Some(&mesh));
        assert!(text.contains("Radial: 2 mm") && text.contains("Round bar Ø × length: Ø"));
    }
    #[test]
    fn a_round_bar_is_its_box_along_x_and_its_sizes_must_be_positive() {
        // Along X on a machine with a rotary A axis.
        let mut s = Setup::new(Machine::new(4), "G54");
        for (k, v) in [
            ("stock_type", "Round bar"),
            ("stock_diameter", "30"),
            ("stock_length", "120"),
        ] {
            s.p.insert(k.into(), v.into());
        }
        assert!(round_bar(&s) && parameter_issues(&s).is_empty());
        let stock = resolve(&s, None).unwrap().stock;
        close(stock.min, [-60., -15., 0.]);
        close(stock.max, [60., 15., 30.]);
        let bar = round(&s, stock).unwrap();
        assert_eq!((bar.axis, bar.radius), ([0., 15.], 15.));
        assert!(bar.contains([0., 10., 25.]) && !bar.contains([0., 14., 29.]));
        assert!(sheet_description(&s, None).contains("Round bar along X: Ø30 × 120 mm"));
        for (key, bad) in [
            ("stock_diameter", "0"),
            ("stock_length", "-3"),
            ("stock_diameter", "x"),
        ] {
            let mut t = s.clone();
            t.p.insert(key.into(), bad.into());
            assert_eq!(parameter_issues(&t).len(), 1, "{key} {bad}");
            assert!(resolve(&t, None).is_err());
        }
        assert!(!is_round(&s) && modes(&s) == ROTARY_MODES);
        // A Relative round bar saved on another machine is refused here.
        let mut relative = s.clone();
        relative
            .p
            .insert("stock_type".into(), "Relative round bar".into());
        assert_eq!(parameter_issues(&relative), [NO_RELATIVE_BAR_ON_A]);
        assert!(resolve(&relative, None).is_err());
        // On a 3-axis mill the same bar stands along Z.
        let mut upright = s.clone();
        upright.machine = Machine::new(3);
        assert!(!round_bar(&upright) && is_round(&upright) && modes(&upright) == MODES);
        let along_z = resolve(&upright, None).unwrap().stock;
        close(along_z.min, [-15., -15., 0.]);
        close(along_z.max, [15., 15., 120.]);
        // A box is no bar.
        s.p.insert("stock_type".into(), "Fixed size box".into());
        assert_eq!(round(&s, stock), None);
    }
    #[test]
    fn sheet_uses_relative_dimensions_and_all_six_offsets() {
        let mesh = fixture();
        let mut d = Document::sample();
        d.setups[0] = Setup::new(d.machines[0].clone(), "G54");
        d.active = d.setups[0].id.clone();
        d.setup_mut().p.insert("stock_top".into(), "10".into());
        d.setup_mut().p.insert("stock_right".into(), "2".into());
        let text = d.setup_sheet(Some(&mesh));
        assert!(text.contains("122.000 × 80.000 × 50.000 mm"));
        for (_, side) in OFFSETS {
            assert!(text.contains(side));
        }
        assert!(!d.setup_sheet(None).contains("150 × 104"));
    }
}
