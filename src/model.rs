use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

pub type Fields = BTreeMap<String, String>;
pub fn fields(values: &[(&str, &str)]) -> Fields {
    values
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
pub fn get<'a>(p: &'a Fields, key: &str) -> &'a str {
    p.get(key).map(String::as_str).unwrap_or("")
}
pub fn num(p: &Fields, key: &str) -> Option<f32> {
    get(p, key).parse::<f32>().ok().filter(|v| v.is_finite())
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Strategy {
    Face,
    Adaptive,
    Pocket,
    Contour,
    Drill,
    Parallel,
    Waterline,
    Scallop,
    Indexed,
    Rotary,
    FiveAxis,
}
impl Strategy {
    pub fn imported_preview(self) -> bool {
        matches!(
            self,
            Self::Face
                | Self::Pocket
                | Self::Contour
                | Self::Drill
                | Self::Parallel
                | Self::Waterline
        )
    }
    pub fn needs_surfaces(self) -> bool {
        matches!(self, Self::Face | Self::Pocket | Self::Drill)
    }
    pub fn availability(self) -> &'static str {
        match self {
            Self::Face => "Available · top-plane rectangular facing preview",
            Self::Pocket => "Available · planar pocket with islands, offset contours",
            Self::Contour => {
                "Available · outside silhouette or selected-floor inside contours; computer compensation"
            }
            Self::Drill => "Available · detected circular openings in a selected horizontal rim",
            Self::Parallel => "Available · sampled vertical flat/ball cutter surface finishing",
            Self::Waterline => "Available · constant-Z contours with a vertical flat cutter",
            Self::Adaptive => {
                "Unavailable · engagement-controlled adaptive kernel is not implemented"
            }
            Self::Scallop => "Unavailable · constant surface-spacing kernel is not implemented",
            Self::Indexed => {
                "Unavailable · indexed tool frames and rotary kinematics are not implemented"
            }
            Self::Rotary => {
                "Unavailable · continuous rotary paths and stock simulation are not implemented"
            }
            Self::FiveAxis => {
                "Unavailable · tool-axis planning and machine kinematics are not implemented"
            }
        }
    }
    pub const ALL: [Self; 11] = [
        Self::Face,
        Self::Adaptive,
        Self::Pocket,
        Self::Contour,
        Self::Drill,
        Self::Parallel,
        Self::Waterline,
        Self::Scallop,
        Self::Indexed,
        Self::Rotary,
        Self::FiveAxis,
    ];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap()
    }
    pub fn name(self) -> &'static str {
        [
            "Facing",
            "Adaptive clearing",
            "Pocket",
            "Contour",
            "Drilling",
            "Parallel finishing",
            "Waterline finishing",
            "Scallop finishing",
            "Indexed face",
            "Rotary finishing",
            "5-axis surface",
        ][self.index()]
    }
    pub fn short(self) -> &'static str {
        [
            "Face",
            "Adaptive",
            "Pocket",
            "Contour",
            "Drill",
            "Parallel",
            "Waterline",
            "Scallop",
            "Indexed",
            "Rotary",
            "5-axis",
        ][self.index()]
    }
    pub fn help(self) -> &'static str {
        [
            "Level the top of the stock. Select a face, set the cutting depth, then set the pass direction and overlap.",
            "Clear a pocket with a controlled radial engagement. Choose its floor, optimal load and stepdown; leave stock for finishing.",
            "Clear a closed pocket in depth layers. Choose the floor, stepover and number of finishing passes.",
            "Follow a wall or outer profile. Choose the chain, cutting side, compensation and depth passes.",
            "Make selected holes. Choose a drill, cycle and depth; peck and dwell settings depend on the cycle.",
            "Finish shallow surfaces using parallel passes. Set direction, surface spacing and the slope range.",
            "Finish steep walls at constant Z levels. Choose surfaces and a vertical stepdown.",
            "Finish surfaces at approximately constant spacing. Use rest machining to target a previous operation’s remaining stock.",
            "Position a rotary axis, lock it, then machine a face. Set the indexed angles within the machine travel limits.",
            "Finish around a rotary axis. Choose an axis, angular range and line, circular or spiral motion.",
            "Orient the cutter along a surface. Set axis control, tilt limits and lead/lean before checking machine motion.",
        ][self.index()]
    }
    pub fn features(self) -> &'static [usize] {
        match self {
            Self::Drill => &[5, 6, 7, 8],
            Self::Face | Self::Indexed => &[0, 1],
            Self::Adaptive | Self::Pocket => &[1],
            Self::Contour => &[2, 3],
            _ => &[0, 1, 2, 4],
        }
    }
    pub fn tabs(self) -> Vec<&'static str> {
        let mut v = vec!["Tool", "Geometry"];
        if matches!(self, Self::Indexed | Self::Rotary | Self::FiveAxis) {
            v.push("Orientation")
        }
        v.push("Heights");
        v.push(if self == Self::Drill {
            "Cycle"
        } else {
            "Passes"
        });
        v.push("Linking");
        v
    }
}
impl std::fmt::Display for Strategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}
pub const FEATURES: [&str; 9] = [
    "Top rim face",
    "Pocket floor",
    "Pocket walls",
    "Outer profile",
    "Center boss",
    "Hole 1 · Ø8",
    "Hole 2 · Ø8",
    "Hole 3 · Ø8",
    "Hole 4 · Ø8",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Machine {
    pub id: String,
    pub version: u32,
    pub p: Fields,
}
impl Machine {
    pub fn new(axes: u8) -> Self {
        let mut p = fields(&[
            ("name", "Workshop Mill"),
            ("axes", "3"),
            ("mode", "3-axis"),
            ("units", "mm"),
            ("controller", "LinuxCNC"),
            ("arrangement", "Fixed spindle"),
            ("x", "600"),
            ("y", "400"),
            ("z", "300"),
            ("rpm", "12000"),
            ("power", "800"),
            ("feed", "5000"),
            ("rotary_min", "-180"),
            ("rotary_max", "180"),
            ("rotary2_min", "-360"),
            ("rotary2_max", "360"),
            ("pivot_x", "0"),
            ("pivot_y", "0"),
            ("pivot_z", "0"),
            ("pivot2_x", "0"),
            ("pivot2_y", "0"),
            ("pivot2_z", "0"),
            ("direction", "Positive"),
            ("direction2", "Positive"),
            ("tool_change", "Manual"),
            ("tcp", "No"),
            ("post_tool_length", "Tool table (G43 H#)"),
            ("post_retract", "Machine Z home (G53 G0 Z0)"),
            ("post_blend", "0.01"),
            ("post_dwell", "2"),
            ("post_air", "M7"),
            ("probe_feed", "100"),
            ("probe_distance", "20"),
            ("probe_retract", "5"),
        ]);
        p.insert("axes".into(), axes.to_string());
        if axes > 3 {
            p.insert(
                "name".into(),
                if axes == 4 {
                    "Rotary Mill"
                } else {
                    "Trunnion Mill"
                }
                .into(),
            );
            p.insert(
                "mode".into(),
                if axes == 4 {
                    "Indexed (3+1)"
                } else {
                    "Indexed (3+2)"
                }
                .into(),
            );
            p.insert(
                "arrangement".into(),
                if axes == 4 {
                    "A table around X"
                } else {
                    "A/C table-table"
                }
                .into(),
            );
            p.insert("pivot_z".into(), "120".into());
        }
        Self {
            id: id(),
            version: 1,
            p,
        }
    }
    pub fn name(&self) -> &str {
        get(&self.p, "name")
    }
    pub fn axes(&self) -> u8 {
        get(&self.p, "axes").parse().unwrap_or(3)
    }
    pub fn issues(&self) -> Vec<String> {
        let mut e = vec![];
        if self.name().trim().is_empty() {
            e.push("Give the machine a name.".into())
        }
        for k in ["x", "y", "z", "rpm", "feed"] {
            positive(&self.p, k, &mut e)
        }
        // Optional: empty takes `forces::SPINDLE_POWER`.
        if !get(&self.p, "power").trim().is_empty() {
            positive(&self.p, "power", &mut e)
        }
        if !["3", "4", "5"].contains(&get(&self.p, "axes")) {
            e.push("Select 3, 4 or 5 axes.".into())
        }
        if self.axes() > 3 {
            range(&self.p, "rotary_min", "rotary_max", &mut e);
            for k in ["pivot_x", "pivot_y", "pivot_z"] {
                finite(&self.p, k, &mut e)
            }
        }
        if self.axes() == 5 {
            range(&self.p, "rotary2_min", "rotary2_max", &mut e);
            for k in ["pivot2_x", "pivot2_y", "pivot2_z"] {
                finite(&self.p, k, &mut e)
            }
        }
        if get(&self.p, "controller") == "UC100" && get(&self.p, "uc100_profile").trim().is_empty()
        {
            e.push("Choose the machine's UC100 profile file.".into())
        }
        e
    }
    pub fn fill_post_defaults(&mut self) {
        // The plate's thickness and the tip's diameter have no default to fill in.
        for key in POST_KEYS.into_iter().chain(PROBE_KEYS) {
            let value = post_setting(&self.p, key).to_string();
            if !value.is_empty() {
                self.p.insert(key.into(), value);
            }
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub id: String,
    pub p: Fields,
}
impl Tool {
    pub fn new(name: &str, kind: &str, diameter: &str) -> Self {
        Self {
            id: id(),
            p: fields(&[
                ("name", name),
                ("type", kind),
                ("diameter", diameter),
                ("flutes", "3"),
                ("length", "25"),
                ("stickout", "35"),
                ("holder", "32"),
            ]),
        }
    }
    pub fn name(&self) -> &str {
        get(&self.p, "name")
    }
    pub fn issues(&self) -> Vec<String> {
        let mut e = vec![];
        if self.name().trim().is_empty() {
            e.push("Give the tool a name.".into())
        }
        for k in ["diameter", "flutes", "length", "stickout", "holder"] {
            positive(&self.p, k, &mut e)
        }
        if num(&self.p, "flutes").is_some_and(|n| n.fract() != 0.) {
            e.push("Flutes must be a whole number.".into())
        }
        if num(&self.p, "stickout") < num(&self.p, "length") {
            e.push("Stickout must be at least the flute length.".into())
        }
        if get(&self.p, "type") == "Chamfer mill" && self.tip_angle().is_none() {
            e.push("Enter a chamfer mill tip angle from 20° to 160°.".into())
        }
        e
    }
    /// A chamfer mill's included tip angle in degrees (90 when not saved),
    /// None when it is outside 20–160°.
    pub fn tip_angle(&self) -> Option<f32> {
        let angle = if self.p.contains_key("tip_angle") {
            num(&self.p, "tip_angle")?
        } else {
            90.
        };
        (20. ..=160.).contains(&angle).then_some(angle)
    }
    /// Controller tool number (T#), when it is a whole number 1–9999.
    pub fn number(&self) -> Option<u32> {
        get(&self.p, "number")
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=TOOL_NUMBER_MAX).contains(n))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub strategy: Strategy,
    pub tool: String,
    pub enabled: bool,
    pub geometry: Vec<usize>,
    #[serde(default)]
    pub surfaces: Option<crate::surface::Selection>,
    pub p: Fields,
    #[serde(skip)]
    pub preview: bool,
}
pub const FACING_PATTERNS: [&str; 2] = ["Both ways (zigzag)", "One way"];
pub const FACING_DEPTH_ORDER: [&str; 2] = ["Continue from previous end", "Return to first start"];
pub const FACING_LINKS: [&str; 2] = ["Keep tool down", "Retract each pass"];
pub const SPINDLE_ROTATIONS: [&str; 2] = ["Clockwise (CW)", "Counterclockwise (CCW)"];
pub const FACING_MILLING: [&str; 2] = ["Climb", "Conventional"];
pub const POCKET_ENTRIES: [&str; 2] = ["Profile ramp", "Plunge"];
/// What a Pocket does with the holes through its floor that nothing of the
/// part covers or underlies: clears over them at the floor's height, or
/// cuts them through, Breakthrough past the bottom of their walls.
pub const POCKET_OPENINGS: [&str; 2] = ["Leave", "Cut through"];
pub fn pocket_setting<'a>(p: &'a Fields, key: &str) -> &'a str {
    p.get(key).map(String::as_str).unwrap_or(match key {
        "pocket_milling" => "Climb",
        "pocket_entry" => POCKET_ENTRIES[0],
        "openings" => POCKET_OPENINGS[0],
        _ => "",
    })
}
pub fn pocket_setting_issues(p: &Fields) -> Vec<String> {
    let mut issues = Vec::new();
    for (key, value, options) in [
        (
            "pocket_milling",
            pocket_setting(p, "pocket_milling"),
            FACING_MILLING,
        ),
        (
            "pocket_entry",
            pocket_setting(p, "pocket_entry"),
            POCKET_ENTRIES,
        ),
        ("openings", pocket_setting(p, "openings"), POCKET_OPENINGS),
        (
            "spindle_rotation",
            facing_setting(p, "spindle_rotation"),
            SPINDLE_ROTATIONS,
        ),
    ] {
        if !options.contains(&value) {
            issues.push(format!("Choose a valid {} setting.", key.replace('_', " ")));
        }
    }
    issues
}
/// Missing settings retain the behavior of projects saved before these controls.
pub fn facing_setting<'a>(p: &'a Fields, key: &str) -> &'a str {
    p.get(key).map(String::as_str).unwrap_or(match key {
        "facing_pattern" => FACING_PATTERNS[0],
        "facing_depth_order" => FACING_DEPTH_ORDER[1],
        "facing_link" => FACING_LINKS[1],
        "spindle_rotation" => SPINDLE_ROTATIONS[0],
        // Earlier one-way previews traversed +pass-angle with rows stepping left.
        "facing_milling" => FACING_MILLING[1],
        _ => "",
    })
}
pub fn facing_setting_issues(p: &Fields) -> Vec<String> {
    [
        ("facing_pattern", FACING_PATTERNS),
        ("facing_depth_order", FACING_DEPTH_ORDER),
        ("facing_link", FACING_LINKS),
        ("spindle_rotation", SPINDLE_ROTATIONS),
        ("facing_milling", FACING_MILLING),
    ]
    .into_iter()
    .filter(|&(key, values)| !values.contains(&facing_setting(p, key)))
    .map(|(key, _values)| format!("Choose a valid {} setting.", key.replace('_', " ")))
    .collect()
}
pub const TOOL_NUMBER_MAX: u32 = 9999;
pub const POST_TOOL_LENGTH: [&str; 2] = [
    "Tool table (G43 H#)",
    "Touch off each tool (one program per tool)",
];
pub const POST_RETRACT: [&str; 2] = ["Machine Z home (G53 G0 Z0)", "Setup clearance plane"];
pub const POST_AIR: [&str; 3] = ["M7", "M8", "Not available"];
pub const POST_KEYS: [&str; 5] = [
    "post_tool_length",
    "post_retract",
    "post_blend",
    "post_dwell",
    "post_air",
];
/// A UC100 machine's probe settings (`control::probe_settings`). The plate's
/// thickness and the probe tip's diameter have no default: they must be
/// measured.
pub const PROBE_KEYS: [&str; 5] = [
    "probe_plate",
    "probe_tip",
    "probe_feed",
    "probe_distance",
    "probe_retract",
];
/// Machines saved before the postprocessor or probe settings use these defaults.
pub fn post_setting<'a>(p: &'a Fields, key: &str) -> &'a str {
    p.get(key).map(String::as_str).unwrap_or(match key {
        "post_tool_length" => POST_TOOL_LENGTH[0],
        "post_retract" => POST_RETRACT[0],
        "post_blend" => "0.01",
        "post_dwell" => "2",
        "post_air" => POST_AIR[0],
        "probe_feed" => "100",
        "probe_distance" => "20",
        "probe_retract" => "5",
        _ => "",
    })
}
/// Export-only checks; they are deliberately not part of `Machine::issues`.
pub fn post_setting_issues(p: &Fields) -> Vec<String> {
    let mut e = vec![];
    for (key, values) in [
        ("post_tool_length", &POST_TOOL_LENGTH[..]),
        ("post_retract", &POST_RETRACT[..]),
        ("post_air", &POST_AIR[..]),
    ] {
        if !values.contains(&post_setting(p, key)) {
            e.push(format!("Choose a valid {} setting.", label(key)));
        }
    }
    let value = |k: &str| {
        post_setting(p, k)
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
    };
    if !value("post_blend").is_some_and(|v| v == 0. || (0.001..=1.).contains(&v)) {
        e.push("Path blending tolerance must be 0 or between 0.001 and 1 mm.".into());
    }
    if !value("post_dwell").is_some_and(|v| (0. ..=60.).contains(&v)) {
        e.push("Spindle start dwell must be between 0 and 60 seconds.".into());
    }
    e
}
/// The height above the stock top at which a new operation's tool travels
/// between cuts, mm (the "clearance" `Operation::new` starts with).
pub const CLEARANCE: f32 = 15.;
impl Operation {
    pub fn new(s: Strategy, tool: String) -> Self {
        let mut p = fields(&[
            ("name", s.name()),
            ("rpm", "8000"),
            ("feed", "1000"),
            ("coolant", "Flood"),
            ("boundary", "Selected geometry"),
            ("containment", "Tool inside boundary"),
            ("clearance", &CLEARANCE.to_string()),
            ("retract", "5"),
            ("top", "0"),
            ("bottom", "0"),
            ("stepdown", "2"),
            ("stepover", "40"),
            ("allowance", "0.3"),
            ("axial_allowance", "0.2"),
            ("tolerance", "0.01"),
            ("optimal_load", "1"),
            ("minimum_radius", "0.5"),
            ("direction", "0"),
            ("facing_pattern", "Both ways (zigzag)"),
            ("facing_depth_order", "Continue from previous end"),
            ("facing_link", "Keep tool down"),
            ("spindle_rotation", "Clockwise (CW)"),
            ("facing_milling", "Climb"),
            ("pocket_milling", "Climb"),
            ("pocket_entry", "Profile ramp"),
            ("finish_passes", "1"),
            ("compensation", "In computer"),
            ("side", "Inside"),
            ("surface_stepover", "0.5"),
            ("slope_min", "0"),
            ("slope_max", "90"),
            ("rest", "No"),
            ("rest_source", ""),
            ("cycle", "Drilling"),
            ("peck", "3"),
            ("dwell", "0.5"),
            ("through", "Yes"),
            ("breakthrough", "1"),
            ("hole_order", "Optimize travel"),
            ("hole_diameter", "0"),
            ("ramp", "Helix"),
            ("ramp_angle", "2"),
            ("lead_in", "2"),
            ("keep_down", "Yes"),
            ("angle_a", "0"),
            ("angle_b", "0"),
            ("rotary_axis", "A around X"),
            ("rotary_start", "0"),
            ("rotary_end", "360"),
            ("rotary_style", "Line"),
            ("axis_control", "Surface normal"),
            ("tilt_min", "0"),
            ("tilt_max", "60"),
            ("lead_angle", "0"),
            ("lean_angle", "0"),
            ("collision", "Retract and relink"),
            ("pattern", "Parallel"),
        ]);
        if s == Strategy::Drill {
            p.insert("rpm".into(), "2400".into());
            p.insert("feed".into(), "280".into());
        }
        if matches!(
            s,
            Strategy::Contour | Strategy::Parallel | Strategy::Waterline
        ) {
            p.insert("boundary".into(), "Model silhouette".into());
            p.insert("side".into(), "Outside".into());
            p.insert("containment".into(), "Tool outside boundary".into());
        }
        if s == Strategy::Drill {
            p.insert("through".into(), "No".into());
        }
        if s == Strategy::Parallel {
            p.insert("surface_stepover".into(), "1".into());
        }
        let geometry = match s {
            Strategy::Drill => vec![5, 6, 7, 8],
            Strategy::Face | Strategy::Indexed => vec![0],
            Strategy::Contour => vec![2],
            Strategy::Adaptive | Strategy::Pocket => vec![1],
            _ => vec![0, 1, 2, 4],
        };
        Self {
            id: id(),
            strategy: s,
            tool,
            enabled: true,
            geometry,
            surfaces: None,
            p,
            preview: false,
        }
    }
    pub fn name(&self) -> &str {
        get(&self.p, "name")
    }
    pub fn needs_surfaces(&self) -> bool {
        self.strategy.needs_surfaces()
            || (self.strategy == Strategy::Contour
                && get(&self.p, "boundary") == "Selected geometry")
    }
    pub fn toggle_feature(&mut self, n: usize) {
        if !self.strategy.features().contains(&n) {
            return;
        }
        if self.geometry.contains(&n) {
            self.geometry.retain(|x| *x != n)
        } else {
            self.geometry.push(n)
        }
        self.preview = false;
    }
    pub fn issues(&self, m: &Machine, tools: &[Tool], preceding: &[Operation]) -> Vec<String> {
        let mut e = vec![];
        if self.strategy == Strategy::Face {
            e.extend(facing_setting_issues(&self.p));
        }
        if self.strategy == Strategy::Pocket {
            e.extend(pocket_setting_issues(&self.p));
            nonnegative(&self.p, "axial_allowance", &mut e);
        }
        if self.name().trim().is_empty() {
            e.push("Give this operation a name.".into())
        }
        if self.needs_surfaces()
            && self.geometry.is_empty()
            && self.surfaces.as_ref().is_none_or(|s| s.ids.is_empty())
        {
            e.push("Select at least one machining feature or surface.".into())
        }
        if self
            .geometry
            .iter()
            .any(|f| !self.strategy.features().contains(f))
        {
            e.push("Geometry is incompatible with this strategy.".into())
        }
        if let Some(t) = tools.iter().find(|t| t.id == self.tool) {
            e.extend(t.issues());
            if self.strategy == Strategy::Drill
                && !["Drill", "Chamfer mill"].contains(&get(&t.p, "type"))
            {
                e.push(
                    "Choose a drill, or a chamfer mill to countersink, for a drilling cycle."
                        .into(),
                )
            }
        } else {
            e.push("Choose a tool from the library.".into())
        }
        for k in ["rpm", "feed"] {
            positive(&self.p, k, &mut e);
            let cap = num(&m.p, k).map(|x| {
                if k == "feed" && get(&m.p, "units") == "inch" {
                    x * 25.4
                } else {
                    x
                }
            });
            if num(&self.p, k).zip(cap).is_some_and(|(v, c)| v > c) {
                e.push(format!("{} exceeds the saved machine limit.", label(k)));
            }
        }
        for k in ["clearance", "retract"] {
            nonnegative(&self.p, k, &mut e)
        }
        for k in ["top", "bottom"] {
            finite(&self.p, k, &mut e)
        }
        if num(&self.p, "clearance") < num(&self.p, "retract") {
            e.push("Clearance must be above retract height.".into())
        }
        let s = self.strategy;
        if matches!(s, Strategy::Indexed | Strategy::Rotary) && m.axes() < 4 {
            e.push("This strategy needs at least 4 machine axes.".into())
        }
        if s == Strategy::FiveAxis && (m.axes() != 5 || get(&m.p, "mode") != "Simultaneous 5-axis")
        {
            e.push("Choose a machine configured for simultaneous 5-axis motion.".into())
        }
        if s == Strategy::Drill {
            if ["Chip breaking", "Deep drilling"].contains(&get(&self.p, "cycle")) {
                positive(&self.p, "peck", &mut e)
            }
            if get(&self.p, "cycle") == "Counterboring" {
                nonnegative(&self.p, "dwell", &mut e)
            }
            if get(&self.p, "through") == "Yes" {
                nonnegative(&self.p, "breakthrough", &mut e)
            }
        } else {
            if s == Strategy::Parallel || !s.imported_preview() {
                positive(&self.p, "tolerance", &mut e);
            }
            nonnegative(&self.p, "allowance", &mut e);
            if !s.imported_preview() {
                nonnegative(&self.p, "lead_in", &mut e);
            }
            if matches!(
                s,
                Strategy::Face
                    | Strategy::Indexed
                    | Strategy::Adaptive
                    | Strategy::Pocket
                    | Strategy::Contour
                    | Strategy::Waterline
            ) {
                positive(&self.p, "stepdown", &mut e)
            }
            if matches!(s, Strategy::Face | Strategy::Indexed) {
                bounded(&self.p, "stepover", 0.01, 100., &mut e)
            }
            // The pocket generator refuses a stepover over half the tool diameter
            // (offset contours must overlap), so the editor says so in its words.
            if s == Strategy::Pocket
                && !num(&self.p, "stepover").is_some_and(|n| (0.01..=50.).contains(&n))
            {
                e.push(
                    "Pocket stepover must be between 0.01 and 50% for overlapping offset contours."
                        .into(),
                )
            }
            if matches!(
                s,
                Strategy::Parallel | Strategy::Scallop | Strategy::Rotary | Strategy::FiveAxis
            ) {
                positive(&self.p, "surface_stepover", &mut e);
                bounded(&self.p, "slope_min", 0., 90., &mut e);
                bounded(&self.p, "slope_max", 0., 90., &mut e);
                range_equal(&self.p, "slope_min", "slope_max", &mut e)
            }
            if s == Strategy::Adaptive {
                positive(&self.p, "optimal_load", &mut e);
                nonnegative(&self.p, "minimum_radius", &mut e);
                nonnegative(&self.p, "axial_allowance", &mut e)
            }
            if matches!(s, Strategy::Pocket | Strategy::Contour) {
                bounded(&self.p, "finish_passes", 0., 20., &mut e);
                if num(&self.p, "finish_passes").is_some_and(|x| x.fract() != 0.) {
                    e.push("Finishing passes must be a whole number.".into())
                }
            }
            if matches!(s, Strategy::Adaptive | Strategy::Pocket)
                && get(&self.p, "ramp") != "Plunge"
            {
                bounded(&self.p, "ramp_angle", 0.01, 90., &mut e)
            }
        }
        if get(&self.p, "rest") == "Yes"
            && matches!(s, Strategy::Adaptive | Strategy::Scallop)
            && !preceding
                .iter()
                .any(|o| o.id == get(&self.p, "rest_source") && o.enabled)
        {
            e.push("Rest machining needs an enabled preceding operation.".into())
        }
        if s == Strategy::Indexed {
            bounded(
                &self.p,
                "angle_a",
                num(&m.p, "rotary_min").unwrap_or(0.),
                num(&m.p, "rotary_max").unwrap_or(0.),
                &mut e,
            );
            if m.axes() == 5 {
                bounded(
                    &self.p,
                    "angle_b",
                    num(&m.p, "rotary2_min").unwrap_or(0.),
                    num(&m.p, "rotary2_max").unwrap_or(0.),
                    &mut e,
                )
            }
        }
        if s == Strategy::Rotary {
            range(&self.p, "rotary_start", "rotary_end", &mut e);
            if get(&self.p, "rotary_style") != "Line"
                && !["Continuous 4-axis", "Simultaneous 5-axis"].contains(&get(&m.p, "mode"))
            {
                e.push("Spiral/circular rotary motion needs a continuous machine mode.".into())
            }
        }
        if s == Strategy::FiveAxis {
            bounded(&self.p, "tilt_min", 0., 90., &mut e);
            bounded(&self.p, "tilt_max", 0., 90., &mut e);
            range_equal(&self.p, "tilt_min", "tilt_max", &mut e);
            finite(&self.p, "lead_angle", &mut e);
            finite(&self.p, "lean_angle", &mut e)
        }
        e
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setup {
    #[serde(skip)]
    pub preview_stock: Option<crate::mesh::Bounds>,
    #[serde(skip)]
    pub preview_remaining: Option<std::sync::Arc<crate::rest::RestStock>>,
    pub id: String,
    pub machine: Machine,
    pub p: Fields,
    pub operations: Vec<Operation>,
}
impl Setup {
    pub fn new(machine: Machine, offset: &str) -> Self {
        Self {
            preview_stock: None,
            preview_remaining: None,
            id: id(),
            machine,
            p: fields(&[
                ("name", "Top side"),
                ("offset", offset),
                ("stock_type", "Relative size box"),
                ("stock_left", "0"),
                ("stock_right", "0"),
                ("stock_front", "0"),
                ("stock_back", "0"),
                ("stock_bottom", "0"),
                ("stock_top", "0"),
                ("stock_x", "150"),
                ("stock_y", "104"),
                ("stock_z", "40"),
                ("stock_source", "fresh"),
                ("origin", "Stock top center"),
                ("clearance", "15"),
                ("rx", "0"),
                ("ry", "0"),
                ("rz", "0"),
                ("tx", "0"),
                ("ty", "0"),
                ("tz", "0"),
                ("reposition", "Initial placement"),
                ("notes", ""),
            ]),
            operations: vec![],
        }
    }
    pub fn name(&self) -> &str {
        get(&self.p, "name")
    }
    pub fn duplicate(&self, offset: &str) -> Self {
        let mut s = self.clone();
        s.id = id();
        s.p.insert("name".into(), format!("{} · copy", self.name()));
        s.p.insert("offset".into(), offset.into());
        s.p.insert("stock_source".into(), self.id.clone());
        let mapping: BTreeMap<_, _> = s.operations.iter().map(|o| (o.id.clone(), id())).collect();
        for o in &mut s.operations {
            o.id = mapping[&o.id].clone();
            o.preview = false;
            if let Some(new) = mapping.get(get(&o.p, "rest_source")) {
                o.p.insert("rest_source".into(), new.clone());
            }
        }
        s
    }
    pub fn is_repositioned(&self) -> bool {
        ["rx", "ry", "rz", "tx", "ty", "tz"]
            .iter()
            .any(|k| num(&self.p, k).unwrap_or(0.).abs() > 0.001)
    }
    pub fn issues(&self) -> Vec<String> {
        let mut e = self.machine.issues();
        if self.name().trim().is_empty() {
            e.push("Give the setup a name.".into())
        }
        if get(&self.p, "offset").is_empty() {
            e.push("Choose a work offset.".into())
        }
        e.extend(crate::stock::parameter_issues(self));
        for k in ["rx", "ry", "rz", "tx", "ty", "tz"] {
            finite(&self.p, k, &mut e)
        }
        nonnegative(&self.p, "clearance", &mut e);
        e
    }
}
/// A path as JSON: text when it is valid Unicode (which is all an older
/// project file holds), otherwise its raw bytes (Unix) or UTF-16 units
/// (Windows), which serde's own `PathBuf` refuses to write. Both array forms
/// are read on every platform, so a project written on the other OS family
/// still loads: the foreign form decodes lossily to a path that names no
/// file, and the model then reports itself unreadable instead of the whole
/// project being refused.
mod path_json {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::path::{Path, PathBuf};
    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Text(String),
        Bytes(Vec<u8>),
        Wide(Vec<u16>),
    }
    pub fn serialize<S: Serializer>(path: &Path, s: S) -> Result<S::Ok, S::Error> {
        if let Some(text) = path.to_str() {
            return Repr::Text(text.to_owned()).serialize(s);
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Repr::Bytes(path.as_os_str().as_bytes().to_vec()).serialize(s)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Repr::Wide(path.as_os_str().encode_wide().collect()).serialize(s)
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
        Ok(match Repr::deserialize(d)? {
            Repr::Text(text) => PathBuf::from(text),
            #[cfg(unix)]
            Repr::Bytes(bytes) => {
                use std::os::unix::ffi::OsStringExt;
                PathBuf::from(std::ffi::OsString::from_vec(bytes))
            }
            #[cfg(not(unix))]
            Repr::Bytes(bytes) => PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()),
            #[cfg(windows)]
            Repr::Wide(units) => {
                use std::os::windows::ffi::OsStringExt;
                PathBuf::from(std::ffi::OsString::from_wide(&units))
            }
            #[cfg(not(windows))]
            Repr::Wide(units) => PathBuf::from(String::from_utf16_lossy(&units)),
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFile {
    pub name: String,
    #[serde(with = "path_json")]
    pub path: PathBuf,
    pub format: String,
    pub bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub schema: u32,
    pub title: String,
    pub material: String,
    /// The material library, carried to new projects like tools.
    #[serde(default)]
    pub materials: Vec<crate::material::Material>,
    /// The selected library material's id; `material` keeps its name.
    #[serde(default)]
    pub material_id: String,
    /// The starter set (`material::STARTER_SET`) the library has been
    /// given; 0 for a library saved before sets were counted.
    #[serde(default)]
    pub starter_materials: u32,
    /// The workholding library: vises and clamps, carried to new projects
    /// like tools.
    #[serde(default)]
    pub workholding: Vec<crate::holding::Holder>,
    pub file: Option<ModelFile>,
    #[serde(default)]
    pub sample_model: bool,
    pub machines: Vec<Machine>,
    pub tools: Vec<Tool>,
    pub setups: Vec<Setup>,
    pub active: String,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            schema: 1,
            title: "Untitled project".into(),
            material: String::new(),
            materials: vec![],
            material_id: String::new(),
            starter_materials: 0,
            workholding: vec![],
            file: None,
            sample_model: false,
            machines: vec![],
            tools: vec![],
            setups: vec![],
            active: String::new(),
        }
    }
}
impl Document {
    pub fn sample() -> Self {
        let machines = vec![Machine::new(3), Machine::new(4), Machine::new(5)];
        let mut drill = Tool::new("Twist drill · Ø8", "Drill", "8");
        // A twist drill has two flutes, not an end mill's three.
        drill.p.insert("flutes".into(), "2".into());
        let tools = vec![
            Tool::new("Flat end mill · Ø10", "Flat end mill", "10"),
            Tool::new("Ball end mill · Ø6", "Ball end mill", "6"),
            drill,
        ];
        let mut s = Setup::new(machines[0].clone(), "G54");
        for (key, value) in [
            ("stock_left", "5"),
            ("stock_right", "5"),
            ("stock_front", "5"),
            ("stock_back", "5"),
            ("stock_bottom", "4"),
            ("stock_top", "8"),
        ] {
            s.p.insert(key.into(), value.into());
        }
        s.operations = vec![
            Operation::new(Strategy::Face, tools[0].id.clone()),
            Operation::new(Strategy::Adaptive, tools[0].id.clone()),
            Operation::new(Strategy::Contour, tools[0].id.clone()),
            Operation::new(Strategy::Drill, tools[2].id.clone()),
        ];
        let mut doc = Self {
            schema: 1,
            sample_model: true,
            title: "Bearing housing".into(),
            material: "Aluminium 6061-T6".into(),
            file: None,
            active: s.id.clone(),
            machines,
            tools,
            setups: vec![s],
            ..Self::default()
        };
        doc.number_tools();
        doc
    }
}
impl Document {
    pub fn selected_material(&self) -> Option<&crate::material::Material> {
        self.materials
            .iter()
            .find(|m| !self.material_id.is_empty() && m.id == self.material_id)
    }
    /// `op`'s speed and feed against the selected material; None without a
    /// material.
    pub fn speed_feed(&self, setup: &Setup, op: &Operation) -> Option<crate::material::SpeedFeed> {
        let material = self.selected_material()?;
        let Some(tool) = self.tools.iter().find(|t| t.id == op.tool) else {
            return Some(crate::material::SpeedFeed::Unchecked(
                "The operation has no tool in the library.".into(),
            ));
        };
        Some(crate::material::check(material, tool, op, &setup.machine))
    }
    /// Why `material` cannot be saved to the library: another material has
    /// its name (ignoring case).
    pub fn material_name_issues(&self, material: &crate::material::Material) -> Vec<String> {
        let name = material.name().trim();
        let taken = !name.is_empty()
            && self
                .materials
                .iter()
                .any(|m| m.id != material.id && crate::material::same_name(m.name(), name));
        taken
            .then(|| format!("A material named {name} is already in the library."))
            .into_iter()
            .collect()
    }
    pub fn select_material(&mut self, id: &str) -> bool {
        let Some(name) = self
            .materials
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.name().to_string())
        else {
            return false;
        };
        self.material_id = id.into();
        self.material = name;
        true
    }
    /// Gives every operation the speed and feed `material::recommend` gives
    /// for the selected material, its tool and its setup's machine, whatever
    /// it had: the same material always gives the same speeds. Drilling
    /// operations keep theirs (it would give them an end mill's chip load),
    /// and so do those it cannot give one: without a tool, whose tool has no
    /// diameter or flutes, or whose machine has no maximum spindle speed or
    /// feed. Returns what changed, old to new, and why any was not set, for
    /// the status; nothing without a material or an operation.
    pub fn fit_speeds(&mut self) -> String {
        use crate::material::SpeedFeed;
        let Some(material) = self.selected_material().cloned() else {
            return String::new();
        };
        let name = crate::proposal::clip(material.name(), crate::material::NAME_LIMIT);
        if material.cutting_data().is_none() {
            return format!("{name} has no cutting data: speeds and feeds are unchanged.");
        }
        let mut changes = vec![];
        let (mut same, mut drills) = (0, 0);
        // Why operations could not be set, each with how many.
        let mut skipped: Vec<(String, usize)> = vec![];
        for si in 0..self.setups.len() {
            for oi in 0..self.setups[si].operations.len() {
                let (setup, op) = (&self.setups[si], &self.setups[si].operations[oi]);
                // The chip loads are an end mill's per tooth; a drill's feed
                // is per revolution, which the library has no data for.
                if op.strategy == Strategy::Drill {
                    if !matches!(self.speed_feed(setup, op), Some(SpeedFeed::InRange { .. })) {
                        drills += 1;
                    }
                    continue;
                }
                let recommended = match self.tools.iter().find(|t| t.id == op.tool) {
                    Some(tool) => crate::material::recommend(&material, tool, &setup.machine),
                    None => Err("The operation has no tool.".into()),
                };
                let (rpm, feed) = match recommended {
                    Ok(speeds) => speeds,
                    Err(why) => {
                        match skipped.iter_mut().find(|(w, _)| *w == why) {
                            Some((_, n)) => *n += 1,
                            None => skipped.push((why, 1)),
                        }
                        continue;
                    }
                };
                let label = format!("{} / {}", setup.name(), op.name());
                let op = &mut self.setups[si].operations[oi];
                let mut said = vec![];
                for (key, value, unit) in [("rpm", rpm, "rpm"), ("feed", feed, "mm/min")] {
                    let old = get(&op.p, key).trim().to_string();
                    let new = value.to_string();
                    if old != new {
                        let old = if old.is_empty() { "none" } else { &old };
                        said.push(format!("{old} → {new} {unit}"));
                        op.p.insert(key.into(), new);
                    }
                }
                if said.is_empty() {
                    same += 1;
                } else {
                    changes.push(format!("{label} {}", said.join(", ")));
                }
            }
        }
        let unset: usize = skipped.iter().map(|(_, n)| n).sum();
        if changes.is_empty() && same + unset + drills == 0 {
            return String::new();
        }
        let mut said = match (changes.is_empty(), unset + drills) {
            (true, 0) => format!("Every operation already runs at {name}'s speed and feed."),
            (true, _) => format!("No speed or feed changed for {name}."),
            (false, _) => format!("Speeds and feeds set for {name}: {}.", changes.join("; ")),
        };
        for (why, n) in skipped {
            let who = match n {
                1 => "1 operation".to_string(),
                n => format!("{n} operations"),
            };
            let mut why = why.chars();
            let first = why
                .next()
                .map(|c| c.to_lowercase().to_string())
                .unwrap_or_default();
            said.push_str(&format!(" {who} not set: {first}{}", why.as_str()));
        }
        if drills > 0 {
            let who = match drills {
                1 => "A drilling operation keeps its".to_string(),
                n => format!("{n} drilling operations keep their"),
            };
            said.push_str(&format!(
                " {who} speed and feed outside the data: set a drill's by hand, the library has no drill data."
            ));
        }
        said
    }
    /// Gives a document with an empty library the starter materials, brings an
    /// older library up to the current starter set once, and links a legacy
    /// material name to the library. Returns the note to show, if any.
    pub fn adopt_materials(&mut self) -> Option<String> {
        let shown = |name: &str| crate::proposal::clip(name, crate::material::NAME_LIMIT);
        if self.materials.is_empty() {
            self.materials = crate::material::starters();
        } else {
            let missing = crate::material::added_since(self.starter_materials);
            for m in crate::material::starters()
                .into_iter()
                .filter(|m| missing.contains(&m.name()))
            {
                if !self
                    .materials
                    .iter()
                    .any(|x| crate::material::same_name(x.name(), m.name()))
                {
                    self.materials.push(m);
                }
            }
        }
        self.starter_materials = crate::material::STARTER_SET;
        if !self.material_id.is_empty() && self.selected_material().is_none() {
            self.material_id.clear();
        }
        let name = self.material.trim().to_string();
        if !self.material_id.is_empty() || name.is_empty() {
            return None;
        }
        if let Some(m) = self
            .materials
            .iter()
            .find(|m| crate::material::same_name(m.name(), &name))
        {
            let id = m.id.clone();
            self.select_material(&id);
            return Some(format!(
                "The project material \"{}\" was linked to the library.",
                shown(&self.material)
            ));
        }
        let added = crate::material::Material::new(&name);
        let id = added.id.clone();
        self.materials.push(added);
        self.select_material(&id);
        Some(format!(
            "The project material \"{}\" was added to the material library without cutting data; add it on the Materials page.",
            shown(&name)
        ))
    }
    pub fn holder(&self, id: &str) -> Option<&crate::holding::Holder> {
        self.workholding.iter().find(|h| h.id == id)
    }
    /// Why `holder` cannot be saved to the library: another entry has its
    /// name (ignoring case).
    pub fn holder_name_issues(&self, holder: &crate::holding::Holder) -> Vec<String> {
        let name = holder.name().trim();
        let taken = !name.is_empty()
            && self
                .workholding
                .iter()
                .any(|h| h.id != holder.id && crate::material::same_name(h.name(), name));
        taken
            .then(|| format!("A workholding entry named {name} is already in the library."))
            .into_iter()
            .collect()
    }
    /// Gives a document with an empty library the starter vise and clamp.
    pub fn adopt_holders(&mut self) {
        if self.workholding.is_empty() {
            self.workholding = crate::holding::starters();
        }
    }
    /// Brings back the library entries the modelled `setups` use that this
    /// library has lost and `from` still has, each with its own id; a name
    /// another entry has taken since is made new. Says which came back, a
    /// sentence each, to end a status.
    pub fn restore_holders(&mut self, from: &Document, setups: &[Setup]) -> String {
        let mut said = String::new();
        for id in setups
            .iter()
            .filter(|s| crate::holding::modelled(s))
            .map(|s| get(&s.p, "holder"))
        {
            if let (None, Some(entry)) = (self.holder(id), from.holder(id)) {
                let mut entry = entry.clone();
                if !self.holder_name_issues(&entry).is_empty() {
                    let name =
                        crate::proposal::clip(entry.name(), crate::material::NAME_LIMIT - 11);
                    entry.p.insert("name".into(), format!("{name} (restored)"));
                }
                said += &format!(" {} is back in the workholding library.", entry.name());
                self.workholding.push(entry);
            }
        }
        said
    }
    /// The starting stock box of setup `setup` in that setup's frame, in
    /// millimetres: the first setup's own stock, and for a later one the first
    /// setup's stock carried into its frame (its own stock keys are ignored),
    /// which the simulation then trims by the layers earlier setups removed
    /// (`rest::RestStock::left`); this cannot simulate. None when it cannot
    /// be told: the first setup's stock cannot be resolved, a later setup
    /// needs the part `mesh` (to place its frame) or has no valid rotation
    /// and translation.
    pub fn setup_stock(
        &self,
        setup: usize,
        mesh: Option<&crate::mesh::Mesh>,
    ) -> Option<crate::mesh::Bounds> {
        use crate::volume::Frame;
        let first = crate::stock::resolve(self.setups.first()?, mesh)
            .ok()?
            .stock;
        if setup == 0 {
            return Some(first);
        }
        let (later, mesh) = (self.setups.get(setup)?, mesh?);
        let placed = ["rx", "ry", "rz", "tx", "ty", "tz"]
            .iter()
            .all(|k| num(&later.p, k).is_some());
        placed.then(|| {
            Frame::new(&self.setups[0], mesh).transfer_bounds(first, Frame::new(later, mesh))
        })
    }
    /// What is wrong with setup `setup`'s modelled workholding: its library
    /// entry (none chosen, gone), then its layout, then the entry not fitting
    /// the layout, then (when the stock can be told: `setup_stock`) a vise
    /// opening less than the stock is wide across the jaws, jaws or clamps
    /// placed off the stock and a vise gripping all but the last millimetre
    /// of the stock's height, and last a later setup not square to setup 1.
    /// The jaws close on the whole box here; what earlier setups cut away is
    /// checked by the export (`holding::holdable`), which simulates them.
    pub fn holding_issues(&self, setup: usize, mesh: Option<&crate::mesh::Mesh>) -> Vec<String> {
        let Some(s) = self.setups.get(setup) else {
            return vec![];
        };
        self.holding_problems(s, || {
            self.setup_stock(setup, mesh)
                .map(crate::holding::Gripped::whole)
        })
    }
    /// `holding_issues` with the stock `setup` starts from told by the caller
    /// (mm, in that setup's frame; None when it is not known) instead of
    /// `setup_stock`, with the band its vise closes on: the editor passes the
    /// box the earlier setups left and that band.
    pub fn holding_issues_with(
        &self,
        setup: usize,
        held: Option<crate::holding::Gripped>,
    ) -> Vec<String> {
        let Some(s) = self.setups.get(setup) else {
            return vec![];
        };
        self.holding_problems(s, || held)
    }
    /// `holding_issues` of `s`, the stock being told by `held` when needed.
    fn holding_problems(
        &self,
        s: &Setup,
        held: impl FnOnce() -> Option<crate::holding::Gripped>,
    ) -> Vec<String> {
        use crate::holding::{
            DELETED, NOT_SQUARE, height_issue, layout, opening_issue, position_issue, refusal,
        };
        let kind = match get(&s.p, "holding") {
            "Vise" => "Vise",
            "Clamps" => "Clamp",
            _ => "",
        };
        let id = get(&s.p, "holder");
        let entry = self.holder(id);
        let mut issues = vec![];
        if !kind.is_empty() && id.is_empty() {
            let none = !self.workholding.iter().any(|h| h.kind() == kind);
            issues.push(format!(
                "Choose a {} from the library.{}",
                kind.to_lowercase(),
                if none {
                    " Add one on the Workholding page."
                } else {
                    ""
                }
            ));
        } else if !kind.is_empty() && entry.is_none() {
            issues.push(DELETED.into());
        }
        // The jaws and clamps are boxes along the axes of setup 1's frame, which
        // a later setup turned from it by another angle does not share.
        let square = self
            .setups
            .first()
            .is_none_or(|first| crate::volume::Frame::setups_square(first, s));
        match (layout(s), entry) {
            (Err(error), _) => issues.push(error),
            (Ok(layout), Some(entry)) if !kind.is_empty() => {
                if let Some(refused) = refusal(&layout, entry) {
                    issues.push(refused);
                } else if square && let Some(held) = held() {
                    issues.extend(opening_issue(&layout, entry, held));
                    issues.extend(position_issue(&layout, entry, held));
                    issues.extend(height_issue(&layout, held.stock));
                }
            }
            _ => {}
        }
        if !kind.is_empty() && !square {
            issues.push(NOT_SQUARE.into());
        }
        issues
    }
    /// The jaws or clamps of `setup` round `held`, the stock it starts from
    /// and the band its vise closes on (in the setup's frame, mm), without
    /// the clearance gap; none without workholding. Refuses with the first
    /// line `holding_issues` shows for it, so that the editor and the export
    /// agree: an entry that is gone or not chosen, a layout that does not
    /// read, an entry that does not fit, a band wider than the vise opens,
    /// jaws or clamps off the stock, a stock not standing 1 mm above the
    /// jaws, a setup not square to setup 1.
    pub fn holding_boxes(
        &self,
        setup: &Setup,
        held: impl Into<crate::holding::Gripped>,
    ) -> Result<Vec<crate::holding::HoldBox>, String> {
        let held = held.into();
        if let Some(issue) = self
            .holding_problems(setup, || Some(held))
            .into_iter()
            .next()
        {
            return Err(issue);
        }
        let layout = crate::holding::layout(setup)?;
        match self.holder(get(&setup.p, "holder")) {
            Some(entry) => crate::holding::boxes(&layout, entry, held),
            None => Ok(vec![]),
        }
    }
    /// `setup`'s modelled workholding in a sentence, for the setup sheet and
    /// the program header (`holding::sentence`). None without workholding and
    /// when its layout and library entry cannot be read, which
    /// `holding_issues` says.
    pub fn holding_text(&self, setup: &Setup) -> Option<String> {
        use crate::holding::{Layout, refusal, sentence};
        let layout = crate::holding::layout(setup).ok()?;
        let entry = self.holder(get(&setup.p, "holder"))?;
        (layout != Layout::None && refusal(&layout, entry).is_none())
            .then(|| sentence(&layout, entry))
    }
    /// What holds a setup's stock, in a few words for its header: the
    /// modelled vise or clamps ("4 × Toe clamp 60 mm"), else its fixture note
    /// said to be not modelled, else that nothing is modelled.
    pub fn holding_label(&self, setup: &Setup) -> String {
        use crate::holding::{Layout, fixture_note, layout};
        let name = |kind: &str| {
            self.holder(get(&setup.p, "holder"))
                .map_or(kind.to_string(), |h| h.name().to_string())
        };
        match layout(setup) {
            Ok(Layout::None) => fixture_note(setup).map_or("No workholding modelled".into(), |n| {
                format!("{n}, not modelled")
            }),
            Ok(Layout::Clamps { clamps, .. }) => format!("{} × {}", clamps.len(), name("clamp")),
            _ => name(get(&setup.p, "holding")),
        }
    }
    /// After a setup's `holding` or `holder` changed: a vise or clamps setup
    /// points at a library entry of its kind (its own when it has one, else
    /// the first), a vise setup without a grip starts at the vise's grip, and
    /// the clearance gap at 2 mm.
    pub fn settle_holding(&mut self, setup: usize) {
        let Some(s) = self.setups.get(setup) else {
            return;
        };
        let kind = match get(&s.p, "holding") {
            "Vise" => "Vise",
            "Clamps" => "Clamp",
            _ => return,
        };
        let mine = self
            .holder(get(&s.p, "holder"))
            .filter(|h| h.kind() == kind);
        let chosen = mine.or_else(|| self.workholding.iter().find(|h| h.kind() == kind));
        let holder = chosen.map(|h| h.id.clone()).unwrap_or_default();
        let grip = chosen.map(|h| get(&h.p, "grip").to_string());
        let p = &mut self.setups[setup].p;
        p.insert("holder".into(), holder);
        if let Some(grip) = grip.filter(|g| kind == "Vise" && !g.is_empty())
            && get(p, "vise_grip").trim().is_empty()
        {
            p.insert("vise_grip".into(), grip);
        }
        if get(p, "holding_gap").trim().is_empty() {
            p.insert("holding_gap".into(), "2".into());
        }
    }
    pub fn has_model(&self) -> bool {
        self.file.is_some() || self.sample_model
    }
    pub fn active_setup(&self) -> Option<&Setup> {
        self.setups.iter().find(|s| s.id == self.active)
    }
    pub fn active_setup_mut(&mut self) -> Option<&mut Setup> {
        self.setups.iter_mut().find(|s| s.id == self.active)
    }
    pub fn index(&self) -> usize {
        self.setups
            .iter()
            .position(|s| s.id == self.active)
            .unwrap_or(0)
    }
    pub fn setup(&self) -> &Setup {
        &self.setups[self.index()]
    }
    pub fn setup_mut(&mut self) -> &mut Setup {
        let i = self.index();
        &mut self.setups[i]
    }
    pub fn next_offset(&self) -> String {
        (54..=59)
            .map(|i| format!("G{i}"))
            .find(|v| !self.setups.iter().any(|s| get(&s.p, "offset") == v))
            .unwrap_or_default()
    }
    pub fn next_tool_number(&self) -> u32 {
        (1..=TOOL_NUMBER_MAX)
            .find(|n| !self.tools.iter().any(|t| t.number() == Some(*n)))
            .unwrap_or(TOOL_NUMBER_MAX)
    }
    /// Number tools saved before tool numbers existed. Entered values are kept.
    pub fn number_tools(&mut self) -> bool {
        let mut changed = false;
        for i in 0..self.tools.len() {
            if get(&self.tools[i].p, "number").trim().is_empty() {
                let n = self.next_tool_number();
                self.tools[i].p.insert("number".into(), n.to_string());
                changed = true;
            }
        }
        changed
    }
    pub fn tool_number_issues(&self, tool: &Tool) -> Vec<String> {
        let Some(n) = tool.number() else {
            return vec![format!(
                "{}: enter a tool number from 1 to {TOOL_NUMBER_MAX}.",
                tool.name()
            )];
        };
        self.tools
            .iter()
            .filter(|t| t.id != tool.id && t.number() == Some(n))
            .map(|t| {
                format!(
                    "{}: tool number T{n} is also used by {}.",
                    tool.name(),
                    t.name()
                )
            })
            .collect()
    }
    pub fn validate_structure(&self) -> Result<(), String> {
        if self.schema != 1 {
            return Err("Unsupported project schema. Expected AXIS desktop schema 1.".into());
        }
        if self.setups.len() > 100 {
            return Err("Maximum 100 setups per project.".into());
        }
        for ids in [
            self.machines
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            self.tools.iter().map(|t| t.id.as_str()).collect(),
            self.materials.iter().map(|m| m.id.as_str()).collect(),
            self.workholding.iter().map(|h| h.id.as_str()).collect(),
        ] {
            let mut seen = HashSet::new();
            if ids.iter().any(|id| id.is_empty() || !seen.insert(id)) {
                return Err("Invalid or duplicate library identifier.".into());
            }
        }
        let mut ids = HashSet::new();
        for s in &self.setups {
            if !ids.insert(&s.id) {
                return Err("Duplicate setup identifier.".into());
            }
            let mut ops = HashSet::new();
            if s.operations.iter().any(|o| {
                !ops.insert(&o.id)
                    || o.geometry.iter().any(|g| *g >= FEATURES.len())
                    || o.surfaces.as_ref().is_some_and(|s| !s.valid_structure())
            }) {
                return Err("Invalid operation identifiers or geometry.".into());
            }
        }
        if (self.setups.is_empty() && !self.active.is_empty())
            || (!self.setups.is_empty() && !ids.contains(&self.active))
        {
            return Err("Active setup is missing.".into());
        }
        self.validate_order(&self.setups)?;
        Ok(())
    }
    pub fn sync_stock_sources(&mut self) {
        let mut source = "fresh".to_string();
        for setup in &mut self.setups {
            setup.p.insert("stock_source".into(), source);
            source = setup.id.clone();
        }
    }
    pub fn validate_order(&self, setups: &[Setup]) -> Result<(), String> {
        let mut seen = HashSet::new();
        for s in setups {
            let source = get(&s.p, "stock_source");
            if source != "fresh" && !seen.contains(source) {
                return Err("Stock must come from a preceding setup. Change the stock source before reordering.".into());
            }
            seen.insert(s.id.as_str());
        }
        Ok(())
    }
    pub fn move_setup(&mut self, from: usize, delta: isize) -> Result<(), String> {
        let to = from as isize + delta;
        if to < 0 || to as usize >= self.setups.len() {
            return Ok(());
        }
        let mut v = self.setups.clone();
        v.swap(from, to as usize);
        self.setups = v;
        self.sync_stock_sources();
        Ok(())
    }
    /// Removes setup `i`. The setup that took its stock from it is chained to
    /// the one before, and the returned line says so (its notes are the
    /// user's and stay as typed).
    pub fn remove_setup(&mut self, i: usize) -> Result<Option<String>, String> {
        if i >= self.setups.len() {
            return Err("Setup is missing.".into());
        }
        let old = self.setups.remove(i);
        let rechained = self
            .setups
            .iter()
            .find(|s| get(&s.p, "stock_source") == old.id)
            .map(|s| s.name().to_string());
        if self.active == old.id {
            self.active = self
                .setups
                .first()
                .map(|s| s.id.clone())
                .unwrap_or_default()
        }
        self.sync_stock_sources();
        Ok(rechained.map(|name| {
            let from = match i.checked_sub(1).and_then(|p| self.setups.get(p)) {
                Some(previous) => previous.name().to_string(),
                None => "fresh stock".to_string(),
            };
            format!(
                "Stock source was removed: {name} now takes its stock from {from}. Recheck stock and workholding."
            )
        }))
    }
    pub fn issues(&self) -> Vec<String> {
        let mut e = vec![];
        if !self.has_model() {
            e.push("Import a model before machining.".into());
        }
        if self.setups.is_empty() {
            e.push("Create a machining setup.".into());
        }
        let mut offsets = HashSet::new();
        for s in &self.setups {
            e.extend(s.issues().into_iter().map(|x| format!("{}: {x}", s.name())));
            if !offsets.insert(get(&s.p, "offset")) {
                e.push(format!(
                    "{}: work offset is also used by another setup.",
                    s.name()
                ))
            }
            for (i, o) in s.operations.iter().enumerate().filter(|(_, o)| o.enabled) {
                e.extend(
                    o.issues(&s.machine, &self.tools, &s.operations[..i])
                        .into_iter()
                        .map(|x| format!("{} / {}: {x}", s.name(), o.name())),
                );
            }
        }
        if let Err(x) = self.validate_order(&self.setups) {
            e.push(x)
        }
        e
    }
    pub fn setup_sheet(&self, mesh: Option<&crate::mesh::Mesh>) -> String {
        let mut s = format!(
            "# {}\n\nNative UI prototype — sample planning data, not machining instructions.\n\nMaterial: {}\n",
            self.title, self.material
        );
        for (i, setup) in self.setups.iter().enumerate() {
            // What holds the stock, then what the editor and the export say is wrong with it.
            let holding: String = [
                self.holding_text(setup)
                    .map(|text| format!("Workholding: {text}\n\n")),
                self.holding_issues(i, mesh)
                    .first()
                    .map(|issue| format!("Workholding problem: {issue}\n\n")),
            ]
            .into_iter()
            .flatten()
            .collect();
            // The operator's requested fixture clearance: a note, checked by nothing.
            let requested = match crate::holding::requested_clearance(setup) {
                Some(mm) => format!("{mm} mm"),
                None => "not set".into(),
            };
            s += &format!(
                "\n## {} · {}\n\nMachine: {} (v{})\n\n{}\n\nOrigin: {}\n\n{}{}Rotation: [{}, {}, {}]°\n\nTranslation: [{}, {}, {}] mm\n\nReposition: {}\n\nRequested fixture clearance (not checked): {}\n\n{}\n\n",
                setup.name(),
                get(&setup.p, "offset"),
                setup.machine.name(),
                setup.machine.version,
                crate::stock::sheet_description(setup, mesh),
                get(&setup.p, "origin"),
                crate::holding::fixture_note(setup)
                    .map(|note| format!("Fixture: {note}\n\n"))
                    .unwrap_or_default(),
                holding,
                get(&setup.p, "rx"),
                get(&setup.p, "ry"),
                get(&setup.p, "rz"),
                get(&setup.p, "tx"),
                get(&setup.p, "ty"),
                get(&setup.p, "tz"),
                get(&setup.p, "reposition"),
                requested,
                get(&setup.p, "notes")
            );
            for o in &setup.operations {
                s += &format!(
                    "- {}: {}{}\n",
                    o.name(),
                    o.strategy,
                    if o.enabled { "" } else { " (suppressed)" }
                );
            }
        }
        s
    }
}
fn label(k: &str) -> String {
    k.replace('_', " ")
}
/// The operation editor's text fields, by key: the ones a check's message
/// names by its `label`.
const OPERATION_FIELDS: [&str; 33] = [
    "name",
    "hole_diameter",
    "rpm",
    "feed",
    "clearance",
    "retract",
    "top",
    "bottom",
    "breakthrough",
    "angle_a",
    "angle_b",
    "rotary_start",
    "rotary_end",
    "tilt_min",
    "tilt_max",
    "lead_angle",
    "lean_angle",
    "peck",
    "dwell",
    "direction",
    "stepover",
    "surface_stepover",
    "slope_min",
    "slope_max",
    "stepdown",
    "tolerance",
    "allowance",
    "axial_allowance",
    "finish_passes",
    "optimal_load",
    "minimum_radius",
    "ramp_angle",
    "lead_in",
];
/// The operation editor's text field a check's message is about, by its key,
/// so the editor can show the message at the field. The messages of
/// `Operation::issues` start with the field's `label` (a Pocket's stepover
/// message with the strategy before it, and the finishing passes under the
/// name their field has); the speed and feed checks (`material::check`) name
/// the surface speed, which the spindle speed sets, and the chip load, which
/// the feed sets. None for a message about no text field.
pub fn issue_field(issue: &str) -> Option<&'static str> {
    let lower = issue.to_lowercase();
    let m = lower.strip_prefix("pocket ").unwrap_or(&lower);
    if m.starts_with("surface speed ") {
        return Some("rpm");
    }
    if m.starts_with("chip load ") {
        return Some("feed");
    }
    if m.starts_with("finishing passes ") {
        return Some("finish_passes");
    }
    if m == "give this operation a name." {
        return Some("name");
    }
    OPERATION_FIELDS.into_iter().find(|k| {
        m.strip_prefix(label(k).as_str())
            .is_some_and(|rest| rest.starts_with(' '))
    })
}
fn finite(p: &Fields, k: &str, e: &mut Vec<String>) {
    if num(p, k).is_none() {
        e.push(format!("{} must be a finite number.", label(k)))
    }
}
fn positive(p: &Fields, k: &str, e: &mut Vec<String>) {
    if !num(p, k).is_some_and(|n| n > 0.) {
        e.push(format!("{} must be greater than zero.", label(k)))
    }
}
fn nonnegative(p: &Fields, k: &str, e: &mut Vec<String>) {
    if !num(p, k).is_some_and(|n| n >= 0.) {
        e.push(format!("{} cannot be negative or blank.", label(k)))
    }
}
fn bounded(p: &Fields, k: &str, a: f32, b: f32, e: &mut Vec<String>) {
    if !num(p, k).is_some_and(|n| n >= a && n <= b) {
        e.push(format!("{} must be between {a} and {b}.", label(k)))
    }
}
fn range(p: &Fields, a: &str, b: &str, e: &mut Vec<String>) {
    if !num(p, a).zip(num(p, b)).is_some_and(|(a, b)| b > a) {
        e.push(format!("{} must be greater than {}.", label(b), label(a)))
    }
}
fn range_equal(p: &Fields, a: &str, b: &str, e: &mut Vec<String>) {
    if !num(p, a).zip(num(p, b)).is_some_and(|(a, b)| b >= a) {
        e.push(format!("{} must not be below {}.", label(b), label(a)))
    }
}

#[test]
fn the_model_path_is_read_from_a_string_and_written_as_one_when_it_is_text() {
    let file: ModelFile =
        serde_json::from_str(r#"{"name":"a.stl","path":"/parts/a.stl","format":"stl","bytes":3}"#)
            .unwrap();
    assert_eq!(file.path, PathBuf::from("/parts/a.stl"));
    let text = serde_json::to_value(&file).unwrap();
    assert_eq!(text["path"], "/parts/a.stl");
}
#[cfg(unix)]
#[test]
fn a_model_path_that_is_not_utf8_survives_a_save_and_load() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let path = PathBuf::from(OsStr::from_bytes(b"/parts/caf\xe9/\xff.stl"));
    assert!(path.to_str().is_none());
    let file = ModelFile {
        name: "part.stl".into(),
        path: path.clone(),
        format: "stl".into(),
        bytes: 3,
    };
    let json = serde_json::to_string(&file).unwrap();
    let back: ModelFile = serde_json::from_str(&json).unwrap();
    assert_eq!(back.path, path);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_valid() {
        let d = Document::sample();
        assert!(d.issues().is_empty(), "{:?}", d.issues());
        assert!(d.validate_structure().is_ok())
    }
    #[test]
    fn empty_geometry_is_invalid() {
        let mut d = Document::sample();
        d.setup_mut().operations[0].geometry.clear();
        assert!(d.issues().iter().any(|s| s.contains("Select at least")))
    }
    #[test]
    fn incompatible_pick_ignored() {
        let mut o = Operation::new(Strategy::Drill, "t".into());
        let before = o.geometry.clone();
        o.toggle_feature(0);
        assert_eq!(o.geometry, before);
        o.toggle_feature(5);
        assert!(!o.geometry.contains(&5))
    }
    #[test]
    fn duplicate_remaps_rest_dependency() {
        let mut d = Document::sample();
        let s = d.setup_mut();
        s.operations[1].p.insert("rest".into(), "Yes".into());
        let source = s.operations[0].id.clone();
        s.operations[1].p.insert("rest_source".into(), source);
        let copy = s.duplicate("G55");
        assert_ne!(copy.operations[0].id, s.operations[0].id);
        assert_eq!(
            get(&copy.operations[1].p, "rest_source"),
            copy.operations[0].id
        )
    }
    #[test]
    fn stock_order_enforced() {
        let mut d = Document::sample();
        d.setups.push(d.setup().duplicate("G55"));
        assert!(d.move_setup(1, -1).is_ok());
        assert_eq!(get(&d.setups[0].p, "stock_source"), "fresh");
        assert_eq!(get(&d.setups[1].p, "stock_source"), d.setups[0].id);
        d.remove_setup(0).unwrap();
        assert_eq!(get(&d.setup().p, "stock_source"), "fresh")
    }
    #[test]
    fn removing_a_setup_keeps_the_next_setups_notes() {
        let mut d = Document::sample();
        d.setups.push(d.setup().duplicate("G55"));
        d.sync_stock_sources();
        d.setups[1]
            .p
            .insert("notes".into(), "Clamp on the soft jaws.".into());
        d.remove_setup(0).unwrap();
        assert_eq!(get(&d.setups[0].p, "notes"), "Clamp on the soft jaws.");
        assert_eq!(get(&d.setups[0].p, "stock_source"), "fresh")
    }
    #[test]
    fn removing_a_setup_reports_the_stock_source_change() {
        let mut d = Document::sample();
        d.setups.push(d.setup().duplicate("G55"));
        d.setups.push(d.setup().duplicate("G56"));
        for (s, name) in d.setups.iter_mut().zip(["One", "Two", "Three"]) {
            s.p.insert("name".into(), name.into());
        }
        d.sync_stock_sources();
        let first = d.setups[0].id.clone();
        // The setup after the removed one is chained to the one before it.
        let note = d.remove_setup(1).unwrap().unwrap();
        assert!(note.contains("Three") && note.contains("One"), "{note}");
        assert_eq!(get(&d.setups[1].p, "stock_source"), first);
        // Removing the first sends the next one back to the stock blank.
        let note = d.remove_setup(0).unwrap().unwrap();
        assert!(
            note.contains("Three") && note.contains("fresh stock"),
            "{note}"
        );
        assert_eq!(get(&d.setups[0].p, "stock_source"), "fresh");
        // The last setup has no successor, so there is nothing to report.
        assert_eq!(d.remove_setup(0).unwrap(), None);
    }
    #[test]
    fn machine_limit_checked() {
        let mut d = Document::sample();
        d.setup_mut().operations[0]
            .p
            .insert("rpm".into(), "99999".into());
        assert!(d.issues().iter().any(|s| s.contains("exceeds")))
    }
    #[test]
    fn a_pocket_stepover_over_50_percent_is_an_issue_but_a_facing_one_is_not() {
        let d = Document::sample();
        let s = d.setup();
        let issues = |strategy, stepover: &str| {
            let mut o = Operation::new(strategy, d.tools[0].id.clone());
            o.p.insert("stepover".into(), stepover.into());
            o.issues(&s.machine, &d.tools, &[])
                .into_iter()
                .filter(|x| x.contains("tepover"))
                .collect::<Vec<_>>()
        };
        assert!(issues(Strategy::Pocket, "50").is_empty());
        let over = issues(Strategy::Pocket, "60");
        assert!(
            over.iter()
                .any(|x| x.contains("Pocket stepover") && x.contains("50%")),
            "{over:?}"
        );
        assert!(!issues(Strategy::Pocket, "0").is_empty());
        assert!(issues(Strategy::Face, "60").is_empty());
        assert!(issues(Strategy::Face, "100").is_empty());
        assert!(!issues(Strategy::Face, "101").is_empty());
    }
    #[test]
    fn each_check_message_names_the_field_it_is_about() {
        let d = Document::sample();
        let s = d.setup();
        let mut o = Operation::new(Strategy::Pocket, d.tools[0].id.clone());
        o.p.insert("stepover".into(), "60".into());
        o.p.insert("axial_allowance".into(), "-1".into());
        o.p.insert("rpm".into(), "0".into());
        o.p.insert("name".into(), " ".into());
        let issues = o.issues(&s.machine, &d.tools, &[]);
        let field = |part: &str| {
            let issue = issues
                .iter()
                .find(|x| x.contains(part))
                .unwrap_or_else(|| panic!("no issue with {part:?} in {issues:?}"));
            issue_field(issue)
        };
        assert_eq!(field("Pocket stepover"), Some("stepover"));
        assert_eq!(field("axial allowance"), Some("axial_allowance"));
        assert_eq!(field("rpm must"), Some("rpm"));
        assert_eq!(field("a name"), Some("name"));
        // The other forms the messages take, and the ones about no field.
        for (issue, key) in [
            (
                "surface stepover must be greater than zero.",
                Some("surface_stepover"),
            ),
            ("allowance cannot be negative or blank.", Some("allowance")),
            ("slope max must not be below slope min.", Some("slope_max")),
            ("Clearance must be above retract height.", Some("clearance")),
            ("feed exceeds the saved machine limit.", Some("feed")),
            (
                "Finishing passes must be a whole number.",
                Some("finish_passes"),
            ),
            (
                "Surface speed 377 m/min is above Aluminium's 150–300 m/min.",
                Some("rpm"),
            ),
            (
                "Chip load 0.008 mm per tooth is below Aluminium's 0.020–0.060 mm for a Ø6 mm tool.",
                Some("feed"),
            ),
            ("Choose a tool from the library.", None),
            ("Select at least one machining feature or surface.", None),
            ("Flutes must be a whole number.", None),
            ("Rest machining needs an enabled preceding operation.", None),
        ] {
            assert_eq!(issue_field(issue), key, "{issue}");
        }
    }
    #[test]
    fn drill_only_relevant_cycle_fields() {
        let mut d = Document::sample();
        let o = &mut d.setup_mut().operations[3];
        o.p.insert("peck".into(), "-1".into());
        assert!(d.issues().is_empty());
        d.setup_mut().operations[3]
            .p
            .insert("cycle".into(), "Deep drilling".into());
        assert!(!d.issues().is_empty())
    }
    #[test]
    fn five_axis_requires_continuous_profile() {
        let mut d = Document::sample();
        let o = Operation::new(Strategy::FiveAxis, d.tools[1].id.clone());
        d.setup_mut().operations.push(o);
        assert!(d.issues().iter().any(|s| s.contains("simultaneous")));
        let mut m = Machine::new(5);
        m.p.insert("mode".into(), "Simultaneous 5-axis".into());
        d.setup_mut().machine = m;
        assert!(d.issues().is_empty())
    }
    #[test]
    fn roundtrip_and_preview_not_persisted() {
        let mut d = Document::sample();
        d.setup_mut().operations[0].preview = true;
        let decoded: Document = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert!(!decoded.setup().operations[0].preview);
        assert!(decoded.validate_structure().is_ok())
    }
    #[test]
    fn invalid_numeric_input_not_hidden() {
        let mut d = Document::sample();
        d.setup_mut().operations[0]
            .p
            .insert("feed".into(), "NaN".into());
        assert!(!d.issues().is_empty())
    }
    #[test]
    fn tool_numbers_migrate_once_and_flag_duplicates() {
        let mut d = Document::sample();
        let numbers = |d: &Document| d.tools.iter().map(|t| t.number()).collect::<Vec<_>>();
        assert_eq!(numbers(&d), vec![Some(1), Some(2), Some(3)]);
        for t in &mut d.tools {
            t.p.remove("number");
        }
        d.tools[1].p.insert("number".into(), "1".into());
        assert!(d.number_tools());
        assert_eq!(numbers(&d), vec![Some(2), Some(1), Some(3)]);
        assert!(!d.number_tools());
        d.tools[2].p.insert("number".into(), "2".into());
        assert!(
            d.tool_number_issues(&d.tools[2])
                .iter()
                .any(|e| e.contains("T2"))
        );
        d.tools[2].p.insert("number".into(), "0".into());
        assert!(!d.tool_number_issues(&d.tools[2]).is_empty());
        assert!(!d.number_tools());
        assert_eq!(d.next_tool_number(), 3);
    }
    #[test]
    fn legacy_machines_use_post_defaults_and_bad_values_are_reported() {
        let mut m = Machine::new(3);
        assert!(post_setting_issues(&m.p).is_empty());
        for k in POST_KEYS {
            m.p.remove(k);
        }
        assert_eq!(
            post_setting(&m.p, "post_tool_length"),
            "Tool table (G43 H#)"
        );
        assert_eq!(
            post_setting(&m.p, "post_retract"),
            "Machine Z home (G53 G0 Z0)"
        );
        assert_eq!(post_setting(&m.p, "post_blend"), "0.01");
        assert!(post_setting_issues(&m.p).is_empty());
        m.fill_post_defaults();
        assert_eq!(get(&m.p, "post_dwell"), "2");
        m.p.insert("post_blend".into(), "-1".into());
        m.p.insert("post_dwell".into(), "90".into());
        m.p.insert("post_air".into(), "M50".into());
        assert_eq!(post_setting_issues(&m.p).len(), 3);
        m.p.insert("post_air".into(), POST_AIR[0].into());
        m.p.insert("post_dwell".into(), "2".into());
        m.p.insert("post_blend".into(), "0.0004".into());
        assert_eq!(
            post_setting_issues(&m.p),
            vec!["Path blending tolerance must be 0 or between 0.001 and 1 mm.".to_string()]
        );
        m.p.insert("post_blend".into(), "0.001".into());
        assert!(post_setting_issues(&m.p).is_empty());
        assert!(
            m.issues().is_empty(),
            "post settings never block simulation"
        );
    }
    #[test]
    fn adopting_materials_adds_starters_and_links_legacy_names() {
        let mut doc = Document {
            material: "aluminium 6061-t6".into(),
            ..Default::default()
        };
        let note = doc.adopt_materials().unwrap();
        assert_eq!(doc.materials.len(), 22);
        assert!(note.contains("linked to the library"), "{note}");
        assert_eq!(doc.selected_material().unwrap().name(), "Aluminium 6061-T6");
        assert_eq!(doc.material, "Aluminium 6061-T6");
        assert!(
            doc.adopt_materials().is_none(),
            "a second call changes nothing"
        );
        assert_eq!(doc.materials.len(), 22);

        let mut odd = Document {
            material: "Inconel 718".into(),
            ..Default::default()
        };
        let note = odd.adopt_materials().unwrap();
        assert!(note.contains("without cutting data"), "{note}");
        assert_eq!(odd.materials.len(), 23);
        assert!(odd.selected_material().unwrap().cutting_data().is_none());

        let mut none = Document::default();
        assert!(none.adopt_materials().is_none());
        assert_eq!(none.materials.len(), 22);
        assert!(none.material_id.is_empty());

        let mut stale = none.clone();
        stale.material_id = "gone".into();
        stale.adopt_materials();
        assert!(stale.material_id.is_empty());
    }
    #[test]
    fn selecting_a_material_sets_its_id_and_name() {
        let mut doc = Document::default();
        doc.adopt_materials();
        let id = doc.materials[2].id.clone();
        assert!(doc.select_material(&id));
        assert_eq!(doc.material, "Mild steel 1018");
        assert!(!doc.select_material("missing"));
    }
    #[test]
    fn a_project_with_duplicate_or_empty_material_ids_is_refused() {
        use crate::material::Material;
        let mut doc = Document::default();
        doc.adopt_materials();
        assert!(doc.validate_structure().is_ok());
        let mut copy = doc.materials[0].clone();
        copy.p.insert("name".into(), "Other name".into());
        doc.materials.push(copy);
        let error = doc.validate_structure().unwrap_err();
        assert!(error.contains("duplicate library identifier"), "{error}");
        doc.materials.pop();
        let mut blank = Material::new("No id");
        blank.id.clear();
        doc.materials.push(blank);
        assert!(doc.validate_structure().is_err());
        doc.materials.pop();
        assert!(doc.validate_structure().is_ok());
    }
    #[test]
    fn a_material_name_is_taken_regardless_of_case_and_spaces() {
        use crate::material::Material;
        let mut doc = Document::default();
        doc.adopt_materials();
        let mut same = Material::new(" aluminium 6061-t6 ");
        assert_eq!(
            doc.material_name_issues(&same),
            ["A material named aluminium 6061-t6 is already in the library."]
        );
        // The material itself keeps its name, and a blank name is not a clash.
        same.id = doc.materials[0].id.clone();
        assert!(doc.material_name_issues(&same).is_empty());
        assert!(doc.material_name_issues(&Material::new("  ")).is_empty());
        assert!(
            doc.material_name_issues(&Material::new("Titanium"))
                .is_empty()
        );
    }
    #[test]
    fn a_legacy_name_differing_only_by_a_non_ascii_capital_links_to_the_entry() {
        use crate::material::Material;
        let mut doc = Document {
            materials: vec![Material::new("Édelstahl 1.4301")],
            material: "  édelstahl 1.4301 ".into(),
            starter_materials: crate::material::STARTER_SET,
            ..Default::default()
        };
        let note = doc.adopt_materials().unwrap();
        assert!(note.contains("linked to the library"), "{note}");
        assert_eq!(doc.materials.len(), 1);
        assert_eq!(doc.material, "Édelstahl 1.4301");
        // The library's own name check compares the same way.
        assert_eq!(
            doc.material_name_issues(&Material::new(" ÉDELSTAHL 1.4301")),
            ["A material named ÉDELSTAHL 1.4301 is already in the library."]
        );
    }
    #[test]
    fn a_long_legacy_name_is_clipped_to_one_line_of_60_characters_in_the_note() {
        let long = format!("Legacy\nalloy {}", "x".repeat(300));
        let mut doc = Document {
            material: long.clone(),
            ..Default::default()
        };
        let note = doc.adopt_materials().unwrap();
        let name = note
            .strip_prefix("The project material \"")
            .and_then(|rest| rest.split_once("\" was added to the material library"))
            .map(|(name, _)| name)
            .unwrap_or_else(|| panic!("{note}"));
        assert!(name.starts_with("Legacy alloy xxx"), "{note}");
        assert_eq!(name.chars().count(), 60, "{note}");
        assert!(!note.contains('\n'));
        // The library keeps the name as it was.
        assert_eq!(doc.selected_material().unwrap().name(), long.trim());
        // A linked name is clipped too.
        let mut linked = Document {
            materials: vec![crate::material::Material::new(&long)],
            material: long,
            ..Default::default()
        };
        let note = linked.adopt_materials().unwrap();
        assert!(note.contains("linked to the library"), "{note}");
        assert!(note.chars().count() < 120, "{note}");
    }
    #[test]
    fn a_library_with_entries_gets_no_starters() {
        use crate::material::Material;
        let mut doc = Document {
            materials: vec![Material::new("Titanium Gr5")],
            starter_materials: crate::material::STARTER_SET,
            ..Default::default()
        };
        assert!(doc.adopt_materials().is_none());
        assert_eq!(doc.materials.len(), 1);
        // A legacy name still links to the library, or is added to it.
        doc.material = "titanium GR5".into();
        let note = doc.adopt_materials().unwrap();
        assert!(note.contains("linked to the library"), "{note}");
        assert_eq!(doc.materials.len(), 1);
        doc.material_id.clear();
        doc.material = "Aluminium 6061-T6".into();
        let note = doc.adopt_materials().unwrap();
        assert!(note.contains("without cutting data"), "{note}");
        assert_eq!(doc.materials.len(), 2);
        assert!(doc.selected_material().unwrap().cutting_data().is_none());
        // No selection never matches a material that has no id.
        let mut blank = Material::new("No id");
        blank.id.clear();
        doc.materials.push(blank);
        doc.material_id.clear();
        assert!(doc.selected_material().is_none());
    }
    #[test]
    fn an_existing_library_gets_the_new_alloys_once() {
        let mut doc = Document {
            materials: vec![crate::material::Material::new("My alloy")],
            ..Document::default()
        };
        assert!(doc.adopt_materials().is_none());
        let names: Vec<_> = doc.materials.iter().map(|m| m.name().to_string()).collect();
        assert_eq!(names.len(), 14, "{names:?}");
        assert!(names.contains(&"Aluminium 6082-T6".to_string()));
        assert!(names.contains(&"Walnut".to_string()));
        assert!(!names.contains(&"Aluminium 6061-T6".to_string()));
        assert_eq!(doc.starter_materials, crate::material::STARTER_SET);
        // Deleted after the addition: it does not come back.
        doc.materials.retain(|m| m.name() != "Aluminium 6063-T6");
        doc.adopt_materials();
        assert!(
            !doc.materials
                .iter()
                .any(|m| m.name() == "Aluminium 6063-T6")
        );
        // A name the user already has (any case) is not added twice.
        let mut mine = Document {
            materials: vec![crate::material::Material::new("aluminium 5083-h111")],
            ..Document::default()
        };
        mine.adopt_materials();
        assert_eq!(
            mine.materials
                .iter()
                .filter(|m| crate::material::same_name(m.name(), "Aluminium 5083-H111"))
                .count(),
            1
        );
        // An empty library gets all twenty-two and the current set.
        let mut empty = Document::default();
        empty.adopt_materials();
        assert_eq!((empty.materials.len(), empty.starter_materials), (22, 3));
    }
    #[test]
    fn choosing_a_material_sets_every_operations_speed_and_feed() {
        let mut router = Machine::new(3);
        router.p.insert("rpm".into(), "24000".into());
        router.p.insert("feed".into(), "1000".into());
        let tool = Tool::new("Flat 3", "Flat end mill", "3");
        let mut setup = Setup::new(router, "G54");
        let op = |name: &str, tool: &str, rpm: &str| {
            let mut o = Operation::new(Strategy::Contour, tool.into());
            o.p.insert("name".into(), name.into());
            o.p.insert("rpm".into(), rpm.into());
            o.p.insert("feed".into(), "1000".into());
            o
        };
        setup.operations = vec![
            op("Facing", &tool.id, "12000"),
            op("Contour", &tool.id, "8000"),
            op("Loose", "", "8000"),
        ];
        let mut doc = Document {
            tools: vec![tool],
            setups: vec![setup],
            ..Default::default()
        };
        doc.adopt_materials();
        // No material chosen: nothing is said or changed.
        assert_eq!(doc.fit_speeds(), "");
        let pine = doc
            .materials
            .iter()
            .find(|m| m.name() == "Pine")
            .unwrap()
            .id
            .clone();
        assert!(doc.select_material(&pine));
        // Both get pine's speed, whatever they had: at this router's 1000
        // mm/min the least surface speed, for the thickest chip. The
        // operation without a tool is left alone.
        let loose = "1 operation not set: the operation has no tool.";
        assert_eq!(
            doc.fit_speeds(),
            format!(
                "Speeds and feeds set for Pine: Top side / Facing 12000 → 10611 rpm; Top side / Contour 8000 → 10611 rpm. {loose}"
            )
        );
        let rpm = |doc: &Document, i: usize| get(&doc.setups[0].operations[i].p, "rpm").to_string();
        assert_eq!(
            [rpm(&doc, 0), rpm(&doc, 1), rpm(&doc, 2)],
            ["10611", "10611", "8000"]
        );
        assert_eq!(
            doc.fit_speeds(),
            format!("No speed or feed changed for Pine. {loose}")
        );
        doc.setups[0].operations.pop();
        assert_eq!(
            doc.fit_speeds(),
            "Every operation already runs at Pine's speed and feed."
        );
        // The same material always gives the same speeds, whatever came
        // before it: pine, acrylic, pine.
        let acrylic = doc
            .materials
            .iter()
            .find(|m| m.name() == "Acrylic (PMMA)")
            .unwrap()
            .id
            .clone();
        assert!(doc.select_material(&acrylic));
        assert!(
            doc.fit_speeds()
                .starts_with("Speeds and feeds set for Acrylic (PMMA): ")
        );
        assert_ne!(rpm(&doc, 0), "10611");
        assert!(doc.select_material(&pine));
        doc.fit_speeds();
        assert_eq!([rpm(&doc, 0), rpm(&doc, 1)], ["10611", "10611"]);
        // Aluminium wants a slower spindle and, at it, more feed than the
        // machine gives: both operations change.
        let alu = doc
            .materials
            .iter()
            .find(|m| m.name() == "Aluminium 6061-T6")
            .unwrap()
            .id
            .clone();
        assert!(doc.select_material(&alu));
        let said = doc.fit_speeds();
        assert!(
            said.starts_with(
                "Speeds and feeds set for Aluminium 6061-T6: Top side / Facing 10611 → "
            ),
            "{said}"
        );
        assert!(said.contains("; Top side / Contour 10611 → "), "{said}");
        for o in &doc.setups[0].operations {
            assert!(
                matches!(
                    doc.speed_feed(&doc.setups[0], o),
                    Some(crate::material::SpeedFeed::InRange { .. })
                ),
                "{:?}",
                o.p
            );
        }
        // A material without cutting data changes nothing.
        let mut bare = crate::material::Material::new("Bare");
        bare.p.insert("group".into(), "Other".into());
        let bare_id = bare.id.clone();
        doc.materials.push(bare);
        assert!(doc.select_material(&bare_id));
        let speeds = |doc: &Document| -> Vec<_> {
            doc.setups[0]
                .operations
                .iter()
                .map(|o| (get(&o.p, "rpm").to_string(), get(&o.p, "feed").to_string()))
                .collect()
        };
        let before = speeds(&doc);
        assert_eq!(
            doc.fit_speeds(),
            "Bare has no cutting data: speeds and feeds are unchanged."
        );
        assert_eq!(speeds(&doc), before);
    }
    #[test]
    fn a_library_given_the_alloys_gets_only_the_woods_once() {
        let mut doc = Document {
            materials: vec![crate::material::Material::new("My alloy")],
            starter_materials: 2,
            ..Document::default()
        };
        assert!(doc.adopt_materials().is_none());
        let names: Vec<_> = doc.materials.iter().map(|m| m.name()).collect();
        assert_eq!(names[0], "My alloy");
        assert_eq!(names[1..], crate::material::ADDED_IN_SET_3);
        assert_eq!(doc.starter_materials, 3);
        // Deleted after the addition: it does not come back.
        doc.materials.retain(|m| m.name() != "Cherry");
        doc.adopt_materials();
        assert_eq!(doc.materials.len(), 7);
        // The user's own spelling of a wood is kept, not doubled.
        let mut mine = Document {
            materials: vec![crate::material::Material::new("oak")],
            starter_materials: 2,
            ..Document::default()
        };
        mine.adopt_materials();
        assert_eq!(mine.materials.len(), 7);
        assert_eq!(mine.materials[0].name(), "oak");
    }
    fn doc_with_holding(pairs: &[(&str, &str)]) -> Document {
        let mut doc = Document::sample();
        doc.adopt_holders();
        for (k, v) in pairs {
            doc.setup_mut().p.insert((*k).into(), (*v).into());
        }
        doc
    }
    fn starter(doc: &Document, kind: &str) -> String {
        doc.workholding
            .iter()
            .find(|h| h.kind() == kind)
            .unwrap()
            .id
            .clone()
    }
    #[test]
    fn adopting_holders_adds_the_starters_only_to_an_empty_library() {
        let mut doc = Document::default();
        assert!(doc.workholding.is_empty());
        doc.adopt_holders();
        let names: Vec<_> = doc.workholding.iter().map(|h| h.name()).collect();
        assert_eq!(names, ["100 mm machine vise", "Toe clamp 60 mm"]);
        assert!(doc.validate_structure().is_ok());
        // A library the user edited keeps their edits and is not topped up.
        doc.workholding[0].p.insert("name".into(), "My vise".into());
        doc.workholding.pop();
        let id = doc.workholding[0].id.clone();
        doc.adopt_holders();
        assert_eq!(doc.workholding.len(), 1);
        assert_eq!(
            (doc.workholding[0].name(), &doc.workholding[0].id),
            ("My vise", &id)
        );
    }
    #[test]
    fn a_project_with_duplicate_or_empty_holder_ids_is_refused() {
        use crate::holding::Holder;
        let mut doc = Document::default();
        doc.adopt_holders();
        assert!(doc.validate_structure().is_ok());
        let mut copy = doc.workholding[0].clone();
        copy.p.insert("name".into(), "Other name".into());
        doc.workholding.push(copy);
        let error = doc.validate_structure().unwrap_err();
        assert!(error.contains("duplicate library identifier"), "{error}");
        doc.workholding.pop();
        let mut blank = Holder::new("Vise", "No id");
        blank.id.clear();
        doc.workholding.push(blank);
        assert!(doc.validate_structure().is_err());
        doc.workholding.pop();
        assert!(doc.validate_structure().is_ok());
    }
    #[test]
    fn a_holder_name_is_taken_regardless_of_case_and_spaces() {
        use crate::holding::Holder;
        let mut doc = Document::default();
        doc.adopt_holders();
        let mut same = Holder::new("Vise", " 100 MM machine vise ");
        assert_eq!(
            doc.holder_name_issues(&same),
            ["A workholding entry named 100 MM machine vise is already in the library."]
        );
        same.id = doc.workholding[0].id.clone();
        assert!(doc.holder_name_issues(&same).is_empty());
        assert!(
            doc.holder_name_issues(&Holder::new("Vise", "  "))
                .is_empty()
        );
        assert!(
            doc.holder_name_issues(&Holder::new("Clamp", "Strap"))
                .is_empty()
        );
    }
    #[test]
    fn a_setup_without_holding_keys_is_none_and_has_no_issues() {
        let doc = doc_with_holding(&[]);
        let setup = doc.setup();
        for key in ["holding", "holder", "vise_axis", "vise_grip", "clamps"] {
            assert!(!setup.p.contains_key(key), "{key}");
        }
        assert_eq!(
            crate::holding::layout(setup),
            Ok(crate::holding::Layout::None)
        );
        assert!(doc.holding_issues(0, None).is_empty());
        // An unknown setup index has none either.
        assert!(doc.holding_issues(9, None).is_empty());
        let json = serde_json::to_value(&doc).unwrap();
        let mut old = json.clone();
        old.as_object_mut().unwrap().remove("workholding");
        let back: Document = serde_json::from_value(old).unwrap();
        assert!(back.workholding.is_empty());
    }
    #[test]
    fn holding_issues_name_the_layout_the_entry_and_the_opening() {
        use crate::holding::DELETED;
        let mut doc = doc_with_holding(&[
            ("stock_type", "Fixed size box"),
            ("stock_x", "140"),
            ("stock_y", "100"),
            ("stock_z", "40"),
        ]);
        let vise = starter(&doc, "Vise");
        let clamp = starter(&doc, "Clamp");
        // Without workholding nothing is said, however wide the stock is.
        assert!(doc.holding_issues(0, None).is_empty());
        let set = |doc: &mut Document, pairs: &[(&str, &str)]| {
            for (k, v) in pairs {
                doc.setup_mut().p.insert((*k).into(), (*v).into());
            }
        };
        // The vise opens to 120 mm: 140 across X is too wide, 100 across Y fits.
        set(
            &mut doc,
            &[("holding", "Vise"), ("holder", &vise), ("vise_grip", "8")],
        );
        assert_eq!(
            doc.holding_issues(0, None),
            ["The stock is 140 mm across the jaws; this vise opens to 120 mm."]
        );
        set(&mut doc, &[("vise_axis", "Y")]);
        assert!(doc.holding_issues(0, None).is_empty());
        // The 40 mm stock must stand at least 1 mm above the jaws.
        set(&mut doc, &[("vise_grip", "39.5")]);
        let short = "The stock is 40 mm tall; the vise grips 39.5 mm of it, which leaves less than 1 mm above the jaws.";
        assert_eq!(doc.holding_issues(0, None), [short]);
        let stock = doc.setup_stock(0, None).unwrap();
        assert_eq!(
            doc.holding_boxes(doc.setup(), stock),
            Err(short.to_string())
        );
        set(&mut doc, &[("vise_grip", "39")]);
        assert!(doc.holding_issues(0, None).is_empty());
        set(&mut doc, &[("vise_grip", "8")]);
        set(&mut doc, &[("vise_axis", "X"), ("stock_x", "140.5")]);
        assert_eq!(
            doc.holding_issues(0, None),
            ["The stock is 140.5 mm across the jaws; this vise opens to 120 mm."]
        );
        // The layout's own error comes alone.
        set(&mut doc, &[("vise_grip", "-1")]);
        assert_eq!(
            doc.holding_issues(0, None),
            ["The vise grip must be a positive number."]
        );
        set(&mut doc, &[("vise_grip", "8")]);
        // An entry that is gone, none chosen, one of the other kind, and one
        // that is not finished.
        set(&mut doc, &[("holder", "gone")]);
        assert_eq!(doc.holding_issues(0, None), [DELETED]);
        assert_eq!(
            DELETED,
            "The workholding entry was deleted; choose another."
        );
        set(&mut doc, &[("holder", "")]);
        assert_eq!(
            doc.holding_issues(0, None),
            ["Choose a vise from the library."]
        );
        set(&mut doc, &[("holder", &clamp)]);
        assert_eq!(
            doc.holding_issues(0, None),
            ["The chosen workholding entry is a clamp, not a vise."]
        );
        set(&mut doc, &[("holder", &vise)]);
        doc.workholding[0].p.remove("jaw_thickness");
        let issues = doc.holding_issues(0, None);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("jaw thickness"), "{issues:?}");
        doc.workholding[0]
            .p
            .insert("jaw_thickness".into(), "20".into());
        // Clamps: the opening does not apply.
        set(
            &mut doc,
            &[
                ("holding", "Clamps"),
                ("holder", &clamp),
                ("clamps", "-X:0;+X:0"),
            ],
        );
        assert!(doc.holding_issues(0, None).is_empty());
        set(&mut doc, &[("clamps", "-Z:0")]);
        assert_eq!(
            doc.holding_issues(0, None),
            ["A clamp side must be -X, +X, -Y or +Y."]
        );
        set(&mut doc, &[("clamps", "")]);
        assert_eq!(doc.holding_issues(0, None), ["Place 1 to 8 clamps."]);
        set(&mut doc, &[("clamps", "-X:0"), ("holder", "")]);
        assert_eq!(
            doc.holding_issues(0, None),
            ["Choose a clamp from the library."]
        );
    }
    #[test]
    fn holding_issues_with_checks_the_stock_it_is_told() {
        let mut doc = doc_with_holding(&[
            ("stock_type", "Fixed size box"),
            ("stock_x", "100"),
            ("stock_y", "100"),
            ("stock_z", "40"),
        ]);
        let vise = starter(&doc, "Vise");
        for (k, v) in [
            ("holding", "Vise"),
            ("holder", &vise),
            ("vise_axis", "X"),
            ("vise_grip", "8"),
        ] {
            doc.setup_mut().p.insert(k.into(), v.into());
        }
        let own = doc.setup_stock(0, None).unwrap();
        // The setup's own box is what `holding_issues` takes.
        assert!(doc.holding_issues(0, None).is_empty());
        assert_eq!(
            doc.holding_issues_with(0, Some(own.into())),
            doc.holding_issues(0, None)
        );
        // A box wider across the jaws than the vise opens, or too low for the grip.
        let wide = crate::mesh::Bounds {
            max: [own.min[0] + 130., own.max[1], own.max[2]],
            ..own
        };
        assert_eq!(
            doc.holding_issues_with(0, Some(wide.into())),
            ["The stock is 130 mm across the jaws; this vise opens to 120 mm."]
        );
        let low = crate::mesh::Bounds {
            max: [own.max[0], own.max[1], own.min[2] + 8.5],
            ..own
        };
        assert_eq!(
            doc.holding_issues_with(0, Some(low.into())),
            [
                "The stock is 8.5 mm tall; the vise grips 8 mm of it, which leaves less than 1 mm above the jaws."
            ]
        );
        // No box is told: only what does not need one is checked.
        assert!(doc.holding_issues_with(0, None).is_empty());
        doc.setup_mut().p.insert("holder".into(), "gone".into());
        assert_eq!(doc.holding_issues_with(0, None), [crate::holding::DELETED]);
        // An unknown setup has none.
        assert!(doc.holding_issues_with(9, Some(wide.into())).is_empty());
    }
    #[test]
    fn holding_boxes_refuse_with_the_first_line_the_editor_shows() {
        use crate::holding::{self, DELETED};
        let mut doc = doc_with_holding(&[
            ("stock_type", "Fixed size box"),
            ("stock_x", "140"),
            ("stock_y", "100"),
            ("stock_z", "40"),
        ]);
        let vise = starter(&doc, "Vise");
        let clamp = starter(&doc, "Clamp");
        let stock = doc.setup_stock(0, None).unwrap();
        let set = |doc: &mut Document, pairs: &[(&str, &str)]| {
            for (k, v) in pairs {
                doc.setup_mut().p.insert((*k).into(), (*v).into());
            }
        };
        // Without workholding there is nothing to refuse, whatever the entry says.
        assert_eq!(doc.holding_boxes(doc.setup(), stock), Ok(vec![]));
        set(&mut doc, &[("holder", "gone")]);
        assert_eq!(doc.holding_boxes(doc.setup(), stock), Ok(vec![]));
        // The jaws and the clamps are the ones the library entry and the layout make.
        set(
            &mut doc,
            &[
                ("holding", "Vise"),
                ("holder", &vise),
                ("vise_grip", "8"),
                ("vise_axis", "Y"),
            ],
        );
        let boxes = doc.holding_boxes(doc.setup(), stock).unwrap();
        let labels: Vec<_> = boxes.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["vise jaw on -Y", "vise jaw on +Y"]);
        let layout = holding::layout(doc.setup()).unwrap();
        let entry = doc.holder(&vise).unwrap();
        assert_eq!(boxes, holding::boxes(&layout, entry, stock).unwrap());
        set(
            &mut doc,
            &[
                ("holding", "Clamps"),
                ("holder", &clamp),
                ("clamps", "-X:0;+Y:10"),
            ],
        );
        let boxes = doc.holding_boxes(doc.setup(), stock).unwrap();
        let labels: Vec<_> = boxes.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(
            labels,
            ["clamp on -X (offset 0)", "clamp on +Y (offset 10)"]
        );
        // Every refusal is the first line `holding_issues` shows, the opening included.
        set(
            &mut doc,
            &[
                ("holding", "Vise"),
                ("holder", &vise),
                ("vise_axis", "X"),
                ("vise_grip", "8"),
            ],
        );
        let cases: [&[(&str, &str)]; 8] = [
            &[],
            &[("holder", "")],
            &[("holder", "gone")],
            &[("holder", &clamp)],
            &[("holder", &vise), ("vise_grip", "-1")],
            &[("vise_grip", "8"), ("vise_axis", "Z")],
            &[("vise_axis", "X"), ("holding_gap", "-1")],
            &[("holding_gap", "2"), ("holding", "Gripper")],
        ];
        for (n, pairs) in cases.into_iter().enumerate() {
            set(&mut doc, pairs);
            let issues = doc.holding_issues(0, None);
            assert!(!issues.is_empty(), "case {n}");
            assert_eq!(
                doc.holding_boxes(doc.setup(), stock),
                Err(issues[0].clone()),
                "case {n}"
            );
        }
        set(&mut doc, &[("holding", "Vise"), ("holder", "gone")]);
        assert_eq!(
            doc.holding_boxes(doc.setup(), stock),
            Err(DELETED.to_string())
        );
        // The stock across the jaws is wider than the vise opens (140 against 120).
        set(&mut doc, &[("holder", &vise), ("vise_axis", "X")]);
        assert_eq!(
            doc.holding_boxes(doc.setup(), stock),
            Err("The stock is 140 mm across the jaws; this vise opens to 120 mm.".to_string())
        );
        // The stock given is the one that counts, not the setup's own.
        let narrow = crate::mesh::Bounds {
            min: [-50., -30., 0.],
            max: [50., 30., 40.],
        };
        assert_eq!(doc.holding_boxes(doc.setup(), narrow).unwrap().len(), 2);
    }
    #[test]
    fn the_setup_sheet_describes_modelled_workholding_in_place_of_the_fixture_note() {
        let mut doc = doc_with_holding(&[]);
        let vise = starter(&doc, "Vise");
        let clamp = starter(&doc, "Clamp");
        assert!(!doc.setup_sheet(None).contains("Workholding"));
        assert_eq!(doc.holding_text(doc.setup()), None);
        let origin = get(&doc.setup().p, "origin").to_string();
        // What follows the origin line, up to the rotation.
        let after = |doc: &Document, paragraphs: &str| {
            let sheet = doc.setup_sheet(None);
            assert!(
                sheet.contains(&format!("Origin: {origin}\n\n{paragraphs}Rotation:")),
                "{sheet}"
            );
        };
        let set = |doc: &mut Document, pairs: &[(&str, &str)]| {
            for (k, v) in pairs {
                doc.setup_mut().p.insert((*k).into(), (*v).into());
            }
        };
        after(&doc, "");
        set(&mut doc, &[("fixture", "Chuck")]);
        after(&doc, "Fixture: Chuck\n\n");
        set(
            &mut doc,
            &[
                ("holding", "Vise"),
                ("holder", &vise),
                ("vise_grip", "8"),
                ("vise_axis", "Y"),
                ("holding_gap", "2"),
            ],
        );
        after(
            &doc,
            "Workholding: 100 mm machine vise on ±Y, grip 8 mm, offset 0 mm, gap 2 mm.\n\n",
        );
        set(
            &mut doc,
            &[
                ("holding", "Clamps"),
                ("holder", &clamp),
                ("clamps", "-X:0;+X:0;-Y:0;+Y:0"),
            ],
        );
        after(
            &doc,
            "Workholding: 4 × Toe clamp 60 mm at -X 0, +X 0, -Y 0, +Y 0, gap 2 mm.\n\n",
        );
        assert_eq!(
            doc.holding_text(doc.setup()).as_deref(),
            Some("4 × Toe clamp 60 mm at -X 0, +X 0, -Y 0, +Y 0, gap 2 mm.")
        );
        // A vise the stock does not fit in is described, and the editor's and
        // the export's problem with it follows (the stock is 140 mm across X).
        set(
            &mut doc,
            &[
                ("stock_type", "Fixed size box"),
                ("stock_x", "140"),
                ("stock_y", "100"),
                ("stock_z", "40"),
                ("holding", "Vise"),
                ("holder", &vise),
                ("vise_axis", "X"),
            ],
        );
        after(
            &doc,
            "Workholding: 100 mm machine vise on ±X, grip 8 mm, offset 0 mm, gap 2 mm.\n\nWorkholding problem: The stock is 140 mm across the jaws; this vise opens to 120 mm.\n\n",
        );
        set(&mut doc, &[("vise_axis", "Y")]);
        after(
            &doc,
            "Workholding: 100 mm machine vise on ±Y, grip 8 mm, offset 0 mm, gap 2 mm.\n\n",
        );
        // A setup that cannot be modelled says why, never what it would have been.
        set(&mut doc, &[("holder", "gone")]);
        after(
            &doc,
            "Workholding problem: The workholding entry was deleted; choose another.\n\n",
        );
        assert_eq!(doc.holding_text(doc.setup()), None);
        set(
            &mut doc,
            &[
                ("holding", "Clamps"),
                ("holder", &clamp),
                ("clamps", "-Z:0"),
            ],
        );
        after(
            &doc,
            "Workholding problem: A clamp side must be -X, +X, -Y or +Y.\n\n",
        );
        set(&mut doc, &[("holder", &vise), ("clamps", "-X:0")]);
        after(
            &doc,
            "Workholding problem: The chosen workholding entry is a vise, not a clamp.\n\n",
        );
        set(&mut doc, &[("holding", "None")]);
        assert!(!doc.setup_sheet(None).contains("Workholding"));
    }
    #[test]
    fn the_opening_check_is_skipped_silently_when_the_stock_cannot_be_resolved() {
        // Relative stock needs the part; with none loaded the stock is unknown.
        let mut doc = doc_with_holding(&[("holding", "Vise"), ("vise_grip", "8")]);
        let vise = starter(&doc, "Vise");
        doc.setup_mut().p.insert("holder".into(), vise);
        assert!(doc.holding_issues(0, None).is_empty());
        // Invalid stock numbers are the stock section's to report.
        let p = &mut doc.setup_mut().p;
        p.insert("stock_type".into(), "Fixed size box".into());
        p.insert("stock_x".into(), "wide".into());
        assert!(doc.holding_issues(0, None).is_empty());
        // With the part loaded the relative stock is resolved: the sample part
        // (140 × 94 mm) plus 5 mm on each side is 150 across X and 104 across Y.
        let mesh = crate::mesh::sample();
        let p = &mut doc.setup_mut().p;
        p.insert("stock_type".into(), "Relative size box".into());
        assert_eq!(
            doc.holding_issues(0, Some(&mesh)),
            ["The stock is 150 mm across the jaws; this vise opens to 120 mm."]
        );
        doc.setup_mut().p.insert("vise_axis".into(), "Y".into());
        assert!(doc.holding_issues(0, Some(&mesh)).is_empty());
    }
    #[test]
    fn settling_a_setups_holding_picks_an_entry_of_its_kind_a_grip_and_a_gap() {
        let mut doc = doc_with_holding(&[("holding", "Vise")]);
        let vise = starter(&doc, "Vise");
        doc.settle_holding(0);
        let p = &doc.setups[0].p;
        assert_eq!(get(p, "holder"), vise);
        assert_eq!(get(p, "vise_grip"), "8");
        assert_eq!(get(p, "holding_gap"), "2");
        // A grip the user has set stays; so does a gap.
        let p = &mut doc.setups[0].p;
        p.insert("vise_grip".into(), "5".into());
        p.insert("holding_gap".into(), "3".into());
        doc.settle_holding(0);
        let p = &doc.setups[0].p;
        assert_eq!((get(p, "vise_grip"), get(p, "holding_gap")), ("5", "3"));
        // Clamps replace a holder of the other kind and take no grip.
        let mut other = doc_with_holding(&[("holding", "Clamps")]);
        let wrong = starter(&other, "Vise");
        other.setup_mut().p.insert("holder".into(), wrong);
        other.settle_holding(0);
        assert_eq!(get(&other.setups[0].p, "holder"), starter(&other, "Clamp"));
        assert_eq!(get(&other.setups[0].p, "vise_grip"), "");
        // A holder that was deleted is replaced by one that exists, and a kind
        // the library has none of leaves none chosen.
        let mut gone = doc_with_holding(&[("holding", "Vise"), ("holder", "gone")]);
        gone.settle_holding(0);
        assert_eq!(get(&gone.setups[0].p, "holder"), starter(&gone, "Vise"));
        gone.workholding.retain(|h| h.kind() != "Vise");
        gone.setups[0].p.insert("holder".into(), "gone".into());
        gone.settle_holding(0);
        assert_eq!(get(&gone.setups[0].p, "holder"), "");
        // None leaves the setup alone.
        let mut none = doc_with_holding(&[("holding", "None"), ("holder", "keep")]);
        none.settle_holding(0);
        assert_eq!(get(&none.setups[0].p, "holder"), "keep");
        assert_eq!(get(&none.setups[0].p, "holding_gap"), "");
        none.settle_holding(9);
    }
    fn two_setup_doc() -> Document {
        let mut doc = doc_with_holding(&[
            ("stock_type", "Fixed size box"),
            ("stock_x", "140"),
            ("stock_y", "100"),
            ("stock_z", "40"),
        ]);
        // The second setup is turned 90 degrees about Z; its own stock keys
        // are hidden in the editor and ignored.
        let mut second = doc.setups[0].duplicate("G55");
        for (k, v) in [
            ("rz", "90"),
            ("stock_x", "10"),
            ("stock_y", "10"),
            ("stock_z", "10"),
        ] {
            second.p.insert(k.into(), v.into());
        }
        doc.setups.push(second);
        doc
    }
    fn dims(b: crate::mesh::Bounds) -> [f32; 3] {
        b.dimensions().map(|v| (v * 100.).round() / 100.)
    }
    #[test]
    fn a_later_setups_stock_is_the_first_setups_carried_into_its_frame() {
        use crate::volume::{Frame, Volume};
        let doc = two_setup_doc();
        let mesh = crate::mesh::sample();
        let first = doc.setup_stock(0, Some(&mesh)).unwrap();
        assert_eq!(dims(first), [140., 100., 40.]);
        let second = doc.setup_stock(1, Some(&mesh)).unwrap();
        assert_eq!(dims(second), [100., 140., 40.]);
        // What the simulation hands the second setup before earlier cuts
        // trim it.
        let carried = Volume::new(first, Frame::new(&doc.setups[0], &mesh))
            .transformed_bounds(Frame::new(&doc.setups[1], &mesh));
        assert_eq!(second, carried);
        // The first setup's fixed stock needs no part; a later setup's frame does.
        assert_eq!(dims(doc.setup_stock(0, None).unwrap()), [140., 100., 40.]);
        assert_eq!(doc.setup_stock(1, None), None);
        assert_eq!(doc.setup_stock(2, Some(&mesh)), None);
        // Relative stock needs the part even for the first setup.
        let mut relative = doc.clone();
        relative.setups[0]
            .p
            .insert("stock_type".into(), "Relative size box".into());
        assert_eq!(relative.setup_stock(0, None), None);
        assert!(relative.setup_stock(0, Some(&mesh)).is_some());
        // A first stock that cannot be told, or a later setup with no valid
        // placement, gives none.
        let mut bad = doc.clone();
        bad.setups[0].p.insert("stock_x".into(), "wide".into());
        assert_eq!(bad.setup_stock(0, Some(&mesh)), None);
        assert_eq!(bad.setup_stock(1, Some(&mesh)), None);
        let mut unplaced = doc.clone();
        unplaced.setups[1].p.insert("rz".into(), "turn".into());
        assert_eq!(unplaced.setup_stock(1, Some(&mesh)), None);
        assert!(unplaced.setup_stock(0, Some(&mesh)).is_some());
        assert_eq!(Document::default().setup_stock(0, Some(&mesh)), None);
    }
    #[test]
    fn the_opening_check_of_a_later_setup_uses_the_stock_it_really_starts_from() {
        let mut doc = two_setup_doc();
        let mesh = crate::mesh::sample();
        let vise = starter(&doc, "Vise");
        for s in &mut doc.setups {
            for (k, v) in [("holding", "Vise"), ("vise_grip", "8")] {
                s.p.insert(k.into(), v.into());
            }
            s.p.insert("holder".into(), vise.clone());
        }
        let issues = |doc: &Document, i: usize| doc.holding_issues(i, Some(&mesh));
        let wide = "The stock is 140 mm across the jaws; this vise opens to 120 mm.";
        // Setup 1 is 140 across X and 100 across Y.
        assert_eq!(issues(&doc, 0), [wide]);
        doc.setups[0].p.insert("vise_axis".into(), "Y".into());
        assert!(issues(&doc, 0).is_empty());
        // Setup 2 starts from the same stock turned: 100 across X, 140 across Y.
        assert!(issues(&doc, 1).is_empty());
        doc.setups[1].p.insert("vise_axis".into(), "Y".into());
        assert_eq!(issues(&doc, 1), [wide]);
        // Without the part the later setup's frame is not known: no opinion.
        assert!(doc.holding_issues(1, None).is_empty());
    }
    /// `two_setup_doc` with a vise that opens wide on both setups.
    fn two_vises(rz: &str) -> Document {
        let mut doc = two_setup_doc();
        let vise = starter(&doc, "Vise");
        for h in doc.workholding.iter_mut().filter(|h| h.id == vise) {
            h.p.insert("max_opening".into(), "300".into());
        }
        for s in &mut doc.setups {
            for (k, v) in [("holding", "Vise"), ("vise_grip", "8")] {
                s.p.insert(k.into(), v.into());
            }
            s.p.insert("holder".into(), vise.clone());
        }
        doc.setups[1].p.insert("rz".into(), rz.into());
        doc
    }
    #[test]
    fn a_later_setup_not_square_to_setup_1_cannot_model_its_workholding() {
        use crate::holding::NOT_SQUARE;
        let mesh = crate::mesh::sample();
        assert_eq!(
            NOT_SQUARE,
            "This setup is not square to setup 1, so its vise or clamps cannot be modelled. Choose None, or square the setup."
        );
        // A quarter turn about Z, in either direction, a half turn and none
        // keep the axes parallel: the jaws turn with the stock.
        for rz in ["0", "90", "180", "270", "-90", "360"] {
            let doc = two_vises(rz);
            let stock = doc.setup_stock(1, Some(&mesh)).unwrap();
            assert!(doc.holding_issues(1, Some(&mesh)).is_empty(), "rz {rz}");
            assert_eq!(doc.holding_boxes(&doc.setups[1], stock).unwrap().len(), 2);
        }
        // Any other angle: the jaws would be on the box round the turned stock.
        let mut doc = two_vises("30");
        let stock = doc.setup_stock(1, Some(&mesh)).unwrap();
        assert_eq!(doc.holding_issues(1, Some(&mesh)), [NOT_SQUARE]);
        // The angles tell it, with or without the part.
        assert_eq!(doc.holding_issues(1, None), [NOT_SQUARE]);
        assert_eq!(
            doc.holding_boxes(&doc.setups[1], stock),
            Err(NOT_SQUARE.to_string())
        );
        // Setup 1 is square to itself, and the check is relative to it.
        assert!(doc.holding_issues(0, Some(&mesh)).is_empty());
        doc.setups[0].p.insert("rz".into(), "30".into());
        assert!(doc.holding_issues(1, Some(&mesh)).is_empty());
        // A tilt is not square either; clamps are refused alike; None says nothing.
        let mut tilted = two_vises("90");
        tilted.setups[1].p.insert("rx".into(), "45".into());
        assert_eq!(tilted.holding_issues(1, None), [NOT_SQUARE]);
        let clamp = starter(&tilted, "Clamp");
        for (k, v) in [
            ("holding", "Clamps"),
            ("holder", clamp.as_str()),
            ("clamps", "-X:0"),
        ] {
            tilted.setups[1].p.insert(k.into(), v.into());
        }
        assert_eq!(tilted.holding_issues(1, None), [NOT_SQUARE]);
        tilted.setups[1].p.insert("holding".into(), "None".into());
        assert!(tilted.holding_issues(1, None).is_empty());
    }
    #[test]
    fn an_offset_must_leave_the_jaws_on_the_stock_and_each_clamp_on_its_side() {
        let mut doc = doc_with_holding(&[
            ("stock_type", "Fixed size box"),
            ("stock_x", "140"),
            ("stock_y", "100"),
            ("stock_z", "40"),
        ]);
        let (vise, clamp) = (starter(&doc, "Vise"), starter(&doc, "Clamp"));
        let set = |doc: &mut Document, pairs: &[(&str, &str)]| {
            for (k, v) in pairs {
                doc.setup_mut().p.insert((*k).into(), (*v).into());
            }
        };
        let stock = doc.setup_stock(0, None).unwrap();
        // Jaws on the Y sides run along X: the stock is 140 long there and
        // the jaws 100 wide, so they overlap it at offsets under 120 mm.
        set(
            &mut doc,
            &[
                ("holding", "Vise"),
                ("holder", &vise),
                ("vise_grip", "8"),
                ("vise_axis", "Y"),
            ],
        );
        for offset in ["-119.9", "0", "119.9", "  60 "] {
            set(&mut doc, &[("vise_offset", offset)]);
            assert!(doc.holding_issues(0, None).is_empty(), "{offset}");
        }
        let beside = "The vise offset of 120 mm puts the jaws beside the stock, not on it. Keep it under 120 mm either way.";
        for offset in ["120", "-120", "300"] {
            set(&mut doc, &[("vise_offset", offset)]);
            let issues = doc.holding_issues(0, None);
            assert_eq!(issues.len(), 1, "{offset}");
            assert!(issues[0].starts_with("The vise offset of "), "{issues:?}");
            assert!(issues[0].ends_with("Keep it under 120 mm either way."));
            assert_eq!(
                doc.holding_boxes(doc.setup(), stock),
                Err(issues[0].clone())
            );
        }
        set(&mut doc, &[("vise_offset", "120")]);
        assert_eq!(doc.holding_issues(0, None), [beside]);
        // The jaws on the X sides run along Y (100 long): under 100 mm.
        set(
            &mut doc,
            &[
                ("vise_axis", "X"),
                ("stock_x", "110"),
                ("vise_offset", "99"),
            ],
        );
        assert!(doc.holding_issues(0, None).is_empty());
        set(&mut doc, &[("vise_offset", "100")]);
        assert_eq!(
            doc.holding_issues(0, None),
            [
                "The vise offset of 100 mm puts the jaws beside the stock, not on it. Keep it under 100 mm either way."
            ]
        );
        // A clamp's centre stays within its side: ±X clamps move along Y
        // (100 long), ±Y clamps along X (140).
        set(
            &mut doc,
            &[
                ("stock_x", "140"),
                ("holding", "Clamps"),
                ("holder", &clamp),
                ("clamps", "-X:50;+X:-50;+Y:70;-Y:-70"),
            ],
        );
        assert!(doc.holding_issues(0, None).is_empty());
        for (clamps, issue) in [
            (
                "-X:0;+X:50.1",
                "The clamp on +X is at offset 50.1 mm, beyond its 100 mm side. Keep the offset within ±50 mm.",
            ),
            (
                "-Y:-70.5",
                "The clamp on -Y is at offset -70.5 mm, beyond its 140 mm side. Keep the offset within ±70 mm.",
            ),
            (
                "+Y:1000;-X:0",
                "The clamp on +Y is at offset 1000 mm, beyond its 140 mm side. Keep the offset within ±70 mm.",
            ),
        ] {
            set(&mut doc, &[("clamps", clamps)]);
            assert_eq!(doc.holding_issues(0, None), [issue], "{clamps}");
            assert_eq!(
                doc.holding_boxes(doc.setup(), stock),
                Err(issue.to_string())
            );
        }
        // The stock is the one the other checks use: a later setup's is the
        // first setup's turned, so a clamp that fits setup 1 may not fit it.
        let mesh = crate::mesh::sample();
        let mut two = two_setup_doc();
        let clamp = starter(&two, "Clamp");
        for s in &mut two.setups {
            for (k, v) in [("holding", "Clamps"), ("clamps", "-X:60")] {
                s.p.insert(k.into(), v.into());
            }
            s.p.insert("holder".into(), clamp.clone());
        }
        // Setup 1 is 140 x 100: -X moves along Y, 60 is off the 100 side.
        assert_eq!(two.holding_issues(0, Some(&mesh)).len(), 1);
        // Setup 2 is 100 x 140: -X moves along Y, 140 long, 60 is on it.
        assert!(two.holding_issues(1, Some(&mesh)).is_empty());
    }
    #[test]
    fn a_missing_entry_comes_first_and_says_where_to_add_one() {
        use crate::holding::DELETED;
        let mut doc = doc_with_holding(&[("holding", "Vise")]);
        // No vise in the library and no grip: the entry first, then the grip.
        doc.workholding.retain(|h| h.kind() != "Vise");
        assert_eq!(
            doc.holding_issues(0, None),
            [
                "Choose a vise from the library. Add one on the Workholding page.",
                "The vise grip must be a positive number."
            ]
        );
        doc.setup_mut().p.insert("vise_grip".into(), "8".into());
        assert_eq!(
            doc.holding_issues(0, None),
            ["Choose a vise from the library. Add one on the Workholding page."]
        );
        // The library has a vise, but none is chosen: no pointer to the page.
        doc.workholding = crate::holding::starters();
        assert_eq!(
            doc.holding_issues(0, None),
            ["Choose a vise from the library."]
        );
        // Clamps likewise.
        let mut clamps = doc_with_holding(&[("holding", "Clamps")]);
        clamps.workholding.retain(|h| h.kind() != "Clamp");
        assert_eq!(
            clamps.holding_issues(0, None),
            [
                "Choose a clamp from the library. Add one on the Workholding page.",
                "Place 1 to 8 clamps."
            ]
        );
        // An entry that was deleted comes before the layout's error too.
        doc.setup_mut().p.insert("holder".into(), "gone".into());
        doc.setup_mut().p.insert("vise_offset".into(), "far".into());
        assert_eq!(
            doc.holding_issues(0, None),
            [DELETED, "The vise offset must be a number."]
        );
        // No workholding and a choice that is not one: only the layout speaks.
        let mut none = doc_with_holding(&[("holder", "gone")]);
        assert!(none.holding_issues(0, None).is_empty());
        none.setup_mut().p.insert("holding".into(), "Glue".into());
        assert_eq!(
            none.holding_issues(0, None),
            ["\"Glue\" is not a workholding choice."]
        );
    }
}
