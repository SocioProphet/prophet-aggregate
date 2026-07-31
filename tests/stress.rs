//! Scale-hardening: canonicalization stays exact under real instance
//! multiplicity, and the search stays sound, complete-to-count, and deterministic
//! at larger bounds.

mod common;
use common::*;

use prophet_aggregate::{canonical_form, canonical_id, Aggregator, Bound};
use prophet_sheaf::{section_from_parts, Bond, MatingWitness, NoOntology, PortClass, PortRef, Policy, Section};

fn w() -> MatingWitness {
    MatingWitness::capability("d", PortClass::Flow)
}
fn pref(g: &str, inst: u32, port: u32) -> PortRef {
    PortRef {
        placement: place(g, inst),
        port,
    }
}

/// Two disjoint `source→transform→sink` pipelines. Swapping the two pipelines is
/// an automorphism, so every instance labeling is isomorphic — the perm search
/// (2!·2!·2! = 8 relabelings) must return one canonical id for all of them.
fn two_pipelines(s0: u32, s1: u32, t0: u32, t1: u32, k0: u32, k1: u32) -> Section {
    section_from_parts(
        vec![
            place("source", s0),
            place("source", s1),
            place("transform", t0),
            place("transform", t1),
            place("sink", k0),
            place("sink", k1),
        ],
        vec![
            Bond { from: pref("source", s0, SOURCE_OUT), to: pref("transform", t0, TRANSFORM_IN), witness: w() },
            Bond { from: pref("transform", t0, TRANSFORM_OUT), to: pref("sink", k0, SINK_IN), witness: w() },
            Bond { from: pref("source", s1, SOURCE_OUT), to: pref("transform", t1, TRANSFORM_IN), witness: w() },
            Bond { from: pref("transform", t1, TRANSFORM_OUT), to: pref("sink", k1, SINK_IN), witness: w() },
        ],
    )
}

#[test]
fn canonicalization_is_invariant_under_instance_multiplicity() {
    let base = canonical_id(&two_pipelines(0, 1, 0, 1, 0, 1));
    // Whole-pipeline swap, and arbitrary relabelings, all collapse.
    for (s0, s1, t0, t1, k0, k1) in [
        (1, 0, 1, 0, 1, 0),
        (5, 9, 2, 7, 3, 8),
        (9, 5, 7, 2, 8, 3),
        (0, 2, 0, 4, 0, 6),
    ] {
        assert_eq!(
            canonical_id(&two_pipelines(s0, s1, t0, t1, k0, k1)),
            base,
            "relabeling changed the canonical id"
        );
    }
}

#[test]
fn canonical_form_is_idempotent() {
    let s = two_pipelines(3, 8, 1, 6, 4, 9);
    let once = canonical_form(&s);
    let twice = canonical_form(&once);
    assert_eq!(once.id(), twice.id());
    assert_eq!(
        <Section as prophet_sheaf::Canonical>::canonical_bytes(&once),
        <Section as prophet_sheaf::Canonical>::canonical_bytes(&twice)
    );
}

#[test]
fn non_isomorphic_multiplicity_sections_stay_distinct() {
    // Two pipelines vs. one pipeline plus a lone (unbonded) extra are different
    // shapes and must not collapse.
    let two = two_pipelines(0, 1, 0, 1, 0, 1);
    let different = section_from_parts(
        vec![place("source", 0), place("transform", 0), place("sink", 0)],
        vec![
            Bond { from: pref("source", 0, SOURCE_OUT), to: pref("transform", 0, TRANSFORM_IN), witness: w() },
            Bond { from: pref("transform", 0, TRANSFORM_OUT), to: pref("sink", 0, SINK_IN), witness: w() },
        ],
    );
    assert_ne!(canonical_id(&two), canonical_id(&different));
}

#[test]
fn large_bound_search_is_sound_and_deterministic() {
    let m = dataflow_with_hub();
    let bound = 8;
    let agg = Aggregator::new(&m, &NoOntology, Policy::default(), Bound::placements(bound));
    let sols = agg.complete(&[place("source", 0)]).unwrap();

    // From a source, completions are pipelines with 0..=(bound-2) transforms.
    assert_eq!(sols.len(), bound - 1);
    for s in &sols {
        assert!(s.section.is_coherent(&m, &Policy::default(), &NoOntology).is_ok());
        assert!(s.section.is_closed(&m).is_ok());
    }
    // Deterministic + SectionId-ordered.
    let ids: Vec<_> = sols.iter().map(|s| s.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    let again: Vec<_> = agg.complete(&[place("source", 0)]).unwrap().iter().map(|s| s.id).collect();
    assert_eq!(ids, again);
}
