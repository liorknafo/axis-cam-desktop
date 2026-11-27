//! Parallels under the part (LIO-537): a setup on a mill may say that its
//! stock stands on parallels, how high and wide they are, which way they run
//! and where. The part is then raised off the table (or the vise's floor) by
//! their height: a through hole or slot over the open space between them
//! may break through the underside, and only a cut into a parallel, or past
//! their height into what they stand on, is refused.
//!
//! The parallels are boxes under the stock the setup starts from
//! (`boxes`, `holding::Shape::Support`), drawn in the 3D view and on the
//! manual Steps' Zero page and checked as a jaw is (`holding::hits`), but
//! for their top: the part's own material lies between a parallel and a
//! tool that does not break through, so only a tool below the underside
//! over a parallel (or within the gap beside one) hits it. The floor the
//! plan check keeps every tool above (`plan_check::axis_problems_over`) is
//! then the top of what they stand on.
use crate::holding::{HoldBox, Shape};
use crate::mesh::Bounds;
use crate::model::{Setup, get};

/// The setup key that says which way the parallels run: one of `RUNS`; empty
/// is "None".
pub const RUN: &str = "parallels";
/// The setup key of where they are: each one's centre across its run, in mm
/// from the centre of the stock the setup starts from, separated by `;`
/// ("-18; 18").
pub const AT: &str = "parallel_at";
/// The setup key of their height, mm: how far they raise the part.
pub const HEIGHT: &str = "parallel_height";
/// The setup key of their width across their run, mm; empty is `WIDTH_DEFAULT`.
pub const WIDTH: &str = "parallel_width";
/// What `RUN` may say: none, along the setup's Y, along its X.
pub const NONE: &str = "None";
pub const FRONT_TO_BACK: &str = "Front to back";
pub const LEFT_TO_RIGHT: &str = "Left to right";
pub const RUNS: [&str; 3] = [NONE, FRONT_TO_BACK, LEFT_TO_RIGHT];
/// How a parallel's box is named, before where it is: "parallel at X-18".
pub const LABEL: &str = "parallel at";
/// A parallel's width when the field is empty, mm.
pub const WIDTH_DEFAULT: f32 = 10.;
/// The most parallels a setup may stand on.
pub const MAX: usize = 6;

/// The parallels a setup's stock stands on.
#[derive(Debug, Clone, PartialEq)]
pub struct Parallels {
    /// They run along Y, front to back (each `at` is an X); else along X.
    pub along_y: bool,
    /// Each one's centre across its run, mm from the stock's centre.
    pub at: Vec<f32>,
    pub height: f32,
    pub width: f32,
}
impl Parallels {
    /// The axis each `at` is measured along: X for parallels running
    /// front to back.
    pub fn across(&self) -> usize {
        usize::from(!self.along_y)
    }
    /// The axis named: "X" or "Y".
    pub fn axis(&self) -> &'static str {
        ["X", "Y"][self.across()]
    }
    /// "X-18" for the parallel at `at`.
    pub fn place(&self, at: f32) -> String {
        format!("{}{}", self.axis(), signed(at))
    }
    /// Where they all are, as a sentence lists them: "X-18 and X+18".
    pub fn places(&self) -> String {
        let places: Vec<String> = self.at.iter().map(|a| self.place(*a)).collect();
        match places.as_slice() {
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
            [] => String::new(),
        }
    }
}

/// A number typed into a field: surrounding spaces are ignored.
fn number(text: &str) -> Option<f32> {
    text.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}
/// `v` mm for a message: up to two decimals, without trailing zeros.
pub fn mm(v: f32) -> String {
    match format!("{v:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
    {
        "-0" => "0".into(),
        text => text.into(),
    }
}
/// `v` mm with its sign: "+18", "-18", "0".
pub(crate) fn signed(v: f32) -> String {
    let t = mm(v);
    if t == "0" || t.starts_with('-') {
        t
    } else {
        format!("+{t}")
    }
}

/// Whether `setup` can stand on parallels as modelled here: on a mill, in a
/// vise, under clamps or with no workholding modelled. A lathe's chuck, the
/// chuck on a rotary A axis, a round bar's holder and a sheet's hold-down
/// hold the stock otherwise.
pub fn applies(setup: &Setup) -> bool {
    !crate::lathe::is_lathe(&setup.machine)
        && !crate::sheet::is_sheet(setup)
        && !crate::chuck::holds(setup)
        && matches!(get(&setup.p, "holding"), "" | "None" | "Vise" | "Clamps")
}

/// Whether `setup` says it stands on parallels, whatever else it says.
pub fn chosen(setup: &Setup) -> bool {
    !matches!(get(&setup.p, RUN).trim(), "" | NONE)
}

/// The parallels `setup` stands on; None when it says none. Refuses with
/// what is wrong when it says some but not readably, or where they cannot
/// be modelled (`applies`).
pub fn of(setup: &Setup) -> Result<Option<Parallels>, String> {
    let p = &setup.p;
    let along_y = match get(p, RUN).trim() {
        "" | NONE => return Ok(None),
        FRONT_TO_BACK => true,
        LEFT_TO_RIGHT => false,
        _ => {
            return Err(format!(
                "Parallels run {FRONT_TO_BACK} or {LEFT_TO_RIGHT}, or choose {NONE}."
            ));
        }
    };
    if !applies(setup) {
        return Err(format!(
            "Parallels are modelled under a vise's or clamps' stock on a mill, or with no workholding: choose {NONE} for the parallels."
        ));
    }
    let axis = if along_y { "X" } else { "Y" };
    let height = number(get(p, HEIGHT))
        .filter(|h| *h > 0.)
        .ok_or("Give the parallels' height in mm: how far they raise the part.")?;
    let width = match get(p, WIDTH).trim() {
        "" => WIDTH_DEFAULT,
        text => number(text)
            .filter(|w| *w > 0.)
            .ok_or("The parallels' width must be a positive number of mm.")?,
    };
    let where_ = || {
        format!(
            "Give where each parallel runs: its {axis} in mm from the stock's centre, separated by ; (for example -18; 18)."
        )
    };
    let mut at = vec![];
    for part in get(p, AT).split(';').filter(|s| !s.trim().is_empty()) {
        at.push(number(part).ok_or_else(where_)?);
    }
    if at.is_empty() {
        return Err(where_());
    }
    if at.len() > MAX {
        return Err(format!("Stand the part on 1 to {MAX} parallels."));
    }
    Ok(Some(Parallels {
        along_y,
        at,
        height,
        width,
    }))
}

/// How far `setup`'s parallels raise its stock, mm: 0 without parallels or
/// when they do not read (`of` says why, as a workholding issue).
pub fn height(setup: &Setup) -> f32 {
    of(setup).ok().flatten().map_or(0., |p| p.height)
}

/// What is wrong with `setup`'s parallels: they do not read, or (with the
/// stock it starts from, `stock`, when known) one is not under it.
pub fn issues(setup: &Setup, stock: Option<Bounds>) -> Vec<String> {
    let parallels = match of(setup) {
        Ok(Some(p)) => p,
        Ok(None) => return vec![],
        Err(why) => return vec![why],
    };
    let Some(stock) = stock else {
        return vec![];
    };
    let k = parallels.across();
    let half = (stock.max[k] - stock.min[k]) / 2.;
    (parallels.at.iter())
        .filter(|at| at.abs() > half)
        .map(|at| {
            format!(
                "The parallel at {} is not under the stock, which spans {} to {} from its centre.",
                parallels.place(*at),
                parallels.place(-half),
                parallels.place(half)
            )
        })
        .collect()
}

/// The parallels of `setup` as boxes under `stock`, the box it starts from
/// (its frame, mm): each its width wide, centred where it is said to be,
/// as long as the stock along its run and as high as the parallels, from
/// the stock's bottom down. None without parallels or when they do not read.
pub fn boxes(setup: &Setup, stock: Bounds) -> Vec<HoldBox> {
    match of(setup) {
        Ok(Some(p)) => boxes_of(&p, stock),
        _ => vec![],
    }
}
/// `boxes` of `parallels`.
pub fn boxes_of(parallels: &Parallels, stock: Bounds) -> Vec<HoldBox> {
    let k = parallels.across();
    let centre = (stock.min[k] + stock.max[k]) / 2.;
    (parallels.at.iter())
        .map(|&at| {
            let mut b = stock;
            let c = centre + at;
            (b.min[k], b.max[k]) = (c - parallels.width / 2., c + parallels.width / 2.);
            (b.min[2], b.max[2]) = (stock.min[2] - parallels.height, stock.min[2]);
            HoldBox {
                label: format!("{LABEL} {}", parallels.place(at)),
                bounds: b,
                shape: Shape::Support,
            }
        })
        .collect()
}

/// `setup`'s parallels in a sentence, for the Zero page and the setup's
/// description, as the machinist is told it: "Stand it on two 15 mm
/// parallels, 10 mm wide, running front to back at X-18 and X+18 from the
/// stock's centre." None without parallels or when they do not read.
pub fn sentence(setup: &Setup) -> Option<String> {
    let p = of(setup).ok().flatten()?;
    let places = p.places();
    let count = match p.at.len() {
        1 => "a",
        2 => "two",
        3 => "three",
        4 => "four",
        5 => "five",
        _ => "six",
    };
    Some(format!(
        "Stand it on {count} {} mm {}, {} mm wide, running {} at {places} from the stock's centre.",
        mm(p.height),
        if p.at.len() == 1 {
            "parallel"
        } else {
            "parallels"
        },
        mm(p.width),
        if p.along_y {
            "front to back"
        } else {
            "left to right"
        },
    ))
}

/// The line a through cut's Steps and editor say about what is under the
/// part (LIO-537): with parallels, that the passes are checked against them
/// and the table under them; without, that the machinist checks it.
pub fn under_the_part(setup: &Setup) -> Option<String> {
    let p = of(setup).ok().flatten()?;
    Some(format!(
        "Below the part, the passes are checked against the parallels at {} and the table {} mm down; verify anything else under it, such as a stop.",
        p.places(),
        mm(p.height)
    ))
}

/// How `past_the_table` ends what it says, before its advice.
const PAST_THE_TABLE: &str = " the part stands on, into the table or vise under them.";
/// Whether `message` is one `past_the_table` says (as the export's refusal
/// does), which the plan check's gouge stage says too.
pub fn past_the_table_said(message: &str) -> bool {
    message.contains(PAST_THE_TABLE)
}
/// How far the moves of `plan` go past the parallels `setup` stands the
/// stock (`stock`, the box it starts from) on, into the table or vise under
/// them, as a sentence; None without parallels or when every move keeps
/// above that (the floor `plan_check::axis_problems_over` holds a plan to).
pub fn past_the_table(
    setup: &Setup,
    stock: Bounds,
    plan: &crate::simulation::Plan,
) -> Option<String> {
    let p = of(setup).ok().flatten()?;
    let lowest = (plan.moves.iter())
        .map(|m| m.from[2].min(m.to[2]))
        .fold(f32::INFINITY, f32::min);
    let below = stock.min[2] - p.height - lowest;
    (below > 0.001).then(|| {
        format!(
            "the tool goes {} mm past the {} mm parallels{PAST_THE_TABLE} Reduce the depth or the breakthrough.",
            if below < 0.01 { format!("{below:.3}") } else { format!("{below:.2}") },
            mm(p.height)
        )
    })
}

/// Why the moves of `plan`, an operation named `operation` of `setup`,
/// cannot be followed on the parallels `setup` stands its stock (`stock`)
/// on: a move within `gap` (plus the machine's `blend`) of a parallel below
/// the part's underside, or one past their height into the table
/// (`past_the_table`); said in work coordinates (`datum`, the work zero).
/// Parallels chosen but not readable (a blank or bad height, say) or not
/// under the stock refuse with what is wrong (`issues`). None without parallels or when every move keeps
/// clear of them.
pub fn refusal(
    setup: &Setup,
    stock: Bounds,
    plan: &crate::simulation::Plan,
    gap: f32,
    blend: f32,
    operation: &str,
    datum: [f32; 3],
) -> Option<String> {
    // Parallels chosen but not readable, or not under the stock, are no
    // support to cut over: said as the setup's holding says it.
    if let Some(why) = issues(setup, Some(stock)).into_iter().next() {
        return Some(format!("{}: {why}", setup.name()));
    }
    let under = boxes(setup, stock);
    if under.is_empty() {
        return None;
    }
    if let Some(hit) = crate::holding::hits(plan, &under, gap + blend).first() {
        return Some(format!(
            "{} Stand the part so that the parallels are clear of its cuts.",
            crate::holding::message(operation, hit, gap, datum)
        ));
    }
    past_the_table(setup, stock, plan).map(|past| format!("{operation}: {past}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Machine, Setup};

    fn setup(fields: &[(&str, &str)]) -> Setup {
        let mut s = Setup::new(Machine::new(3), "G54");
        s.p.insert("holding".into(), "Clamps".into());
        for (k, v) in fields {
            s.p.insert(k.to_string(), v.to_string());
        }
        s
    }
    fn stock() -> Bounds {
        Bounds {
            min: [-69., -63.5, -20.],
            max: [69., 63.5, 0.],
        }
    }

    #[test]
    fn none_by_default_and_when_chosen_none() {
        assert_eq!(of(&setup(&[])), Ok(None));
        assert_eq!(of(&setup(&[(RUN, NONE), (HEIGHT, "15")])), Ok(None));
        assert!(boxes(&setup(&[]), stock()).is_empty());
        assert_eq!(height(&setup(&[])), 0.);
        assert_eq!(sentence(&setup(&[])), None);
    }

    #[test]
    fn two_parallels_front_to_back_stand_under_the_stock() {
        let s = setup(&[(RUN, FRONT_TO_BACK), (AT, "-18; 18"), (HEIGHT, "15")]);
        let p = of(&s).unwrap().unwrap();
        assert!(p.along_y);
        assert_eq!(p.at, vec![-18., 18.]);
        assert_eq!(p.width, WIDTH_DEFAULT);
        let b = boxes(&s, stock());
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].label, "parallel at X-18");
        assert_eq!(b[1].label, "parallel at X+18");
        assert_eq!(b[0].bounds.min, [-23., -63.5, -35.]);
        assert_eq!(b[0].bounds.max, [-13., 63.5, -20.]);
        assert_eq!(b[1].bounds.min[0], 13.);
        assert!(b.iter().all(|b| b.shape == Shape::Support));
        assert_eq!(
            sentence(&s).unwrap(),
            "Stand it on two 15 mm parallels, 10 mm wide, running front to back at X-18 and X+18 from the stock's centre."
        );
        assert!(issues(&s, Some(stock())).is_empty());
        assert_eq!(
            under_the_part(&s).unwrap(),
            "Below the part, the passes are checked against the parallels at X-18 and X+18 and the table 15 mm down; verify anything else under it, such as a stop."
        );
    }

    #[test]
    fn left_to_right_parallels_are_placed_in_y_from_an_off_centre_stock() {
        let s = setup(&[
            (RUN, LEFT_TO_RIGHT),
            (AT, "-20;20"),
            (HEIGHT, "8"),
            (WIDTH, "6"),
        ]);
        let stock = Bounds {
            min: [0., 10., 5.],
            max: [100., 70., 25.],
        };
        let b = boxes(&s, stock);
        assert_eq!(b[0].bounds.min, [0., 17., -3.]);
        assert_eq!(b[0].bounds.max, [100., 23., 5.]);
        assert_eq!(b[1].label, "parallel at Y+20");
    }

    #[test]
    fn what_does_not_read_is_said() {
        let no_height = setup(&[(RUN, FRONT_TO_BACK), (AT, "0")]);
        assert!(of(&no_height).unwrap_err().contains("height"));
        let no_place = setup(&[(RUN, FRONT_TO_BACK), (HEIGHT, "15")]);
        assert!(of(&no_place).unwrap_err().contains("its X"));
        let bad = setup(&[(RUN, LEFT_TO_RIGHT), (HEIGHT, "15"), (AT, "1; x")]);
        assert!(of(&bad).unwrap_err().contains("its Y"));
        let width = setup(&[
            (RUN, FRONT_TO_BACK),
            (HEIGHT, "15"),
            (AT, "0"),
            (WIDTH, "0"),
        ]);
        assert!(of(&width).unwrap_err().contains("width"));
        let way = setup(&[(RUN, "Diagonal"), (HEIGHT, "15"), (AT, "0")]);
        assert!(of(&way).is_err());
        // Unreadable parallels raise nothing and are said as an issue.
        assert_eq!(height(&no_height), 0.);
        assert_eq!(issues(&no_height, None).len(), 1);
    }

    #[test]
    fn a_parallel_off_the_stock_is_an_issue() {
        let s = setup(&[(RUN, FRONT_TO_BACK), (AT, "-18; 90"), (HEIGHT, "15")]);
        let found = issues(&s, Some(stock()));
        assert_eq!(
            found,
            [
                "The parallel at X+90 is not under the stock, which spans X-69 to X+69 from its centre."
            ]
        );
    }

    #[test]
    fn a_lathe_or_a_sheet_has_no_parallels() {
        let mut s = setup(&[(RUN, FRONT_TO_BACK), (AT, "0"), (HEIGHT, "15")]);
        s.p.insert("holding".into(), crate::round_hold::V_BLOCK.into());
        assert!(!applies(&s));
        assert!(of(&s).is_err());
        assert!(boxes(&s, stock()).is_empty());
    }
}
