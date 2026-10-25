//! The milling direction a material asks for its finishing passes
//! (`Material::finish_milling`): a Pocket or Contour that finishes in it
//! must be milled that way. Only these two strategies have finishing passes
//! (`finish_passes`) and a direction (`pocket_milling`).
use crate::material::Material;
use crate::model::{Operation, Strategy, get, num, pocket_setting};

/// Whether `op` has finishing passes to mill in a set direction.
fn finishes(op: &Operation) -> bool {
    matches!(op.strategy, Strategy::Pocket | Strategy::Contour)
        && num(&op.p, "finish_passes").is_some_and(|n| n >= 1.)
}
/// Why `op` is not milled the way `material` asks to be finished, if it is
/// not: one line naming the material.
pub fn issue(material: &Material, op: &Operation) -> Option<String> {
    let want = material.finish_milling()?;
    let have = pocket_setting(&op.p, "pocket_milling");
    (finishes(op) && have != want).then(|| {
        format!(
            "{}'s finishing passes are milled {}, not {}: set the milling direction to {want}, or the finishing passes to 0.",
            crate::proposal::clip(material.name(), crate::material::NAME_LIMIT),
            want.to_lowercase(),
            have.to_lowercase()
        )
    })
}
/// Gives `op` the direction `material` asks for its finishing passes, when it
/// has finishing passes. Whether it changed anything.
pub fn apply(material: &Material, op: &mut Operation) -> bool {
    let Some(want) = material.finish_milling() else {
        return false;
    };
    if !finishes(op) || get(&op.p, "pocket_milling") == want {
        return false;
    }
    op.p.insert("pocket_milling".into(), want.into());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Fields;

    fn bronze() -> Material {
        crate::material::starters()
            .into_iter()
            .find(|m| m.name() == "Bronze CuSn12")
            .unwrap()
    }
    fn op(strategy: Strategy, extra: &[(&str, &str)]) -> Operation {
        let mut o = Operation::new(strategy, String::new());
        let set: Fields = extra
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        o.p.extend(set);
        o
    }

    #[test]
    fn a_climbing_finish_in_bronze_is_named_and_a_conventional_one_is_not() {
        let climb = op(Strategy::Pocket, &[("finish_passes", "1")]);
        // A Pocket starts climbing.
        let said = issue(&bronze(), &climb).unwrap();
        assert!(
            said.starts_with("Bronze CuSn12's finishing passes are milled conventional, not climb"),
            "{said}"
        );
        let conventional = op(
            Strategy::Contour,
            &[("finish_passes", "2"), ("pocket_milling", "Conventional")],
        );
        assert_eq!(issue(&bronze(), &conventional), None);
        // Contour climbs by default too.
        assert!(issue(&bronze(), &op(Strategy::Contour, &[("finish_passes", "1")])).is_some());
    }
    #[test]
    fn only_a_pocket_or_contour_with_finishing_passes_has_a_finish_to_check() {
        let none = op(Strategy::Pocket, &[("finish_passes", "0")]);
        assert_eq!(issue(&bronze(), &none), None);
        for s in [Strategy::Face, Strategy::Drill, Strategy::Parallel] {
            assert_eq!(issue(&bronze(), &op(s, &[("finish_passes", "1")])), None);
        }
        // A material without the rule never complains.
        let steel = crate::material::starters().remove(2);
        assert_eq!(steel.finish_milling(), None);
        assert_eq!(
            issue(&steel, &op(Strategy::Pocket, &[("finish_passes", "1")])),
            None
        );
    }
    #[test]
    fn applying_the_rule_sets_the_direction_once() {
        let mut o = op(Strategy::Pocket, &[("finish_passes", "1")]);
        assert!(apply(&bronze(), &mut o));
        assert_eq!(get(&o.p, "pocket_milling"), "Conventional");
        assert_eq!(issue(&bronze(), &o), None);
        assert!(!apply(&bronze(), &mut o));
        let mut rough = op(Strategy::Pocket, &[("finish_passes", "0")]);
        let before = rough.p.clone();
        assert!(!apply(&bronze(), &mut rough));
        assert_eq!(rough.p, before);
    }
}
