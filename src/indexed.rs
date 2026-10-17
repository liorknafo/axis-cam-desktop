//! Indexed setups on the rotary A axis (LIO-188). A later setup whose
//! Reposition action is "Index rotary axis" does not re-clamp the stock: A
//! turns it, in the clamping of the setup before it that is not indexed
//! itself (its base: setup 1, or a later true re-clamp). Its frame is the
//! base's frame turned about the A centreline by the setup's A angle, its
//! stock is what the setups before it leave, and its work zero, its work
//! offset, its machine and its workholding are the base's: nothing is
//! probed again. A setup may list several A angles for the same operations
//! (a pattern of index steps, 0, 90, 180, 270); the 3D view shows the first.
//!
//! The setup's frame keeps the machine's axes, as `chuck` has it: X along
//! the A axis, the A centreline through the middle of the Y and Z of the
//! stock the base starts from. Turning about it leaves that line, and a
//! work zero on it, where they are.
//!
//! THE SIGN OF A. Positive A turns the part by the right-hand rule about
//! +X, as LinuxCNC has it: a point on +Y of the centreline goes up to +Z at
//! A90, and +Z goes to -Y. Seen from the chuck looking along +X (towards
//! the tailstock), positive A turns the part clockwise. The UC100 driver's
//! simulated bar ("positive A turns +Y toward +Z"), the Control page's A
//! zero on a flat and the chuck's solids (`chuck::Rig::solids`) turn the
//! same way. A machine whose A direction is Negative turns it the other way.
use crate::mesh::{Bounds, Mesh, rotation};
use crate::model::{Document, Machine, Setup, get, num};

/// The Reposition action of an indexed setup.
pub const ACTION: &str = "Index rotary axis";
/// The setup field that lists its A angles, degrees. A setup whose action
/// is `ACTION` without it is from a project saved before indexing turned
/// the stock: the action is a label there, and the setup keeps its own
/// placement, as it did.
pub const ANGLES: &str = "a_angles";
/// What a setup indexed from the Reposition choice starts with.
pub const DEFAULT_ANGLES: &str = "90";
/// The work zeros for work on A: X at the stock's free (+X) end or at the
/// chuck's face, Y and Z on the A centreline. A0 is where the Control
/// page's rotary calibration (Find A zero on a flat) put it.
pub const ORIGINS: [&str; 2] = ["A axis at stock end", "A axis at chuck face"];
/// The fields of a setup that are its own when it is indexed: everything
/// else (placement, work zero and offset, workholding) is its base's.
const OWN: [&str; 5] = ["name", "notes", "reposition", ANGLES, "stock_source"];

/// Whether `setup` is indexed: its Reposition action is `ACTION` and it
/// lists A angles.
pub fn is_indexed(setup: &Setup) -> bool {
    get(&setup.p, "reposition") == ACTION && setup.p.contains_key(ANGLES)
}
/// Whether the Reposition action of `setup` is `ACTION` from a project
/// saved before indexing turned the stock: a label only.
pub fn is_label_only(setup: &Setup) -> bool {
    get(&setup.p, "reposition") == ACTION && !setup.p.contains_key(ANGLES)
}
/// The A angles `setup` lists, degrees, in order: separated by commas,
/// slashes, semicolons or spaces ("0, 90, 180, 270").
pub fn angles(setup: &Setup) -> Result<Vec<f32>, String> {
    let text = get(&setup.p, ANGLES);
    let mut out: Vec<f32> = vec![];
    for word in text
        .split(|c: char| c == ',' || c == '/' || c == ';' || c.is_whitespace())
        .filter(|w| !w.is_empty())
    {
        let a = word
            .trim_end_matches('°')
            .parse::<f32>()
            .ok()
            .filter(|a| a.is_finite())
            .ok_or_else(|| {
                format!("\"{word}\" is not an A angle: list them in degrees, as 0, 90, 180, 270.")
            })?;
        if out.iter().any(|b| (a - b).abs() < 1e-3) {
            return Err(format!("A {}° is listed twice.", degrees(a)));
        }
        out.push(a);
    }
    if out.is_empty() {
        return Err("List the A angles to index to, in degrees: 90, or 0, 90, 180, 270.".into());
    }
    Ok(out)
}
/// `a` degrees for a message, without a trailing ".0".
pub fn degrees(a: f32) -> String {
    let s = format!("{a:.3}");
    match s.trim_end_matches('0').trim_end_matches('.') {
        "-0" => "0".into(),
        t => t.into(),
    }
}
/// The A angles of `v` in words: "A 90°", "A 0°, 90°, 180°, 270°".
pub fn angles_text(v: &[f32]) -> String {
    let list: Vec<String> = v.iter().map(|a| format!("{}°", degrees(*a))).collect();
    format!("A {}", list.join(", "))
}
/// The A angle of `setup` for the setups list, the Job view and the Run
/// panel: its listed angles when it is indexed, A 0° for another setup on
/// a machine with an A axis, None without one.
pub fn label(setup: &Setup) -> Option<String> {
    if is_indexed(setup) {
        return Some(match angles(setup) {
            Ok(v) => angles_text(&v),
            Err(_) => "A ?".into(),
        });
    }
    setup.machine.has_a().then(|| "A 0°".into())
}
/// +1 when the machine's positive A turns the part by the right-hand rule
/// about +X (direction Positive), -1 for Negative.
pub fn sign(m: &Machine) -> f32 {
    if get(&m.p, "direction") == "Negative" {
        -1.
    } else {
        1.
    }
}
/// The point `p` of a setup's frame turned with the part about the line
/// along X through Y, Z `axis` by `turn` degrees, by the right-hand rule
/// about +X: +Y towards +Z for a positive turn.
pub fn turn_point(p: [f32; 3], axis: [f32; 2], turn: f32) -> [f32; 3] {
    let (s, c) = turn.to_radians().sin_cos();
    let (y, z) = (p[1] - axis[0], p[2] - axis[1]);
    [p[0], axis[0] + y * c - z * s, axis[1] + y * s + z * c]
}
/// The angles (`mesh::rotation`'s, X then Y then Z) that turn as `angles`
/// do followed by `turn` degrees about +X.
pub fn turned_angles(angles: [f32; 3], turn: f32) -> [f32; 3] {
    // The columns of the whole rotation: where it takes X, Y and Z.
    let col = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
        .map(|e| turn_point(rotation(e, angles), [0., 0.], turn));
    let m = |r: usize, c: usize| col[c][r];
    // rotation(p, [a, b, c]) is Rz(c) Ry(b) Rx(a) p: its row 2 is
    // (-sin b, cos b sin a, cos b cos a), its column 0 (cos c cos b,
    // sin c cos b, -sin b).
    let b = (-m(2, 0)).clamp(-1., 1.).asin();
    let (a, c) = if m(2, 0).abs() < 1. - 1e-6 {
        (m(2, 1).atan2(m(2, 2)), m(1, 0).atan2(m(0, 0)))
    } else {
        // Y at ±90°: X and Z turn about the same line; Z is taken as 0.
        ((-m(1, 2)).atan2(m(1, 1)), 0.)
    };
    [a, b, c].map(f32::to_degrees)
}
/// How an indexed setup's part is turned from its base's placement, as
/// `Document::sync_turns` works it out. Not saved: it follows from the job.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Turn {
    /// The base setup: the clamping A turns the stock in.
    pub base: usize,
    /// The setup's first A angle, degrees, and what it turns the part
    /// about +X (`sign` × A).
    pub a: f32,
    pub turn: f32,
    /// The A centreline in the base's frame: Y and Z, mm.
    pub axis: [f32; 2],
    /// The box the base starts from, mm, in its frame.
    pub stock: Bounds,
}
/// The A centreline in a setup's frame for the `stock` box it starts
/// from: the middle of its Y and Z, where the chuck centres it.
pub fn axis_of(stock: Bounds) -> [f32; 2] {
    [
        (stock.min[1] + stock.max[1]) / 2.,
        (stock.min[2] + stock.max[2]) / 2.,
    ]
}
/// Where the work zero `origin` of `ORIGINS` is in a setup's frame: X at
/// the free end of `stock` (the box the clamping starts from) or the chuck's
/// face, Y and Z on `axis`. None for another work zero.
pub fn rotary_datum(
    setup: &Setup,
    origin: &str,
    stock: Bounds,
    axis: [f32; 2],
) -> Option<Result<[f32; 3], String>> {
    let x = match ORIGINS.iter().position(|o| *o == origin)? {
        0 => Ok(stock.max[0]),
        _ if !crate::chuck::holds(setup) => Err(
            "A axis at chuck face needs the stock held in the chuck: choose Chuck under Modelled workholding, or A axis at stock end.".to_string(),
        ),
        _ => crate::chuck::rig(setup, stock, false).map(|r| r.face),
    };
    Some(x.map(|x| [x, axis[0], axis[1]]))
}
/// How the operator finds a work zero of `ORIGINS`, for the program header
/// and the Work zero step. None for another work zero.
pub fn zero_words(origin: &str) -> Option<&'static str> {
    match ORIGINS.iter().position(|o| *o == origin)? {
        0 => Some(
            "Y0 Z0 on the A centreline from the rotary calibration (Rotary A on the Control page); touch off X0 on the stock's free end face; A0 where Find A zero on a flat put it.",
        ),
        _ => Some(
            "Y0 Z0 on the A centreline from the rotary calibration (Rotary A on the Control page); X0 on the chuck's face; A0 where Find A zero on a flat put it.",
        ),
    }
}
/// A setup at one A angle, for running its operations there: the angle,
/// how a point of the part comes into the setup's frame turned to it, and
/// where the work zero is, in that frame and on the machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtAngle {
    /// The A angle, degrees, as the machine is commanded.
    pub a: f32,
    /// From the part's own frame (its file's mm) to the setup's frame with
    /// the stock turned to `a`.
    pub frame: crate::volume::Frame,
    /// The work zero in the setup's frame (mm): the same at every angle.
    pub zero: [f32; 3],
    /// The work zero in machine coordinates (mm): Y and Z from the A
    /// centreline of the machine's rotary geometry, X from its chuck face.
    /// None without that geometry or the stock held in the chuck.
    pub machine_zero: Option<[f32; 3]>,
    /// Whether that centreline was probed and has not gone stale since
    /// (`control::Rotary::measured`); else it is assumed.
    pub measured: bool,
}
impl AtAngle {
    /// The point `p` of the part (its file's mm) in work coordinates at
    /// this angle.
    pub fn work(&self, p: [f32; 3]) -> [f32; 3] {
        let q = self.frame.map(p);
        std::array::from_fn(|k| q[k] - self.zero[k])
    }
    /// The point `p` of the part in machine coordinates at this angle,
    /// when the work zero's place on the machine is known.
    pub fn machine(&self, p: [f32; 3]) -> Option<[f32; 3]> {
        let (w, z) = (self.work(p), self.machine_zero?);
        Some(std::array::from_fn(|k| w[k] + z[k]))
    }
}
impl Document {
    /// The base of setup `si` when it is indexed: the nearest setup before
    /// it that is not. None when it is not indexed, or is the first.
    pub fn index_base(&self, si: usize) -> Option<usize> {
        if !is_indexed(self.setups.get(si)?) {
            return None;
        }
        (0..si).rev().find(|&i| !is_indexed(&self.setups[i]))
    }
    /// Gives each indexed setup its base's machine and fields (`OWN`
    /// aside): its placement, work zero, work offset and workholding. Its
    /// turn (`sync_turns`) is not changed.
    pub fn sync_indexed(&mut self) {
        for si in 0..self.setups.len() {
            let Some(base) = self.index_base(si) else {
                continue;
            };
            let (machine, fields) = (
                self.setups[base].machine.clone(),
                self.setups[base].p.clone(),
            );
            let s = &mut self.setups[si];
            s.machine = machine;
            s.p.retain(|k, _| OWN.contains(&k.as_str()));
            for (k, v) in fields {
                if !OWN.contains(&k.as_str()) {
                    s.p.insert(k, v);
                }
            }
        }
    }
    /// Works out how each indexed setup's part is turned (`Setup::turn`),
    /// from its base's starting stock: with the part when its stock is
    /// sized from it, else without. Without its base's stock or its angles
    /// a setup is not turned.
    pub fn sync_turns(&mut self, mesh: Option<&Mesh>) {
        for si in 0..self.setups.len() {
            let turn = self.index_base(si).and_then(|base| {
                let a = *angles(&self.setups[si]).ok()?.first()?;
                let stock = self.setup_stock(base, mesh)?;
                Some(Turn {
                    base,
                    a,
                    turn: sign(&self.setups[si].machine) * a,
                    axis: axis_of(stock),
                    stock,
                })
            });
            self.setups[si].turn = turn;
        }
    }
    /// Setup `si` at A angle `a` (degrees): its clamping's frame (its base's
    /// when it is indexed, else its own) turned about the A centreline by
    /// `a`, and its work zero, which `a` does not move. Every operation of
    /// an indexed setup is run this way at each of its angles.
    pub fn at_angle(&self, si: usize, a: f32, mesh: &Mesh) -> Result<AtAngle, String> {
        let setup = self.setups.get(si).ok_or("There is no such setup.")?;
        let base = self.index_base(si).unwrap_or(si);
        let clamped = &self.setups[base];
        if !clamped.machine.has_a() {
            return Err(NO_A.into());
        }
        let stock = self.setup_stock(base, Some(mesh)).ok_or(
            "The stock the setup starts from is not known: check the first setup's stock.",
        )?;
        let axis = axis_of(stock);
        let frame = crate::volume::Frame::new(clamped, mesh).turned(axis, sign(&setup.machine) * a);
        let zero = crate::post::datum(clamped, stock, mesh)?;
        let unit = clamped.machine.rotary_unit();
        let face = crate::chuck::holds(clamped)
            .then(|| crate::chuck::rig(clamped, stock, false).ok())
            .flatten()
            .map(|r| r.face);
        let machine_zero = unit.zip(face).map(|(u, face)| {
            [
                u.chuck_x + zero[0] - face,
                u.centre_y + zero[1] - axis[0],
                u.centre_z + zero[2] - axis[1],
            ]
        });
        Ok(AtAngle {
            a,
            frame,
            zero,
            machine_zero,
            measured: self.centre_mark(&clamped.machine) == crate::control::MEASURED,
        })
    }
    /// How the A centreline of machine `m` was found (`control::CENTRE_MARK`),
    /// from the machine library, which the Control page keeps it in, else
    /// from the setup's copy of the machine.
    pub fn centre_mark<'a>(&'a self, m: &'a Machine) -> &'a str {
        let library = self.machines.iter().find(|x| x.id == m.id).unwrap_or(m);
        get(&library.p, crate::control::CENTRE_MARK).trim()
    }
    /// What is wrong with setup `si` being indexed that the setup alone
    /// does not tell (`Setup::issues` tells the rest).
    pub fn index_issues(&self, si: usize) -> Vec<String> {
        match self.setups.get(si) {
            Some(s) if is_indexed(s) && self.index_base(si).is_none() => vec![
                "The first setup cannot index A: it is the clamping later setups turn. Choose Initial placement.".into(),
            ],
            _ => vec![],
        }
    }
    /// What is not ready about indexing setup `si`, said beside it: the A
    /// centreline it turns about assumed, gone stale, or not recorded.
    pub fn index_warnings(&self, si: usize) -> Vec<String> {
        let Some(s) = self
            .setups
            .get(si)
            .filter(|s| is_indexed(s) && s.machine.has_a())
        else {
            return vec![];
        };
        if s.machine.rotary_unit().is_none() {
            return vec![
                "The machine's rotary unit geometry is not recorded: edit the machine to fill it in, then probe its A centreline on the Control page (Rotary A).".into(),
            ];
        }
        match self.centre_mark(&s.machine) {
            crate::control::MEASURED => vec![],
            "" => vec![
                "The A centreline is assumed, not measured: probe it on the Control page (Rotary A) before trusting where A turns the stock.".into(),
            ],
            why => vec![format!(
                "The measured A centreline went {why}: probe it again on the Control page (Rotary A) before trusting where A turns the stock."
            )],
        }
    }
    /// The Reposition line of setup `si` for the setup sheet.
    pub fn reposition_text(&self, si: usize) -> String {
        let s = &self.setups[si];
        match self.index_base(si) {
            Some(base) => format!(
                "{ACTION}: {} in the clamping of setup {} ({}), work zero kept",
                label(s).unwrap_or_default(),
                base + 1,
                self.setups[base].name()
            ),
            None => get(&s.p, "reposition").to_string(),
        }
    }
}
/// Why an indexed setup has no program yet.
pub const NOT_POSTED: &str =
    "it indexes A, which programs do not command yet, so it is not exported or run.";
/// Why a setup cannot be indexed on its machine.
pub const NO_A: &str = "Index rotary axis turns the stock about A: the setup's machine has no rotary A axis around X. Choose one, or Reclamp and probe origin.";
/// What is wrong with `setup`'s A angles on its machine, when it is indexed.
pub fn issues(setup: &Setup) -> Vec<String> {
    if !is_indexed(setup) {
        return vec![];
    }
    if !setup.machine.has_a() {
        return vec![NO_A.into()];
    }
    let v = match angles(setup) {
        Ok(v) => v,
        Err(e) => return vec![e],
    };
    let m = &setup.machine.p;
    let (lo, hi) = (num(m, "rotary_min"), num(m, "rotary_max"));
    v.iter()
        .filter(|a| lo.is_some_and(|lo| **a < lo) || hi.is_some_and(|hi| **a > hi))
        .map(|a| {
            format!(
                "A {}° is outside the machine's A range, {}° to {}°.",
                degrees(*a),
                get(m, "rotary_min"),
                get(m, "rotary_max")
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Document, Machine, Setup};

    /// A machine with A and its rotary geometry.
    fn rotary() -> Machine {
        let mut m = Machine::new(4);
        m.fill_rotary_defaults();
        m
    }
    /// A job on a Ø40 × 120 bar in the chuck, A axis at stock end, and a
    /// second setup indexed to `angles`.
    fn job(angles: &str) -> (Document, Mesh) {
        let mesh = crate::mesh::sample();
        let mut doc = Document::sample();
        doc.file = None;
        let m = rotary();
        let mut first = Setup::new(m.clone(), "G54");
        for (k, v) in [
            ("stock_type", "Round bar"),
            ("stock_diameter", "40"),
            ("stock_length", "120"),
            ("holding", "Chuck"),
            ("origin", ORIGINS[0]),
        ] {
            first.p.insert(k.into(), v.into());
        }
        for (k, v) in crate::chuck::DEFAULTS {
            first.p.insert(k.into(), v.into());
        }
        let mut second = Setup::new(m, "G55");
        second.p.insert("name".into(), "Flats".into());
        second.p.insert("reposition".into(), ACTION.into());
        second.p.insert(ANGLES.into(), angles.into());
        doc.setups = vec![first, second];
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        (doc, mesh)
    }
    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-3)
    }

    #[test]
    fn positive_a_turns_the_part_by_the_right_hand_rule_about_x() {
        // +Y goes up to +Z at A90 and +Z to -Y: seen from the chuck looking
        // along +X (screen right is -Y, up is +Z), +Y on the left goes up,
        // clockwise. The driver's bar and the chuck's jaws turn this way.
        assert!(close(turn_point([5., 1., 0.], [0., 0.], 90.), [5., 0., 1.]));
        assert!(close(
            turn_point([5., 0., 1.], [0., 0.], 90.),
            [5., -1., 0.]
        ));
        // About the centreline, which stays put.
        assert!(close(
            turn_point([0., 12., 20.], [2., 20.], 90.),
            [0., 2., 30.]
        ));
        assert!(close(
            turn_point([0., 2., 20.], [2., 20.], 37.),
            [0., 2., 20.]
        ));
        // The same sense as the chuck's: jaw 1 up at A0 is towards -Y at A90.
        assert!(close(
            turn_point([0., 0., 1.], [0., 0.], 90.),
            [0., -1., 0.]
        ));
        let mut m = rotary();
        assert_eq!(sign(&m), 1.);
        m.p.insert("direction".into(), "Negative".into());
        assert_eq!(sign(&m), -1.);
    }

    #[test]
    fn turned_angles_turn_as_the_placement_then_a_does() {
        for angles in [
            [0., 0., 0.],
            [180., 0., 0.],
            [0., 90., 0.],
            [12., -33., 71.],
            [0., -90., 25.],
        ] {
            for turn in [0., 90., -90., 180., 37.5] {
                let e = turned_angles(angles, turn);
                for p in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [3., -2., 5.]] {
                    let want = turn_point(rotation(p, angles), [0., 0.], turn);
                    assert!(close(rotation(p, e), want), "{angles:?} {turn}: {p:?}");
                }
            }
        }
    }

    #[test]
    fn angles_are_listed_in_degrees() {
        let mut s = Setup::new(rotary(), "G54");
        s.p.insert("reposition".into(), ACTION.into());
        for (text, want) in [
            ("90", vec![90.]),
            ("0, 90, 180, 270", vec![0., 90., 180., 270.]),
            ("0/90/180/270", vec![0., 90., 180., 270.]),
            (" -45° 45 ", vec![-45., 45.]),
        ] {
            s.p.insert(ANGLES.into(), text.into());
            assert_eq!(angles(&s).unwrap(), want, "{text}");
        }
        for bad in ["", "x", "90, 90", "nan"] {
            s.p.insert(ANGLES.into(), bad.into());
            assert!(angles(&s).is_err(), "{bad}");
        }
        s.p.insert(ANGLES.into(), "0, 90, 180, 270".into());
        assert_eq!(label(&s).unwrap(), "A 0°, 90°, 180°, 270°");
        // Outside the machine's range of ±180°.
        assert!(issues(&s)[0].contains("A 270° is outside the machine's A range, -180° to 180°."));
        s.p.insert(ANGLES.into(), "90".into());
        assert!(issues(&s).is_empty());
        // Not on a machine without A.
        s.machine = Machine::new(3);
        assert_eq!(issues(&s), [NO_A]);
        assert_eq!(label(&Setup::new(Machine::new(3), "G54")), None);
        assert_eq!(label(&Setup::new(rotary(), "G54")).unwrap(), "A 0°");
    }

    #[test]
    fn an_indexed_setup_turns_setup_1s_frame_about_the_centreline_and_keeps_work_zero() {
        let (doc, mesh) = job("90, 180");
        let (first, second) = (&doc.setups[0], &doc.setups[1]);
        assert!(is_indexed(second) && !is_indexed(first));
        assert_eq!(doc.index_base(1), Some(0));
        // In setup 1's clamping: its work zero, offset and chuck, no probing.
        for k in ["origin", "offset", "holding", "chuck_stickout", "rx", "tz"] {
            assert_eq!(get(&second.p, k), get(&first.p, k), "{k}");
        }
        assert_eq!(get(&second.p, "name"), "Flats");
        // The bar's axis: Y0, Z20 (Ø40 standing on Z0).
        let turn = second.turn.unwrap();
        assert_eq!(
            (turn.base, turn.a, turn.turn, turn.axis),
            (0, 90., 90., [0., 20.])
        );
        let at0 = doc.at_angle(1, 0., &mesh).unwrap();
        let at90 = doc.at_angle(1, 90., &mesh).unwrap();
        // Work zero on the axis at the bar's free end, the same at each angle.
        assert!(close(at0.zero, [60., 0., 20.]) && at90.zero == at0.zero);
        // Every point of the part turns +Y towards +Z about the axis.
        let base = crate::volume::Frame::new(first, &mesh);
        for p in [[1., 2., 3.], [-20., 15., -4.], mesh.origin] {
            assert!(close(at0.frame.map(p), base.map(p)));
            assert!(close(
                at90.frame.map(p),
                turn_point(base.map(p), [0., 20.], 90.)
            ));
            let (w0, w90) = (at0.work(p), at90.work(p));
            assert!(close(w90, [w0[0], -w0[2], w0[1]]), "{w0:?} {w90:?}");
        }
        // The setup's own frame is its first angle's: the 3D view and the
        // stock it inherits.
        let own = crate::volume::Frame::new(second, &mesh);
        for p in [[1., 2., 3.], [-20., 15., -4.]] {
            assert!(close(own.map(p), at90.frame.map(p)));
        }
        let placed = crate::stock::Placement::new(&mesh, Some(second));
        let s = mesh.display_scale();
        let p = mesh.faces[0].points[0];
        let q = own.map(p.map(|v| v / s));
        assert!(close(placed.transform(p).map(|v| v / s), q));
        // Its work zero is setup 1's, though its stock is turned.
        let stock = doc.setup_stock(1, Some(&mesh)).unwrap();
        assert!(close(
            crate::post::datum(second, stock, &mesh).unwrap(),
            at0.zero
        ));
        // On the machine: the work zero on the rotary unit's centreline;
        // the bar's free end 40 mm out of the jaws, 25 mm long, from the face.
        let m = at90.machine_zero.unwrap();
        assert!(close(m, [20. + 40. + 25., 140., -40.]), "{m:?}");
        assert!(close(at90.machine([0.; 3]).unwrap(), {
            let w = at90.work([0.; 3]);
            [w[0] + m[0], w[1] + m[1], w[2] + m[2]]
        }));
        // A machine whose A turns the other way turns the part back.
        let (mut doc, mesh) = job("90");
        for s in &mut doc.setups {
            s.machine.p.insert("direction".into(), "Negative".into());
        }
        let at = doc.at_angle(1, 90., &mesh).unwrap();
        let base = crate::volume::Frame::new(&doc.setups[0], &mesh);
        assert!(close(
            at.frame.map([1., 2., 3.]),
            turn_point(base.map([1., 2., 3.]), [0., 20.], -90.)
        ));
    }

    #[test]
    fn a_re_clamp_after_indexed_setups_keeps_its_own_placement() {
        let (mut doc, mesh) = job("90");
        let mut third = doc.setups[0].clone();
        third.id = "third".into();
        for (k, v) in [
            ("reposition", "Reclamp and probe origin"),
            ("rx", "180"),
            ("origin", "Stock top center"),
        ] {
            third.p.insert(k.into(), v.into());
        }
        let mut fourth = doc.setups[1].clone();
        fourth.id = "fourth".into();
        doc.setups.extend([third, fourth]);
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        assert_eq!(doc.index_base(2), None);
        assert_eq!(doc.index_base(3), Some(2));
        assert_eq!(get(&doc.setups[2].p, "rx"), "180");
        assert_eq!(doc.setups[2].turn, None);
        // The fourth turns the re-clamp's frame and takes its work zero.
        assert_eq!(get(&doc.setups[3].p, "rx"), "180");
        assert_eq!(get(&doc.setups[3].p, "origin"), "Stock top center");
        assert_eq!(doc.setups[3].turn.unwrap().base, 2);
        let base = crate::volume::Frame::new(&doc.setups[2], &mesh);
        let at = doc.at_angle(3, 90., &mesh).unwrap();
        let axis = doc.setups[3].turn.unwrap().axis;
        assert!(close(
            at.frame.map([1., 2., 3.]),
            turn_point(base.map([1., 2., 3.]), axis, 90.)
        ));
        // Indexing the first setup is refused.
        doc.setups[0].p.insert("reposition".into(), ACTION.into());
        doc.setups[0].p.insert(ANGLES.into(), "90".into());
        assert!(doc.index_issues(0)[0].contains("first setup cannot index A"));
    }

    #[test]
    fn indexed_setups_warn_while_the_centreline_is_not_measured() {
        let (mut doc, _) = job("90");
        let assumed = doc.index_warnings(1);
        assert!(assumed[0].contains("assumed, not measured"), "{assumed:?}");
        assert!(doc.index_warnings(0).is_empty());
        let id = doc.setups[0].machine.id.clone();
        let mut library = doc.setups[0].machine.clone();
        library.p.insert(
            crate::control::CENTRE_MARK.into(),
            crate::control::MEASURED.into(),
        );
        doc.machines = vec![library];
        assert!(doc.index_warnings(1).is_empty());
        let mesh = crate::mesh::sample();
        doc.sync_turns(Some(&mesh));
        assert!(doc.at_angle(1, 90., &mesh).unwrap().measured);
        doc.machines[0].p.insert(
            crate::control::CENTRE_MARK.into(),
            "stale after an E-stop".into(),
        );
        assert_eq!(doc.machines[0].id, id);
        assert!(doc.index_warnings(1)[0].contains("went stale after an E-stop"));
        assert!(!doc.at_angle(1, 90., &mesh).unwrap().measured);
    }

    #[test]
    fn a_label_only_index_from_an_older_project_keeps_its_own_placement() {
        let (mut doc, mesh) = job("90");
        doc.setups[1].p.remove(ANGLES);
        for (k, v) in [("rx", "90"), ("ty", "5"), ("origin", "Stock top center")] {
            doc.setups[1].p.insert(k.into(), v.into());
        }
        let before = doc.setups[1].p.clone();
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        let s = &doc.setups[1];
        assert!(is_label_only(s) && !is_indexed(s));
        assert_eq!(s.p, before);
        assert_eq!(s.turn, None);
        assert_eq!(label(s).unwrap(), "A 0°");
        assert_eq!(doc.reposition_text(1), ACTION);
    }
}
