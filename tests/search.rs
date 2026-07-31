//! The enumerator: soundness, expected completions, determinism, dedup,
//! subsumption-driven completion, and resolver-outage handling.

mod common;
use common::*;

use prophet_aggregate::{AggregateError, Aggregator, Bound, ResolverBuilder};
use prophet_sheaf::{NoOntology, Policy};

fn assert_all_sound(manifest: &prophet_sheaf::Manifest, r: &dyn prophet_sheaf::Subsumption, sols: &[prophet_aggregate::Solution]) {
    for s in sols {
        assert!(
            s.section.is_coherent(manifest, &Policy::default(), r).is_ok(),
            "solution not coherent: {:?}",
            s.section
        );
        assert!(
            s.section.is_closed(manifest).is_ok(),
            "solution not closed: {:?}",
            s.section
        );
        assert_eq!(s.id, s.section.id(), "reported id does not match section");
    }
}

#[test]
fn every_solution_is_closed_and_coherent() {
    let m = dataflow_with_hub();
    let agg = Aggregator::new(&m, &NoOntology, Policy::default(), Bound::placements(5));
    let sols = agg.complete(&[place("source", 0)]).unwrap();
    assert!(!sols.is_empty());
    assert_all_sound(&m, &NoOntology, &sols);
}

#[test]
fn finds_exactly_the_bounded_pipelines() {
    // From a source, the closed completions are the pipelines
    // source→sink, source→transform→sink, source→transform→transform→sink,
    // i.e. one per transform count that fits the 4-placement bound.
    let m = dataflow_manifest();
    let agg = Aggregator::new(&m, &NoOntology, Policy::default(), Bound::placements(4));
    let sols = agg.complete(&[place("source", 0)]).unwrap();
    assert_all_sound(&m, &NoOntology, &sols);
    assert_eq!(sols.len(), 3, "expected 3 pipelines, got {}", sols.len());

    // Placement counts are 2, 3, 4.
    let mut sizes: Vec<usize> = sols.iter().map(|s| s.section.placements().len()).collect();
    sizes.sort_unstable();
    assert_eq!(sizes, vec![2, 3, 4]);
}

#[test]
fn search_is_deterministic() {
    let m = dataflow_manifest();
    let agg = Aggregator::new(&m, &NoOntology, Policy::default(), Bound::placements(5));
    let a = agg.complete(&[place("source", 0)]).unwrap();
    let b = agg.complete(&[place("source", 0)]).unwrap();
    let ids_a: Vec<_> = a.iter().map(|s| s.id).collect();
    let ids_b: Vec<_> = b.iter().map(|s| s.id).collect();
    assert_eq!(ids_a, ids_b);
    // Output is ordered by SectionId.
    let mut sorted = ids_a.clone();
    sorted.sort();
    assert_eq!(ids_a, sorted);
}

#[test]
fn results_are_pairwise_non_isomorphic() {
    let m = dataflow_with_hub();
    let agg = Aggregator::new(&m, &NoOntology, Policy::default(), Bound::placements(5));
    let sols = agg.complete(&[place("source", 0)]).unwrap();
    let mut ids: Vec<_> = sols.iter().map(|s| s.id).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate isomorphism classes in output");
}

#[test]
fn a_germ_with_no_required_ports_is_immediately_closed() {
    // hub's only port is Multi{min:0}, so a lone hub is already closed.
    let m = dataflow_with_hub();
    let agg = Aggregator::new(&m, &NoOntology, Policy::default(), Bound::placements(4));
    let sols = agg.complete(&[place("hub", 0)]).unwrap();
    assert!(sols.iter().any(|s| s.section.placements().len() == 1));
    assert_all_sound(&m, &NoOntology, &sols);
}

#[test]
fn completes_across_subsumption() {
    // ssn-source offers ex:SSN; pii-sink requires ex:PII; SSN ⊑ PII, so the
    // aggregator completes the source with the sink.
    let m = concept_manifest();
    let r = ResolverBuilder::new().edge("ex:SSN", "ex:PII").build();
    let agg = Aggregator::new(&m, &r, Policy::default(), Bound::placements(4));
    let sols = agg.complete(&[place("ssn-source", 0)]).unwrap();
    assert_eq!(sols.len(), 1, "expected one subsumption completion");
    assert_all_sound(&m, &r, &sols);
    assert_eq!(sols[0].section.placements().len(), 2);
}

#[test]
fn solution_epistemic_is_the_meet_of_its_germs() {
    use prophet_sheaf::{EpistemicLevel, GermId};

    // transform is only Speculative; source and sink are Empirical.
    let m = graded_manifest(EpistemicLevel::Speculative);
    let agg = Aggregator::new(&m, &NoOntology, Policy::default(), Bound::placements(4));
    let sols = agg.complete(&[place("source", 0)]).unwrap();
    assert!(!sols.is_empty());
    for s in &sols {
        let uses_transform = s
            .section
            .placements()
            .iter()
            .any(|p| p.germ == GermId::from("transform"));
        if uses_transform {
            // Empirical ∧ Speculative ∧ Empirical = Speculative.
            assert_eq!(s.epistemic, EpistemicLevel::Speculative);
        } else {
            // source → sink: Empirical ∧ Empirical = Empirical.
            assert_eq!(s.epistemic, EpistemicLevel::Empirical);
        }
    }

    // A Rejected germ is absorbing: any pipeline through it is Rejected.
    let m2 = graded_manifest(EpistemicLevel::Rejected);
    let agg2 = Aggregator::new(&m2, &NoOntology, Policy::default(), Bound::placements(4));
    for s in agg2.complete(&[place("source", 0)]).unwrap() {
        let uses_transform = s
            .section
            .placements()
            .iter()
            .any(|p| p.germ == GermId::from("transform"));
        if uses_transform {
            assert_eq!(s.epistemic, EpistemicLevel::Rejected);
        }
    }
}

#[test]
fn unknown_seed_is_an_error() {
    let m = dataflow_manifest();
    let agg = Aggregator::new(&m, &NoOntology, Policy::default(), Bound::placements(4));
    let err = agg.complete(&[place("nonexistent", 0)]).unwrap_err();
    assert!(matches!(err, AggregateError::UnknownSeed { .. }));
}

#[test]
fn resolver_outage_aborts_the_search() {
    // A concept manifest driven by a resolver that is always down: satisfying a
    // concept port requires the resolver, so the search aborts with a resolver
    // error rather than silently returning no completions.
    let m = concept_manifest();
    let agg = Aggregator::new(&m, &NoOntology, Policy::default(), Bound::placements(4));
    let err = agg.complete(&[place("ssn-source", 0)]).unwrap_err();
    assert!(matches!(err, AggregateError::Resolver(_)));
}
