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
            let turn = angles(&self.setups[si])
                .ok()
                .and_then(|v| self.turn_at(si, *v.first()?, mesh));
            self.setups[si].turn = turn;
        }
    }
    /// How A turns setup `si`'s part at A `a` (degrees) from its base's
    /// placement, from its base's starting stock. None when it is not
    /// indexed, or that stock cannot be told.
    pub fn turn_at(&self, si: usize, a: f32, mesh: Option<&Mesh>) -> Option<Turn> {
        let base = self.index_base(si)?;
        let stock = self.setup_stock(base, mesh)?;
        Some(Turn {
            base,
            a,
            turn: sign(&self.setups[si].machine) * a,
            axis: axis_of(stock),
            stock,
        })
    }
    /// `setup` (indexed setup `si`, or the copy a simulation runs) posed at
    /// A `a`: its turn is `a`'s, so its placement, its frame (`at_angle`'s)
    /// and its chuck are where A turns them.
    pub fn setup_at(&self, setup: &Setup, si: usize, a: f32, mesh: &Mesh) -> Result<Setup, String> {
        let mut s = setup.clone();
        s.turn = Some(self.turn_at(si, a, Some(mesh)).ok_or(STOCK_UNKNOWN)?);
        Ok(s)
    }
    /// What an index move of setup `si` clears with `clearance` (mm, the
    /// operation's): the farthest the stock, the part or the chuck's jaws
    /// reach from the A centreline, at any angle, and the height above it.
    /// Refused when that height is above the machine's Z travel: it is
    /// never cut short to fit.
    pub fn swing(&self, si: usize, mesh: &Mesh, clearance: f32) -> Result<Swing, String> {
        let base = self.index_base(si).ok_or(NOT_INDEXED)?;
        let clamped = &self.setups[base];
        let stock = self.setup_stock(base, Some(mesh)).ok_or(STOCK_UNKNOWN)?;
        let unit = self.rotary_unit(&clamped.machine).ok_or(NO_GEOMETRY)?;
        let axis = axis_of(stock);
        let [_, dy, dz] = stock.dimensions();
        let mut reach = vec![("the stock reaches", dy.hypot(dz) / 2.)];
        if crate::chuck::holds(clamped) {
            let rig = self.chuck_rig(clamped, stock)?;
            reach[0].1 = rig.outer;
            let jaws = rig
                .solids(0.)
                .into_iter()
                .filter(|(label, _, _)| label.starts_with("chuck jaw"))
                .filter_map(|(_, _, r)| r)
                .fold(0., f32::max);
            reach.push(("the chuck's jaws reach", jaws));
        }
        let placed = crate::stock::Placement::new(mesh, Some(clamped));
        let scale = mesh.display_scale();
        let part = mesh
            .faces
            .iter()
            .flat_map(|f| &f.points)
            .map(|p| {
                let q = placed.transform(*p).map(|v| v / scale);
                (q[1] - axis[0]).hypot(q[2] - axis[1])
            })
            .fold(0., f32::max);
        reach.push(("the part reaches", part));
        let (what, radius) =
            reach.into_iter().fold(
                ("the stock reaches", 0.),
                |m, r| if r.1 > m.1 { r } else { m },
            );
        let swing = Swing {
            radius,
            clearance,
            z: axis[1] + radius + clearance,
            machine_z: unit.centre_z + radius + clearance,
        };
        // Machine Z is zero at the top of its travel and negative down.
        if swing.machine_z > 0. {
            let travel = crate::post_check::machine_mm(&clamped.machine, "z");
            return Err(format!(
                "Index moves turn A with the tool above all that A swings: at machine Z {} (the A centreline at Z {} + {} mm, as far as {what} from it at any angle, + {} mm clearance). That is {} mm above the top of the machine's {} mm Z travel (Z 0), so it cannot be reached and is not cut short. Measure the A centreline (Rotary A on the Control page) or reach less far from it: a smaller stock or chuck, or less clearance.",
                mm(swing.machine_z),
                mm(unit.centre_z),
                mm(radius),
                mm(clearance),
                mm(swing.machine_z),
                mm(travel),
            ));
        }
        Ok(swing)
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
        let unit = self.rotary_unit(&clamped.machine);
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
    /// The rotary unit of machine `m`: its A centreline as the Control page
    /// measured it, in the machine library, while it is marked measured;
    /// else the setup's copy's.
    pub fn rotary_unit(&self, m: &Machine) -> Option<crate::model::RotaryUnit> {
        let mut unit = m.rotary_unit()?;
        if self.centre_mark(m) == crate::control::MEASURED
            && let Some(library) = self
                .machines
                .iter()
                .find(|x| x.id == m.id)
                .and_then(Machine::rotary_unit)
        {
            (unit.centre_y, unit.centre_z) = (library.centre_y, library.centre_z);
        }
        Some(unit)
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
/// Why an indexed setup's cuts are not checked against its workholding: its
/// index moves are, as they are generated (`generate`).
pub const NOT_HELD: &str = "it indexes A, and its cuts are checked against the chuck at each A angle only once programs command A; its index moves are checked as they are made";
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

/// What the ribbon's Indexed tile says now: indexing is a setup's, not an
/// operation's.
pub const POINTER: &str = "Indexed work is a setup now: add a setup, choose Reposition → Index rotary axis and list its A angles. Every operation in it runs at each angle.";
/// The Indexed strategy's availability, in the words of the others'.
pub const AVAILABILITY: &str = "Unavailable · indexed work is a setup now: add a setup, choose Reposition → Index rotary axis and list its A angles, and every operation in it runs at each angle";
/// Why an Indexed operation of an older project that could not be moved
/// into an indexed setup (`Document::migrate_indexed_operations`) is not
/// ready.
pub const LEGACY: &str = "This Indexed operation is from an older project and was not moved into an indexed setup: it was disabled at an angle, its machine has no A axis around X, or it sets a B angle or an A angle that is not a number. Indexing is a setup now: add a setup on a machine with A, choose Reposition → Index rotary axis at the angle, and add a Facing operation there.";
impl Document {
    /// Moves each Indexed operation of an older project (an Indexed face,
    /// which only recorded the A angle `angle_a`) into the setups that index
    /// now: it becomes the Facing operation it was, in a new indexed setup at
    /// its angle right after its own, in that setup's clamping; the
    /// operations after it follow in an indexed setup at A 0°, so that
    /// everything still runs in its order. At A 0°, or in a setup that is
    /// indexed already, it stays where it is. One that cannot be moved (no A
    /// axis around X, a B angle, an angle that is not a number) stays an
    /// Indexed operation, which `LEGACY` refuses. Returns a note for each.
    pub fn migrate_indexed_operations(&mut self) -> Vec<String> {
        use crate::model::Strategy;
        let mut notes = vec![];
        let mut out = vec![];
        let mut added = false;
        for mut setup in std::mem::take(&mut self.setups) {
            if !setup
                .operations
                .iter()
                .any(|o| o.strategy == Strategy::Indexed)
            {
                out.push(setup);
                continue;
            }
            let own = is_indexed(&setup);
            // The operations by the A angle they now run at: the setup's own
            // first, then each indexed setup's.
            let mut groups: Vec<(Option<f32>, Vec<crate::model::Operation>)> = vec![(None, vec![])];
            for mut op in std::mem::take(&mut setup.operations) {
                let mut at = None;
                if op.strategy == Strategy::Indexed {
                    let a = match get(&op.p, "angle_a").trim() {
                        "" => Some(0.),
                        _ => num(&op.p, "angle_a"),
                    };
                    let b = num(&op.p, "angle_b").unwrap_or(0.);
                    // A disabled one at an angle would split the setup for
                    // nothing it cuts: it stays, as it did.
                    let moved = |a: &f32| {
                        setup.machine.has_a() && b.abs() < 1e-3 && (op.enabled || a.abs() < 1e-3)
                    };
                    match a.filter(moved) {
                        Some(a) => {
                            op.strategy = Strategy::Face;
                            op.p.remove("angle_a");
                            op.p.remove("angle_b");
                            at = (!own && a.abs() > 1e-3).then_some(a);
                            notes.push(match at {
                                Some(a) => format!(
                                    "{}: the Indexed operation at A {}° is a Facing operation in a new indexed setup at that angle, after {}.",
                                    op.name(),
                                    degrees(a),
                                    setup.name()
                                ),
                                None if own => format!(
                                    "{}: the Indexed operation is a Facing operation in {}, at that setup's A angles.",
                                    op.name(),
                                    setup.name()
                                ),
                                None => format!(
                                    "{}: the Indexed operation at A 0° is a Facing operation in {}.",
                                    op.name(),
                                    setup.name()
                                ),
                            });
                        }
                        None => notes.push(format!("{}: {LEGACY}", op.name())),
                    }
                }
                // After an indexed group, the setup's own operations run at A 0°.
                let at = at.or((groups.len() > 1).then_some(0.));
                match groups.last_mut() {
                    Some((key, ops)) if *key == at => ops.push(op),
                    _ => groups.push((at, vec![op])),
                }
            }
            let mut groups = groups.into_iter();
            let (_, first) = groups.next().unwrap_or_default();
            setup.operations = first;
            let template = setup.clone();
            out.push(setup);
            for (a, ops) in groups {
                let a = a.unwrap_or(0.);
                let mut s = template.clone();
                s.id = crate::model::id();
                s.turn = None;
                s.p.insert(
                    "name".into(),
                    format!("{} · A {}°", template.name(), degrees(a)),
                );
                s.p.insert("reposition".into(), ACTION.into());
                s.p.insert(ANGLES.into(), degrees(a));
                s.p.remove("notes");
                s.operations = ops;
                out.push(s);
                added = true;
            }
        }
        self.setups = out;
        if added {
            self.sync_stock_sources();
        }
        notes
    }
}
/// `v` mm for a message, to the hundredth, without trailing zeros.
fn mm(v: f32) -> String {
    let s = format!("{v:.2}");
    match s.trim_end_matches('0').trim_end_matches('.') {
        "-0" => "0".into(),
        t => t.into(),
    }
}
/// Why an index move cannot be planned without the base's stock.
const STOCK_UNKNOWN: &str =
    "The stock the setup starts from is not known: check the first setup's stock.";
const NOT_INDEXED: &str = "The setup does not index A.";
/// Why an index move cannot be checked without the rotary unit's geometry.
pub const NO_GEOMETRY: &str = "The machine's rotary unit geometry is not recorded, so the index moves' retract above what A swings cannot be checked against the Z travel: edit the machine to fill it in.";
/// The rate A turns at in an index move, degrees per minute, for the time
/// it takes: an assumed 60°/s.
pub const INDEX_RATE: f32 = 3600.;
/// The retract of an indexed setup's index moves (`Document::swing`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Swing {
    /// The farthest the stock, the part or the chuck's jaws reach from the
    /// A centreline at any angle, and the clearance kept above it, mm.
    pub radius: f32,
    pub clearance: f32,
    /// The tool tip's height while A turns: in the setup's frame, and in
    /// machine coordinates (zero at the top of the Z travel).
    pub z: f32,
    pub machine_z: f32,
}
/// An indexed setup's operation, generated at each of its A angles.
pub struct Posed {
    /// The moves at every angle, in the order of the angles, each carrying
    /// its A, joined by index moves.
    pub plan: crate::simulation::Plan,
    /// The frame the part is posed in at each angle (`AtAngle::frame`).
    pub turns: Vec<(f32, crate::volume::Frame)>,
}
/// What `generate` starts from: the stock left when the setup began and
/// now (None when it is not known), the A the machine is at, and the
/// source the operation's cuts are stamped with.
pub struct Start<'a> {
    pub begin: Option<&'a crate::volume::Volume>,
    pub now: Option<&'a crate::volume::Volume>,
    pub a: f32,
    pub source: (u16, u16),
}
/// Generates `op` of indexed setup `si` with `tool`, as an indexed setup's
/// operations all are: once at each of its A angles, in order. Indexing is
/// a frame change, not other toolpaths: at each angle the 3-axis kernel
/// runs on `setup` (the copy a simulation runs, fresh stock) posed as A
/// turns the part and its stock about the centreline (`Document::setup_at`,
/// whose frame is `Document::at_angle`'s), the stock cut at the angles
/// before. Its moves stay in the setup's frame, which keeps the machine's
/// axes, so they are machine X Y Z less the work offset, which A does not
/// move; each carries its A. Between two angles, and from the A the
/// machine is at to the first, the tool retracts above all A swings
/// (`Document::swing`), A turns, and the tool comes over the next first cut
/// and down to it. The chuck is checked along each of those moves and the
/// whole turn (`chuck::Rig::hits_turning`).
pub fn generate(
    doc: &Document,
    si: usize,
    setup: &Setup,
    op: &crate::model::Operation,
    tool: &crate::model::Tool,
    mesh: &Mesh,
    start: Start,
) -> Result<Posed, String> {
    use crate::simulation::{Motion, Move, Simulation};
    let list = angles(setup)?;
    let begin = start.begin.ok_or(STOCK_UNKNOWN)?;
    let mut now = start.now.cloned();
    let mut parts = vec![];
    let mut turns = vec![];
    for (k, &a) in list.iter().enumerate() {
        let at = doc.at_angle(si, a, mesh)?;
        let mut posed = doc.setup_at(setup, si, a, mesh)?;
        posed.preview_stock = Some(crate::simulation::leftover_box(begin, &posed, mesh));
        posed.preview_remaining = now
            .as_ref()
            .filter(|v| !v.cuts.is_empty())
            .map(|v| std::sync::Arc::new(crate::rest::RestStock::new(v, at.frame)));
        let mut plan = crate::simulation::generate(&posed, op, tool, mesh)
            .map_err(|e| format!("At A {}°: {e}", degrees(a)))?;
        for m in &mut plan.moves {
            m.a = Some(a);
        }
        // The next angle starts from the stock this one leaves.
        if k + 1 < list.len()
            && let Some(v) = now.take()
        {
            let mut sim = Simulation::new(plan.clone());
            (sim.volume, sim.frame, sim.source) = (Some(v), Some(at.frame), start.source);
            sim.seek(1.);
            now = sim.volume;
        }
        parts.push((a, plan));
        turns.push((a, at.frame));
    }
    let turning = list.len() > 1 || (list[0] - start.a).abs() > 1e-4;
    let swing = if turning {
        Some(doc.swing(si, mesh, crate::cam::number(op, "clearance")?)?)
    } else {
        None
    };
    let bi = doc.index_base(si).ok_or(NOT_INDEXED)?;
    let base = &doc.setups[bi];
    let rig = if turning && crate::chuck::holds(base) {
        let stock = doc.setup_stock(bi, Some(mesh)).ok_or(STOCK_UNKNOWN)?;
        Some((doc.chuck_rig(base, stock)?, crate::holding::gap(base)?))
    } else {
        None
    };
    let mut plan = parts[0].1.clone();
    plan.moves.clear();
    plan.duration = 0.;
    let push = |plan: &mut crate::simulation::Plan, m: Move| {
        let seconds = m.end - m.start;
        plan.moves.push(Move {
            start: plan.duration,
            end: plan.duration + seconds,
            ..m
        });
        plan.duration += seconds;
    };
    // A rapid at the rate the 3-axis kernels rapid at, none when it does not move.
    let rapid = |plan: &mut crate::simulation::Plan, from: [f32; 3], to: [f32; 3], a: f32| {
        let d = (0..3)
            .map(|k| (to[k] - from[k]).powi(2))
            .sum::<f32>()
            .sqrt();
        if d > 1e-5 {
            push(
                plan,
                Move {
                    from,
                    to,
                    motion: Motion::Rapid,
                    cutting_pass: false,
                    feed: RAPID,
                    start: 0.,
                    end: d / RAPID * 60.,
                    a: Some(a),
                },
            );
        }
    };
    let (mut a_now, mut at): (f32, Option<[f32; 3]>) = (start.a, None);
    for (a, part) in &parts {
        let mut moves = &part.moves[..];
        if (a - a_now).abs() > 1e-4 {
            let swing = swing.ok_or(NOT_INDEXED)?;
            // The plan's own approach is replaced: over its first cut at the
            // swing height, straight down to where that cut starts.
            let first = moves
                .iter()
                .position(|m| m.motion == Motion::Feed)
                .unwrap_or(0);
            let to = moves[first].from;
            // Before the first angle the tool waits over its first cut.
            let from = at.unwrap_or([to[0], to[1], swing.z]);
            let (up, over) = ([from[0], from[1], swing.z], [to[0], to[1], swing.z]);
            if let Some((rig, gap)) = &rig {
                let tool = (part.radius, part.tool_length, part.holder_radius);
                let hits = [
                    rig.hits_at(a_now, &[(from, up)], tool, *gap),
                    rig.hits_turning(a_now, *a, &[(up, up)], tool, *gap),
                    rig.hits_at(*a, &[(up, over), (over, to)], tool, *gap),
                ];
                if let Some(hit) = hits.iter().flatten().next() {
                    return Err(format!(
                        "The index move from A {}° to A {}° brings the {} within the {} mm gap of the {} at X {} Y {} Z {}.",
                        degrees(a_now),
                        degrees(*a),
                        hit.part,
                        mm(*gap),
                        hit.label,
                        mm(hit.at[0]),
                        mm(hit.at[1]),
                        mm(hit.at[2]),
                    ));
                }
            }
            rapid(&mut plan, from, up, a_now);
            push(
                &mut plan,
                Move {
                    from: up,
                    to: up,
                    motion: Motion::Index,
                    cutting_pass: false,
                    feed: INDEX_RATE,
                    start: 0.,
                    end: (a - a_now).abs() / INDEX_RATE * 60.,
                    a: Some(*a),
                },
            );
            rapid(&mut plan, up, over, *a);
            rapid(&mut plan, over, to, *a);
            moves = &moves[first..];
        }
        for m in moves {
            push(&mut plan, m.clone());
        }
        at = plan.moves.last().map(|m| m.to);
        a_now = *a;
    }
    Ok(Posed { plan, turns })
}
/// The rapid rate of the index moves, mm/min: the 3-axis kernels'.
const RAPID: f32 = 6000.;
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

    /// A square bar `len` mm long, 20 × 20, as the part: the frames' part
    /// coordinates (`volume::Frame::map`) put it about X with its middle at
    /// Y0, Z `centre_z`.
    fn prism(len: f32) -> Mesh {
        block(len, 20., 20.)
    }
    /// A block `l` × `w` (Y) × `h` (Z) as the part.
    fn block(l: f32, w: f32, h: f32) -> Mesh {
        let mut t = vec![];
        let mut quad = |p: [[f32; 3]; 4]| {
            t.push([p[0], p[1], p[2]]);
            t.push([p[0], p[2], p[3]]);
        };
        quad([[0., 0., h], [l, 0., h], [l, w, h], [0., w, h]]);
        quad([[0., 0., 0.], [0., w, 0.], [l, w, 0.], [l, 0., 0.]]);
        quad([[0., 0., 0.], [l, 0., 0.], [l, 0., h], [0., 0., h]]);
        quad([[l, w, 0.], [0., w, 0.], [0., w, h], [l, w, h]]);
        quad([[0., w, 0.], [0., 0., 0.], [0., 0., h], [0., w, h]]);
        quad([[l, 0., 0.], [l, w, 0.], [l, w, h], [l, 0., h]]);
        crate::mesh::normalize(t).unwrap()
    }
    fn centre_z(mesh: &Mesh) -> f32 {
        4. / mesh.display_scale() + mesh.dimensions[2] / 2.
    }
    /// A flat end mill: Ø`d`, 35 mm of flute, 50 mm out of its holder.
    fn cutter(d: &str) -> crate::model::Tool {
        let mut t = crate::model::Tool::new("Flat", "Flat end mill", d);
        t.p.insert("length".into(), "35".into());
        t.p.insert("stickout".into(), "50".into());
        t
    }
    /// The part `mesh` in a Ø40 × 120 bar `stickout` mm out of a 3-jaw
    /// chuck, its square on the bar's axis (Y0 Z20) from X `x0`, on a rotary
    /// unit whose centreline is at machine Z `centre_z`; setup 2 is indexed
    /// to `angles` with the one operation `op` (a Ø12 flat end mill's).
    fn bar_job(
        mesh: &Mesh,
        x0: f32,
        stickout: &str,
        centre_z: &str,
        angles: &str,
        op: crate::model::Operation,
    ) -> Document {
        let (mut doc, _) = job(angles);
        let s = mesh.display_scale();
        let len = mesh.dimensions[0];
        // Setup 2 takes setup 1's machine (`sync_indexed`).
        for (k, v) in [
            ("a_centre_z", centre_z),
            ("rotary_min", "-360"),
            ("rotary_max", "360"),
        ] {
            doc.setups[0].machine.p.insert(k.into(), v.into());
        }
        let p = &mut doc.setups[0].p;
        p.insert("chuck_stickout".into(), stickout.into());
        p.insert("tx".into(), (x0 + len / 2.).to_string());
        p.insert(
            "tz".into(),
            (20. - mesh.dimensions[2] / 2. - 4. / s).to_string(),
        );
        doc.tools = vec![cutter("12")];
        let mut op = op;
        op.tool = doc.tools[0].id.clone();
        doc.setups[1].operations = vec![op];
        doc.sync_stock_sources();
        doc.sync_turns(Some(mesh));
        doc
    }
    /// An outside contour round the part from the stock's top to its bottom.
    fn contour(tool: &crate::model::Tool) -> crate::model::Operation {
        let mut op = crate::model::Operation::new(crate::model::Strategy::Contour, tool.id.clone());
        op.geometry.clear();
        op.p.insert("allowance".into(), "0".into());
        op
    }
    /// The angles the moves of `plan` are made at, each once, in order.
    fn angle_order(plan: &crate::simulation::Plan) -> Vec<f32> {
        let mut v: Vec<f32> = vec![];
        for a in plan.moves.iter().map(|m| m.a.unwrap()) {
            if v.last() != Some(&a) {
                v.push(a);
            }
        }
        v
    }

    #[test]
    fn a_flat_on_the_parts_plus_y_face_is_cut_on_top_at_a_plus_90() {
        use crate::model::{Operation, Strategy};
        use crate::simulation::{Motion, prepare};
        let mesh = prism(30.);
        let tool = cutter("12");
        // Facing the part's +Y face, which only A90 brings to the top.
        let mut op = Operation::new(Strategy::Face, tool.id.clone());
        let plus_y = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, f)| f.normal[1].abs() > 0.999)
            .max_by(|a, b| a.1.center[1].total_cmp(&b.1.center[1]))
            .unwrap()
            .0;
        op.surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![plus_y],
        });
        let doc = bar_job(&mesh, 0., "80", "-150", "90", op.clone());
        let sim = prepare(&doc, &mesh, 1, 0).unwrap();
        let plan = &sim.plan;
        // From A0, where setup 1 left it: the index comes first.
        assert_eq!(plan.moves[0].motion, Motion::Index);
        assert!(plan.moves.iter().all(|m| m.a == Some(90.)));
        // Every cut, taken back to the part, is on its +Y face or above it
        // (Y10 of the part's middle), across the face.
        let at = doc.at_angle(1, 90., &mesh).unwrap();
        let cuts: Vec<[f32; 3]> = plan
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Feed && m.cutting_pass)
            .flat_map(|m| [m.from, m.to])
            .map(|p| at.frame.unmap(p))
            .collect();
        assert!(!cuts.is_empty());
        assert!(cuts.iter().all(|q| q[1] > 10. - 0.01), "{cuts:?}");
        assert!(cuts.iter().any(|q| q[1] < 10. + 0.01));
        // On the machine those tips are on top: Z above the A centreline.
        let axis = doc.setups[1].turn.unwrap().axis;
        assert!(
            plan.moves
                .iter()
                .filter(|m| m.cutting_pass)
                .all(|m| m.to[2] > axis[1] + 9.99)
        );
        // The stock is cut over the +Y face, and not over the -Y face.
        let mut done = sim.clone();
        done.seek(1.);
        let (volume, base) = (
            done.volume.unwrap(),
            crate::volume::Frame::new(&doc.setups[0], &mesh),
        );
        let c = centre_z(&mesh);
        let filled = |q: [f32; 3]| volume.filled(base.map(q), base);
        assert!(!filled([0., 11.5, c]), "over the +Y face");
        assert!(filled([0., -11.5, c]), "over the -Y face");
        // At A-90 the +Y face is underneath: facing it is refused.
        let doc = bar_job(&mesh, 0., "80", "-150", "-90", op);
        let refused = prepare(&doc, &mesh, 1, 0).unwrap_err();
        assert!(
            refused.starts_with("At A -90°: Facing preview supports the highest plane"),
            "{refused}"
        );
    }

    #[test]
    fn a_square_milled_at_0_90_180_270_comes_out_square_on_the_axis() {
        use crate::simulation::prepare;
        let mesh = prism(30.);
        let doc = bar_job(
            &mesh,
            0.,
            "80",
            "-150",
            "0, 90, 180, 270",
            contour(&cutter("12")),
        );
        let mut sim = prepare(&doc, &mesh, 1, 0).unwrap();
        assert_eq!(angle_order(&sim.plan), [0., 90., 180., 270.]);
        sim.seek(1.);
        let (volume, base) = (
            sim.volume.unwrap(),
            crate::volume::Frame::new(&doc.setups[0], &mesh),
        );
        let c = centre_z(&mesh);
        let filled = |dy: f32, dz: f32| volume.filled(base.map([0., dy, c + dz]), base);
        // Halfway along the part: the square's 20 × 20 left on the bar's
        // axis, the bar round it (Ø40) cut away on all four sides.
        for (dy, dz) in [
            (8., 0.),
            (-8., 0.),
            (0., 8.),
            (0., -8.),
            (8., 8.),
            (-8., -8.),
            (8., -8.),
            (-8., 8.),
        ] {
            assert!(filled(dy, dz), "inside the square at {dy} {dz}");
        }
        for (dy, dz) in [
            (13., 0.),
            (-13., 0.),
            (0., 13.),
            (0., -13.),
            (12., 12.),
            (-12., 12.),
            (12., -12.),
            (-12., -12.),
        ] {
            assert!(!filled(dy, dz), "outside the square at {dy} {dz}");
        }
        // The same faces on each side: as far from the axis either way.
        let edge = |dir: [f32; 2]| {
            (0..200)
                .map(|i| i as f32 * 0.1)
                .find(|r| !filled(dir[0] * r, dir[1] * r))
                .unwrap()
        };
        let sides = [[1., 0.], [-1., 0.], [0., 1.], [0., -1.]].map(edge);
        for r in sides {
            assert!(
                (r - sides[0]).abs() <= volume.cell[1].max(volume.cell[2]) + 0.01,
                "{sides:?}"
            );
        }
    }

    #[test]
    fn an_index_move_retracts_above_the_swing_before_a_turns() {
        use crate::simulation::{Motion, prepare};
        let mesh = prism(30.);
        let doc = bar_job(
            &mesh,
            0.,
            "80",
            "-150",
            "0, 90, 180, 270",
            contour(&cutter("12")),
        );
        let plan = prepare(&doc, &mesh, 1, 0).unwrap().plan;
        // The jaws reach farthest: the Ø40 bar's radius and 35 mm of jaw,
        // 20 mm wide; 15 mm of clearance over them.
        let swing = doc.swing(1, &mesh, 15.).unwrap();
        assert!((swing.radius - 55f32.hypot(10.)).abs() < 1e-3, "{swing:?}");
        assert!((swing.z - (20. + swing.radius + 15.)).abs() < 1e-3);
        assert!((swing.machine_z - (-150. + swing.radius + 15.)).abs() < 1e-3);
        let index: Vec<usize> = (0..plan.moves.len())
            .filter(|&i| plan.moves[i].motion == Motion::Index)
            .collect();
        assert_eq!(index.len(), 3);
        let mut before = 0.;
        for i in index {
            let (up, turn, over, down) = (
                &plan.moves[i - 1],
                &plan.moves[i],
                &plan.moves[i + 1],
                &plan.moves[i + 2],
            );
            // Straight up to the swing height at the angle it cut at...
            assert_eq!((up.motion, up.a), (Motion::Rapid, Some(before)));
            assert!(up.from[2] < up.to[2] && up.from[..2] == up.to[..2]);
            assert!((up.to[2] - swing.z).abs() < 1e-4);
            // ...then A turns, the tool still there...
            assert_eq!((turn.from, turn.to), (up.to, up.to));
            let a = turn.a.unwrap();
            assert_eq!(a, before + 90.);
            assert!((turn.end - turn.start - 90. / INDEX_RATE * 60.).abs() < 1e-4);
            // ...then over the next first cut at that height (unless it is
            // there already) and down to it.
            let (down, next) = if over.from[..2] == over.to[..2] {
                (over, i + 2)
            } else {
                assert!(
                    (over.to[2] - swing.z).abs() < 1e-4 && (over.from[2] - swing.z).abs() < 1e-4
                );
                assert_eq!((over.motion, over.a), (Motion::Rapid, Some(a)));
                (down, i + 3)
            };
            assert_eq!((down.motion, down.a), (Motion::Rapid, Some(a)));
            assert!(down.to[2] < down.from[2] && down.from[..2] == down.to[..2]);
            assert_eq!(plan.moves[next].motion, Motion::Feed);
            before = a;
        }
        // The moves are one path, in time.
        for pair in plan.moves.windows(2) {
            assert_eq!(pair[0].to, pair[1].from);
            assert!((pair[0].end - pair[1].start).abs() < 1e-3);
        }
    }

    #[test]
    fn an_index_retract_above_the_z_travel_is_refused_with_its_numbers() {
        let mesh = prism(30.);
        // The assumed centreline, Z -40: the jaws' 55.9 mm and 15 mm of
        // clearance over it end 30.9 mm above the top of the Z travel.
        let doc = bar_job(&mesh, 0., "80", "-40", "0, 90", contour(&cutter("12")));
        let refused = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap_err();
        for words in [
            "at machine Z 30.9 (the A centreline at Z -40 + 55.9 mm, as far as the chuck's jaws reach from it at any angle, + 15 mm clearance)",
            "30.9 mm above the top of the machine's 300 mm Z travel (Z 0)",
            "not cut short",
        ] {
            assert!(refused.contains(words), "{refused}");
        }
        // Even one angle away from A0.
        let doc = bar_job(&mesh, 0., "80", "-40", "90", contour(&cutter("12")));
        assert!(
            crate::simulation::prepare(&doc, &mesh, 1, 0)
                .unwrap_err()
                .contains("machine Z 30.9")
        );
        // At A0 alone A does not turn: nothing to retract for.
        let doc = bar_job(&mesh, 0., "80", "-40", "0", contour(&cutter("12")));
        assert!(crate::simulation::prepare(&doc, &mesh, 1, 0).is_ok());
    }

    #[test]
    fn an_index_move_nearer_the_jaws_than_the_gap_is_refused() {
        // A 10 mm long part over the jaws, 1 mm of clearance: waiting 1 mm
        // over the jaws' reach is inside the chuck's 2 mm gap.
        let mesh = prism(10.);
        let mut op = contour(&cutter("12"));
        op.p.insert("clearance".into(), "1".into());
        op.p.insert("retract".into(), "1".into());
        let doc = bar_job(&mesh, -17., "60", "-150", "0, 90", op);
        let refused = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap_err();
        assert!(
            refused.starts_with("The index move from A 0° to A 90° brings the tool within the 2 mm gap of the chuck jaw"),
            "{refused}"
        );
    }

    #[test]
    fn three_axis_moves_carry_no_a_and_no_index() {
        use crate::simulation::{Motion, prepare};
        let mesh = prism(30.);
        let mut doc = bar_job(&mesh, 0., "80", "-150", "90", contour(&cutter("12")));
        // Setup 1's own contour is a 3-axis operation, as it was.
        let op = contour(&doc.tools[0]);
        doc.setups[0].operations = vec![op.clone()];
        let sim = prepare(&doc, &mesh, 0, 0).unwrap();
        assert!(
            sim.plan
                .moves
                .iter()
                .all(|m| m.a.is_none() && m.motion != Motion::Index)
        );
        assert!(sim.turns.is_empty());
        let mut fresh = doc.setups[0].clone();
        fresh.p.insert("stock_source".into(), "fresh".into());
        let direct = crate::simulation::generate(&fresh, &op, &doc.tools[0], &mesh).unwrap();
        assert_eq!(direct.moves.len(), sim.plan.moves.len());
        for (a, b) in direct.moves.iter().zip(&sim.plan.moves) {
            assert_eq!(
                (a.from, a.to, a.motion, a.start, a.end),
                (b.from, b.to, b.motion, b.start, b.end)
            );
        }
        // The indexed setup after it starts with A turning from 0 to 90.
        let indexed = prepare(&doc, &mesh, 1, 0).unwrap();
        assert_eq!(indexed.plan.moves[0].motion, Motion::Index);
        assert_eq!(indexed.turns.len(), 1);
        assert_eq!(
            indexed.turns[0].1,
            doc.at_angle(1, 90., &mesh).unwrap().frame
        );
    }

    #[test]
    fn a_setup_posed_at_an_angle_has_the_frame_at_angle_gives() {
        let (doc, mesh) = job("0, 90, 180, -90");
        for a in [0., 90., 180., -90., 37.] {
            let posed = doc.setup_at(&doc.setups[1], 1, a, &mesh).unwrap();
            let frame = crate::volume::Frame::new(&posed, &mesh);
            let at = doc.at_angle(1, a, &mesh).unwrap().frame;
            for p in [[1., 2., 3.], [-20., 15., -4.], mesh.origin] {
                assert!(close(frame.map(p), at.map(p)), "A{a}");
            }
        }
    }

    #[test]
    fn indexed_operations_of_an_older_project_move_into_indexed_setups() {
        use crate::model::{Operation, Strategy};
        let (mut doc, _) = job("90");
        doc.setups.truncate(1);
        let tool = doc.tools[0].id.clone();
        let old = |name: &str, a: &str| {
            let mut op = Operation::new(Strategy::Indexed, tool.clone());
            op.p.insert("name".into(), name.into());
            op.p.insert("angle_a".into(), a.into());
            op.p.insert("angle_b".into(), "0".into());
            op
        };
        let face = Operation::new(Strategy::Face, tool.clone());
        let after = Operation::new(Strategy::Contour, tool.clone());
        let ids = [face.id.clone(), after.id.clone()];
        doc.setups[0].operations = vec![face, old("At zero", "0"), old("Side", "90"), after];
        let notes = doc.migrate_indexed_operations();
        let names: Vec<&str> = doc.setups.iter().map(|s| s.name()).collect();
        let first = doc.setups[0].name().to_string();
        assert_eq!(
            names,
            [
                first.clone(),
                format!("{first} · A 90°"),
                format!("{first} · A 0°")
            ]
        );
        let ops = |i: usize| -> Vec<(Strategy, String)> {
            doc.setups[i]
                .operations
                .iter()
                .map(|o| (o.strategy, o.name().to_string()))
                .collect()
        };
        assert_eq!(ops(0)[1], (Strategy::Face, "At zero".to_string()));
        assert_eq!(doc.setups[0].operations[0].id, ids[0]);
        assert_eq!(ops(1), [(Strategy::Face, "Side".to_string())]);
        assert_eq!(doc.setups[2].operations[0].id, ids[1]);
        // The new setups index A in setup 1's clamping, in its order.
        for (i, a) in [(1, "90"), (2, "0")] {
            assert!(is_indexed(&doc.setups[i]));
            assert_eq!(get(&doc.setups[i].p, ANGLES), a);
            assert_eq!(doc.index_base(i), Some(0));
            assert_eq!(
                get(&doc.setups[i].p, "origin"),
                get(&doc.setups[0].p, "origin")
            );
        }
        assert!(!doc.setups[1].operations[0].p.contains_key("angle_a"));
        assert!(notes[0].contains("Facing operation in"), "{notes:?}");
        assert!(
            notes[1].contains("new indexed setup at that angle"),
            "{notes:?}"
        );
        // On a machine without A around X it stays, and says why.
        let (mut doc, _) = job("90");
        doc.setups.truncate(1);
        doc.setups[0].machine = Machine::new(5);
        doc.setups[0].operations = vec![old("Tilted", "30")];
        let notes = doc.migrate_indexed_operations();
        assert_eq!(doc.setups.len(), 1);
        let op = &doc.setups[0].operations[0];
        assert_eq!(op.strategy, Strategy::Indexed);
        assert!(notes[0].ends_with(LEGACY));
        assert_eq!(
            op.issues(&doc.setups[0].machine, &doc.tools, &[])[0],
            LEGACY
        );
    }

    #[test]
    fn the_plan_check_checks_each_angles_moves_against_the_part_posed_there() {
        // 30 wide in Y and 10 high at A0, 10 wide at A90: the contour at
        // A90 runs 11 mm from the axis, inside where the part is at A0.
        let mesh = block(30., 30., 10.);
        let doc = bar_job(&mesh, 0., "80", "-150", "0, 90", contour(&cutter("12")));
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        let first = crate::cam::Geometry::new(&doc.setups[1], &mesh);
        let index = crate::analysis::Index::new(&first);
        let found = crate::plan_check::angle_problems(&doc, &mesh, 1, &sim, &first, &index);
        assert!(found.is_empty(), "{found:?}");
        // Against the part at A0 alone, the A90 moves would go through it.
        let wrong = crate::plan_check::axis_problems(&sim.plan, &first, &index, 1);
        assert!(wrong[0].contains("passes through"), "{wrong:?}");
    }

    #[test]
    fn the_swing_takes_the_centreline_the_control_page_measured() {
        let mesh = prism(30.);
        let mut doc = bar_job(&mesh, 0., "80", "-40", "0, 90", contour(&cutter("12")));
        assert!(crate::simulation::prepare(&doc, &mesh, 1, 0).is_err());
        // Measured at Z -150 in the machine library; the setup's copy still
        // says -40.
        let mut library = doc.setups[0].machine.clone();
        library.p.insert("a_centre_z".into(), "-150".into());
        library.p.insert(
            crate::control::CENTRE_MARK.into(),
            crate::control::MEASURED.into(),
        );
        doc.machines = vec![library];
        assert_eq!(get(&doc.setups[1].machine.p, "a_centre_z"), "-40");
        let swing = doc.swing(1, &mesh, 15.).unwrap();
        assert!((swing.machine_z - (-150. + swing.radius + 15.)).abs() < 1e-3);
        assert!(crate::simulation::prepare(&doc, &mesh, 1, 0).is_ok());
        // Gone stale, it is not trusted: the setup's copy is used again.
        doc.machines[0].p.insert(
            crate::control::CENTRE_MARK.into(),
            "stale after an E-stop".into(),
        );
        assert!(crate::simulation::prepare(&doc, &mesh, 1, 0).is_err());
    }

    #[test]
    fn an_indexed_setups_cuts_are_not_said_held_by_the_chuck_at_its_first_angle() {
        let mesh = prism(30.);
        let doc = bar_job(&mesh, 0., "80", "-150", "0, 90", contour(&cutter("12")));
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        let held = crate::post::held(&doc, &mesh, 1, &[(0, Ok(sim))]);
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].message, format!("Holding not checked: {NOT_HELD}."));
    }

    #[test]
    fn a_disabled_indexed_operation_at_an_angle_stays_where_it_is() {
        use crate::model::{Operation, Strategy};
        let (mut doc, _) = job("90");
        doc.setups.truncate(1);
        let tool = doc.tools[0].id.clone();
        let mut old = Operation::new(Strategy::Indexed, tool.clone());
        old.p.insert("angle_a".into(), "90".into());
        old.enabled = false;
        let after = Operation::new(Strategy::Contour, tool);
        doc.setups[0].operations = vec![old, after];
        let notes = doc.migrate_indexed_operations();
        assert_eq!(doc.setups.len(), 1);
        let ops = &doc.setups[0].operations;
        assert_eq!(
            (ops[0].strategy, ops[1].strategy),
            (Strategy::Indexed, Strategy::Contour)
        );
        assert!(notes[0].ends_with(LEGACY), "{notes:?}");
    }
}
