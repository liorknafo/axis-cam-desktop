use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

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
        e
    }
    pub fn fill_post_defaults(&mut self) {
        for key in POST_KEYS {
            if !self.p.contains_key(key) {
                let value = post_setting(&self.p, key).to_string();
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
        e
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
pub fn pocket_setting<'a>(p: &'a Fields, key: &str) -> &'a str {
    p.get(key).map(String::as_str).unwrap_or(match key {
        "pocket_milling" => "Climb",
        "pocket_entry" => POCKET_ENTRIES[0],
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
    .filter_map(|(key, values)| {
        (!values.contains(&facing_setting(p, key)))
            .then(|| format!("Choose a valid {} setting.", key.replace('_', " ")))
    })
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
/// Machines saved before the postprocessor settings use these defaults.
pub fn post_setting<'a>(p: &'a Fields, key: &str) -> &'a str {
    p.get(key).map(String::as_str).unwrap_or(match key {
        "post_tool_length" => POST_TOOL_LENGTH[0],
        "post_retract" => POST_RETRACT[0],
        "post_blend" => "0.01",
        "post_dwell" => "2",
        "post_air" => POST_AIR[0],
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
impl Operation {
    pub fn new(s: Strategy, tool: String) -> Self {
        let mut p = fields(&[
            ("name", s.name()),
            ("rpm", "8000"),
            ("feed", "1000"),
            ("coolant", "Flood"),
            ("boundary", "Selected geometry"),
            ("containment", "Tool inside boundary"),
            ("clearance", "15"),
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
            if self.strategy == Strategy::Drill && get(&t.p, "type") != "Drill" {
                e.push("Choose a drill for a drilling cycle.".into())
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
            if matches!(s, Strategy::Face | Strategy::Indexed | Strategy::Pocket) {
                bounded(&self.p, "stepover", 0.01, 100., &mut e)
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
                ("fixture", "Low profile clamps"),
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFile {
    pub name: String,
    pub path: String,
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
        let tools = vec![
            Tool::new("Flat end mill · Ø10", "Flat end mill", "10"),
            Tool::new("Ball end mill · Ø6", "Ball end mill", "6"),
            Tool::new("Twist drill · Ø8", "Drill", "8"),
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
            material: "Aluminum 6061".into(),
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
    pub fn remove_setup(&mut self, i: usize) -> Result<(), String> {
        if i >= self.setups.len() {
            return Err("Setup is missing.".into());
        }
        let old = self.setups.remove(i);
        for s in &mut self.setups {
            if get(&s.p, "stock_source") == old.id {
                s.p.insert("stock_source".into(), "fresh".into());
                s.p.insert(
                    "notes".into(),
                    "Stock source was removed. Recheck stock and workholding.".into(),
                );
            }
        }
        if self.active == old.id {
            self.active = self
                .setups
                .first()
                .map(|s| s.id.clone())
                .unwrap_or_default()
        }
        self.sync_stock_sources();
        Ok(())
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
        for setup in &self.setups {
            s += &format!(
                "\n## {} · {}\n\nMachine: {} (v{})\n\n{}\n\nOrigin: {}\n\nFixture: {}\n\nRotation: [{}, {}, {}]°\n\nTranslation: [{}, {}, {}] mm\n\nReposition: {}\n\n{}\n\n",
                setup.name(),
                get(&setup.p, "offset"),
                setup.machine.name(),
                setup.machine.version,
                crate::stock::sheet_description(setup, mesh),
                get(&setup.p, "origin"),
                get(&setup.p, "fixture"),
                get(&setup.p, "rx"),
                get(&setup.p, "ry"),
                get(&setup.p, "rz"),
                get(&setup.p, "tx"),
                get(&setup.p, "ty"),
                get(&setup.p, "tz"),
                get(&setup.p, "reposition"),
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
    fn machine_limit_checked() {
        let mut d = Document::sample();
        d.setup_mut().operations[0]
            .p
            .insert("rpm".into(), "99999".into());
        assert!(d.issues().iter().any(|s| s.contains("exceeds")))
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
}
