//! The material library: what a material holds, the starter materials, and
//! the speed and feed check against a material's cutting data.
use crate::model::{Fields, Machine, Operation, Tool, fields, get, id, num};
use serde::{Deserialize, Serialize};

pub const GROUPS: [&str; 7] = [
    "Aluminium",
    "Steel",
    "Stainless steel",
    "Brass and copper",
    "Plastic",
    "Wood and composite",
    "Other",
];
/// Chip-load bands: key prefix and the largest tool diameter (mm) in the band.
pub const BANDS: [(&str, f32); 5] = [
    ("fz3", 3.),
    ("fz6", 6.),
    ("fz10", 10.),
    ("fz16", 16.),
    ("fzbig", f32::INFINITY),
];
/// Whether two material names are the same: trimmed, ignoring case (Unicode).
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}
/// The most characters a material's name may have; the checks name it.
pub const NAME_LIMIT: usize = 60;
pub const STARTER_NOTE: &str =
    "Starting point for carbide tools: adjust to your machine and your tool maker's data.";
/// The starter set a library has been given: 1 is the first nine, 2 adds
/// six common aluminium alloys, 3 adds seven woods. A library is brought up
/// to it once.
pub const STARTER_SET: u32 = 3;
/// The starters set 2 added, by name.
pub const ADDED_IN_SET_2: [&str; 6] = [
    "Aluminium 6082-T6",
    "Aluminium 6063-T6",
    "Aluminium 5083-H111",
    "Aluminium 5052-H32",
    "Aluminium 1050A-H14",
    "Cast aluminium plate (MIC-6)",
];
/// The starters set 3 added, by name.
pub const ADDED_IN_SET_3: [&str; 7] = [
    "Pine",
    "Oak",
    "Hard maple",
    "Beech",
    "Walnut",
    "Cherry",
    "Birch plywood",
];
/// The starters added after set `set`, by name: what a library given that
/// set is still missing.
pub fn added_since(set: u32) -> Vec<&'static str> {
    [(2, &ADDED_IN_SET_2[..]), (3, &ADDED_IN_SET_3[..])]
        .into_iter()
        .filter(|(added, _)| set < *added)
        .flat_map(|(_, names)| names.iter().copied())
        .collect()
}
/// Every cutting-data key prefix: the surface speed, then each chip-load band.
fn bases() -> impl Iterator<Item = &'static str> {
    std::iter::once("vc").chain(BANDS.iter().map(|(k, _)| *k))
}
/// Every cutting-data key, in pairs.
fn keys() -> Vec<String> {
    bases()
        .flat_map(|k| [format!("{k}_min"), format!("{k}_max")])
        .collect()
}
/// What a cutting-data key prefix measures, such as "Chip load up to 6 mm".
fn what(base: &str) -> Option<String> {
    if base == "vc" {
        return Some("Surface speed".into());
    }
    let i = BANDS.iter().position(|(k, _)| *k == base)?;
    Some(match BANDS[i].1 {
        limit if limit.is_finite() => format!("Chip load up to {limit} mm"),
        _ => format!("Chip load over {} mm", BANDS[i - 1].1),
    })
}
/// A cutting-data key in words, such as "Chip load up to 6 mm, minimum".
pub fn label(key: &str) -> Option<String> {
    let (base, side) = key.rsplit_once('_')?;
    let side = match side {
        "min" => "minimum",
        "max" => "maximum",
        _ => return None,
    };
    Some(format!("{}, {side}", what(base)?))
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub p: Fields,
}
/// A material's complete cutting data: surface speed (m/min) and chip load
/// per tooth (mm) for each band of `BANDS`, as (min, max).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CuttingData {
    pub vc: (f32, f32),
    pub fz: [(f32, f32); 5],
}
impl Material {
    pub fn new(name: &str) -> Self {
        Self {
            id: id(),
            p: fields(&[("name", name), ("group", "Other")]),
        }
    }
    pub fn name(&self) -> &str {
        get(&self.p, "name")
    }
    fn positive(&self, key: &str) -> Option<f32> {
        num(&self.p, key).filter(|v| *v > 0.)
    }
    fn bounds(&self, base: &str) -> Option<(f32, f32)> {
        Some((
            self.positive(&format!("{base}_min"))?,
            self.positive(&format!("{base}_max"))?,
        ))
    }
    fn pair(&self, key: &str) -> Option<(f32, f32)> {
        self.bounds(key).filter(|(min, max)| min <= max)
    }
    pub fn cutting_data(&self) -> Option<CuttingData> {
        let vc = self.pair("vc")?;
        let mut fz = [(0., 0.); 5];
        for (slot, (key, _)) in fz.iter_mut().zip(BANDS) {
            *slot = self.pair(key)?;
        }
        Some(CuttingData { vc, fz })
    }
    pub fn issues(&self) -> Vec<String> {
        let mut e = vec![];
        if self.name().trim().is_empty() {
            e.push("Give the material a name.".into());
        } else if self.name().trim().chars().count() > NAME_LIMIT {
            e.push(format!("The name can be at most {NAME_LIMIT} characters."));
        }
        if !GROUPS.contains(&get(&self.p, "group")) {
            e.push("Choose a material group.".into());
        }
        if get(&self.p, "notes").chars().count() > 500 {
            e.push("Notes can be at most 500 characters.".into());
        }
        let keys = keys();
        let filled = keys
            .iter()
            .filter(|k| !get(&self.p, k).trim().is_empty())
            .count();
        if filled > 0 && filled < keys.len() {
            e.push("Fill in every cutting-data field, or leave them all empty.".into());
        }
        if filled == keys.len() {
            for k in &keys {
                if self.positive(k).is_none() {
                    e.push(format!(
                        "{}, must be a positive number.",
                        label(k).unwrap_or_else(|| k.clone())
                    ));
                }
            }
            for base in bases() {
                if self.bounds(base).is_some() && self.pair(base).is_none() {
                    e.push(format!(
                        "{}: the minimum must not exceed the maximum.",
                        what(base).unwrap_or_else(|| base.into())
                    ));
                }
            }
        }
        e
    }
}
/// The chip-load band of a tool diameter (index into `BANDS`).
pub fn band(diameter: f32) -> usize {
    BANDS
        .iter()
        .position(|(_, limit)| diameter <= *limit)
        .unwrap_or(4)
}
/// The result of checking one operation's speed and feed.
#[derive(Debug, Clone, PartialEq)]
pub enum SpeedFeed {
    InRange {
        vc: f32,
        fz: f32,
    },
    /// Each problem names the value and the material's range.
    Out(Vec<String>),
    /// Why the operation could not be checked.
    Unchecked(String),
}
/// `machine`'s maximum spindle speed (rpm) and feed (mm/min), infinite
/// when it gives none: the same maximum the model is told
/// (`analysis::machine_report`), the UC100 profile's spindle maximum when it
/// is lower; a profile that cannot be read leaves the saved rpm.
fn limits(machine: &Machine) -> (f32, f32) {
    let inch = if get(&machine.p, "units") == "inch" {
        25.4
    } else {
        1.
    };
    let max_rpm = num(&machine.p, "rpm").unwrap_or(f32::INFINITY).min(
        crate::control::spindle_cap(machine)
            .ok()
            .flatten()
            .unwrap_or(f32::INFINITY),
    );
    let max_feed = num(&machine.p, "feed").map_or(f32::INFINITY, |v| v * inch);
    (max_rpm, max_feed)
}
/// `value` rounded to 10 when that stays within `low`–`high`, else to a
/// whole number within them.
fn tens(value: f32, low: f32, high: f32) -> f32 {
    let ten = (value / 10.).round() * 10.;
    if (low..=high).contains(&ten) {
        ten
    } else if low.ceil() <= high.floor() {
        value.round().clamp(low.ceil(), high.floor())
    } else {
        value
    }
}
/// How much of its band's chip load a Ø`diameter` tool takes: all of it from
/// the smallest band's 3 mm up, and under that in proportion to the
/// diameter, since a small cutter takes a thinner chip (a Ø0.5 a sixth of a
/// Ø3's) and snaps at a big one.
pub fn chip_share(diameter: f32) -> f32 {
    (diameter / BANDS[0].1).clamp(0., 1.)
}
/// The chip-load range (mm per tooth) a Ø`diameter` tool takes in `data`:
/// its band's, scaled by `chip_share`.
pub fn chip_load(data: &CuttingData, diameter: f32) -> (f32, f32) {
    let (min, max) = data.fz[band(diameter)];
    let share = chip_share(diameter);
    (min * share, max * share)
}
/// The spindle speed (rpm) and feed (mm/min) to run at within `bottom`–`top`
/// rpm and a chip load of `fz` (mm per tooth) with `flutes`, on a machine
/// whose maximums are `max_rpm` and `max_feed`. The speed is the middle of
/// the range, but no faster than lets the maximum feed reach the least chip
/// load (a capped feed at a fast spindle makes thin chips that rub), and no
/// slower than a quarter of the maximum rpm, where a router's spindle loses
/// its torque, unless the range is. The feed is the middle of the chip-load
/// range at that speed, up to the maximum feed. Both are rounded to 10 where
/// that stays in range.
pub fn pick(
    (bottom, top): (f32, f32),
    (fz_min, fz_max): (f32, f32),
    flutes: f32,
    max_rpm: f32,
    max_feed: f32,
) -> (f32, f32) {
    let slowest = (max_rpm / 4.).max(bottom).min(top);
    let fastest = (max_feed / (fz_min * flutes)).max(slowest);
    let rpm = tens(((bottom + top) / 2.).clamp(slowest, fastest), bottom, top);
    let high = (fz_max * flutes * rpm).min(max_feed);
    let low = (fz_min * flutes * rpm).min(high);
    (rpm, tens((low + high) / 2., low, high))
}
/// The spindle speed (rpm) and feed (mm/min) for `tool` in `material` on
/// `machine`, which `check` finds in range: `pick` within the material's
/// surface speed range up to the machine's maximum rpm, at the chip load the
/// tool takes (`chip_load`). None without the material's cutting data, the
/// tool's diameter and flutes, or the machine's maximum rpm and feed: without
/// those a recommendation could be any speed.
pub fn recommend(
    material: &Material,
    tool: &Tool,
    machine: &Machine,
) -> Result<(f32, f32), String> {
    let name = crate::proposal::clip(material.name(), NAME_LIMIT);
    let Some(data) = material.cutting_data() else {
        return Err(format!("{name} has no cutting data."));
    };
    let (Some(d), Some(z)) = (
        num(&tool.p, "diameter").filter(|d| *d > 0.),
        num(&tool.p, "flutes").filter(|z| *z >= 1.),
    ) else {
        return Err("The tool has no diameter or flutes.".into());
    };
    let (max_rpm, max_feed) = limits(machine);
    if !(max_rpm.is_finite() && max_rpm > 0. && max_feed.is_finite() && max_feed > 0.) {
        return Err("The machine has no maximum spindle speed or feed.".into());
    }
    let rpm_at = |vc: f32| vc * 1000. / (std::f32::consts::PI * d);
    let top = rpm_at(data.vc.1).min(max_rpm);
    let bottom = rpm_at(data.vc.0).min(top);
    Ok(pick(
        (bottom, top),
        chip_load(&data, d),
        z,
        max_rpm,
        max_feed,
    ))
}
/// A chip load in mm as a check says it: 3 decimals, and a fourth under
/// 0.01 mm, where a small cutter's lie, unless it is 0.
fn chip(mm: f32) -> String {
    if mm < 0.01 {
        let four = format!("{mm:.4}");
        four.strip_suffix('0').map_or(four.clone(), str::to_string)
    } else {
        format!("{mm:.3}")
    }
}
/// Checks an operation's spindle speed and feed against `material`, for
/// `tool` on `machine` (whose maximum rpm and feed count as in range when the
/// material asks for more; on a UC100 machine the profile's spindle maximum
/// counts when it is lower), at the chip load the tool takes (`chip_load`).
/// 1 % slack on every limit absorbs rounding.
pub fn check(material: &Material, tool: &Tool, op: &Operation, machine: &Machine) -> SpeedFeed {
    let name = crate::proposal::clip(material.name(), NAME_LIMIT);
    let Some(data) = material.cutting_data() else {
        return SpeedFeed::Unchecked(format!("{name} has no cutting data."));
    };
    let (Some(d), Some(z)) = (num(&tool.p, "diameter"), num(&tool.p, "flutes")) else {
        return SpeedFeed::Unchecked("The tool has no diameter or flutes.".into());
    };
    let (Some(n), Some(f)) = (num(&op.p, "rpm"), num(&op.p, "feed")) else {
        return SpeedFeed::Unchecked("The operation has no spindle speed or feed.".into());
    };
    if d <= 0. || z < 1. || n <= 0. || f <= 0. {
        return SpeedFeed::Unchecked(
            "The tool or the operation has no positive speed data.".into(),
        );
    }
    let (max_rpm, max_feed) = limits(machine);
    let vc = std::f32::consts::PI * d * n / 1000.;
    let fz = f / (n * z);
    let (vc_min, vc_max) = data.vc;
    let (fz_min, fz_max) = chip_load(&data, d);
    let mut out = vec![];
    if vc > vc_max * 1.01 {
        out.push(format!(
            "Surface speed {vc:.0} m/min is above {name}'s {vc_min:.0}–{vc_max:.0} m/min."
        ));
    } else if vc < vc_min * 0.99 && n < max_rpm - 0.5 {
        out.push(format!(
            "Surface speed {vc:.0} m/min is below {name}'s {vc_min:.0}–{vc_max:.0} m/min."
        ));
    }
    let (shown, low, high) = (chip(fz), chip(fz_min), chip(fz_max));
    if fz > fz_max * 1.01 {
        out.push(format!(
            "Chip load {shown} mm per tooth is above {name}'s {low}–{high} mm for a Ø{d} mm tool."
        ));
    } else if fz < fz_min * 0.99 && f < max_feed - 0.5 {
        out.push(format!(
            "Chip load {shown} mm per tooth is below {name}'s {low}–{high} mm for a Ø{d} mm tool."
        ));
    }
    if out.is_empty() {
        SpeedFeed::InRange { vc, fz }
    } else {
        SpeedFeed::Out(out)
    }
}
/// The starter library (spec table "Starter materials").
pub fn starters() -> Vec<Material> {
    type Row = (
        &'static str,
        &'static str,
        &'static str,
        (f32, f32),
        [(f32, f32); 5],
        &'static str,
    );
    let rows: [Row; 22] = [
        (
            "Aluminium 6061-T6",
            "Aluminium",
            "95 HB",
            (150., 300.),
            [
                (0.010, 0.030),
                (0.020, 0.050),
                (0.030, 0.080),
                (0.050, 0.100),
                (0.060, 0.120),
            ],
            "Flood or mist; clear chips.",
        ),
        (
            "Aluminium 7075-T6",
            "Aluminium",
            "150 HB",
            (150., 280.),
            [
                (0.010, 0.025),
                (0.020, 0.045),
                (0.030, 0.070),
                (0.040, 0.090),
                (0.050, 0.110),
            ],
            "Flood or mist; clear chips.",
        ),
        (
            "Mild steel 1018",
            "Steel",
            "126 HB",
            (80., 150.),
            [
                (0.005, 0.015),
                (0.010, 0.030),
                (0.020, 0.050),
                (0.030, 0.070),
                (0.040, 0.090),
            ],
            "Flood; avoid rubbing at low feed.",
        ),
        (
            "Stainless steel 304",
            "Stainless steel",
            "201 HB",
            (50., 100.),
            [
                (0.005, 0.012),
                (0.010, 0.025),
                (0.015, 0.040),
                (0.020, 0.050),
                (0.030, 0.070),
            ],
            "Flood; keep the tool cutting, it work-hardens.",
        ),
        (
            "Brass C360",
            "Brass and copper",
            "78 HRB",
            (150., 300.),
            [
                (0.010, 0.030),
                (0.020, 0.050),
                (0.030, 0.070),
                (0.040, 0.090),
                (0.050, 0.110),
            ],
            "Dry or air; sharp tools.",
        ),
        (
            "POM (Delrin)",
            "Plastic",
            "",
            (150., 400.),
            [
                (0.020, 0.050),
                (0.040, 0.100),
                (0.060, 0.150),
                (0.080, 0.200),
                (0.100, 0.250),
            ],
            "Air; avoid melting with high feed per tooth.",
        ),
        (
            "Acrylic (PMMA)",
            "Plastic",
            "",
            (100., 300.),
            [
                (0.020, 0.050),
                (0.030, 0.080),
                (0.050, 0.120),
                (0.060, 0.150),
                (0.080, 0.180),
            ],
            "Air; single-flute tools give the best finish.",
        ),
        (
            "HDPE",
            "Plastic",
            "",
            (150., 400.),
            [
                (0.030, 0.070),
                (0.050, 0.120),
                (0.080, 0.180),
                (0.100, 0.220),
                (0.120, 0.250),
            ],
            "Air; it deflects, use light finishing passes.",
        ),
        (
            "MDF",
            "Wood and composite",
            "",
            (200., 500.),
            [
                (0.030, 0.080),
                (0.050, 0.150),
                (0.080, 0.200),
                (0.100, 0.250),
                (0.120, 0.300),
            ],
            "Dust extraction; no coolant.",
        ),
        (
            "Aluminium 6082-T6",
            "Aluminium",
            "95 HB",
            (150., 300.),
            [
                (0.010, 0.030),
                (0.020, 0.050),
                (0.030, 0.080),
                (0.050, 0.100),
                (0.060, 0.120),
            ],
            "Flood or mist; clear chips. The common European plate and bar alloy; machines like 6061.",
        ),
        (
            "Aluminium 6063-T6",
            "Aluminium",
            "73 HB",
            (150., 300.),
            [
                (0.010, 0.030),
                (0.020, 0.050),
                (0.030, 0.080),
                (0.050, 0.100),
                (0.060, 0.120),
            ],
            "Flood or mist; cheap extrusion alloy, softer and gummier than 6061: sharp, polished tools.",
        ),
        (
            "Aluminium 5083-H111",
            "Aluminium",
            "75 HB",
            (120., 250.),
            [
                (0.010, 0.025),
                (0.020, 0.045),
                (0.030, 0.070),
                (0.040, 0.090),
                (0.050, 0.110),
            ],
            "Flood; gummy plate alloy: sharp, polished tools and clear chips.",
        ),
        (
            "Aluminium 5052-H32",
            "Aluminium",
            "60 HB",
            (120., 250.),
            [
                (0.010, 0.025),
                (0.020, 0.045),
                (0.030, 0.070),
                (0.040, 0.090),
                (0.050, 0.110),
            ],
            "Flood; gummy sheet alloy: sharp, polished tools and clear chips.",
        ),
        (
            "Aluminium 1050A-H14",
            "Aluminium",
            "34 HB",
            (100., 250.),
            [
                (0.010, 0.025),
                (0.015, 0.040),
                (0.025, 0.060),
                (0.035, 0.080),
                (0.045, 0.100),
            ],
            "Flood with lubricant; very soft and gummy: sharp, polished one- or two-flute tools.",
        ),
        (
            "Cast aluminium plate (MIC-6)",
            "Aluminium",
            "65 HB",
            (150., 300.),
            [
                (0.010, 0.030),
                (0.020, 0.050),
                (0.030, 0.080),
                (0.050, 0.100),
                (0.060, 0.120),
            ],
            "Mist or air; stable cast tooling plate that stays flat.",
        ),
        (
            "Pine",
            "Wood and composite",
            "Janka 1.7–3.1 kN",
            (100., 1200.),
            [
                (0.040, 0.100),
                (0.060, 0.180),
                (0.100, 0.250),
                (0.120, 0.300),
                (0.150, 0.350),
            ],
            "Dust extraction; no coolant. Soft and fibrous, as are spruce and fir: sharp tools and a light finishing pass, it fuzzes and tears across the grain. Resin gums up tools.",
        ),
        (
            "Oak",
            "Wood and composite",
            "Janka 5.7–6.0 kN",
            (100., 1000.),
            [
                (0.030, 0.080),
                (0.050, 0.130),
                (0.080, 0.180),
                (0.100, 0.220),
                (0.120, 0.280),
            ],
            "Dust extraction; no coolant. Open grain that chips out at edges: sharp tools and a light finishing pass.",
        ),
        (
            "Hard maple",
            "Wood and composite",
            "Janka 6.4 kN",
            (100., 1000.),
            [
                (0.030, 0.080),
                (0.050, 0.130),
                (0.080, 0.180),
                (0.100, 0.220),
                (0.120, 0.280),
            ],
            "Dust extraction; no coolant. Dense and burns: keep the chip load up and do not dwell.",
        ),
        (
            "Beech",
            "Wood and composite",
            "Janka 5.8 kN",
            (100., 1000.),
            [
                (0.030, 0.080),
                (0.050, 0.130),
                (0.080, 0.180),
                (0.100, 0.220),
                (0.120, 0.280),
            ],
            "Dust extraction; no coolant. Close grain that takes fine detail; burns if the tool rubs.",
        ),
        (
            "Walnut",
            "Wood and composite",
            "Janka 4.5 kN",
            (100., 1000.),
            [
                (0.030, 0.090),
                (0.050, 0.150),
                (0.080, 0.200),
                (0.100, 0.250),
                (0.120, 0.300),
            ],
            "Dust extraction; no coolant. Machines cleanly and takes fine detail.",
        ),
        (
            "Cherry",
            "Wood and composite",
            "Janka 4.2 kN",
            (100., 1000.),
            [
                (0.030, 0.090),
                (0.050, 0.150),
                (0.080, 0.200),
                (0.100, 0.250),
                (0.120, 0.300),
            ],
            "Dust extraction; no coolant. Burns easily: keep the chip load up and do not dwell.",
        ),
        (
            "Birch plywood",
            "Wood and composite",
            "",
            (100., 1000.),
            [
                (0.030, 0.080),
                (0.050, 0.150),
                (0.080, 0.200),
                (0.100, 0.250),
                (0.120, 0.300),
            ],
            "Dust extraction; no coolant. The glue lines dull tools; compression or down-cut tools keep the faces from splintering.",
        ),
    ];
    rows.into_iter()
        .map(|(name, group, hardness, vc, fz, note)| {
            let mut m = Material::new(name);
            m.p.insert("group".into(), group.into());
            m.p.insert("hardness".into(), hardness.into());
            m.p.insert("notes".into(), format!("{STARTER_NOTE} {note}"));
            m.p.insert("vc_min".into(), vc.0.to_string());
            m.p.insert("vc_max".into(), vc.1.to_string());
            for ((key, _), (min, max)) in BANDS.iter().zip(fz) {
                m.p.insert(format!("{key}_min"), min.to_string());
                m.p.insert(format!("{key}_max"), max.to_string());
            }
            m
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Machine, Operation, Strategy, Tool};
    fn op(rpm: &str, feed: &str) -> Operation {
        let mut o = Operation::new(Strategy::Pocket, String::new());
        o.p.insert("rpm".into(), rpm.into());
        o.p.insert("feed".into(), feed.into());
        o
    }
    fn alu() -> Material {
        starters()
            .into_iter()
            .find(|m| m.name() == "Aluminium 6061-T6")
            .unwrap()
    }
    #[test]
    fn starters_are_valid_and_complete() {
        let all = starters();
        assert_eq!(all.len(), 22);
        for m in &all {
            assert!(m.issues().is_empty(), "{}: {:?}", m.name(), m.issues());
            assert!(m.cutting_data().is_some(), "{}", m.name());
            assert!(get(&m.p, "notes").starts_with(STARTER_NOTE));
            assert!(GROUPS.contains(&get(&m.p, "group")));
        }
        let mut names: Vec<_> = all.iter().map(|m| m.name().to_lowercase()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 22);
        let data = alu().cutting_data().unwrap();
        assert_eq!(data.vc, (150., 300.));
        assert_eq!(data.fz[1], (0.02, 0.05));
    }
    #[test]
    fn the_second_starter_set_is_six_common_aluminium_alloys() {
        let all = starters();
        for name in ADDED_IN_SET_2 {
            let m = all.iter().find(|m| m.name() == name).expect(name);
            assert_eq!(get(&m.p, "group"), "Aluminium", "{name}");
        }
        let alloy = |name: &str| {
            all.iter()
                .find(|m| m.name() == name)
                .unwrap()
                .cutting_data()
                .unwrap()
        };
        assert_eq!(alloy("Aluminium 6082-T6").vc, (150., 300.));
        assert_eq!(alloy("Aluminium 5083-H111").vc, (120., 250.));
        assert_eq!(alloy("Aluminium 1050A-H14").fz[0], (0.010, 0.025));
        // The first nine come first, in their old order.
        assert_eq!(all[0].name(), "Aluminium 6061-T6");
        assert_eq!(all[8].name(), "MDF");
    }
    #[test]
    fn the_third_starter_set_is_seven_woods() {
        let all = starters();
        for name in ADDED_IN_SET_3 {
            let m = all.iter().find(|m| m.name() == name).expect(name);
            assert_eq!(get(&m.p, "group"), "Wood and composite", "{name}");
        }
        let wood = |name: &str| {
            all.iter()
                .find(|m| m.name() == name)
                .unwrap()
                .cutting_data()
                .unwrap()
        };
        assert_eq!(wood("Pine").vc, (100., 1200.));
        assert_eq!(wood("Oak").vc, (100., 1000.));
        assert_eq!(wood("Hard maple").fz[1], (0.05, 0.13));
        // The first fifteen come first, in their old order.
        assert_eq!(all[14].name(), "Cast aluminium plate (MIC-6)");
        assert_eq!(all[15].name(), "Pine");
        // What a library given each set is still missing.
        assert_eq!(added_since(1).len(), 13);
        assert_eq!(added_since(2), ADDED_IN_SET_3);
        assert!(added_since(STARTER_SET).is_empty());
    }
    #[test]
    fn cutting_data_must_be_complete_positive_and_ordered() {
        let mut m = Material::new("Test");
        assert!(m.cutting_data().is_none() && m.issues().is_empty());
        m.p.insert("vc_min".into(), "100".into());
        assert!(
            m.issues()
                .iter()
                .any(|e| e.contains("every cutting-data field"))
        );
        let mut full = alu();
        full.p.insert("vc_min".into(), "400".into());
        assert!(full.issues().iter().any(|e| e.contains("Surface speed")));
        assert!(full.cutting_data().is_none());
        let mut negative = alu();
        negative.p.insert("fz6_min".into(), "-0.01".into());
        assert!(!negative.issues().is_empty());
        let mut blank = alu();
        blank.p.insert("name".into(), " ".into());
        assert!(blank.issues().iter().any(|e| e.contains("name")));
        let mut long = alu();
        long.p.insert("notes".into(), "x".repeat(501));
        assert!(long.issues().iter().any(|e| e.contains("500")));
    }
    #[test]
    fn a_bad_value_is_named_in_words_without_a_false_ordering_message() {
        for (key, value, expected) in [
            (
                "fz6_min",
                "-0.01",
                "Chip load up to 6 mm, minimum, must be a positive number.",
            ),
            (
                "vc_max",
                "abc",
                "Surface speed, maximum, must be a positive number.",
            ),
            (
                "fzbig_max",
                "0",
                "Chip load over 16 mm, maximum, must be a positive number.",
            ),
            (
                "fz3_min",
                "",
                "Fill in every cutting-data field, or leave them all empty.",
            ),
        ] {
            let mut m = alu();
            m.p.insert(key.into(), value.into());
            assert_eq!(m.issues(), [expected], "{key}");
        }
        let mut both = alu();
        both.p.insert("vc_min".into(), "x".into());
        both.p.insert("fz10_max".into(), "-1".into());
        assert_eq!(
            both.issues(),
            [
                "Surface speed, minimum, must be a positive number.",
                "Chip load up to 10 mm, maximum, must be a positive number."
            ]
        );
        assert_eq!(label("fz16_max").unwrap(), "Chip load up to 16 mm, maximum");
        assert!(label("name").is_none() && label("fz7_min").is_none());
    }
    #[test]
    fn a_name_is_at_most_60_characters() {
        let issues = |name: &str| {
            let mut m = alu();
            m.p.insert("name".into(), name.into());
            m.issues()
        };
        assert!(issues(&"n".repeat(60)).is_empty());
        assert!(issues(&format!("  {}  ", "é".repeat(60))).is_empty());
        assert_eq!(
            issues(&"n".repeat(61)),
            ["The name can be at most 60 characters."]
        );
        assert_eq!(NAME_LIMIT, 60);
    }
    #[test]
    fn names_match_ignoring_case_and_surrounding_spaces() {
        assert!(same_name(" Édelstahl 304", "édelstahl 304  "));
        assert!(same_name("MDF", "mdf"));
        assert!(!same_name("MDF", "MDF 2"));
    }
    #[test]
    fn the_ordering_message_needs_two_positive_numbers() {
        let mut m = alu();
        m.p.insert("vc_min".into(), "400".into());
        m.p.insert("fzbig_min".into(), "0.5".into());
        assert_eq!(
            m.issues(),
            [
                "Surface speed: the minimum must not exceed the maximum.",
                "Chip load over 16 mm: the minimum must not exceed the maximum."
            ]
        );
        // A minimum equal to the maximum is fine.
        let mut equal = alu();
        equal.p.insert("vc_min".into(), "300".into());
        assert!(equal.issues().is_empty() && equal.cutting_data().is_some());
        // A non-positive or non-numeric partner is reported on its own.
        let mut zero = alu();
        zero.p.insert("vc_max".into(), "0".into());
        assert_eq!(zero.issues().len(), 1);
        assert!(zero.issues()[0].contains("must be a positive number"));
        assert!(zero.cutting_data().is_none());
    }
    #[test]
    fn bands_follow_the_tool_diameter() {
        assert_eq!(
            [2.9, 3., 3.1, 6., 10., 16., 16.1, 50.].map(band),
            [0, 0, 1, 1, 2, 3, 4, 4]
        );
    }
    #[test]
    fn surface_speed_and_chip_load_are_checked_with_machine_limits() {
        let machine = Machine::new(3); // 12000 rpm, 5000 mm/min
        let tool = Tool::new("Flat 6", "Flat end mill", "6"); // 3 flutes
        // 8000 rpm: Vc = π·6·8000/1000 = 150.8 m/min; 1000 mm/min: fz = 1000/(8000·3) = 0.0417
        assert!(matches!(
            check(&alu(), &tool, &op("8000", "1000"), &machine),
            SpeedFeed::InRange { .. }
        ));
        let SpeedFeed::Out(problems) = check(&alu(), &tool, &op("20000", "1000"), &machine) else {
            panic!()
        };
        assert!(
            problems[0]
                .contains("Surface speed 377 m/min is above Aluminium 6061-T6's 150–300 m/min"),
            "{problems:?}"
        );
        let SpeedFeed::Out(problems) = check(&alu(), &tool, &op("8000", "200"), &machine) else {
            panic!()
        };
        assert!(
            problems[0].contains("Chip load 0.0083 mm per tooth is below"),
            "{problems:?}"
        );
        // A 1 mm tool cannot reach 150 m/min under 12000 rpm: the machine's
        // maximum counts. (300 mm/min is 0.0083 mm a tooth, inside the
        // 0.003–0.010 mm a Ø1 takes.)
        let small = Tool::new("Flat 1", "Flat end mill", "1");
        assert!(matches!(
            check(&alu(), &small, &op("12000", "300"), &machine),
            SpeedFeed::InRange { .. }
        ));
        assert!(matches!(
            check(&alu(), &small, &op("11000", "300"), &machine),
            SpeedFeed::Out(_)
        ));
        // 1 % slack for rounding.
        assert!(matches!(
            check(&alu(), &tool, &op("7900", "1000"), &machine),
            SpeedFeed::InRange { .. }
        ));
        // Nothing to check against.
        assert!(matches!(
            check(
                &Material::new("Unknown"),
                &tool,
                &op("8000", "1000"),
                &machine
            ),
            SpeedFeed::Unchecked(_)
        ));
        let mut broken = tool.clone();
        broken.p.insert("flutes".into(), "0".into());
        assert!(matches!(
            check(&alu(), &broken, &op("8000", "1000"), &machine),
            SpeedFeed::Unchecked(_)
        ));
    }
    #[test]
    fn common_router_cuts_in_wood_are_in_range() {
        // A hobby router: 24000 rpm, 1000 mm/min.
        let mut router = Machine::new(3);
        router.p.insert("rpm".into(), "24000".into());
        router.p.insert("feed".into(), "1000".into());
        let wood = |name: &str| starters().into_iter().find(|m| m.name() == name).unwrap();
        let bit = |d: &str| {
            let mut t = Tool::new("Bit", "Flat end mill", d);
            t.p.insert("flutes".into(), "2".into());
            t
        };
        for (name, d, rpm, feed) in [
            // 1/8" at 18000 rpm: 180 m/min, the feed at the machine's maximum.
            ("Oak", "3.175", "18000", "1000"),
            // Ø6 slowed to 10000 rpm for a 0.05 mm chip: 188 m/min.
            ("Hard maple", "6", "10000", "1000"),
            // 1/2" at 18000 rpm: 718 m/min.
            ("Walnut", "12.7", "18000", "1000"),
            // Ø25 surfacing bit at 12000 rpm: 942 m/min.
            ("Birch plywood", "25", "12000", "1000"),
            ("Pine", "25", "15000", "1000"),
        ] {
            assert!(
                matches!(
                    check(&wood(name), &bit(d), &op(rpm, feed), &router),
                    SpeedFeed::InRange { .. }
                ),
                "{name} Ø{d} {rpm} rpm: {:?}",
                check(&wood(name), &bit(d), &op(rpm, feed), &router)
            );
        }
        // Rubbing still counts where the machine could feed faster.
        let SpeedFeed::Out(problems) = check(
            &wood("Cherry"),
            &bit("6"),
            &op("12000", "1000"),
            &Machine::new(3),
        ) else {
            panic!()
        };
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].starts_with("Chip load 0.042 mm per tooth is below"),
            "{problems:?}"
        );
    }
    fn machine(feed: &str, units: &str) -> Machine {
        let mut m = Machine::new(3);
        m.p.insert("feed".into(), feed.into());
        m.p.insert("units".into(), units.into());
        m
    }
    #[test]
    fn a_recommended_speed_and_feed_pass_the_check_for_every_starter_tool_and_machine() {
        let mut router = Machine::new(3);
        router.p.insert("rpm".into(), "24000".into());
        router.p.insert("feed".into(), "1000".into());
        let mut open = Machine::new(3);
        open.p.remove("rpm");
        open.p.remove("feed");
        // Without a maximum rpm and feed there is nothing to recommend from.
        assert_eq!(
            recommend(&alu(), &flat_6(), &open),
            Err("The machine has no maximum spindle speed or feed.".into())
        );
        let machines = [Machine::new(3), router.clone(), machine("40", "inch")];
        let mut checked = 0;
        for m in starters() {
            for machine in &machines {
                for d in ["1", "3", "6", "10", "20"] {
                    for z in ["1", "2", "3", "4"] {
                        let mut tool = Tool::new("Cutter", "Flat end mill", d);
                        tool.p.insert("flutes".into(), z.into());
                        let (rpm, feed) = recommend(&m, &tool, machine).unwrap();
                        let o = op(&rpm.to_string(), &feed.to_string());
                        assert!(
                            matches!(check(&m, &tool, &o, machine), SpeedFeed::InRange { .. }),
                            "{} Ø{d} z{z} {rpm} rpm {feed} mm/min: {:?}",
                            m.name(),
                            check(&m, &tool, &o, machine)
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 22 * 3 * 5 * 4);
        // Aluminium, Ø6 three-flute, 12000 rpm and 5000 mm/min: the middle of
        // 7958–12000 rpm, and of the 0.02–0.05 mm band's 599–1497 mm/min there.
        assert_eq!(
            recommend(&alu(), &flat_6(), &Machine::new(3)),
            Ok((9980., 1050.))
        );
        // Pine with a Ø3 three-flute on a 24000 rpm, 1000 mm/min router: the
        // feed cannot reach 0.04 mm a tooth even at the least surface speed,
        // so the spindle runs at that, for the thickest chip (0.031 mm).
        let pine = starters().into_iter().find(|m| m.name() == "Pine").unwrap();
        let three = Tool::new("Flat 3", "Flat end mill", "3");
        assert_eq!(recommend(&pine, &three, &router), Ok((10611., 1000.)));
        // A Ø10 would want 3333 rpm for the least chip load at 1000 mm/min:
        // it stops at a quarter of the router's 24000, where its spindle
        // still has torque.
        let ten = Tool::new("Flat 10", "Flat end mill", "10");
        assert_eq!(recommend(&pine, &ten, &router), Ok((6000., 1000.)));
        // A Ø50 surfacing cutter's middle, 4138 rpm, is under that too.
        let fifty = Tool::new("Surfacing 50", "Flat end mill", "50");
        assert_eq!(recommend(&pine, &fifty, &router).unwrap().0, 6000.);
        // A Ø0.5 two-flute takes a sixth of a Ø3's chip load: 0.0017–0.005 mm
        // in aluminium, not the 0.010–0.030 mm that snaps it.
        let mut tiny = Tool::new("Flat 0.5", "Flat end mill", "0.5");
        tiny.p.insert("flutes".into(), "2".into());
        let (rpm, feed) = recommend(&alu(), &tiny, &router).unwrap();
        let fz = feed / (rpm * 2.);
        assert!(
            (0.0016..=0.0051).contains(&fz),
            "{rpm} rpm {feed} mm/min: {fz}"
        );
        let too_much = op(&rpm.to_string(), &(0.015 * rpm * 2.).round().to_string());
        let SpeedFeed::Out(problems) = check(&alu(), &tiny, &too_much, &router) else {
            panic!()
        };
        assert!(
            problems[0].contains("is above Aluminium 6061-T6's 0.0017–0.005 mm for a Ø0.5 mm tool"),
            "{problems:?}"
        );
        // Nothing to recommend from.
        assert!(recommend(&Material::new("Unknown"), &three, &router).is_err());
        let mut fluteless = three.clone();
        fluteless.p.insert("flutes".into(), "0".into());
        assert!(recommend(&pine, &fluteless, &router).is_err());
    }
    #[test]
    fn the_machine_feed_limit_counts_in_its_own_units() {
        let tool = Tool::new("Flat 6", "Flat end mill", "6");
        // 8000 rpm, Ø6, 3 flutes: 300 mm/min is fz 0.0125, below the 0.02 minimum.
        let mm = machine("300", "mm");
        assert!(matches!(
            check(&alu(), &tool, &op("8000", "300"), &mm),
            SpeedFeed::InRange { .. }
        ));
        let SpeedFeed::Out(problems) = check(&alu(), &tool, &op("8000", "299"), &mm) else {
            panic!()
        };
        assert!(
            problems[0].contains("Chip load") && problems[0].contains("below"),
            "{problems:?}"
        );
        // The machine's 11.811 in/min is 300 mm/min; operation feeds are mm/min.
        let inch = machine("11.811", "inch");
        assert!(matches!(
            check(&alu(), &tool, &op("8000", "300"), &inch),
            SpeedFeed::InRange { .. }
        ));
        let SpeedFeed::Out(problems) = check(&alu(), &tool, &op("8000", "299"), &inch) else {
            panic!()
        };
        assert!(
            problems[0].contains("Chip load") && problems[0].contains("below"),
            "{problems:?}"
        );
    }
    /// An operation for the Ø6, 3-flute `flat_6` at `vc` m/min and `fz` mm per
    /// tooth, rounded to whole rpm and mm/min.
    fn at(vc: f32, fz: f32) -> Operation {
        let n = (vc * 1000. / (std::f32::consts::PI * 6.)).round();
        op(&n.to_string(), &(fz * n * 3.).round().to_string())
    }
    fn flat_6() -> Tool {
        Tool::new("Flat 6", "Flat end mill", "6")
    }
    /// The words of the problems `at(vc, fz)` runs into, none when it is in range.
    fn found(vc: f32, fz: f32) -> Vec<&'static str> {
        let machine = Machine::new(3);
        let SpeedFeed::Out(problems) = check(&alu(), &flat_6(), &at(vc, fz), &machine) else {
            return vec![];
        };
        problems
            .iter()
            .map(
                |p| match (p.starts_with("Surface"), p.contains(" is above ")) {
                    (true, true) => "speed above",
                    (true, false) => "speed below",
                    (false, true) => "load above",
                    (false, false) => "load below",
                },
            )
            .collect()
    }
    #[test]
    fn the_upper_limits_have_one_percent_slack() {
        // Ø6: 150–300 m/min, 0.02–0.05 mm
        assert!(found(300. * 1.005, 0.03).is_empty());
        assert_eq!(found(300. * 1.02, 0.03), ["speed above"]);
        assert!(found(200., 0.05 * 1.005).is_empty());
        assert_eq!(found(200., 0.05 * 1.02), ["load above"]);
    }
    #[test]
    fn a_value_between_one_and_two_percent_past_a_limit_is_refused() {
        // The slack is 1 %, not 2 %: halfway to 2 % is already out.
        assert_eq!(found(300. * 1.015, 0.03), ["speed above"]);
        assert_eq!(found(200., 0.05 * 1.015), ["load above"]);
        assert_eq!(found(150. * 0.985, 0.03), ["speed below"]);
        assert_eq!(found(200., 0.02 * 0.985), ["load below"]);
    }
    #[test]
    fn the_lower_limits_have_one_percent_slack() {
        assert!(found(150. * 0.995, 0.03).is_empty());
        assert_eq!(found(150. * 0.98, 0.03), ["speed below"]);
        assert!(found(200., 0.02 * 0.995).is_empty());
        assert_eq!(found(200., 0.02 * 0.98), ["load below"]);
    }
    #[test]
    fn the_materials_name_in_a_message_is_one_line_of_at_most_60_characters() {
        let machine = Machine::new(3);
        let mut odd = alu();
        odd.p.insert(
            "name".into(),
            format!("Odd\nalloy \u{202e}{}", "x".repeat(200)),
        );
        let SpeedFeed::Out(problems) = check(&odd, &flat_6(), &at(100., 0.03), &machine) else {
            panic!()
        };
        let name = problems[0]
            .strip_prefix("Surface speed 100 m/min is below ")
            .and_then(|rest| rest.strip_suffix("'s 150–300 m/min."))
            .unwrap();
        assert!(name.starts_with("Odd alloy xxx") && name.chars().count() == 60);
        let bare = Material::new(&format!("Odd\n{}", "y".repeat(200)));
        let SpeedFeed::Unchecked(why) = check(&bare, &flat_6(), &at(100., 0.03), &machine) else {
            panic!()
        };
        let name = why.strip_suffix(" has no cutting data.").unwrap();
        assert!(
            name.starts_with("Odd yyy") && name.chars().count() == 60,
            "{why}"
        );
    }
    #[test]
    fn documents_saved_before_the_library_still_load() {
        let old = r#"{"schema":1,"title":"Old","material":"Aluminum 6061","file":null,"machines":[],"tools":[],"setups":[],"active":""}"#;
        let doc: crate::model::Document = serde_json::from_str(old).unwrap();
        assert!(doc.materials.is_empty() && doc.material_id.is_empty());
        assert_eq!(doc.material, "Aluminum 6061");
    }
}
