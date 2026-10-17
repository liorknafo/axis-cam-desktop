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
pub fn relative(setup: &Setup) -> bool {
    get(&setup.p, "stock_type") == "Relative size box"
}
/// Whether the setup's stock is a round bar along its Z axis, X = Y = 0:
/// "Round bar" (`stock_diameter` × `stock_length`) or "Relative round bar"
/// (`stock_radial` round the part, `stock_top` and `stock_bottom` along
/// the axis). Its bounds are the bar's box (`bar`).
pub fn is_round(setup: &Setup) -> bool {
    matches!(
        get(&setup.p, "stock_type"),
        "Round bar" | "Relative round bar"
    )
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
pub fn parameter_issues(setup: &Setup) -> Vec<String> {
    if relative_round(setup) {
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
    pub fn new(mesh: &Mesh, setup: Option<&Setup>) -> Self {
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
    } else if get(&setup.p, "stock_type") == "Round bar" {
        let [d, l] = ["stock_diameter", "stock_length"].map(|k| num(&setup.p, k).unwrap());
        Bounds {
            min: [-d / 2., -d / 2., 0.],
            max: [d / 2., d / 2., l],
        }
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
