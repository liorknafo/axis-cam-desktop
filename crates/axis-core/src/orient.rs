//! Which way a machine's X and Y point, seen by the operator standing at
//! its front (LIO-457). The UC100 profile's `reversed` sets which way a
//! motor turns for +; this sets which way + is on the table, so that what
//! the screen shows and names (the machine view, the jog pad and keys, the
//! sides and corners touched off) agrees with the machine.
//!
//! +X may point right (the usual), left, away or toward the operator; +Y
//! follows by the right-hand rule with Z up, so a part is never mirrored:
//! the frame is only turned about Z. The Jingyan CNC4030's own handheld
//! counts X up going left and Y up toward the front: `Orient::Left`.
use crate::model::{Fields, Machine, get};
use crate::post::At;

/// The machine field that holds it, one of `LABELS`.
pub const KEY: &str = "x_points";
/// The choices, in `Orient` order.
pub const LABELS: [&str; 4] = ["Right", "Left", "Away from you", "Toward you"];

/// Which way machine +X points, seen from the front. Right unless set, so
/// a machine saved before LIO-457 is as it was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Orient {
    #[default]
    Right,
    Left,
    Away,
    Toward,
}

/// A direction on the table, seen from the front.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    Right,
    Left,
    Away,
    Toward,
}

impl Dir {
    /// The direction turned a quarter anticlockwise, seen from above.
    fn ccw(self) -> Dir {
        match self {
            Dir::Right => Dir::Away,
            Dir::Away => Dir::Left,
            Dir::Left => Dir::Toward,
            Dir::Toward => Dir::Right,
        }
    }

    fn opposite(self) -> Dir {
        self.ccw().ccw()
    }

    /// The side of the table or the stock it leads to: "right", "left",
    /// "back", "front".
    pub fn side(self) -> &'static str {
        match self {
            Dir::Right => "right",
            Dir::Left => "left",
            Dir::Away => "back",
            Dir::Toward => "front",
        }
    }
}

impl Orient {
    /// Machine `m`'s. A machine worked by hand is always the usual one: its
    /// Steps are drawn and said from the operator's side as the screen
    /// shows them, and its DRO sets its own counting direction.
    pub fn of(m: &Machine) -> Self {
        if crate::manual::is_hand(m) {
            return Orient::Right;
        }
        Self::from_fields(&m.p)
    }

    pub fn from_fields(p: &Fields) -> Self {
        match get(p, KEY).trim() {
            "Left" => Orient::Left,
            "Away from you" => Orient::Away,
            "Toward you" => Orient::Toward,
            _ => Orient::Right,
        }
    }

    pub fn label(self) -> &'static str {
        LABELS[self as usize]
    }

    /// Where machine axis `k` (0 X, 1 Y) goes when it moves + (`plus`) or −.
    pub fn dir(self, k: usize, plus: bool) -> Dir {
        let x = match self {
            Orient::Right => Dir::Right,
            Orient::Left => Dir::Left,
            Orient::Away => Dir::Away,
            Orient::Toward => Dir::Toward,
        };
        let d = if k == 0 { x } else { x.ccw() };
        if plus { d } else { d.opposite() }
    }

    /// The axis and whether it goes − that moves the tool toward `d`, as
    /// a jog takes them.
    pub fn jog(self, d: Dir) -> (usize, bool) {
        for k in 0..2 {
            for plus in [true, false] {
                if self.dir(k, plus) == d {
                    return (k, !plus);
                }
            }
        }
        unreachable!("every direction is an axis's + or −")
    }

    /// The axis and direction that move the tool the way the usual
    /// machine's `axis` and `negative` do: a physical jog (→ is the usual
    /// X+, ↑ the usual Y+) for this machine. Z and A are the same on every
    /// machine.
    pub fn jog_like_usual(self, axis: usize, negative: bool) -> (usize, bool) {
        if axis > 1 {
            return (axis, negative);
        }
        self.jog(Orient::Right.dir(axis, !negative))
    }

    /// The name of axis `k`'s end at its maximum (`max`) or minimum:
    /// "left", "right", "front" or "back".
    pub fn end(self, k: usize, max: bool) -> &'static str {
        self.dir(k, max).side()
    }

    /// A corner of the stock top in words, front or back first:
    /// "front-left" and the like.
    pub fn corner(self, xy: [At; 2]) -> String {
        let a = self.end(0, xy[0] == At::Max);
        let b = self.end(1, xy[1] == At::Max);
        let (fb, lr) = if matches!(a, "front" | "back") {
            (a, b)
        } else {
            (b, a)
        };
        format!("{fb}-{lr}")
    }

    /// The ends of X and Y (`At`) at a spot of a grid seen from the front:
    /// `right` and `away` are −1, 0 or 1 (left, middle, right; front,
    /// middle, back).
    pub fn spot_at(self, right: i8, away: i8) -> [At; 2] {
        std::array::from_fn(|k| {
            let along = match self.dir(k, true) {
                Dir::Right => right,
                Dir::Left => -right,
                Dir::Away => away,
                Dir::Toward => -away,
            };
            match along {
                ..0 => At::Min,
                0 => At::Mid,
                _ => At::Max,
            }
        })
    }

    /// The stock corner at the back (`back`) or front and the right
    /// (`right`) or left, as a grid of corners lays them out.
    pub fn corner_at(self, back: bool, right: bool) -> [At; 2] {
        self.spot_at(if right { 1 } else { -1 }, if back { 1 } else { -1 })
    }

    /// The machine axis (0 X, 1 Y) that runs across the table, left to
    /// right: the one the gantry's width spans.
    pub fn across(self) -> usize {
        match self {
            Orient::Right | Orient::Left => 0,
            Orient::Away | Orient::Toward => 1,
        }
    }

    /// The X and Y travel along the table's width and depth: X's and Y's,
    /// swapped when X runs front to back.
    pub fn world_travel(self, travel: [f32; 3]) -> [f32; 3] {
        match self {
            Orient::Right | Orient::Left => travel,
            Orient::Away | Orient::Toward => [travel[1], travel[0], travel[2]],
        }
    }

    /// Machine point `p` (mm) on the table seen from the front: X to the
    /// right, Y away, Z up, with the travel `travel` (X, Y from machine 0)
    /// from 0 to `world_travel` either way, as with the usual orientation.
    pub fn world(self, p: [f32; 3], travel: [f32; 3]) -> [f32; 3] {
        let [x, y, z] = p;
        let [tx, ty, _] = travel;
        match self {
            Orient::Right => [x, y, z],
            Orient::Left => [tx - x, ty - y, z],
            Orient::Away => [ty - y, x, z],
            Orient::Toward => [y, tx - x, z],
        }
    }

    /// A machine direction (no offset) seen from the front.
    pub fn world_dir(self, v: [f32; 3]) -> [f32; 3] {
        let [x, y, z] = v;
        match self {
            Orient::Right => [x, y, z],
            Orient::Left => [-x, -y, z],
            Orient::Away => [-y, x, z],
            Orient::Toward => [y, -x, z],
        }
    }

    /// Y's + (`plus`) or − end in words: "−Y, the front" on the usual
    /// machine.
    pub fn y_words(self, plus: bool) -> String {
        format!(
            "{}Y, the {}",
            if plus { "+" } else { "−" },
            self.end(1, plus)
        )
    }

    /// On a machine other than the usual one, why the sides a setup's work
    /// zero is named by (the part's own, as the screen draws it: X right,
    /// Y away) are not the ones the machine's words name: None on the
    /// usual machine.
    pub fn turned_note(self) -> Option<String> {
        (self != Orient::Right).then(|| {
            format!(
                "On this machine {}: the stock stands turned from how the screen draws it, so its sides are named here where they are on the table.",
                self.describe()
            )
        })
    }

    /// How the jog pad and the machine settings say it: "+X right, +Y away
    /// from you".
    pub fn describe(self) -> String {
        let say = |d: Dir| match d {
            Dir::Right => "right",
            Dir::Left => "left",
            Dir::Away => "away from you",
            Dir::Toward => "toward you",
        };
        format!(
            "+X {}, +Y {}",
            say(self.dir(0, true)),
            say(self.dir(1, true))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ALL: [Orient; 4] = [Orient::Right, Orient::Left, Orient::Away, Orient::Toward];

    #[test]
    fn y_follows_x_by_the_right_hand_rule_so_nothing_is_mirrored() {
        for o in ALL {
            let x = o.world_dir([1., 0., 0.]);
            let y = o.world_dir([0., 1., 0.]);
            // X × Y = Z: a turn about Z, never a mirror.
            assert_eq!(x[0] * y[1] - x[1] * y[0], 1., "{o:?}");
            assert_eq!(x[2], 0.);
        }
        assert_eq!(Orient::Right.describe(), "+X right, +Y away from you");
        assert_eq!(Orient::Left.describe(), "+X left, +Y toward you");
        assert_eq!(Orient::Away.describe(), "+X away from you, +Y left");
        assert_eq!(Orient::Toward.describe(), "+X toward you, +Y right");
    }

    #[test]
    fn a_point_and_a_direction_map_the_same_way() {
        let travel = [305., 405., 100.];
        for o in ALL {
            let wt = o.world_travel(travel);
            for p in [[0., 0., -5.], [305., 405., 0.], [100., 30., -20.]] {
                let w = o.world(p, travel);
                // Within the travel either way.
                assert!(
                    (0.0..=wt[0]).contains(&w[0]) && (0.0..=wt[1]).contains(&w[1]),
                    "{o:?} {w:?}"
                );
                for (k, d) in [(0, 1.), (1, 1.)] {
                    let mut q = p;
                    q[k] += d;
                    let moved: Vec<f32> = (0..3).map(|i| o.world(q, travel)[i] - w[i]).collect();
                    let mut v = [0.; 3];
                    v[k] = d;
                    assert_eq!(moved, o.world_dir(v).to_vec(), "{o:?} axis {k}");
                }
            }
            // The physical direction of + agrees with `dir`.
            for k in 0..2 {
                let mut v = [0.; 3];
                v[k] = 1.;
                let w = o.world_dir(v);
                let d = match (w[0] as i32, w[1] as i32) {
                    (1, 0) => Dir::Right,
                    (-1, 0) => Dir::Left,
                    (0, 1) => Dir::Away,
                    _ => Dir::Toward,
                };
                assert_eq!(o.dir(k, true), d, "{o:?} axis {k}");
            }
        }
    }

    #[test]
    fn a_jog_moves_the_tool_the_way_its_key_points() {
        assert_eq!(Orient::Right.jog(Dir::Right), (0, false));
        assert_eq!(Orient::Right.jog(Dir::Away), (1, false));
        assert_eq!(Orient::Left.jog(Dir::Right), (0, true));
        assert_eq!(Orient::Left.jog(Dir::Away), (1, true));
        assert_eq!(Orient::Away.jog(Dir::Away), (0, false));
        assert_eq!(Orient::Away.jog(Dir::Left), (1, false));
        assert_eq!(Orient::Right.jog_like_usual(0, false), (0, false));
        assert_eq!(Orient::Left.jog_like_usual(0, false), (0, true));
        assert_eq!(Orient::Left.jog_like_usual(1, false), (1, true));
        assert_eq!(Orient::Away.jog_like_usual(1, false), (0, false));
        assert_eq!(Orient::Left.jog_like_usual(2, true), (2, true));
        for o in ALL {
            for d in [Dir::Right, Dir::Left, Dir::Away, Dir::Toward] {
                let (k, neg) = o.jog(d);
                assert_eq!(o.dir(k, !neg), d);
            }
        }
    }

    #[test]
    fn sides_and_corners_are_named_where_they_are() {
        use At::{Max, Min};
        assert_eq!(Orient::Right.corner([Min, Min]), "front-left");
        assert_eq!(Orient::Right.corner([Max, Max]), "back-right");
        assert_eq!(Orient::Left.corner([Min, Min]), "back-right");
        assert_eq!(Orient::Left.corner([Max, Min]), "back-left");
        assert_eq!(Orient::Away.corner([Min, Min]), "front-right");
        assert_eq!(Orient::Toward.corner([Min, Min]), "back-left");
        assert_eq!(Orient::Left.end(0, false), "right");
        assert_eq!(Orient::Left.end(1, true), "front");
        for o in ALL {
            // The axis across is the one going left or right.
            assert!(
                matches!(o.dir(o.across(), true), Dir::Left | Dir::Right),
                "{o:?}"
            );
        }
        for o in ALL {
            for back in [false, true] {
                for right in [false, true] {
                    let c = o.corner(o.corner_at(back, right));
                    assert!(
                        c.starts_with(if back { "back" } else { "front" })
                            && c.ends_with(if right { "right" } else { "left" })
                    );
                }
            }
        }
        assert_eq!(Orient::Right.corner_at(true, false), [At::Min, At::Max]);
        assert_eq!(Orient::Left.corner_at(true, false), [At::Max, At::Min]);
    }

    #[test]
    fn an_unset_or_unknown_field_is_the_usual_right() {
        let mut m = Machine::new(3);
        assert_eq!(Orient::of(&m), Orient::Right);
        m.p.insert(KEY.into(), "Left".into());
        assert_eq!(Orient::of(&m), Orient::Left);
        m.p.insert(KEY.into(), "sideways".into());
        assert_eq!(Orient::of(&m), Orient::Right);
        // A hand machine keeps the usual one, whatever the field says.
        let mut hand = crate::manual::defaults();
        hand.p.insert(KEY.into(), "Left".into());
        assert_eq!(Orient::of(&hand), Orient::Right);
        for o in ALL {
            m.p.insert(KEY.into(), o.label().into());
            assert_eq!(Orient::of(&m), o);
        }
    }
}
