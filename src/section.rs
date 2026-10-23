//! The section view: the 3D view, and the planner's pictures, cut on a
//! plane square to an axis of the setup's frame or through a flat face of the
//! part, and slid along it. What lies on one side is cut away; where the
//! plane goes through the part or the stock its cut faces are filled and
//! hatched (`viewport`).
use crate::{
    mesh::{Bounds, Mesh},
    model::Setup,
    stock::Placement,
};

/// What a section's plane is square to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Through {
    /// An axis of the setup's frame: 0 X, 1 Y, 2 Z.
    Axis(usize),
    /// A surface of the part (`Mesh::surfaces`, each flat): the plane is
    /// its own, its normal the surface's outward one.
    Surface(usize),
}
/// A section of the scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Section {
    pub through: Through,
    /// Where the plane is, mm: along an axis, from the part's low side on
    /// it; from a surface, along its outward normal (less than 0 is behind
    /// it, inside the part).
    pub at: f32,
    /// Whether what lies on the side the axis, or the surface's outward
    /// normal, points to is cut away; else what lies on the other side.
    pub positive: bool,
}
/// The axes' names, by `Through::Axis`.
pub const AXES: [&str; 3] = ["X", "Y", "Z"];
/// A change to the 3D view's section, from the controls under it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edit {
    /// Turns the section on or off; the plane stays for the next time.
    Toggle,
    /// Cuts square to axis 0 X, 1 Y or 2 Z, through the middle of the part.
    Axis(usize),
    /// Starts, or stops, picking the face the section goes through.
    PickFace,
    /// Slides the plane to `at` (`Section::at`, mm).
    At(f32),
    /// Cuts away the other side.
    Flip,
}
impl Section {
    /// A section square to axis `k` through the middle of the part, the
    /// side `toward` (the viewer's direction) faces cut away, so that the
    /// cut shows.
    pub fn across(k: usize, mesh: &Mesh, setup: Option<&Setup>, toward: [f32; 3]) -> Self {
        let size = Placement::new(mesh, setup).bounds.dimensions()[k] / mesh.display_scale();
        Self {
            through: Through::Axis(k),
            at: size / 2.,
            positive: toward[k] >= 0.,
        }
    }
    /// A section through the part's `surface`, cutting away what is in
    /// front of it: the face it goes through faces the viewer as it is
    /// picked, so sliding the plane back cuts into the part.
    pub fn through(surface: usize) -> Self {
        Self {
            through: Through::Surface(surface),
            at: 0.,
            positive: true,
        }
    }
    /// A section through the part's `surface` as a click picks it: 1 mm
    /// into the part behind the face, or half way through a part thinner
    /// than 2 mm there, so the cut shows at once.
    pub fn picked(surface: usize, mesh: &Mesh, setup: Option<&Setup>) -> Self {
        let cut = Self::through(surface);
        let bounds = Placement::new(mesh, setup).bounds;
        let deepest = cut.range(mesh, setup, bounds).map_or(0., |(low, _)| low);
        Self {
            at: (deepest / 2.).clamp(-1., 0.),
            ..cut
        }
    }
    /// The plane with `at` 0, its normal along the axis or the surface's
    /// (whatever `positive`), in `setup`'s frame (display units). None for a
    /// surface the part does not have.
    fn base(&self, mesh: &Mesh, setup: Option<&Setup>) -> Option<Plane> {
        let placement = Placement::new(mesh, setup);
        let (normal, point) = match self.through {
            Through::Axis(k) => {
                let mut normal = [0.; 3];
                *normal.get_mut(k)? = 1.;
                (normal, placement.bounds.min)
            }
            Through::Surface(id) => {
                let s = mesh.surfaces.get(id)?;
                let centre = placement.transform(s.center);
                let tip = placement.transform(std::array::from_fn(|i| s.center[i] + s.normal[i]));
                (unit(std::array::from_fn(|i| tip[i] - centre[i]))?, centre)
            }
        };
        Some(Plane {
            normal,
            d: dot(normal, point),
        })
    }
    /// The plane in `setup`'s frame (display units), its normal toward
    /// what is cut away. None for a surface the part does not have.
    pub fn plane(&self, mesh: &Mesh, setup: Option<&Setup>) -> Option<Plane> {
        let base = self.base(mesh, setup)?;
        let d = base.d + self.at * mesh.display_scale();
        Some(if self.positive {
            Plane { d, ..base }
        } else {
            Plane {
                normal: base.normal.map(|v| -v),
                d: -d,
            }
        })
    }
    /// The `at` (mm) the plane crosses `bounds` (display units, `setup`'s
    /// frame) between, from the least to the most.
    pub fn range(&self, mesh: &Mesh, setup: Option<&Setup>, bounds: Bounds) -> Option<(f32, f32)> {
        let base = self.base(mesh, setup)?;
        let scale = mesh.display_scale();
        let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
        for corner in 0..8 {
            let v = std::array::from_fn(|k| {
                if corner >> k & 1 == 1 {
                    bounds.max[k]
                } else {
                    bounds.min[k]
                }
            });
            let at = base.distance(v) / scale;
            (low, high) = (low.min(at), high.max(at));
        }
        (low.is_finite() && high.is_finite()).then_some((low, high))
    }
    /// Where the plane is, in words: "Z 12.0 mm from the part's bottom",
    /// "2.0 mm behind surface 14".
    pub fn place(&self) -> String {
        match self.through {
            Through::Axis(k) => format!(
                "{} {:.1} mm from the part's {}",
                AXES.get(k).unwrap_or(&"?"),
                self.at,
                ["−X side", "−Y side", "bottom"]
                    .get(k)
                    .unwrap_or(&"low side")
            ),
            Through::Surface(id) if self.at.abs() < 0.05 => format!("through surface {id}"),
            Through::Surface(id) => format!(
                "{:.1} mm {} surface {id}",
                self.at.abs(),
                if self.at > 0. {
                    "in front of"
                } else {
                    "behind"
                }
            ),
        }
    }
    /// The side cut away, in words: "+Z", "in front of the surface".
    pub fn side(&self) -> String {
        match self.through {
            Through::Axis(k) => format!(
                "{}{}",
                if self.positive { "+" } else { "−" },
                AXES.get(k).unwrap_or(&"?")
            ),
            Through::Surface(_) if self.positive => "in front of the plane".into(),
            Through::Surface(_) => "behind the plane".into(),
        }
    }
}
/// A segment between two points of a frame, as where a plane crosses a face.
pub type Segment = [[f32; 3]; 2];
/// A plane in a frame: the points `v` with `normal` · `v` > `d` are cut
/// away, so `normal`, of length 1, points to them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    pub normal: [f32; 3],
    pub d: f32,
}
impl Plane {
    /// How far `v` is on the side cut away; less than 0 on the side kept.
    pub fn distance(&self, v: [f32; 3]) -> f32 {
        dot(self.normal, v) - self.d
    }
    fn keeps(&self, v: [f32; 3]) -> bool {
        self.distance(v) <= 0.
    }
    /// Where the segment `a`–`b`, which the plane crosses, meets it. The
    /// same whichever end is given first, so two faces that share an edge
    /// cross the plane at the very same point.
    fn meet(&self, a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        let (da, db) = (self.distance(a), self.distance(b));
        let t = da / (da - db);
        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
    }
    /// What the plane keeps of the convex polygon `points`: all of them, a
    /// smaller polygon, or none (fewer than 3 points).
    pub fn clip(&self, points: &[[f32; 3]]) -> Vec<[f32; 3]> {
        if points.len() < 3 {
            return vec![];
        }
        if points.iter().all(|v| self.keeps(*v)) {
            return points.to_vec();
        }
        let mut kept = Vec::with_capacity(points.len() + 1);
        for (i, &a) in points.iter().enumerate() {
            let b = points[(i + 1) % points.len()];
            if self.keeps(a) {
                kept.push(a);
            }
            if self.keeps(a) != self.keeps(b) {
                kept.push(self.meet(a, b));
            }
        }
        if kept.len() < 3 { vec![] } else { kept }
    }
    /// What the plane keeps of the segment `a`–`b`.
    pub fn clip_segment(&self, a: [f32; 3], b: [f32; 3]) -> Option<([f32; 3], [f32; 3])> {
        match (self.keeps(a), self.keeps(b)) {
            (true, true) => Some((a, b)),
            (false, false) => None,
            (true, false) => Some((a, self.meet(a, b))),
            (false, true) => Some((self.meet(a, b), b)),
        }
    }
    /// Where the plane crosses the convex polygon `points`: the segment of
    /// the cut face along it. None where it does not.
    pub fn crossing(&self, points: &[[f32; 3]]) -> Option<Segment> {
        let mut ends = points.iter().enumerate().filter_map(|(i, &a)| {
            let b = points[(i + 1) % points.len()];
            (self.keeps(a) != self.keeps(b)).then(|| self.meet(a, b))
        });
        Some([ends.next()?, ends.next()?])
    }
    /// The cut face of the box `b`, its corners in turn round it; empty
    /// where the plane misses the box.
    pub fn box_face(&self, b: Bounds) -> Vec<[f32; 3]> {
        let corner = |i: usize| -> [f32; 3] {
            std::array::from_fn(|k| if i >> k & 1 == 1 { b.max[k] } else { b.min[k] })
        };
        let mut points = vec![];
        for i in 0..8 {
            for k in 0..3 {
                let j = i | 1 << k;
                if j != i && self.keeps(corner(i)) != self.keeps(corner(j)) {
                    points.push(self.meet(corner(i), corner(j)));
                }
            }
        }
        if points.len() < 3 {
            return vec![];
        }
        // In turn round their middle, seen along the normal.
        let n = points.len() as f32;
        let middle: [f32; 3] =
            std::array::from_fn(|k| points.iter().map(|p| p[k]).sum::<f32>() / n);
        let across = if self.normal[0].abs() < 0.9 {
            [1., 0., 0.]
        } else {
            [0., 1., 0.]
        };
        let u = unit(cross(self.normal, across)).unwrap_or([0., 0., 1.]);
        let w = cross(self.normal, u);
        let angle = |p: &[f32; 3]| {
            let r: [f32; 3] = std::array::from_fn(|k| p[k] - middle[k]);
            dot(r, w).atan2(dot(r, u))
        };
        points.sort_by(|a, b| angle(a).total_cmp(&angle(b)));
        points
    }
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let length = dot(v, v).sqrt();
    (length > 1e-9 && length.is_finite()).then(|| v.map(|c| c / length))
}

#[cfg(test)]
mod tests {
    use super::*;

    const Z: Plane = Plane {
        normal: [0., 0., 1.],
        d: 1.,
    };

    #[test]
    fn a_polygon_keeps_what_lies_on_the_kept_side() {
        let square = [[0., 0., 0.], [2., 0., 0.], [2., 0., 2.], [0., 0., 2.]];
        assert_eq!(
            Z.clip(&square),
            vec![[0., 0., 0.], [2., 0., 0.], [2., 0., 1.], [0., 0., 1.]]
        );
        assert!(Z.clip(&square.map(|p| [p[0], p[1], p[2] + 5.])).is_empty());
        assert!(Z.clip(&square[..2]).is_empty());
        let low = square.map(|p| [p[0], p[1], p[2] / 4.]);
        assert_eq!(Z.clip(&low), low.to_vec());
        assert_eq!(
            Z.crossing(&square),
            Some([[2., 0., 1.], [0., 0., 1.]]),
            "the cut face's edge along the square"
        );
        assert_eq!(Z.crossing(&low), None);
        assert_eq!(
            Z.clip_segment([0., 0., 0.], [0., 0., 4.]),
            Some(([0., 0., 0.], [0., 0., 1.]))
        );
        assert_eq!(Z.clip_segment([0., 0., 3.], [0., 0., 4.]), None);
    }
    #[test]
    fn two_faces_sharing_an_edge_cross_the_plane_at_one_point() {
        let plane = Plane {
            normal: [0.6, 0., 0.8],
            d: 0.37,
        };
        let (a, b) = ([0.1, 0.3, -0.2], [0.9, -0.4, 1.3]);
        assert_eq!(plane.meet(a, b), plane.meet(b, a));
    }
    #[test]
    fn a_box_is_cut_on_a_face_whose_corners_go_round_it() {
        let b = Bounds {
            min: [0.; 3],
            max: [2., 2., 2.],
        };
        let face = Z.box_face(b);
        assert_eq!(face.len(), 4);
        assert!(face.iter().all(|p| (p[2] - 1.).abs() < 1e-6));
        // Neighbours share an x or a y: the corners go round, not across.
        for (i, p) in face.iter().enumerate() {
            let q = face[(i + 1) % 4];
            assert!(p[0] == q[0] || p[1] == q[1], "{face:?}");
        }
        let corner = Plane {
            normal: [1. / 3f32.sqrt(); 3],
            d: 0.5,
        };
        assert_eq!(corner.box_face(b).len(), 3);
        assert!(
            Plane {
                normal: [0., 0., 1.],
                d: 5.,
            }
            .box_face(b)
            .is_empty()
        );
    }
    #[test]
    fn a_section_cuts_away_the_side_it_says() {
        let (doc, mesh) = crate::post::fixture::job();
        let setup = doc.setups.first();
        let scale = mesh.display_scale();
        let part = Placement::new(&mesh, setup).bounds;
        let section = Section {
            through: Through::Axis(2),
            at: 3.,
            positive: true,
        };
        let plane = section.plane(&mesh, setup).unwrap();
        assert_eq!(plane.normal, [0., 0., 1.]);
        assert!((plane.d - (part.min[2] + 3. * scale)).abs() < 1e-4);
        let below = Section {
            positive: false,
            ..section
        }
        .plane(&mesh, setup)
        .unwrap();
        assert_eq!(below.normal, [0., 0., -1.]);
        assert!((below.d + plane.d).abs() < 1e-6);
        // Across the part's height, from its bottom to its top.
        let (low, high) = section.range(&mesh, setup, part).unwrap();
        assert!(low.abs() < 1e-3, "{low}");
        assert!((high - part.dimensions()[2] / scale).abs() < 1e-3, "{high}");
        assert_eq!(section.place(), "Z 3.0 mm from the part's bottom");
        assert_eq!(section.side(), "+Z");
        // Through a surface: the plane is the surface's, moved along its
        // normal, and goes through the surface's centre at 0.
        let id = (0..mesh.surfaces.len())
            .max_by(|a, b| mesh.surfaces[*a].area.total_cmp(&mesh.surfaces[*b].area))
            .unwrap();
        let surface = Section::through(id);
        let plane = surface.plane(&mesh, setup).unwrap();
        let centre = Placement::new(&mesh, setup).transform(mesh.surfaces[id].center);
        assert!(plane.distance(centre).abs() < 1e-3);
        let back = Section { at: -2., ..surface }.plane(&mesh, setup).unwrap();
        assert!((back.distance(centre) - 2. * scale).abs() < 1e-3);
        assert_eq!(
            Section { at: -2., ..surface }.place(),
            format!("2.0 mm behind surface {id}")
        );
        assert_eq!(surface.place(), format!("through surface {id}"));
        // Picked with a click, it starts 1 mm into the part, so the cut shows.
        let picked = Section::picked(id, &mesh, setup);
        assert_eq!(picked, Section { at: -1., ..surface });
        assert_eq!(picked.place(), format!("1.0 mm behind surface {id}"));
        assert!(
            Section::through(mesh.surfaces.len())
                .plane(&mesh, setup)
                .is_none()
        );
    }
}
