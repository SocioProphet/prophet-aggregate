//! Completeness: the search finds *exactly* the closed-coherent, seed-connected
//! sections a brute-force oracle finds (both deduplicated up to isomorphism), and
//! isomorphism-canonicalization is permutation-invariant.

mod common;
use common::*;

use prophet_aggregate::{canonical_id, Aggregator, Bound, ResolverBuilder};
use prophet_sheaf::{
    can_mate, section_from_parts, Bond, Direction, Manifest, NoOntology, Placement, Policy, Port,
    PortRef, Section, SectionId, Subsumption,
};
use std::collections::BTreeSet;

fn resolve(m: &Manifest, r: &PortRef) -> Option<Port> {
    m.get(&r.placement.germ)?
        .disjunct(r.placement.disjunct)?
        .ports()
        .get(r.port as usize)
        .cloned()
}

fn all_refs(m: &Manifest, placements: &[Placement]) -> Vec<PortRef> {
    let mut refs = Vec::new();
    for p in placements {
        if let Some(g) = m.get(&p.germ) {
            if let Some(d) = g.disjunct(p.disjunct) {
                for j in 0..d.ports().len() as u32 {
                    refs.push(PortRef {
                        placement: p.clone(),
                        port: j,
                    });
                }
            }
        }
    }
    refs
}

/// Undirected connectivity: is every placement reachable from the seed set via
/// bonds? (The search only ever grows a connected section, so the oracle filters
/// to the same class.)
fn connected(section: &Section, seed: &[Placement]) -> bool {
    let placements: Vec<&Placement> = section.placements().iter().collect();
    if placements.is_empty() {
        return true;
    }
    let idx = |p: &Placement| placements.iter().position(|q| *q == p);
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); placements.len()];
    for b in section.bonds() {
        if let (Some(u), Some(v)) = (idx(&b.from.placement), idx(&b.to.placement)) {
            adj[u].push(v);
            adj[v].push(u);
        }
    }
    let mut seen = vec![false; placements.len()];
    let mut stack: Vec<usize> = seed.iter().filter_map(&idx).collect();
    if stack.is_empty() {
        return false;
    }
    for &s in &stack {
        seen[s] = true;
    }
    while let Some(n) = stack.pop() {
        for &w in &adj[n] {
            if !seen[w] {
                seen[w] = true;
                stack.push(w);
            }
        }
    }
    seen.into_iter().all(|b| b)
}

/// Brute-force oracle: every subset of `extras` (plus the seed), every subset of
/// the compatible bonds among them, filtered to coherent + closed + seed-
/// connected, deduplicated by isomorphism-canonical id.
fn brute_force(
    m: &Manifest,
    r: &dyn Subsumption,
    seed: &[Placement],
    extras: &[Placement],
    bound: usize,
) -> BTreeSet<SectionId> {
    let mut out = BTreeSet::new();
    assert!(extras.len() <= 16);
    for pmask in 0u32..(1 << extras.len()) {
        let mut placements = seed.to_vec();
        for (i, e) in extras.iter().enumerate() {
            if pmask >> i & 1 == 1 {
                placements.push(e.clone());
            }
        }
        if placements.len() > bound {
            continue;
        }
        // Candidate bonds: every mating pair among the ports of these placements.
        let refs = all_refs(m, &placements);
        let mut cand: Vec<Bond> = Vec::new();
        for i in 0..refs.len() {
            for j in (i + 1)..refs.len() {
                let (Some(pi), Some(pj)) = (resolve(m, &refs[i]), resolve(m, &refs[j])) else {
                    continue;
                };
                if let Ok(w) = can_mate(&pi, &pj, r) {
                    // Orient Out→In.
                    let bond = if pi.polarity.dir == Direction::Out {
                        Bond { from: refs[i].clone(), to: refs[j].clone(), witness: w }
                    } else {
                        Bond { from: refs[j].clone(), to: refs[i].clone(), witness: w }
                    };
                    cand.push(bond);
                }
            }
        }
        assert!(cand.len() <= 20, "too many candidate bonds to brute-force");
        for bmask in 0u32..(1 << cand.len()) {
            let bonds: Vec<Bond> = cand
                .iter()
                .enumerate()
                .filter(|(i, _)| bmask >> i & 1 == 1)
                .map(|(_, b)| b.clone())
                .collect();
            let section = section_from_parts(placements.clone(), bonds);
            if section.is_coherent(m, &Policy::default(), r).is_ok()
                && section.is_closed(m).is_ok()
                && connected(&section, seed)
            {
                out.insert(canonical_id(&section));
            }
        }
    }
    out
}

fn search_ids(
    m: &Manifest,
    r: &dyn Subsumption,
    seed: &[Placement],
    bound: usize,
) -> BTreeSet<SectionId> {
    Aggregator::new(m, r, Policy::default(), Bound::placements(bound))
        .complete(seed)
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect()
}

#[test]
fn search_matches_brute_force_dataflow() {
    let m = dataflow_manifest();
    let seed = vec![place("source", 0)];
    let extras = vec![place("sink", 0), place("transform", 0), place("transform", 1)];
    for bound in 2..=4 {
        let search = search_ids(&m, &NoOntology, &seed, bound);
        let brute = brute_force(&m, &NoOntology, &seed, &extras, bound);
        assert_eq!(search, brute, "mismatch at bound {bound}");
    }
}

#[test]
fn search_matches_brute_force_with_hub_sink_seed() {
    let m = dataflow_with_hub();
    let seed = vec![place("sink", 0)];
    let extras = vec![place("source", 0), place("hub", 0), place("transform", 0)];
    for bound in 1..=3 {
        let search = search_ids(&m, &NoOntology, &seed, bound);
        let brute = brute_force(&m, &NoOntology, &seed, &extras, bound);
        assert_eq!(search, brute, "mismatch at bound {bound}");
    }
}

#[test]
fn search_matches_brute_force_across_subsumption() {
    let m = concept_manifest();
    let r = ResolverBuilder::new().edge("ex:SSN", "ex:PII").build();
    let seed = vec![place("ssn-source", 0)];
    let extras = vec![place("pii-sink", 0)];
    for bound in 1..=3 {
        let search = search_ids(&m, &r, &seed, bound);
        let brute = brute_force(&m, &r, &seed, &extras, bound);
        assert_eq!(search, brute, "mismatch at bound {bound}");
    }
}

// ---- canonicalization is permutation-invariant -----------------------------

fn cap_witness() -> prophet_sheaf::MatingWitness {
    prophet_sheaf::MatingWitness::capability("d", prophet_sheaf::PortClass::Flow)
}

fn pref(g: &str, inst: u32, port: u32) -> PortRef {
    PortRef {
        placement: place(g, inst),
        port,
    }
}

#[test]
fn canonical_id_is_invariant_under_instance_relabeling() {
    // source→transform→sink, built with two different instance labelings, must
    // share a canonical id.
    let build = |s: u32, t: u32, k: u32| {
        section_from_parts(
            vec![place("source", s), place("transform", t), place("sink", k)],
            vec![
                Bond { from: pref("source", s, SOURCE_OUT), to: pref("transform", t, TRANSFORM_IN), witness: cap_witness() },
                Bond { from: pref("transform", t, TRANSFORM_OUT), to: pref("sink", k, SINK_IN), witness: cap_witness() },
            ],
        )
    };
    let a = build(0, 0, 0);
    let b = build(7, 3, 9);
    assert_eq!(canonical_id(&a), canonical_id(&b));
}

#[test]
fn canonical_id_distinguishes_non_isomorphic_sections() {
    // A 2-hop pipeline and a 1-hop pipeline are not isomorphic.
    let two_hop = section_from_parts(
        vec![place("source", 0), place("transform", 0), place("sink", 0)],
        vec![
            Bond { from: pref("source", 0, SOURCE_OUT), to: pref("transform", 0, TRANSFORM_IN), witness: cap_witness() },
            Bond { from: pref("transform", 0, TRANSFORM_OUT), to: pref("sink", 0, SINK_IN), witness: cap_witness() },
        ],
    );
    let one_hop = section_from_parts(
        vec![place("source", 0), place("sink", 0)],
        vec![Bond { from: pref("source", 0, SOURCE_OUT), to: pref("sink", 0, SINK_IN), witness: cap_witness() }],
    );
    assert_ne!(canonical_id(&two_hop), canonical_id(&one_hop));
}

#[test]
fn interchangeable_fanout_targets_collapse() {
    // hub bonded to sink#0 and sink#1: swapping the two sinks is an isomorphism,
    // so both labelings share a canonical id.
    let m = dataflow_with_hub();
    let build = |a: u32, b: u32| {
        let s = section_from_parts(
            vec![place("hub", 0), place("sink", a), place("sink", b)],
            vec![
                Bond { from: pref("hub", 0, HUB_OUT), to: pref("sink", a, SINK_IN), witness: cap_witness() },
                Bond { from: pref("hub", 0, HUB_OUT), to: pref("sink", b, SINK_IN), witness: cap_witness() },
            ],
        );
        assert!(s.is_coherent(&m, &Policy::default(), &NoOntology).is_ok());
        s
    };
    assert_eq!(canonical_id(&build(0, 1)), canonical_id(&build(1, 0)));
}
