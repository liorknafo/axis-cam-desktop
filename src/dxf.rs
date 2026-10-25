//! A reader for flat DXF drawings of sheet parts, and a builder of the flat solid they describe.
//!
//! The reader takes the closed outlines on the DXF's cut layers: one outer outline and the
//! holes inside it, all in millimetres. The builder extrudes that profile into a solid of a
//! given thickness, with triangles that carry CAD face ids.

use std::collections::HashMap;
use std::f64::consts::{PI, TAU};

/// The largest gap in mm between two line or arc ends that the importer still joins.
pub const CHAIN_TOLERANCE: f64 = 0.01;

/// The largest distance in mm a chord may lie inside the arc it replaces.
pub const CHORD_TOLERANCE: f64 = 0.005;

/// Layer name parts that mark a layer as not holding cuts: bend lines, dimensions, notes and
/// the like. A layer whose upper-cased name contains one of them is skipped, as is `DEFPOINTS`.
pub const NON_CUT_LAYER_WORDS: [&str; 12] = [
    "BEND",
    "CENTER",
    "CENTRE",
    "DIM",
    "TEXT",
    "NOTE",
    "ANNOT",
    "HIDDEN",
    "CONSTRUCT",
    "TITLE",
    "BORDER",
    "ETCH",
];

/// The layer AutoCAD keeps its dimension definition points on.
const DEFPOINTS_LAYER: &str = "DEFPOINTS";

/// The entity types the importer refuses to guess at when they sit on a cut layer.
const UNSUPPORTED_TYPES: [&str; 10] = [
    "SPLINE", "ELLIPSE", "INSERT", "3DFACE", "REGION", "3DSOLID", "BODY", "MLINE", "RAY", "XLINE",
];

/// The first bytes of a binary DXF file.
const BINARY_MAGIC: &str = "AutoCAD Binary DXF";

/// The fewest chords a full circle is tessellated into.
const MIN_CIRCLE_CHORDS: f64 = 16.0;

/// The most chords a full circle is tessellated into.
const MAX_CIRCLE_CHORDS: f64 = 360.0;

/// A point in the drawing's plane, in mm.
pub type Point = [f64; 2];

/// One piece of an outline: a straight line or a circular arc.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    /// A straight line from one point to another.
    Line {
        /// Where the line starts.
        from: Point,
        /// Where the line ends.
        to: Point,
    },
    /// A circular arc. The sweep is signed: positive runs anticlockwise.
    Arc {
        /// The centre of the arc's circle.
        centre: Point,
        /// The radius of the arc's circle.
        radius: f64,
        /// The angle of the arc's first point, in radians.
        start: f64,
        /// The angle the arc covers, in radians, positive anticlockwise.
        sweep: f64,
    },
}

impl Segment {
    /// The point where the segment starts.
    pub fn start_point(&self) -> Point {
        match *self {
            Segment::Line { from, .. } => from,
            Segment::Arc {
                centre,
                radius,
                start,
                ..
            } => on_circle(centre, radius, start),
        }
    }

    /// The point where the segment ends.
    pub fn end_point(&self) -> Point {
        match *self {
            Segment::Line { to, .. } => to,
            Segment::Arc {
                centre,
                radius,
                start,
                sweep,
            } => on_circle(centre, radius, start + sweep),
        }
    }

    /// The segment's length in mm.
    #[cfg(test)]
    pub fn length(&self) -> f64 {
        match *self {
            Segment::Line { from, to } => dist(from, to),
            Segment::Arc { radius, sweep, .. } => radius * sweep.abs(),
        }
    }

    /// The same segment walked the other way round.
    fn reversed(&self) -> Segment {
        match *self {
            Segment::Line { from, to } => Segment::Line { from: to, to: from },
            Segment::Arc {
                centre,
                radius,
                start,
                sweep,
            } => Segment::Arc {
                centre,
                radius,
                start: start + sweep,
                sweep: -sweep,
            },
        }
    }

    /// The corners of the box that holds the segment.
    fn bounds(&self) -> (Point, Point) {
        let a = self.start_point();
        let b = self.end_point();
        let mut lo = [a[0].min(b[0]), a[1].min(b[1])];
        let mut hi = [a[0].max(b[0]), a[1].max(b[1])];
        if let Segment::Arc {
            centre,
            radius,
            start,
            sweep,
        } = *self
        {
            for k in 0..4 {
                let angle = f64::from(k) * PI / 2.0;
                let along = if sweep >= 0.0 {
                    (angle - start).rem_euclid(TAU)
                } else {
                    (start - angle).rem_euclid(TAU)
                };
                if along <= sweep.abs() {
                    let p = on_circle(centre, radius, angle);
                    lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                    hi = [hi[0].max(p[0]), hi[1].max(p[1])];
                }
            }
        }
        (lo, hi)
    }

    /// The segment's contribution to the signed area of its loop (Green's theorem).
    fn area_term(&self) -> f64 {
        match *self {
            Segment::Line { from, to } => 0.5 * (from[0] * to[1] - to[0] * from[1]),
            Segment::Arc {
                centre,
                radius,
                start,
                sweep,
            } => {
                let end = start + sweep;
                0.5 * (radius * centre[0] * (end.sin() - start.sin())
                    - radius * centre[1] * (end.cos() - start.cos())
                    + radius * radius * sweep)
            }
        }
    }
}

/// A closed outline made of segments, walked so that its material lies on the left.
#[derive(Clone, Debug)]
pub struct Loop {
    /// The name of the layer the outline was drawn on.
    pub layer: String,
    /// The segments in walking order; each starts where the one before it ends.
    pub segments: Vec<Segment>,
}

impl Loop {
    /// The outline as a polygon with no repeated closing point. Arc vertices lie exactly on the
    /// arc and the chords stay within `CHORD_TOLERANCE` of it.
    pub fn points(&self) -> Vec<Point> {
        self.tessellate().into_iter().map(|(p, _)| p).collect()
    }

    /// The outline's polygon with the index of the segment each vertex starts.
    fn tessellate(&self) -> Vec<(Point, usize)> {
        let mut out: Vec<(Point, usize)> = Vec::new();
        for (index, segment) in self.segments.iter().enumerate() {
            match *segment {
                Segment::Line { from, .. } => out.push((from, index)),
                Segment::Arc {
                    centre,
                    radius,
                    start,
                    sweep,
                } => {
                    let steps = arc_steps(radius, sweep);
                    for k in 0..steps {
                        let angle = start + sweep * k as f64 / steps as f64;
                        out.push((on_circle(centre, radius, angle), index));
                    }
                }
            }
        }
        out.dedup_by(|b, a| dist(a.0, b.0) < 1e-9);
        if out.len() > 1 && dist(out[0].0, out[out.len() - 1].0) < 1e-9 {
            out.pop();
        }
        out
    }

    /// The centre and radius when the outline is one full circle, otherwise `None`.
    pub fn circle(&self) -> Option<(Point, f64)> {
        let mut first: Option<(Point, f64, bool)> = None;
        let mut total = 0.0;
        for segment in &self.segments {
            let Segment::Arc {
                centre,
                radius,
                sweep,
                ..
            } = *segment
            else {
                return None;
            };
            match first {
                None => first = Some((centre, radius, sweep > 0.0)),
                Some((c0, r0, ccw)) => {
                    if dist(c0, centre) > CHAIN_TOLERANCE
                        || (r0 - radius).abs() > CHAIN_TOLERANCE
                        || ccw != (sweep > 0.0)
                    {
                        return None;
                    }
                }
            }
            total += sweep.abs();
        }
        let (centre, radius, _) = first?;
        let slack = CHAIN_TOLERANCE * self.segments.len() as f64 / radius.max(1e-9) + 1e-9;
        ((total - TAU).abs() <= slack).then_some((centre, radius))
    }

    /// The lowest and highest corners of the box that holds the outline.
    pub fn bounds(&self) -> (Point, Point) {
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for segment in &self.segments {
            let (a, b) = segment.bounds();
            lo = [lo[0].min(a[0]), lo[1].min(a[1])];
            hi = [hi[0].max(b[0]), hi[1].max(b[1])];
        }
        (lo, hi)
    }

    /// The area the outline encloses, in mm squared.
    pub fn area(&self) -> f64 {
        self.signed_area().abs()
    }

    /// The enclosed area, positive when the outline runs anticlockwise.
    fn signed_area(&self) -> f64 {
        self.segments.iter().map(Segment::area_term).sum()
    }

    /// The same outline walked the other way round.
    fn reversed(&self) -> Loop {
        Loop {
            layer: self.layer.clone(),
            segments: self.segments.iter().rev().map(Segment::reversed).collect(),
        }
    }

    /// The middle of the outline's bounding box.
    fn centre(&self) -> Point {
        let (lo, hi) = self.bounds();
        [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]
    }
}

/// The flat part a DXF describes: one outer outline and the holes inside it.
#[derive(Clone, Debug)]
pub struct Profile {
    /// The outer outline, anticlockwise.
    pub outer: Loop,
    /// The holes inside the outer outline, each clockwise.
    pub holes: Vec<Loop>,
    /// What the DXF's `$INSUNITS` said: `"mm"`, `"in"` and so on. The geometry is in mm anyway.
    pub units: &'static str,
    /// What the reader skipped, merged or assumed while reading.
    pub notes: Vec<String>,
}

impl Profile {
    /// A one-line description: the outline's size, the holes by size and the reader's notes.
    pub fn describe(&self) -> String {
        let (lo, hi) = self.outer.bounds();
        let mut text = format!("Outline {:.1} × {:.1} mm", hi[0] - lo[0], hi[1] - lo[1]);
        if self.holes.is_empty() {
            text.push_str(" with no holes");
        } else {
            let mut round: Vec<(i64, usize)> = Vec::new();
            let mut cutouts: Vec<((i64, i64), usize)> = Vec::new();
            for hole in &self.holes {
                if let Some((_, radius)) = hole.circle() {
                    let key = (radius * 200.0).round() as i64;
                    match round.iter_mut().find(|(k, _)| *k == key) {
                        Some(entry) => entry.1 += 1,
                        None => round.push((key, 1)),
                    }
                } else {
                    let (a, b) = hole.bounds();
                    let key = (
                        ((b[0] - a[0]) * 100.0).round() as i64,
                        ((b[1] - a[1]) * 100.0).round() as i64,
                    );
                    match cutouts.iter_mut().find(|(k, _)| *k == key) {
                        Some(entry) => entry.1 += 1,
                        None => cutouts.push((key, 1)),
                    }
                }
            }
            round.sort_by_key(|a| std::cmp::Reverse(a.0));
            let mut parts: Vec<String> = round
                .iter()
                .map(|(key, count)| format!("{count} × Ø{:.2}", *key as f64 / 100.0))
                .collect();
            for ((w, h), count) in &cutouts {
                let size = format!("{:.2} × {:.2} mm", *w as f64 / 100.0, *h as f64 / 100.0);
                parts.push(if *count == 1 {
                    format!("1 cut-out {size}")
                } else {
                    format!("{count} cut-outs {size}")
                });
            }
            let count = self.holes.len();
            text.push_str(&format!(
                " with {count} hole{}: ",
                if count == 1 { "" } else { "s" }
            ));
            match parts.split_last() {
                Some((last, rest)) if !rest.is_empty() => {
                    text.push_str(&format!("{} and {last}", rest.join(", ")));
                }
                Some((last, _)) => text.push_str(last),
                None => {}
            }
        }
        text.push('.');
        if self.units != "mm" {
            text.push_str(&format!(" Drawn in {}, read in mm.", self.units));
        }
        for note in &self.notes {
            text.push(' ');
            text.push_str(note);
        }
        text
    }
}

/// The extruded part: its triangles and the CAD face each belongs to.
#[derive(Clone, Debug)]
pub struct Flat {
    /// The triangles, wound anticlockwise seen from outside the solid.
    pub triangles: Vec<[[f32; 3]; 3]>,
    /// The CAD face id of each triangle: 1 top, 2 bottom, then one per profile segment.
    pub faces: Vec<u32>,
}

/// Reads DXF bytes (UTF-8 or Latin-1 text) and builds the solid of the given thickness in mm.
pub fn load(bytes: &[u8], thickness: f64) -> Result<Flat, String> {
    if bytes.starts_with(BINARY_MAGIC.as_bytes()) {
        return Err(binary_message());
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    };
    solid(&parse(&text)?, thickness)
}

/// The error for a binary DXF.
fn binary_message() -> String {
    "This is a binary DXF, which the importer does not read: save it as an ASCII DXF in your CAD."
        .to_string()
}

/// Reads ASCII DXF text into the flat profile on its cut layers.
pub fn parse(text: &str) -> Result<Profile, String> {
    if text
        .trim_start_matches('\u{feff}')
        .starts_with(BINARY_MAGIC)
    {
        return Err(binary_message());
    }
    let pairs = read_pairs(text)?;
    let sections = split_sections(&pairs);
    let section = |name: &str| sections.iter().find(|(n, _)| *n == name).map(|(_, s)| *s);

    let mut notes = Vec::new();
    let (units, scale) = read_units(section("HEADER"), &mut notes)?;
    let layers = read_layers(section("TABLES").unwrap_or(&[]));
    let entities = section("ENTITIES").ok_or("The DXF has no ENTITIES section.")?;

    let mut reader = Reader {
        scale,
        layers: &layers,
        closed: Vec::new(),
        soup: Vec::new(),
        skipped: Vec::new(),
        unsupported: Vec::new(),
    };
    reader.read_entities(entities)?;
    if let Some((kind, layer, count)) = reader.unsupported.first() {
        return Err(format!(
            "The DXF has {count} {kind} {} on layer {layer}, which the importer does not read: convert {} to polylines (or explode blocks) in your CAD.",
            if *count == 1 { "entity" } else { "entities" },
            if *count == 1 { "it" } else { "them" },
        ));
    }
    for skipped in &reader.skipped {
        notes.push(format!(
            "Skipped layer {} ({} {}): {}",
            skipped.layer,
            skipped.count,
            if skipped.count == 1 {
                "entity"
            } else {
                "entities"
            },
            skipped.reason
        ));
    }

    let mut loops: Vec<Loop> = reader
        .closed
        .into_iter()
        .map(|(layer, segments)| Loop { layer, segments })
        .collect();
    for (layer, segments) in chain(&reader.soup)? {
        loops.push(Loop { layer, segments });
    }
    if loops.is_empty() {
        let mut message = "No closed outlines were found on the DXF's cut layers.".to_string();
        if !reader.skipped.is_empty() {
            let names: Vec<&str> = reader.skipped.iter().map(|s| s.layer.as_str()).collect();
            message.push_str(&format!(" Skipped layers: {}.", names.join(", ")));
        }
        return Err(message);
    }

    let loops = merge_duplicates(loops, &mut notes);
    let (outer, holes) = arrange(loops)?;
    Ok(Profile {
        outer,
        holes,
        units,
        notes,
    })
}

/// The DXF's group-code and value pairs.
type Pair<'a> = (i32, &'a str);

/// Splits DXF text into its (group code, value) pairs, trimming both.
fn read_pairs(text: &str) -> Result<Vec<Pair<'_>>, String> {
    let mut out = Vec::new();
    let mut lines = text.trim_start_matches('\u{feff}').lines().enumerate();
    while let Some((number, code_line)) = lines.next() {
        let code_text = code_line.trim();
        if code_text.is_empty() {
            continue;
        }
        let code: i32 = code_text.parse().map_err(|_| {
            format!(
                "Line {}: expected a DXF group code, found '{code_text}'. Is this an ASCII DXF?",
                number + 1
            )
        })?;
        let Some((_, value)) = lines.next() else {
            break;
        };
        let value = value.trim();
        if code == 0 && value == "EOF" {
            break;
        }
        out.push((code, value));
    }
    Ok(out)
}

/// The named sections of a DXF with the pairs inside each.
fn split_sections<'a>(pairs: &'a [Pair<'a>]) -> Vec<(&'a str, &'a [Pair<'a>])> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < pairs.len() {
        if pairs[i] == (0, "SECTION") && i + 1 < pairs.len() && pairs[i + 1].0 == 2 {
            let name = pairs[i + 1].1;
            let start = i + 2;
            let mut end = start;
            while end < pairs.len() && pairs[end] != (0, "ENDSEC") {
                end += 1;
            }
            out.push((name, &pairs[start..end]));
            i = end + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Splits a run of pairs into records, each starting at a group code 0.
fn records<'a>(pairs: &'a [Pair<'a>]) -> Vec<&'a [Pair<'a>]> {
    let mut starts: Vec<usize> = pairs
        .iter()
        .enumerate()
        .filter(|(_, p)| p.0 == 0)
        .map(|(i, _)| i)
        .collect();
    starts.push(pairs.len());
    starts.windows(2).map(|w| &pairs[w[0]..w[1]]).collect()
}

/// The first number stored under a group code in a record.
fn get(record: &[Pair<'_>], code: i32) -> Option<f64> {
    record
        .iter()
        .find(|p| p.0 == code)
        .and_then(|p| p.1.parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

/// The first text stored under a group code in a record.
fn get_text<'a>(record: &[Pair<'a>], code: i32) -> Option<&'a str> {
    record.iter().find(|p| p.0 == code).map(|p| p.1)
}

/// Reads `$INSUNITS` from the header: the unit's name and its size in mm.
fn read_units(
    header: Option<&[Pair<'_>]>,
    notes: &mut Vec<String>,
) -> Result<(&'static str, f64), String> {
    let code = header.and_then(|pairs| {
        pairs
            .windows(2)
            .find(|w| w[0] == (9, "$INSUNITS") && w[1].0 == 70)
            .and_then(|w| w[1].1.parse::<i32>().ok())
    });
    match code {
        None => {
            notes.push("The DXF does not say its units: reading them as millimetres.".to_string());
            Ok(("mm", 1.0))
        }
        Some(0) => {
            notes.push("The DXF is unitless ($INSUNITS 0): reading it as millimetres.".to_string());
            Ok(("mm", 1.0))
        }
        Some(1) => Ok(("in", 25.4)),
        Some(2) => Ok(("ft", 304.8)),
        Some(4) => Ok(("mm", 1.0)),
        Some(5) => Ok(("cm", 10.0)),
        Some(6) => Ok(("m", 1000.0)),
        Some(8) => Ok(("µin", 2.54e-5)),
        Some(9) => Ok(("mil", 0.0254)),
        Some(10) => Ok(("yd", 914.4)),
        Some(13) => Ok(("µm", 0.001)),
        Some(14) => Ok(("dm", 100.0)),
        Some(other) => Err(format!(
            "The DXF uses units the importer does not know ($INSUNITS {other}): save it in millimetres or inches."
        )),
    }
}

/// Reads the LAYER table: the layers that are frozen or switched off, with the reason.
fn read_layers(tables: &[Pair<'_>]) -> HashMap<String, &'static str> {
    let mut hidden = HashMap::new();
    for record in records(tables) {
        if record[0].1 != "LAYER" {
            continue;
        }
        let Some(name) = get_text(record, 2) else {
            continue;
        };
        let flags = get(record, 70).unwrap_or(0.0) as i64;
        let colour = get(record, 62).unwrap_or(7.0);
        if flags & 1 != 0 {
            hidden.insert(name.to_uppercase(), "the layer is frozen.");
        } else if colour < 0.0 {
            hidden.insert(name.to_uppercase(), "the layer is off.");
        }
    }
    hidden
}

/// Why a layer holds no cuts judging by its name, if it does not.
fn non_cut_reason(name: &str) -> Option<&'static str> {
    let upper = name.to_uppercase();
    if upper.contains("BEND") {
        Some("bend lines are not cut.")
    } else if upper == DEFPOINTS_LAYER
        || NON_CUT_LAYER_WORDS.iter().any(|word| upper.contains(word))
    {
        Some("it holds annotations, not cuts.")
    } else {
        None
    }
}

/// A layer the reader skipped, with how many entities it held.
struct SkippedLayer {
    /// The layer's name as the first entity wrote it.
    layer: String,
    /// How many entities were skipped on it.
    count: usize,
    /// Why the layer was skipped.
    reason: &'static str,
}

/// A vertex of a polyline: its point and the bulge of the segment that leaves it.
#[derive(Clone, Copy)]
struct Vertex {
    /// Where the vertex is.
    p: Point,
    /// The bulge of the segment from this vertex to the next.
    bulge: f64,
}

/// The state while the ENTITIES section is read.
struct Reader<'a> {
    /// The factor from drawing units to mm.
    scale: f64,
    /// The frozen and off layers by upper-cased name, with the reason.
    layers: &'a HashMap<String, &'static str>,
    /// The closed polylines and circles, each one loop.
    closed: Vec<(String, Vec<Segment>)>,
    /// The loose lines, arcs and open polylines to chain.
    soup: Vec<(String, Segment)>,
    /// The layers skipped so far.
    skipped: Vec<SkippedLayer>,
    /// Unsupported entities on cut layers: type, layer and count.
    unsupported: Vec<(String, String, usize)>,
}

impl Reader<'_> {
    /// Reads every entity of the ENTITIES section.
    fn read_entities(&mut self, pairs: &[Pair<'_>]) -> Result<(), String> {
        let records = records(pairs);
        let mut i = 0;
        while i < records.len() {
            let record = records[i];
            i += 1;
            let kind = record[0].1;
            let mut vertices: Vec<&[Pair<'_>]> = Vec::new();
            if kind == "POLYLINE" {
                while i < records.len() {
                    let next = records[i][0].1;
                    if next == "VERTEX" {
                        vertices.push(records[i]);
                    } else if next != "SEQEND" {
                        break;
                    }
                    i += 1;
                    if next == "SEQEND" {
                        break;
                    }
                }
            }
            if matches!(kind, "VERTEX" | "SEQEND") || get(record, 67) == Some(1.0) {
                continue;
            }
            let layer = get_text(record, 8).unwrap_or("0");
            if let Some(reason) = self.skip_reason(layer) {
                self.count_skipped(layer, reason);
                continue;
            }
            self.entity(kind, layer, record, &vertices);
        }
        Ok(())
    }

    /// Why the entities of a layer are skipped, if they are.
    fn skip_reason(&self, layer: &str) -> Option<&'static str> {
        self.layers
            .get(&layer.to_uppercase())
            .copied()
            .or_else(|| non_cut_reason(layer))
    }

    /// Counts one entity on a skipped layer.
    fn count_skipped(&mut self, layer: &str, reason: &'static str) {
        let key = layer.to_uppercase();
        match self
            .skipped
            .iter_mut()
            .find(|s| s.layer.to_uppercase() == key)
        {
            Some(entry) => entry.count += 1,
            None => self.skipped.push(SkippedLayer {
                layer: layer.to_string(),
                count: 1,
                reason,
            }),
        }
    }

    /// Counts one entity the importer cannot read on a cut layer.
    fn count_unsupported(&mut self, kind: &str, layer: &str) {
        match self
            .unsupported
            .iter_mut()
            .find(|(k, l, _)| k == kind && l == layer)
        {
            Some(entry) => entry.2 += 1,
            None => self
                .unsupported
                .push((kind.to_string(), layer.to_string(), 1)),
        }
    }

    /// Reads one entity of a cut layer.
    fn entity(&mut self, kind: &str, layer: &str, record: &[Pair<'_>], vertices: &[&[Pair<'_>]]) {
        let scale = self.scale;
        let mirrored = get(record, 230).is_some_and(|z| z < 0.0);
        let tx = |x: f64| if mirrored { -x * scale } else { x * scale };
        let point = |cx: i32, cy: i32| -> Point {
            [
                tx(get(record, cx).unwrap_or(0.0)),
                get(record, cy).unwrap_or(0.0) * scale,
            ]
        };
        match kind {
            "LINE" => {
                let from = point(10, 20);
                let to = point(11, 21);
                if dist(from, to) > 1e-9 {
                    self.soup
                        .push((layer.to_string(), Segment::Line { from, to }));
                }
            }
            "ARC" => {
                let radius = get(record, 40).unwrap_or(0.0) * scale;
                if radius <= 0.0 {
                    return;
                }
                let mut a0 = get(record, 50).unwrap_or(0.0);
                let mut a1 = get(record, 51).unwrap_or(0.0);
                if mirrored {
                    (a0, a1) = (180.0 - a1, 180.0 - a0);
                }
                let mut sweep = (a1 - a0).rem_euclid(360.0);
                if sweep < 1e-9 {
                    sweep = 360.0;
                }
                self.soup.push((
                    layer.to_string(),
                    Segment::Arc {
                        centre: point(10, 20),
                        radius,
                        start: a0.to_radians(),
                        sweep: sweep.to_radians(),
                    },
                ));
            }
            "CIRCLE" => {
                let radius = get(record, 40).unwrap_or(0.0) * scale;
                if radius > 0.0 {
                    self.closed.push((
                        layer.to_string(),
                        vec![Segment::Arc {
                            centre: point(10, 20),
                            radius,
                            start: 0.0,
                            sweep: TAU,
                        }],
                    ));
                }
            }
            "LWPOLYLINE" => {
                let mut verts: Vec<Vertex> = Vec::new();
                for &(code, value) in record {
                    let number = value.parse::<f64>().unwrap_or(0.0);
                    match code {
                        10 => verts.push(Vertex {
                            p: [tx(number), 0.0],
                            bulge: 0.0,
                        }),
                        20 => {
                            if let Some(v) = verts.last_mut() {
                                v.p[1] = number * scale;
                            }
                        }
                        42 => {
                            if let Some(v) = verts.last_mut() {
                                v.bulge = if mirrored { -number } else { number };
                            }
                        }
                        _ => {}
                    }
                }
                let closed = get(record, 70).unwrap_or(0.0) as i64 & 1 != 0;
                self.add_polyline(layer, &verts, closed);
            }
            "POLYLINE" => {
                let flags = get(record, 70).unwrap_or(0.0) as i64;
                if flags & (8 | 16 | 64) != 0 {
                    self.count_unsupported("3D POLYLINE", layer);
                    return;
                }
                let verts: Vec<Vertex> = vertices
                    .iter()
                    .filter(|v| get(v, 70).unwrap_or(0.0) as i64 & 16 == 0)
                    .map(|v| {
                        let bulge = get(v, 42).unwrap_or(0.0);
                        Vertex {
                            p: [
                                tx(get(v, 10).unwrap_or(0.0)),
                                get(v, 20).unwrap_or(0.0) * scale,
                            ],
                            bulge: if mirrored { -bulge } else { bulge },
                        }
                    })
                    .collect();
                self.add_polyline(layer, &verts, flags & 1 != 0);
            }
            other if UNSUPPORTED_TYPES.contains(&other) => self.count_unsupported(other, layer),
            _ => {}
        }
    }

    /// Adds a polyline: a closed one is a loop, an open one goes into the soup.
    fn add_polyline(&mut self, layer: &str, verts: &[Vertex], closed: bool) {
        let n = verts.len();
        if n < 2 {
            return;
        }
        let count = if closed { n } else { n - 1 };
        let segments: Vec<Segment> = (0..count)
            .filter_map(|i| bulge_segment(verts[i].p, verts[(i + 1) % n].p, verts[i].bulge))
            .collect();
        if closed {
            if !segments.is_empty() {
                self.closed.push((layer.to_string(), segments));
            }
        } else {
            self.soup
                .extend(segments.into_iter().map(|s| (layer.to_string(), s)));
        }
    }
}

/// The segment from `p` to `q` with a polyline bulge, or `None` when the two points coincide.
/// A bulge is the tangent of a quarter of the included angle, positive anticlockwise.
fn bulge_segment(p: Point, q: Point, bulge: f64) -> Option<Segment> {
    let chord = dist(p, q);
    if chord < 1e-9 {
        return None;
    }
    if bulge.abs() < 1e-9 {
        return Some(Segment::Line { from: p, to: q });
    }
    let sweep = 4.0 * bulge.atan();
    let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
    let along = chord * (1.0 - bulge * bulge) / (4.0 * bulge);
    let unit = [(q[0] - p[0]) / chord, (q[1] - p[1]) / chord];
    let centre = [
        (p[0] + q[0]) / 2.0 - unit[1] * along,
        (p[1] + q[1]) / 2.0 + unit[0] * along,
    ];
    Some(Segment::Arc {
        centre,
        radius,
        start: (p[1] - centre[1]).atan2(p[0] - centre[0]),
        sweep,
    })
}

/// Chains loose segments into closed loops by joining ends within `CHAIN_TOLERANCE`.
fn chain(soup: &[(String, Segment)]) -> Result<Vec<(String, Vec<Segment>)>, String> {
    let n = soup.len();
    if n == 0 {
        return Ok(Vec::new());
    }
    // End number e belongs to segment e / 2; e & 1 says which end (0 start, 1 end).
    let pts: Vec<Point> = soup
        .iter()
        .flat_map(|(_, s)| [s.start_point(), s.end_point()])
        .collect();
    let mut order: Vec<usize> = (0..2 * n).collect();
    order.sort_by(|&a, &b| pts[a][0].total_cmp(&pts[b][0]));
    let reach = CHAIN_TOLERANCE * (1.0 + 1e-9);
    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); 2 * n];
    for (k, &a) in order.iter().enumerate() {
        for &b in &order[k + 1..] {
            if pts[b][0] - pts[a][0] > reach {
                break;
            }
            if a / 2 != b / 2 && dist(pts[a], pts[b]) <= reach {
                neighbours[a].push(b);
                neighbours[b].push(a);
            }
        }
    }
    for e in 0..2 * n {
        if neighbours[e].len() >= 2 {
            return Err(format!(
                "Three or more lines meet at {} on layer {}: the importer needs closed outlines that do not branch.",
                xy(pts[e]),
                soup[e / 2].0
            ));
        }
    }
    for e in 0..2 * n {
        if neighbours[e].is_empty() {
            let mut other = e ^ 1;
            while let Some(&next) = neighbours[other].first() {
                other = next ^ 1;
            }
            return Err(format!(
                "An outline on layer {} is open: its ends at {} and {} are {:.3} mm apart (the importer joins ends up to {} mm apart). Close it in your CAD.",
                soup[e / 2].0,
                xy(pts[e]),
                xy(pts[other]),
                dist(pts[e], pts[other]),
                CHAIN_TOLERANCE
            ));
        }
    }
    let mut done = vec![false; n];
    let mut loops = Vec::new();
    for first in 0..n {
        if done[first] {
            continue;
        }
        let mut segments = Vec::new();
        let mut e = 2 * first;
        loop {
            let s = e / 2;
            done[s] = true;
            segments.push(if e & 1 == 0 {
                soup[s].1
            } else {
                soup[s].1.reversed()
            });
            let partner = neighbours[e ^ 1][0];
            if partner / 2 == first || done[partner / 2] {
                break;
            }
            e = partner;
        }
        loops.push((soup[first].0.clone(), segments));
    }
    Ok(loops)
}

/// Merges loops that trace the same outline, noting each merge.
fn merge_duplicates(loops: Vec<Loop>, notes: &mut Vec<String>) -> Vec<Loop> {
    let mut kept: Vec<(Loop, Vec<Point>)> = Vec::new();
    for lp in loops {
        let poly = lp.points();
        if kept.iter().any(|(k, kp)| same_outline(k, kp, &lp, &poly)) {
            notes.push(format!(
                "Merged a duplicate outline on layer {} near {}.",
                lp.layer,
                xy(lp.centre())
            ));
        } else {
            kept.push((lp, poly));
        }
    }
    kept.into_iter().map(|(lp, _)| lp).collect()
}

/// Whether two loops trace the same outline, from any start and in either direction.
fn same_outline(a: &Loop, pa: &[Point], b: &Loop, pb: &[Point]) -> bool {
    if let (Some((c1, r1)), Some((c2, r2))) = (a.circle(), b.circle()) {
        return dist(c1, c2) <= CHAIN_TOLERANCE && (r1 - r2).abs() <= CHAIN_TOLERANCE;
    }
    let n = pa.len();
    if n != pb.len() || n == 0 {
        return false;
    }
    (0..n).any(|shift| {
        dist(pa[0], pb[shift]) <= CHAIN_TOLERANCE
            && [1, n - 1].iter().any(|&dir| {
                (0..n).all(|i| dist(pa[i], pb[(shift + i * dir) % n]) <= CHAIN_TOLERANCE)
            })
    })
}

/// Checks the loops against each other, picks the outer one and orients them all.
fn arrange(loops: Vec<Loop>) -> Result<(Loop, Vec<Loop>), String> {
    let polys: Vec<Vec<Point>> = loops.iter().map(Loop::points).collect();
    if let Some((a, b, at)) = find_crossing(&polys) {
        return Err(if a == b {
            format!(
                "The outline on layer {} crosses itself near {}: the importer needs outlines that do not cross.",
                loops[a].layer,
                xy(at)
            )
        } else {
            format!(
                "Two outlines cross near {}: the importer needs outlines that do not cross.",
                xy(at)
            )
        });
    }
    if let Some(lp) = loops.iter().find(|lp| lp.area() < 1e-6) {
        return Err(format!(
            "A closed outline on layer {} near {} has no area.",
            lp.layer,
            xy(lp.centre())
        ));
    }
    let depth: Vec<usize> = (0..loops.len())
        .map(|i| {
            (0..loops.len())
                .filter(|&j| j != i && inside(&polys[j], polys[i][0]))
                .count()
        })
        .collect();
    let roots: Vec<usize> = (0..loops.len()).filter(|&i| depth[i] == 0).collect();
    if roots.len() > 1 {
        let mut spots: Vec<String> = roots
            .iter()
            .take(4)
            .map(|&i| xy(loops[i].centre()))
            .collect();
        if roots.len() > 4 {
            spots.push("…".to_string());
        }
        return Err(format!(
            "The DXF holds {} separate outlines (near {}): import one part per file, its outline and the holes inside it.",
            roots.len(),
            spots.join(", ")
        ));
    }
    if let Some(i) = (0..loops.len()).find(|&i| depth[i] >= 2) {
        return Err(format!(
            "An outline near {} lies inside a hole: the importer reads one flat part with holes, not islands inside holes.",
            xy(loops[i].centre())
        ));
    }
    let mut outer = None;
    let mut holes = Vec::new();
    for (i, lp) in loops.into_iter().enumerate() {
        let ccw = lp.signed_area() > 0.0;
        if depth[i] == 0 {
            outer = Some(if ccw { lp } else { lp.reversed() });
        } else {
            holes.push(if ccw { lp.reversed() } else { lp });
        }
    }
    let outer = outer.ok_or("No closed outlines were found on the DXF's cut layers.")?;
    Ok((outer, holes))
}

/// Whether a point lies inside a polygon (ray casting).
fn inside(poly: &[Point], p: Point) -> bool {
    let mut hit = false;
    let n = poly.len();
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if x > p[0] {
                hit = !hit;
            }
        }
    }
    hit
}

/// One edge of a polygon, for the crossing search.
struct Edge {
    /// The edge's start.
    a: Point,
    /// The edge's end.
    b: Point,
    /// The polygon the edge belongs to.
    poly: usize,
    /// The edge's number in that polygon.
    index: usize,
}

/// Finds two polygon edges that cross or touch, other than neighbours in one polygon.
/// Returns the two polygon numbers (equal for a self-crossing) and the place.
fn find_crossing(polys: &[Vec<Point>]) -> Option<(usize, usize, Point)> {
    let mut edges: Vec<Edge> = Vec::new();
    for (poly, points) in polys.iter().enumerate() {
        let n = points.len();
        for index in 0..n {
            edges.push(Edge {
                a: points[index],
                b: points[(index + 1) % n],
                poly,
                index,
            });
        }
    }
    let slack = 1e-9;
    let min_x = |e: &Edge| e.a[0].min(e.b[0]);
    let max_x = |e: &Edge| e.a[0].max(e.b[0]);
    let mut order: Vec<usize> = (0..edges.len()).collect();
    order.sort_by(|&i, &j| min_x(&edges[i]).total_cmp(&min_x(&edges[j])));
    for (k, &i) in order.iter().enumerate() {
        let e = &edges[i];
        for &j in &order[k + 1..] {
            let f = &edges[j];
            if min_x(f) > max_x(e) + slack {
                break;
            }
            if e.a[1].min(e.b[1]) > f.a[1].max(f.b[1]) + slack
                || f.a[1].min(f.b[1]) > e.a[1].max(e.b[1]) + slack
            {
                continue;
            }
            if e.poly == f.poly {
                let n = polys[e.poly].len();
                let gap = e.index.abs_diff(f.index);
                if gap == 1 || gap == n - 1 {
                    continue;
                }
            }
            if let Some(at) = segments_meet(e.a, e.b, f.a, f.b) {
                return Some((e.poly, f.poly, at));
            }
        }
    }
    None
}

/// Where two segments cross or touch, if they do.
fn segments_meet(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let eps = 1e-9;
    let (la, lc) = (dist(a, b), dist(c, d));
    if la < eps || lc < eps {
        return None;
    }
    let side = |p: Point, q: Point, r: Point, len: f64| {
        let v = ((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])) / len;
        if v > eps {
            1
        } else if v < -eps {
            -1
        } else {
            0
        }
    };
    let (d1, d2) = (side(a, b, c, la), side(a, b, d, la));
    let (d3, d4) = (side(c, d, a, lc), side(c, d, b, lc));
    if d1 * d2 < 0 && d3 * d4 < 0 {
        let r = [b[0] - a[0], b[1] - a[1]];
        let s = [d[0] - c[0], d[1] - c[1]];
        let t = ((c[0] - a[0]) * s[1] - (c[1] - a[1]) * s[0]) / (r[0] * s[1] - r[1] * s[0]);
        return Some([a[0] + t * r[0], a[1] + t * r[1]]);
    }
    let in_box = |p: Point, q: Point, r: Point| {
        r[0] >= p[0].min(q[0]) - eps
            && r[0] <= p[0].max(q[0]) + eps
            && r[1] >= p[1].min(q[1]) - eps
            && r[1] <= p[1].max(q[1]) + eps
    };
    if d1 == 0 && in_box(a, b, c) {
        return Some(c);
    }
    if d2 == 0 && in_box(a, b, d) {
        return Some(d);
    }
    if d3 == 0 && in_box(c, d, a) {
        return Some(a);
    }
    if d4 == 0 && in_box(c, d, b) {
        return Some(b);
    }
    None
}

/// Builds the solid of a profile extruded `thickness` mm: bottom face at z = 0, top at z =
/// `thickness`, walls round every loop. Face ids: 1 top, 2 bottom, then one per segment, outer
/// loop first and then the holes.
pub fn solid(profile: &Profile, thickness: f64) -> Result<Flat, String> {
    if !(thickness.is_finite() && thickness > 0.0) {
        return Err(format!(
            "The thickness must be a number of mm above zero, not {thickness}."
        ));
    }
    let loops: Vec<&Loop> = std::iter::once(&profile.outer)
        .chain(profile.holes.iter())
        .collect();
    let tess: Vec<Vec<(Point, usize)>> = loops.iter().map(|l| l.tessellate()).collect();
    let polys: Vec<Vec<Point>> = tess
        .iter()
        .map(|t| t.iter().map(|(p, _)| *p).collect())
        .collect();
    let mut offsets = Vec::new();
    let mut flat_points: Vec<Point> = Vec::new();
    for poly in &polys {
        offsets.push(flat_points.len());
        flat_points.extend_from_slice(poly);
    }
    let top = |p: Point| [p[0] as f32, p[1] as f32, thickness as f32];
    let bottom = |p: Point| [p[0] as f32, p[1] as f32, 0.0];

    let mut triangles: Vec<[[f32; 3]; 3]> = Vec::new();
    let mut faces: Vec<u32> = Vec::new();
    for [a, b, c] in triangulate(&polys, &flat_points, &offsets) {
        let (pa, pb, pc) = (flat_points[a], flat_points[b], flat_points[c]);
        triangles.push([top(pa), top(pb), top(pc)]);
        faces.push(1);
        triangles.push([bottom(pa), bottom(pc), bottom(pb)]);
        faces.push(2);
    }
    let mut first_face = 3u32;
    for (lp, points) in loops.iter().zip(&tess) {
        let n = points.len();
        for (i, &(a, segment)) in points.iter().enumerate() {
            let b = points[(i + 1) % n].0;
            let id = first_face + segment as u32;
            triangles.push([bottom(a), bottom(b), top(b)]);
            faces.push(id);
            triangles.push([bottom(a), top(b), top(a)]);
            faces.push(id);
        }
        first_face += lp.segments.len() as u32;
    }
    Ok(Flat { triangles, faces })
}

/// Triangulates the face between the outer polygon (anticlockwise) and the holes (clockwise).
/// The triangles are anticlockwise and index `points`, which holds all the polygons in a row.
fn triangulate(polys: &[Vec<Point>], points: &[Point], offsets: &[usize]) -> Vec<[usize; 3]> {
    let mut ring: Vec<usize> = (offsets[0]..offsets[0] + polys[0].len()).collect();
    let mut holes: Vec<usize> = (1..polys.len()).filter(|&h| !polys[h].is_empty()).collect();
    let rightmost = |h: usize| {
        (0..polys[h].len())
            .max_by(|&i, &j| polys[h][i][0].total_cmp(&polys[h][j][0]))
            .unwrap_or(0)
    };
    holes.sort_by(|&a, &b| polys[b][rightmost(b)][0].total_cmp(&polys[a][rightmost(a)][0]));
    for h in holes {
        let n = polys[h].len();
        let start = rightmost(h);
        let hole_point = polys[h][start];
        let at = find_bridge(&ring, points, hole_point);
        let bridge = ring[at];
        let mut insert: Vec<usize> = (0..=n).map(|k| offsets[h] + (start + k) % n).collect();
        insert.push(bridge);
        ring.splice(at + 1..at + 1, insert);
    }
    ear_clip(&ring, points)
}

/// Finds where in the ring a hole's rightmost point should be joined: the position of a ring
/// vertex it can see.
fn find_bridge(ring: &[usize], points: &[Point], h: Point) -> usize {
    let m = ring.len();
    let mut best = f64::INFINITY;
    let mut hit = None;
    for k in 0..m {
        let a = points[ring[k]];
        let b = points[ring[(k + 1) % m]];
        if a[1] != b[1] && (a[1] <= h[1]) != (b[1] < h[1]) {
            let x = a[0] + (h[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if x >= h[0] && x < best {
                best = x;
                hit = Some(k);
            }
        }
    }
    let Some(k) = hit else {
        // No edge to the right: take the nearest vertex.
        return (0..m)
            .min_by(|&i, &j| dist(points[ring[i]], h).total_cmp(&dist(points[ring[j]], h)))
            .unwrap_or(0);
    };
    let (ka, kb) = (k, (k + 1) % m);
    let mut cand = if points[ring[ka]][0] > points[ring[kb]][0] {
        ka
    } else {
        kb
    };
    let corner = points[ring[cand]];
    let meet = [best, h[1]];
    // A vertex inside the triangle hole point, meeting point, corner blocks the view.
    let mut best_key = (f64::INFINITY, f64::INFINITY);
    let mut blocker = None;
    for (pos, &v) in ring.iter().enumerate() {
        let q = points[v];
        if q == corner || q[0] <= h[0] || !in_triangle(h, meet, corner, q) {
            continue;
        }
        let key = ((q[1] - h[1]).abs() / (q[0] - h[0]), dist(q, h));
        if key < best_key {
            best_key = key;
            blocker = Some(pos);
        }
    }
    if let Some(pos) = blocker {
        cand = pos;
    }
    // The vertex may appear more than once in the ring: use the visit that faces the hole.
    let m = ring.len();
    (0..m)
        .filter(|&pos| ring[pos] == ring[cand])
        .find(|&pos| {
            in_cone(
                points[ring[(pos + m - 1) % m]],
                points[ring[pos]],
                points[ring[(pos + 1) % m]],
                h,
            )
        })
        .unwrap_or(cand)
}

/// Whether the direction from `p` to `q` points into the polygon's inside at `p`, between the
/// edge to `next` and the edge from `prev` (anticlockwise polygon, inside on the left).
fn in_cone(prev: Point, p: Point, next: Point, q: Point) -> bool {
    let angle = |t: Point| (t[1] - p[1]).atan2(t[0] - p[0]);
    let a_next = angle(next);
    let mut span = (angle(prev) - a_next).rem_euclid(TAU);
    if span < 1e-12 {
        span = TAU;
    }
    let t = (angle(q) - a_next).rem_euclid(TAU);
    t > 1e-12 && t < span - 1e-12
}

/// Whether a point lies in a triangle, edges included, whichever way the triangle is wound.
fn in_triangle(a: Point, b: Point, c: Point, p: Point) -> bool {
    let d1 = cross(a, b, p);
    let d2 = cross(b, c, p);
    let d3 = cross(c, a, p);
    let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(negative && positive)
}

/// Twice the signed area of the triangle `a`, `b`, `c`: positive when it turns anticlockwise.
fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Cuts an anticlockwise ring (it may visit a vertex twice, along a bridge) into triangles by
/// ear clipping. Every ring vertex ends up used, so the face edges match the walls.
fn ear_clip(ring: &[usize], points: &[Point]) -> Vec<[usize; 3]> {
    let m = ring.len();
    let eps = 1e-12;
    let mut prev: Vec<usize> = (0..m).map(|i| (i + m - 1) % m).collect();
    let mut next: Vec<usize> = (0..m).map(|i| (i + 1) % m).collect();
    let at = |i: usize| points[ring[i]];
    let grid = Grid::new((0..m).map(at).collect());
    let mut alive_flags = vec![true; m];
    let mut triangles = Vec::new();
    let mut alive = m;
    let mut current = 0;
    let mut mode = 0;
    let mut misses = 0;
    while alive > 3 {
        let (p, n) = (prev[current], next[current]);
        let area = cross(at(p), at(current), at(n));
        let ear = match mode {
            0 => {
                area > eps && {
                    let (a, b, c) = (at(p), at(current), at(n));
                    let lo = [a[0].min(b[0]).min(c[0]), a[1].min(b[1]).min(c[1])];
                    let hi = [a[0].max(b[0]).max(c[0]), a[1].max(b[1]).max(c[1])];
                    !grid.any(lo, hi, |q| {
                        alive_flags[q] && {
                            let v = at(q);
                            v != a
                                && v != b
                                && v != c
                                && cross(at(prev[q]), v, at(next[q])) <= eps
                                && in_triangle(a, b, c, v)
                        }
                    })
                }
            }
            1 => area.abs() <= eps,
            _ => true,
        };
        let mut clip = current;
        if ear && mode == 2 {
            // Last resort: clip the most convex vertex.
            let mut best = f64::NEG_INFINITY;
            let mut q = current;
            for _ in 0..alive {
                let a = cross(at(prev[q]), at(q), at(next[q]));
                if a > best {
                    best = a;
                    clip = q;
                }
                q = next[q];
            }
        }
        if ear {
            let (p, n) = (prev[clip], next[clip]);
            triangles.push([ring[p], ring[clip], ring[n]]);
            next[p] = n;
            prev[n] = p;
            alive_flags[clip] = false;
            alive -= 1;
            current = n;
            misses = 0;
            mode = 0;
        } else {
            current = n;
            misses += 1;
            if misses > alive {
                mode += 1;
                misses = 0;
            }
        }
    }
    if alive == 3 {
        triangles.push([ring[prev[current]], ring[current], ring[next[current]]]);
    }
    triangles
}

/// A uniform grid over a set of points, to find the ones inside a box without a full scan.
struct Grid {
    /// The corner of the grid with the lowest coordinates.
    origin: Point,
    /// The side of a grid cell.
    cell: f64,
    /// How many cells the grid has across.
    nx: usize,
    /// How many cells the grid has up.
    ny: usize,
    /// The point numbers in each cell.
    cells: Vec<Vec<usize>>,
}

impl Grid {
    /// Sorts the points into cells, about one point to a cell.
    fn new(points: Vec<Point>) -> Grid {
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for p in &points {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-9);
        let cell = extent / (points.len() as f64).sqrt().max(1.0);
        let nx = ((hi[0] - lo[0]) / cell) as usize + 1;
        let ny = ((hi[1] - lo[1]) / cell) as usize + 1;
        let mut grid = Grid {
            origin: lo,
            cell,
            nx,
            ny,
            cells: vec![Vec::new(); nx * ny],
        };
        for (i, p) in points.iter().enumerate() {
            let (cx, cy) = (grid.column(p[0]), grid.row(p[1]));
            grid.cells[cy * nx + cx].push(i);
        }
        grid
    }

    /// The column a coordinate falls in.
    fn column(&self, x: f64) -> usize {
        (((x - self.origin[0]) / self.cell).max(0.0) as usize).min(self.nx - 1)
    }

    /// The row a coordinate falls in.
    fn row(&self, y: f64) -> usize {
        (((y - self.origin[1]) / self.cell).max(0.0) as usize).min(self.ny - 1)
    }

    /// Whether `test` holds for any point in a cell touching the box.
    fn any(&self, lo: Point, hi: Point, mut test: impl FnMut(usize) -> bool) -> bool {
        for cy in self.row(lo[1])..=self.row(hi[1]) {
            for cx in self.column(lo[0])..=self.column(hi[0]) {
                if self.cells[cy * self.nx + cx].iter().any(|&i| test(i)) {
                    return true;
                }
            }
        }
        false
    }
}

/// The number of chords an arc is cut into.
fn arc_steps(radius: f64, sweep: f64) -> usize {
    let widest = TAU / MIN_CIRCLE_CHORDS;
    let narrowest = TAU / MAX_CIRCLE_CHORDS;
    let step = if radius > CHORD_TOLERANCE {
        2.0 * (1.0 - CHORD_TOLERANCE / radius).acos()
    } else {
        widest
    };
    let step = step.clamp(narrowest, widest);
    ((sweep.abs() / step).ceil() as usize).max(1)
}

/// The point at an angle on a circle.
fn on_circle(centre: Point, radius: f64, angle: f64) -> Point {
    [
        centre[0] + radius * angle.cos(),
        centre[1] + radius * angle.sin(),
    ]
}

/// The distance between two points.
fn dist(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// A point written as `X12.000 Y3.000`.
fn xy(p: Point) -> String {
    format!("X{:.3} Y{:.3}", p[0], p[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fmt::Display;

    /// One code/value pair as DXF text.
    fn g(code: i32, value: impl Display) -> String {
        format!("{code:>3}\n{value}\n")
    }

    /// A whole DXF with the given units, layer table rows (name, flags, colour) and entities.
    fn file(units: Option<i32>, layers: &[(&str, i32, i32)], entities: &str) -> String {
        let mut s = String::new();
        s += &(g(0, "SECTION") + &g(2, "HEADER"));
        if let Some(u) = units {
            s += &(g(9, "$INSUNITS") + &g(70, u));
        }
        s += &g(0, "ENDSEC");
        s += &(g(0, "SECTION") + &g(2, "TABLES") + &g(0, "TABLE") + &g(2, "LAYER"));
        for (name, flags, colour) in layers {
            s += &(g(0, "LAYER") + &g(2, name) + &g(70, flags) + &g(62, colour));
        }
        s += &(g(0, "ENDTAB") + &g(0, "ENDSEC"));
        s += &(g(0, "SECTION") + &g(2, "ENTITIES"));
        s += entities;
        s += &(g(0, "ENDSEC") + &g(0, "EOF"));
        s
    }

    fn mm(entities: &str) -> String {
        file(Some(4), &[], entities)
    }

    fn line(layer: &str, a: Point, b: Point) -> String {
        g(0, "LINE") + &g(8, layer) + &g(10, a[0]) + &g(20, a[1]) + &g(11, b[0]) + &g(21, b[1])
    }

    fn arc(layer: &str, c: Point, r: f64, a0: f64, a1: f64) -> String {
        g(0, "ARC")
            + &g(8, layer)
            + &g(10, c[0])
            + &g(20, c[1])
            + &g(40, r)
            + &g(50, a0)
            + &g(51, a1)
    }

    fn circle(layer: &str, c: Point, r: f64) -> String {
        g(0, "CIRCLE") + &g(8, layer) + &g(10, c[0]) + &g(20, c[1]) + &g(40, r)
    }

    fn lwpoly(layer: &str, closed: bool, verts: &[(f64, f64, f64)]) -> String {
        let mut s =
            g(0, "LWPOLYLINE") + &g(8, layer) + &g(90, verts.len()) + &g(70, i32::from(closed));
        for &(x, y, b) in verts {
            s += &(g(10, x) + &g(20, y));
            if b != 0.0 {
                s += &g(42, b);
            }
        }
        s
    }

    fn rect(layer: &str, x0: f64, y0: f64, x1: f64, y1: f64) -> String {
        lwpoly(
            layer,
            true,
            &[(x0, y0, 0.0), (x1, y0, 0.0), (x1, y1, 0.0), (x0, y1, 0.0)],
        )
    }

    fn near(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    fn size(lp: &Loop) -> (f64, f64) {
        let (lo, hi) = lp.bounds();
        (hi[0] - lo[0], hi[1] - lo[1])
    }

    fn square_lines(layer: &str, side: f64) -> String {
        let p = [[0.0, 0.0], [side, 0.0], [side, side], [0.0, side]];
        (0..4)
            .map(|i| line(layer, p[i], p[(i + 1) % 4]))
            .collect::<String>()
    }

    #[test]
    fn lines_chain_into_a_square() {
        let p = parse(&mm(&square_lines("CUT", 10.0))).unwrap();
        assert_eq!(p.outer.segments.len(), 4);
        assert!(p.holes.is_empty());
        assert_eq!(p.units, "mm");
        assert!(near(p.outer.area(), 100.0, 1e-9));
        assert!(p.outer.signed_area() > 0.0);
        assert_eq!(p.outer.layer, "CUT");
    }

    #[test]
    fn lines_given_out_of_order_and_reversed_still_chain() {
        let e = line("CUT", [10.0, 10.0], [10.0, 0.0])
            + &line("CUT", [0.0, 0.0], [0.0, 10.0])
            + &line("CUT", [10.0, 0.0], [0.0, 0.0])
            + &line("CUT", [0.0, 10.0], [10.0, 10.0]);
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(p.outer.segments.len(), 4);
        assert!(near(p.outer.area(), 100.0, 1e-9));
        assert!(p.outer.signed_area() > 0.0);
    }

    #[test]
    fn arcs_and_lines_chain_into_a_slot() {
        // An obround 20 long and 10 wide: two semicircles of radius 5 and two lines.
        let e = line("CUT", [5.0, 0.0], [15.0, 0.0])
            + &arc("CUT", [15.0, 5.0], 5.0, 270.0, 90.0)
            + &line("CUT", [15.0, 10.0], [5.0, 10.0])
            + &arc("CUT", [5.0, 5.0], 5.0, 90.0, 270.0);
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(p.outer.segments.len(), 4);
        assert!(p.outer.circle().is_none());
        let (lo, hi) = p.outer.bounds();
        assert!(near(lo[0], 0.0, 1e-9) && near(hi[0], 20.0, 1e-9));
        assert!(near(lo[1], 0.0, 1e-9) && near(hi[1], 10.0, 1e-9));
        let area = 10.0 * 10.0 + PI * 25.0;
        assert!(near(p.outer.area(), area, 1e-6));
    }

    #[test]
    fn a_circle_is_one_loop() {
        let p = parse(&mm(&circle("CUT", [3.0, 4.0], 5.0))).unwrap();
        assert_eq!(p.outer.segments.len(), 1);
        let (c, r) = p.outer.circle().unwrap();
        assert!(near(c[0], 3.0, 1e-9) && near(c[1], 4.0, 1e-9) && near(r, 5.0, 1e-9));
        assert!(near(p.outer.area(), PI * 25.0, 1e-9));
        let (lo, hi) = p.outer.bounds();
        assert!(near(lo[0], -2.0, 1e-9) && near(hi[1], 9.0, 1e-9));
    }

    #[test]
    fn two_semicircle_arcs_make_a_circle() {
        let e = arc("CUT", [0.0, 0.0], 5.0, 0.0, 180.0) + &arc("CUT", [0.0, 0.0], 5.0, 180.0, 0.0);
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(p.outer.segments.len(), 2);
        let (c, r) = p.outer.circle().unwrap();
        assert!(near(c[0], 0.0, 1e-9) && near(r, 5.0, 1e-9));
    }

    #[test]
    fn closed_lwpolyline_is_one_loop() {
        let p = parse(&mm(&rect("CUT", 0.0, 0.0, 30.0, 20.0))).unwrap();
        assert_eq!(p.outer.segments.len(), 4);
        assert_eq!(size(&p.outer), (30.0, 20.0));
        assert!(near(p.outer.area(), 600.0, 1e-9));
    }

    #[test]
    fn a_clockwise_outline_is_stored_anticlockwise() {
        let e = lwpoly(
            "CUT",
            true,
            &[
                (0.0, 0.0, 0.0),
                (0.0, 10.0, 0.0),
                (10.0, 10.0, 0.0),
                (10.0, 0.0, 0.0),
            ],
        ) + &lwpoly(
            "CUT",
            true,
            &[
                (2.0, 2.0, 0.0),
                (4.0, 2.0, 0.0),
                (4.0, 4.0, 0.0),
                (2.0, 4.0, 0.0),
            ],
        );
        let p = parse(&mm(&e)).unwrap();
        assert!(p.outer.signed_area() > 0.0);
        assert!(p.holes[0].signed_area() < 0.0);
    }

    #[test]
    fn a_closing_vertex_equal_to_the_first_adds_no_segment() {
        let e = lwpoly(
            "CUT",
            true,
            &[
                (0.0, 0.0, 0.0),
                (10.0, 0.0, 0.0),
                (10.0, 10.0, 0.0),
                (0.0, 10.0, 0.0),
                (0.0, 0.0, 0.0),
            ],
        );
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(p.outer.segments.len(), 4);
        assert!(p.outer.segments.iter().all(|s| s.length() > 1.0));
    }

    #[test]
    fn open_lwpolyline_whose_ends_meet_is_a_loop() {
        let e = lwpoly(
            "CUT",
            false,
            &[
                (0.0, 0.0, 0.0),
                (10.0, 0.0, 0.0),
                (10.0, 10.0, 0.0),
                (0.0, 10.0, 0.0),
                (0.0, 0.0, 0.0),
            ],
        );
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(p.outer.segments.len(), 4);
    }

    #[test]
    fn r12_polyline_with_vertices_and_seqend() {
        let mut e = g(0, "POLYLINE") + &g(8, "CUT") + &g(66, 1) + &g(70, 1);
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 5.0), (0.0, 5.0)] {
            e += &(g(0, "VERTEX") + &g(8, "CUT") + &g(10, x) + &g(20, y));
        }
        e += &(g(0, "SEQEND") + &g(8, "CUT"));
        e += &circle("CUT", [5.0, 2.5], 1.0);
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(p.outer.segments.len(), 4);
        assert_eq!(size(&p.outer), (10.0, 5.0));
        assert_eq!(p.holes.len(), 1);
    }

    #[test]
    fn r12_polyline_with_a_bulge() {
        let mut e = g(0, "POLYLINE") + &g(8, "CUT") + &g(66, 1) + &g(70, 1);
        for (x, y, b) in [(0.0, 0.0, 1.0), (2.0, 0.0, 1.0)] {
            e += &(g(0, "VERTEX") + &g(8, "CUT") + &g(10, x) + &g(20, y) + &g(42, b));
        }
        e += &g(0, "SEQEND");
        let p = parse(&mm(&e)).unwrap();
        let (c, r) = p.outer.circle().unwrap();
        assert!(near(c[0], 1.0, 1e-9) && near(c[1], 0.0, 1e-9) && near(r, 1.0, 1e-9));
    }

    #[test]
    fn a_3d_polyline_on_a_cut_layer_is_an_error() {
        let e = g(0, "POLYLINE") + &g(8, "CUT") + &g(70, 8) + &g(0, "SEQEND");
        let err = parse(&mm(&e)).unwrap_err();
        assert!(err.contains("3D POLYLINE") && err.contains("CUT"), "{err}");
    }

    #[test]
    fn positive_bulge_goes_anticlockwise_and_negative_clockwise() {
        let Some(Segment::Arc {
            centre,
            radius,
            start,
            sweep,
        }) = bulge_segment([0.0, 0.0], [2.0, 0.0], 1.0)
        else {
            panic!("expected an arc");
        };
        assert!(near(centre[0], 1.0, 1e-12) && near(centre[1], 0.0, 1e-12));
        assert!(near(radius, 1.0, 1e-12));
        assert!(near(sweep, PI, 1e-12));
        let mid = on_circle(centre, radius, start + sweep / 2.0);
        assert!(
            near(mid[0], 1.0, 1e-9) && near(mid[1], -1.0, 1e-9),
            "{mid:?}"
        );
        let Some(Segment::Arc {
            centre,
            radius,
            start,
            sweep,
        }) = bulge_segment([0.0, 0.0], [2.0, 0.0], -1.0)
        else {
            panic!("expected an arc");
        };
        assert!(sweep < 0.0);
        let mid = on_circle(centre, radius, start + sweep / 2.0);
        assert!(
            near(mid[0], 1.0, 1e-9) && near(mid[1], 1.0, 1e-9),
            "{mid:?}"
        );
        // A shallow bulge: the arc is smaller than a semicircle with the centre on the far side.
        let Some(Segment::Arc { centre, radius, .. }) = bulge_segment([0.0, 0.0], [2.0, 0.0], 0.5)
        else {
            panic!("expected an arc");
        };
        assert!(centre[1] > 0.0 && radius > 1.0);
        assert!(near(dist(centre, [0.0, 0.0]), radius, 1e-9));
        // A zero bulge is a straight line.
        assert!(matches!(
            bulge_segment([0.0, 0.0], [2.0, 0.0], 0.0),
            Some(Segment::Line { .. })
        ));
    }

    #[test]
    fn a_bulge_one_polyline_is_a_circle_centred_between_its_vertices() {
        let e = lwpoly("CUT", true, &[(0.0, 0.0, 1.0), (2.0, 0.0, 1.0)]);
        let p = parse(&mm(&e)).unwrap();
        let (c, r) = p.outer.circle().unwrap();
        assert!(near(c[0], 1.0, 1e-9) && near(c[1], 0.0, 1e-9) && near(r, 1.0, 1e-9));
        assert!(near(p.outer.area(), PI, 1e-9));
        let e = lwpoly("CUT", true, &[(0.0, 0.0, -1.0), (2.0, 0.0, -1.0)]);
        let p = parse(&mm(&e)).unwrap();
        assert!(p.outer.circle().is_some());
        assert!(p.outer.signed_area() > 0.0);
    }

    #[test]
    fn the_last_bulge_of_a_closed_polyline_is_the_closing_segment() {
        // A D shape: a line along the bottom, then the closing arc (bulge on the last vertex).
        let e = lwpoly("CUT", true, &[(0.0, 0.0, 0.0), (2.0, 0.0, 1.0)]);
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(p.outer.segments.len(), 2);
        assert!(near(p.outer.area(), PI / 2.0, 1e-9));
    }

    #[test]
    fn units_scale_the_geometry() {
        let e = rect("CUT", 0.0, 0.0, 1.0, 1.0);
        let p = parse(&file(Some(1), &[], &e)).unwrap();
        assert_eq!(p.units, "in");
        assert!(near(size(&p.outer).0, 25.4, 1e-9) && near(size(&p.outer).1, 25.4, 1e-9));
        assert!(p.notes.is_empty());
        let p = parse(&file(Some(5), &[], &e)).unwrap();
        assert_eq!(p.units, "cm");
        assert!(near(size(&p.outer).0, 10.0, 1e-9));
        let p = parse(&file(Some(6), &[], &circle("CUT", [0.0, 0.0], 0.1))).unwrap();
        assert!(near(p.outer.circle().unwrap().1, 100.0, 1e-9));
        let p = parse(&file(Some(13), &[], &e)).unwrap();
        assert!(near(size(&p.outer).0, 0.001, 1e-12));
    }

    #[test]
    fn missing_or_zero_units_read_as_mm_with_a_note() {
        let e = rect("CUT", 0.0, 0.0, 5.0, 5.0);
        let p = parse(&file(None, &[], &e)).unwrap();
        assert_eq!(p.units, "mm");
        assert!(near(size(&p.outer).0, 5.0, 1e-9));
        assert!(p.notes.iter().any(|n| n.contains("units")), "{:?}", p.notes);
        let p = parse(&file(Some(0), &[], &e)).unwrap();
        assert_eq!(p.units, "mm");
        assert!(
            p.notes.iter().any(|n| n.contains("unitless")),
            "{:?}",
            p.notes
        );
    }

    #[test]
    fn unknown_units_are_an_error_naming_the_code() {
        let err = parse(&file(Some(3), &[], &rect("CUT", 0.0, 0.0, 5.0, 5.0))).unwrap_err();
        assert!(err.contains("$INSUNITS 3"), "{err}");
    }

    #[test]
    fn ends_within_the_tolerance_join() {
        let e = line("CUT", [0.0, 0.0], [10.0, 0.0])
            + &line("CUT", [10.005, 0.0], [10.0, 10.0])
            + &line("CUT", [10.0, 10.0], [0.0, 10.0])
            + &line("CUT", [0.0, 10.0], [0.0, 0.0]);
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(p.outer.segments.len(), 4);
    }

    #[test]
    fn an_open_outline_names_both_ends() {
        let e = line("CUT", [0.0, 0.0], [10.0, 0.0])
            + &line("CUT", [10.0, 0.0], [10.0, 10.0])
            + &line("CUT", [10.0, 10.0], [0.0, 10.0])
            + &line("CUT", [0.0, 10.0], [0.0, 0.5]);
        let err = parse(&mm(&e)).unwrap_err();
        assert!(
            err.contains("An outline on layer CUT is open")
                && err.contains("X0.000 Y0.000")
                && err.contains("X0.000 Y0.500")
                && err.contains("0.500 mm apart")
                && err.contains("0.01 mm"),
            "{err}"
        );
    }

    #[test]
    fn three_ends_meeting_is_a_branch_error() {
        let e = square_lines("CUT", 10.0) + &line("CUT", [10.0, 10.0], [20.0, 20.0]);
        let err = parse(&mm(&e)).unwrap_err();
        assert!(
            err.contains("Three or more lines meet at X10.000 Y10.000 on layer CUT"),
            "{err}"
        );
    }

    #[test]
    fn a_hole_is_found_whatever_the_entity_order() {
        let outer = rect("CUT", 0.0, 0.0, 50.0, 40.0);
        let hole = circle("CUT", [25.0, 20.0], 5.0);
        for e in [outer.clone() + &hole, hole.clone() + &outer] {
            let p = parse(&mm(&e)).unwrap();
            assert_eq!(size(&p.outer), (50.0, 40.0));
            assert_eq!(p.holes.len(), 1);
            assert!(p.holes[0].circle().is_some());
            assert!(p.holes[0].signed_area() < 0.0);
        }
    }

    #[test]
    fn an_island_inside_a_hole_is_an_error() {
        let e = rect("CUT", 0.0, 0.0, 100.0, 100.0)
            + &circle("CUT", [50.0, 50.0], 30.0)
            + &circle("CUT", [50.0, 50.0], 10.0);
        let err = parse(&mm(&e)).unwrap_err();
        assert!(err.contains("lies inside a hole"), "{err}");
        assert!(err.contains("X50.000 Y50.000"), "{err}");
    }

    #[test]
    fn two_separate_outlines_are_an_error() {
        let e = rect("CUT", 0.0, 0.0, 10.0, 10.0)
            + &rect("CUT", 50.0, 0.0, 60.0, 10.0)
            + &rect("CUT", 100.0, 0.0, 110.0, 10.0);
        let err = parse(&mm(&e)).unwrap_err();
        assert!(
            err.contains("3 separate outlines")
                && err.contains("X5.000 Y5.000")
                && err.contains("one part per file"),
            "{err}"
        );
    }

    #[test]
    fn no_loops_is_an_error_mentioning_skipped_layers() {
        let e = line("BEND", [0.0, 0.0], [10.0, 0.0]);
        let err = parse(&mm(&e)).unwrap_err();
        assert!(err.contains("No closed outlines"), "{err}");
        assert!(err.contains("BEND"), "{err}");
        let err = parse(&mm("")).unwrap_err();
        assert!(err.contains("No closed outlines"), "{err}");
    }

    #[test]
    fn bend_frozen_and_off_layers_are_ignored_with_notes() {
        let e = rect("CUT", 0.0, 0.0, 50.0, 40.0)
            + &line("BEND", [0.0, 20.0], [50.0, 20.0])
            + &line("bend lines", [0.0, 10.0], [50.0, 10.0])
            + &circle("OLD", [10.0, 10.0], 3.0)
            + &circle("HIDE", [30.0, 10.0], 3.0)
            + &circle("CUT", [25.0, 30.0], 2.0);
        let layers = [
            ("CUT", 0, 7),
            ("OLD", 1, 7),
            ("HIDE", 0, -7),
            ("BEND", 0, 7),
        ];
        let p = parse(&file(Some(4), &layers, &e)).unwrap();
        assert_eq!(p.holes.len(), 1);
        assert!(
            p.notes
                .contains(&"Skipped layer BEND (1 entity): bend lines are not cut.".to_string())
        );
        assert!(
            p.notes
                .iter()
                .any(|n| n.starts_with("Skipped layer OLD (1 entity)") && n.contains("frozen"))
        );
        assert!(
            p.notes
                .iter()
                .any(|n| n.starts_with("Skipped layer HIDE (1 entity)") && n.contains("off"))
        );
        assert!(
            p.notes
                .iter()
                .any(|n| n.starts_with("Skipped layer bend lines"))
        );
    }

    #[test]
    fn annotation_layers_and_defpoints_are_skipped() {
        let mut e = rect("CUT", 0.0, 0.0, 50.0, 40.0);
        for name in [
            "Dimensions",
            "centre-lines",
            "NOTES",
            "Title block",
            "defpoints",
            "Hidden",
            "Construction",
            "Etch",
            "BORDER",
            "ANNOTATION",
        ] {
            e += &line(name, [1.0, 1.0], [2.0, 5.0]);
        }
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(p.notes.len(), 10, "{:?}", p.notes);
        assert!(p.holes.is_empty());
    }

    #[test]
    fn paper_space_entities_are_ignored() {
        let e = rect("CUT", 0.0, 0.0, 10.0, 10.0) + &(circle("CUT", [5.0, 5.0], 2.0) + &g(67, 1));
        let p = parse(&mm(&e)).unwrap();
        assert!(p.holes.is_empty());
    }

    #[test]
    fn text_and_dimensions_are_ignored() {
        let e = rect("CUT", 0.0, 0.0, 10.0, 10.0)
            + &(g(0, "TEXT") + &g(8, "CUT") + &g(1, "hello"))
            + &(g(0, "MTEXT") + &g(8, "CUT") + &g(1, "hello"))
            + &(g(0, "DIMENSION") + &g(8, "CUT"))
            + &(g(0, "HATCH") + &g(8, "CUT"))
            + &(g(0, "POINT") + &g(8, "CUT"));
        assert!(parse(&mm(&e)).is_ok());
    }

    #[test]
    fn self_crossing_bow_tie_is_an_error() {
        let e = lwpoly(
            "CUT",
            true,
            &[
                (0.0, 0.0, 0.0),
                (10.0, 10.0, 0.0),
                (10.0, 0.0, 0.0),
                (0.0, 10.0, 0.0),
            ],
        );
        let err = parse(&mm(&e)).unwrap_err();
        assert!(
            err.contains("The outline on layer CUT crosses itself near X5.000 Y5.000"),
            "{err}"
        );
    }

    #[test]
    fn two_crossing_circles_are_an_error() {
        let e = circle("CUT", [0.0, 0.0], 5.0) + &circle("CUT", [6.0, 0.0], 5.0);
        let err = parse(&mm(&e)).unwrap_err();
        assert!(err.starts_with("Two outlines cross near X"), "{err}");
    }

    #[test]
    fn a_duplicate_circle_is_merged_with_a_note() {
        let e = rect("CUT", 0.0, 0.0, 50.0, 40.0)
            + &circle("CUT", [25.0, 20.0], 5.0)
            + &circle("CUT", [25.0, 20.0], 5.0);
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(p.holes.len(), 1);
        assert!(
            p.notes.iter().any(|n| n.contains("duplicate")),
            "{:?}",
            p.notes
        );
    }

    #[test]
    fn a_duplicate_polyline_drawn_backwards_is_merged() {
        let e = rect("CUT", 0.0, 0.0, 50.0, 40.0)
            + &lwpoly(
                "CUT",
                true,
                &[
                    (50.0, 40.0, 0.0),
                    (50.0, 0.0, 0.0),
                    (0.0, 0.0, 0.0),
                    (0.0, 40.0, 0.0),
                ],
            );
        let p = parse(&mm(&e)).unwrap();
        assert!(p.holes.is_empty());
        assert_eq!(p.notes.len(), 1);
    }

    #[test]
    fn unsupported_entities_on_a_cut_layer_are_an_error() {
        let e = rect("CUT", 0.0, 0.0, 10.0, 10.0)
            + &(g(0, "SPLINE") + &g(8, "CUT"))
            + &(g(0, "SPLINE") + &g(8, "CUT"));
        let err = parse(&mm(&e)).unwrap_err();
        assert!(
            err.starts_with("The DXF has 2 SPLINE entities on layer CUT, which the importer does not read: convert them to polylines (or explode blocks) in your CAD."),
            "{err}"
        );
        for kind in [
            "ELLIPSE", "INSERT", "3DFACE", "REGION", "3DSOLID", "BODY", "MLINE", "RAY", "XLINE",
        ] {
            let e = rect("CUT", 0.0, 0.0, 10.0, 10.0) + &(g(0, kind) + &g(8, "CUT"));
            let err = parse(&mm(&e)).unwrap_err();
            assert!(err.contains(kind) && err.contains("1"), "{err}");
        }
    }

    #[test]
    fn unsupported_entities_on_bend_are_ignored() {
        let e = rect("CUT", 0.0, 0.0, 10.0, 10.0) + &(g(0, "SPLINE") + &g(8, "BEND"));
        let p = parse(&mm(&e)).unwrap();
        assert!(p.notes.iter().any(|n| n.contains("BEND")));
    }

    #[test]
    fn a_negative_extrusion_mirrors_x() {
        let flip = g(210, 0) + &g(220, 0) + &g(230, -1);
        let e = rect("CUT", -50.0, 0.0, 0.0, 40.0) + &(circle("CUT", [10.0, 20.0], 3.0) + &flip);
        let p = parse(&mm(&e)).unwrap();
        let (c, _) = p.holes[0].circle().unwrap();
        assert!(near(c[0], -10.0, 1e-9) && near(c[1], 20.0, 1e-9));
        // A mirrored polyline with a bulge: the arc stays on the same side after mirroring.
        let e = lwpoly("CUT", true, &[(0.0, 0.0, 1.0), (2.0, 0.0, 1.0)]) + &flip;
        let p = parse(&mm(&e)).unwrap();
        let (c, r) = p.outer.circle().unwrap();
        assert!(near(c[0], -1.0, 1e-9) && near(r, 1.0, 1e-9));
        // A mirrored arc: a quarter arc from 0 to 90 degrees becomes one from 90 to 180.
        let e = arc("CUT", [0.0, 0.0], 1.0, 0.0, 90.0) + &flip;
        let sweeps = Reader {
            scale: 1.0,
            layers: &HashMap::new(),
            closed: Vec::new(),
            soup: Vec::new(),
            skipped: Vec::new(),
            unsupported: Vec::new(),
        };
        let pairs = read_pairs(&e).unwrap();
        let recs = records(&pairs);
        let mut reader = sweeps;
        reader.entity("ARC", "CUT", recs[0], &[]);
        let Segment::Arc { start, sweep, .. } = reader.soup[0].1 else {
            panic!("expected an arc");
        };
        assert!(near(start, PI / 2.0, 1e-9) && near(sweep, PI / 2.0, 1e-9));
    }

    #[test]
    fn crlf_and_leading_spaces_are_fine() {
        let text = mm(&rect("CUT", 0.0, 0.0, 10.0, 10.0)).replace('\n', "\r\n");
        assert!(parse(&text).is_ok());
    }

    #[test]
    fn binary_dxf_is_refused() {
        let err = parse("AutoCAD Binary DXF\r\n\x1a\0").unwrap_err();
        assert!(err.contains("binary") && err.contains("ASCII"), "{err}");
        let err = load(b"AutoCAD Binary DXF\r\n\x1a\0", 1.0).unwrap_err();
        assert!(err.contains("ASCII"), "{err}");
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse("hello\nworld\n").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn latin1_text_loads() {
        // A comment (group 999) with a Latin-1 e-acute in front of the usual file.
        let mut bytes = b"999\ncaf\xe9\n".to_vec();
        bytes.extend_from_slice(mm(&rect("CUT", 0.0, 0.0, 10.0, 10.0)).as_bytes());
        let flat = load(&bytes, 2.0).unwrap();
        assert!(!flat.triangles.is_empty());
    }

    // ---- the solid ----

    fn edge_key(p: [f32; 3]) -> [u32; 3] {
        [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()]
    }

    /// Checks every edge is used by exactly two triangles, once in each direction.
    fn assert_watertight(flat: &Flat) {
        let mut used: HashMap<([u32; 3], [u32; 3]), usize> = HashMap::new();
        for t in &flat.triangles {
            for i in 0..3 {
                let key = (edge_key(t[i]), edge_key(t[(i + 1) % 3]));
                *used.entry(key).or_insert(0) += 1;
            }
        }
        for (&(a, b), &count) in &used {
            assert_eq!(
                count, 1,
                "edge {a:?} -> {b:?} is used {count} times one way"
            );
            assert_eq!(
                used.get(&(b, a)).copied(),
                Some(1),
                "edge {a:?} -> {b:?} has no opposite"
            );
        }
    }

    fn volume(flat: &Flat) -> f64 {
        flat.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|p| p.map(f64::from));
                let cross = [
                    b[1] * c[2] - b[2] * c[1],
                    b[2] * c[0] - b[0] * c[2],
                    b[0] * c[1] - b[1] * c[0],
                ];
                (a[0] * cross[0] + a[1] * cross[1] + a[2] * cross[2]) / 6.0
            })
            .sum()
    }

    fn normal(t: &[[f32; 3]; 3]) -> [f64; 3] {
        let [a, b, c] = t.map(|p| p.map(f64::from));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len == 0.0 {
            [0.0; 3]
        } else {
            [n[0] / len, n[1] / len, n[2] / len]
        }
    }

    fn profile_area(p: &Profile) -> f64 {
        p.outer.area() - p.holes.iter().map(Loop::area).sum::<f64>()
    }

    #[test]
    fn a_plain_square_makes_a_box() {
        let p = parse(&mm(&rect("CUT", 0.0, 0.0, 10.0, 10.0))).unwrap();
        let flat = solid(&p, 2.0).unwrap();
        assert_eq!(flat.triangles.len(), flat.faces.len());
        assert_eq!(flat.triangles.len(), 12);
        assert_watertight(&flat);
        assert!(near(volume(&flat), 200.0, 1e-3));
    }

    #[test]
    fn the_solid_is_watertight_outward_and_has_the_right_volume() {
        let e = rect("CUT", 0.0, 0.0, 60.0, 40.0)
            + &circle("CUT", [15.0, 20.0], 5.0)
            + &circle("CUT", [45.0, 20.0], 3.0)
            + &rect("CUT", 25.0, 15.0, 35.0, 25.0);
        let p = parse(&mm(&e)).unwrap();
        let flat = solid(&p, 1.5).unwrap();
        assert_watertight(&flat);
        let v = volume(&flat);
        let expected = profile_area(&p) * 1.5;
        assert!(v > 0.0);
        assert!(near(v, expected, expected * 1e-3), "{v} vs {expected}");
        // Bounding box.
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for t in &flat.triangles {
            for q in t {
                for k in 0..3 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
        }
        assert_eq!(lo, [0.0, 0.0, 0.0]);
        assert_eq!(hi, [60.0, 40.0, 1.5]);
        // Top and bottom triangles face up and down.
        for (t, &f) in flat.triangles.iter().zip(&flat.faces) {
            let n = normal(t);
            match f {
                1 => assert!(n[2] > 0.999 && t.iter().all(|q| q[2] == 1.5)),
                2 => assert!(n[2] < -0.999 && t.iter().all(|q| q[2] == 0.0)),
                _ => assert!(n[2].abs() < 1e-6),
            }
        }
    }

    #[test]
    fn walls_face_away_from_the_material_and_carry_face_ids() {
        let e = rect("CUT", 0.0, 0.0, 10.0, 10.0) + &circle("CUT", [5.0, 5.0], 2.0);
        let p = parse(&mm(&e)).unwrap();
        let flat = solid(&p, 3.0).unwrap();
        assert_watertight(&flat);
        let mut ids: Vec<u32> = flat.faces.clone();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids, vec![1, 2, 3, 4, 5, 6, 7]);
        for (t, &f) in flat.triangles.iter().zip(&flat.faces) {
            let n = normal(t);
            match f {
                3..=6 => {
                    // The outer walls: normals point away from the part's centre.
                    let mid = [
                        (t[0][0] + t[1][0] + t[2][0]) as f64 / 3.0,
                        (t[0][1] + t[1][1] + t[2][1]) as f64 / 3.0,
                    ];
                    assert!(n[0] * (mid[0] - 5.0) + n[1] * (mid[1] - 5.0) > 0.0);
                }
                7 => {
                    // The hole wall: normals point to the hole's axis.
                    let mid = [
                        (t[0][0] + t[1][0] + t[2][0]) as f64 / 3.0,
                        (t[0][1] + t[1][1] + t[2][1]) as f64 / 3.0,
                    ];
                    assert!(n[0] * (5.0 - mid[0]) + n[1] * (5.0 - mid[1]) > 0.0);
                    assert!(n[2].abs() < 1e-6);
                }
                _ => {}
            }
        }
        // The first segment of the square is the bottom edge (y = 0), face id 3.
        let first: Vec<_> = flat
            .triangles
            .iter()
            .zip(&flat.faces)
            .filter(|(_, f)| **f == 3)
            .collect();
        assert_eq!(first.len(), 2);
        for (t, _) in first {
            assert!(t.iter().all(|q| q[1] == 0.0));
            let n = normal(t);
            assert!(n[1] < -0.999);
        }
        // The circle's wall is one face with many triangles.
        let hole_tris = flat.faces.iter().filter(|&&f| f == 7).count();
        assert!(hole_tris >= 32);
    }

    #[test]
    fn a_plate_with_collinear_extra_vertices_triangulates() {
        let e = lwpoly(
            "CUT",
            true,
            &[
                (0.0, 0.0, 0.0),
                (5.0, 0.0, 0.0),
                (10.0, 0.0, 0.0),
                (10.0, 5.0, 0.0),
                (10.0, 10.0, 0.0),
                (5.0, 10.0, 0.0),
                (0.0, 10.0, 0.0),
                (0.0, 5.0, 0.0),
            ],
        ) + &circle("CUT", [5.0, 5.0], 2.0);
        let p = parse(&mm(&e)).unwrap();
        let flat = solid(&p, 1.0).unwrap();
        assert_watertight(&flat);
        let expected = profile_area(&p);
        assert!(near(volume(&flat), expected, expected * 1e-3));
    }

    #[test]
    fn an_l_shape_with_holes_and_a_slot_triangulates() {
        let e = lwpoly(
            "CUT",
            true,
            &[
                (0.0, 0.0, 0.0),
                (40.0, 0.0, 0.0),
                (40.0, 10.0, 0.0),
                (10.0, 10.0, 0.0),
                (10.0, 40.0, 0.0),
                (0.0, 40.0, 0.0),
            ],
        ) + &circle("CUT", [5.0, 5.0], 2.0)
            + &circle("CUT", [5.0, 30.0], 2.0)
            + &circle("CUT", [30.0, 5.0], 2.0)
            + &circle("CUT", [20.0, 5.0], 1.0)
            + &circle("CUT", [5.0, 20.0], 1.0);
        let p = parse(&mm(&e)).unwrap();
        let flat = solid(&p, 1.0).unwrap();
        assert_watertight(&flat);
        let expected = profile_area(&p);
        assert!(near(volume(&flat), expected, expected * 1e-3));
    }

    #[test]
    fn many_holes_in_a_row_triangulate() {
        let mut e = rect("CUT", 0.0, 0.0, 200.0, 40.0);
        for i in 0..15 {
            e += &circle("CUT", [10.0 + 12.0 * i as f64, 20.0], 4.0);
        }
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(p.holes.len(), 15);
        let flat = solid(&p, 2.0).unwrap();
        assert_watertight(&flat);
        let expected = profile_area(&p) * 2.0;
        assert!(near(volume(&flat), expected, expected * 1e-3));
    }

    #[test]
    fn a_bad_thickness_is_an_error() {
        let p = parse(&mm(&rect("CUT", 0.0, 0.0, 10.0, 10.0))).unwrap();
        for t in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(solid(&p, t).is_err(), "{t}");
        }
    }

    #[test]
    fn tessellation_stays_on_the_arc_and_within_the_chord_tolerance() {
        let p = parse(&mm(&circle("CUT", [0.0, 0.0], 10.0))).unwrap();
        let pts = p.outer.points();
        assert!(pts.len() >= 16 && pts.len() <= 360);
        for q in &pts {
            assert!(near(q[0].hypot(q[1]), 10.0, 1e-9));
        }
        let mid = [(pts[0][0] + pts[1][0]) / 2.0, (pts[0][1] + pts[1][1]) / 2.0];
        assert!(10.0 - mid[0].hypot(mid[1]) <= CHORD_TOLERANCE * 1.001);
        // Tiny circles still get at least 16 chords; huge ones at most 360.
        let tiny = parse(&mm(&circle("CUT", [0.0, 0.0], 0.01))).unwrap();
        assert_eq!(tiny.outer.points().len(), 16);
        let huge = parse(&mm(&circle("CUT", [0.0, 0.0], 5000.0))).unwrap();
        assert_eq!(huge.outer.points().len(), 360);
    }

    #[test]
    fn describe_lists_the_holes_and_notes() {
        let e = rect("CUT", 0.0, 0.0, 100.0, 60.0)
            + &circle("CUT", [10.0, 10.0], 2.5)
            + &circle("CUT", [90.0, 10.0], 2.5)
            + &circle("CUT", [10.0, 50.0], 1.7)
            + &rect("CUT", 30.0, 20.0, 70.0, 40.0)
            + &line("BEND", [0.0, 30.0], [100.0, 30.0]);
        let p = parse(&mm(&e)).unwrap();
        assert_eq!(
            p.describe(),
            "Outline 100.0 × 60.0 mm with 4 holes: 2 × Ø5.00, 1 × Ø3.40 and 1 cut-out 40.00 × 20.00 mm. Skipped layer BEND (1 entity): bend lines are not cut."
        );
        let p = parse(&mm(&rect("CUT", 0.0, 0.0, 10.0, 10.0))).unwrap();
        assert_eq!(p.describe(), "Outline 10.0 × 10.0 mm with no holes.");
    }

    fn degenerate_triangles(flat: &Flat) -> usize {
        flat.triangles
            .iter()
            .filter(|t| {
                let n = normal(t);
                n == [0.0; 3]
            })
            .count()
    }

    #[test]
    fn no_triangle_is_degenerate_on_the_collinear_plate_or_the_real_files() {
        let e = lwpoly(
            "CUT",
            true,
            &[
                (0.0, 0.0, 0.0),
                (5.0, 0.0, 0.0),
                (10.0, 0.0, 0.0),
                (10.0, 5.0, 0.0),
                (10.0, 10.0, 0.0),
                (5.0, 10.0, 0.0),
                (0.0, 10.0, 0.0),
                (0.0, 5.0, 0.0),
            ],
        ) + &circle("CUT", [5.0, 5.0], 2.0);
        let flat = solid(&parse(&mm(&e)).unwrap(), 1.0).unwrap();
        assert_eq!(degenerate_triangles(&flat), 0);
        assert_eq!(degenerate_triangles(&solid(&real(TRAY), 1.5).unwrap()), 0);
        assert_eq!(degenerate_triangles(&solid(&real(LID), 1.5).unwrap()), 0);
    }

    /// A small deterministic random generator.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    #[test]
    fn random_stars_with_holes_triangulate_watertight() {
        let mut rng = Lcg(7);
        for round in 0..40 {
            let n = 5 + (rng.next() * 40.0) as usize;
            let mut verts = Vec::new();
            for k in 0..n {
                let a = TAU * k as f64 / n as f64;
                let r = 60.0 + 40.0 * rng.next();
                verts.push((r * a.cos(), r * a.sin(), 0.0));
            }
            let mut e = lwpoly("CUT", true, &verts);
            // Holes on a grid, all well inside the star's inner radius of 60.
            for gx in -1..=1 {
                for gy in -1..=1 {
                    if rng.next() < 0.5 {
                        e += &circle(
                            "CUT",
                            [gx as f64 * 25.0, gy as f64 * 25.0],
                            3.0 + 6.0 * rng.next(),
                        );
                    }
                }
            }
            let p = parse(&mm(&e)).unwrap_or_else(|err| panic!("round {round}: {err}"));
            let flat = solid(&p, 1.0).unwrap();
            assert_watertight(&flat);
            let expected = profile_area(&p);
            assert!(
                near(volume(&flat), expected, expected * 2e-3),
                "round {round}: {} vs {expected}",
                volume(&flat)
            );
            assert_eq!(degenerate_triangles(&flat), 0, "round {round}");
        }
    }

    #[test]
    fn hundreds_of_holes_are_fast_enough() {
        let mut e = rect("CUT", 0.0, 0.0, 400.0, 400.0);
        for i in 0..16 {
            for j in 0..16 {
                e += &circle("CUT", [12.0 + 24.0 * i as f64, 12.0 + 24.0 * j as f64], 5.0);
            }
        }
        let started = std::time::Instant::now();
        let p = parse(&mm(&e)).unwrap();
        let parsed = started.elapsed();
        let flat = solid(&p, 2.0).unwrap();
        println!("parse {parsed:?}, solid {:?}", started.elapsed() - parsed);
        assert_eq!(p.holes.len(), 256);
        assert_watertight(&flat);
    }

    // ---- the real files ----

    const TRAY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/mains-tray-flat.dxf");
    const LID: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/mains-tray-lid-flat.dxf"
    );

    fn real(path: &str) -> Profile {
        let text = std::fs::read_to_string(path).unwrap();
        parse(&text).unwrap()
    }

    #[test]
    fn the_real_tray_reads() {
        let p = real(TRAY);
        assert_eq!(p.units, "mm");
        let (lo, hi) = p.outer.bounds();
        assert!(near(lo[0], 0.0, 1e-6) && near(lo[1], 0.0, 1e-6), "{lo:?}");
        assert!(near(hi[0] - lo[0], 197.535, 1e-3), "{hi:?}");
        assert!(near(hi[1] - lo[1], 295.134, 1e-3), "{hi:?}");
        assert!(
            p.notes
                .contains(&"Skipped layer BEND (5 entities): bend lines are not cut.".to_string())
        );
        assert_eq!(p.holes.len(), 9);
        let mut radii: Vec<f64> = p
            .holes
            .iter()
            .filter_map(|h| h.circle().map(|(_, r)| r))
            .collect();
        radii.sort_by(f64::total_cmp);
        assert_eq!(radii.len(), 8);
        let count = |r: f64| radii.iter().filter(|x| near(**x, r, 1e-9)).count();
        assert_eq!((count(1.7), count(2.2), count(2.5)), (2, 2, 4));
        let cutouts: Vec<&Loop> = p.holes.iter().filter(|h| h.circle().is_none()).collect();
        assert_eq!(cutouts.len(), 1);
        let (w, h) = size(cutouts[0]);
        assert!(near(w, 47.2, 1e-6) && near(h, 28.2, 1e-6), "{w} x {h}");
        assert!(p.outer.signed_area() > 0.0);
        assert!(p.holes.iter().all(|h| h.signed_area() < 0.0));
        println!("{}", p.describe());
    }

    #[test]
    fn the_real_tray_has_the_bend_relief_arcs() {
        let p = real(TRAY);
        let arcs: Vec<(f64, f64)> = p
            .outer
            .segments
            .iter()
            .filter_map(|s| match *s {
                Segment::Arc { radius, sweep, .. } => Some((radius, sweep)),
                Segment::Line { .. } => None,
            })
            .collect();
        assert_eq!(arcs.len(), 3, "{arcs:?}");
        let bulge = -3.4391662435706_f64;
        let relief = arcs
            .iter()
            .filter(|(_, sweep)| near(*sweep, 4.0 * bulge.atan(), 1e-6))
            .count();
        assert_eq!(relief, 2, "{arcs:?}");
        assert!(arcs.iter().all(|(_, sweep)| *sweep < 0.0));
        // The loop is closed within the chain tolerance.
        let segs = &p.outer.segments;
        for i in 0..segs.len() {
            let end = segs[i].end_point();
            let start = segs[(i + 1) % segs.len()].start_point();
            assert!(dist(end, start) < 1e-5, "gap {} at {i}", dist(end, start));
        }
    }

    #[test]
    fn the_real_lid_reads() {
        let p = real(LID);
        let (lo, hi) = p.outer.bounds();
        assert!(near(hi[0] - lo[0], 104.0, 1e-3), "{hi:?}");
        assert!(near(hi[1] - lo[1], 93.134, 1e-3), "{hi:?}");
        assert_eq!(p.holes.len(), 5);
        let radii: Vec<f64> = p
            .holes
            .iter()
            .map(|h| h.circle().expect("round hole").1)
            .collect();
        let count = |r: f64| radii.iter().filter(|x| near(**x, r, 1e-9)).count();
        assert_eq!((count(4.25), count(5.0)), (4, 1));
        println!("{}", p.describe());
    }

    #[test]
    fn the_real_tray_solid_is_watertight() {
        let p = real(TRAY);
        let flat = solid(&p, 1.5).unwrap();
        assert_watertight(&flat);
        let expected = profile_area(&p) * 1.5;
        let v = volume(&flat);
        assert!(near(v, expected, expected * 1e-3), "{v} vs {expected}");
        // The same through `load`.
        let bytes = std::fs::read(TRAY).unwrap();
        let loaded = load(&bytes, 1.5).unwrap();
        assert_eq!(loaded.triangles.len(), flat.triangles.len());
        assert_eq!(loaded.faces.len(), loaded.triangles.len());
    }

    #[test]
    fn the_real_lid_solid_is_watertight() {
        let p = real(LID);
        let flat = solid(&p, 2.0).unwrap();
        assert_watertight(&flat);
        let expected = profile_area(&p) * 2.0;
        assert!(near(volume(&flat), expected, expected * 1e-3));
    }
}
