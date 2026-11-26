//! Landmark profile gates: identifiability clocking, assembly shape and the
//! linear monorail record.

use manhattan_viewer::landmarks::{ACTS, LANDMARKS, Shape, signature_survival};

#[test]
fn every_landmark_profile_is_valid() {
    assert!(LANDMARKS.len() >= 8, "expected at least eight landmarks");
    for landmark in &LANDMARKS {
        let result = manhattan_viewer::landmarks::profile_valid(landmark);
        assert!(result.is_ok(), "{}: {:?}", landmark.name, result.err());
    }
}

#[test]
fn signature_components_survive_half_the_sequence() {
    let mut signatures = 0;
    for landmark in &LANDMARKS {
        for component in landmark.components {
            if !component.signature {
                continue;
            }
            signatures += 1;
            let survival = signature_survival(component);
            assert!(
                survival >= 0.5,
                "{}/{} sheds at {:.0}%",
                landmark.name,
                component.name,
                survival * 100.0
            );
            assert!(component.phase < ACTS);
        }
    }
    assert!(signatures >= 9, "signature components: {signatures}");
}

#[test]
fn sacrificial_mass_sheds_first() {
    for landmark in &LANDMARKS {
        let earliest_signature = landmark
            .components
            .iter()
            .filter(|component| component.signature)
            .map(|component| component.phase)
            .min()
            .expect("signature");
        let earliest_sacrifice = landmark
            .components
            .iter()
            .filter(|component| !component.signature)
            .map(|component| component.phase)
            .min();
        if let Some(sacrifice) = earliest_sacrifice {
            assert!(
                sacrifice < earliest_signature,
                "{} sheds signature before sacrificial mass",
                landmark.name
            );
        }
    }
}

#[test]
fn monorail_is_a_linear_span_assembly() {
    let monorail = LANDMARKS
        .iter()
        .find(|landmark| landmark.key == "monorail")
        .expect("monorail");
    assert_eq!(
        monorail.components.len(),
        12,
        "one damageable unit per span"
    );
    let mut xs: Vec<f32> = monorail.components.iter().map(|c| c.offset[0]).collect();
    xs.sort_by(f32::total_cmp);
    xs.dedup();
    assert_eq!(
        xs.len(),
        monorail.components.len(),
        "spans must not overlap"
    );
    let signature_spans = monorail
        .components
        .iter()
        .filter(|component| component.signature)
        .count();
    assert_eq!(signature_spans, 2, "the curve spans are the signature");
    let early = monorail
        .components
        .iter()
        .filter(|component| component.phase == 0)
        .count();
    assert!(early >= 4, "mid-span deck units shed first");
}

#[test]
fn bridges_carry_signature_arch_or_leaves() {
    for key in ["aurora", "fremont", "ballard"] {
        let bridge = LANDMARKS
            .iter()
            .find(|landmark| landmark.key == key)
            .expect(key);
        let signature: Vec<&str> = bridge
            .components
            .iter()
            .filter(|component| component.signature)
            .map(|component| component.name)
            .collect();
        assert!(!signature.is_empty(), "{key} has no signature member");
        assert_eq!(
            signature.len(),
            bridge
                .components
                .iter()
                .filter(|component| component.signature)
                .count()
        );
        assert!(bridge.components.iter().any(|c| c.shape == Shape::Rect));
    }
}

#[test]
fn needle_signature_is_stem_and_saucer() {
    let needle = LANDMARKS
        .iter()
        .find(|landmark| landmark.key == "needle")
        .expect("needle");
    let names: Vec<&str> = needle
        .components
        .iter()
        .filter(|component| component.signature)
        .map(|component| component.name)
        .collect();
    assert_eq!(names, vec!["stem", "saucer"]);
    let tallest = needle
        .components
        .iter()
        .map(|component| component.base_y + component.height)
        .fold(0.0_f32, f32::max);
    assert!(tallest > 170.0, "needle height {tallest} m");
}
