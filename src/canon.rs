//! Isomorphism canonicalization of sections.
//!
//! SP-1 sorts placements by `(germ, disjunct, instance)` but does **not**
//! normalize *up to* relabeling interchangeable instances — two sections that
//! differ only by swapping the instance numbers of two otherwise-identical
//! placements have different `SectionId`s there. SP-1 flagged that normalization
//! as a search concern; this is it.
//!
//! [`canonical_form`] returns the section relabeled so that isomorphic sections
//! encode to identical bytes (and therefore share a `SectionId`). It does so
//! **exactly** — by minimizing the canonical encoding over every valid instance
//! relabeling — so it never merges two sections that are not truly isomorphic.
//! Relabelings are enumerated only *within* each `(germ, disjunct)` group (only
//! same-germ, same-disjunct placements are interchangeable), which keeps the
//! search tiny for the bounded assemblies SP-3 produces. Above a guard, it falls
//! back to a dense per-group relabeling that is still sound (never over-merges),
//! only potentially less aggressive.

use prophet_sheaf::{
    section_from_parts, Bond, Canonical, DisjunctIdx, GermId, Placement, PortRef, Section,
};
use std::collections::BTreeMap;

/// Above this many total relabelings, fall back to the dense relabeling rather
/// than exhaustively minimizing. `8! = 40320 < 50_000 <= 9!`, so any single
/// `(germ, disjunct)` group with up to 8 interchangeable instances is handled
/// exactly.
const RELABEL_CAP: u128 = 50_000;

/// The isomorphism-canonical form of a section: relabeled so isomorphic sections
/// are byte-identical.
#[must_use]
pub fn canonical_form(section: &Section) -> Section {
    let mut groups: BTreeMap<(GermId, DisjunctIdx), Vec<u32>> = BTreeMap::new();
    for p in section.placements() {
        groups
            .entry((p.germ.clone(), p.disjunct))
            .or_default()
            .push(p.instance);
    }
    for v in groups.values_mut() {
        v.sort_unstable();
        v.dedup();
    }

    let total: u128 = groups
        .values()
        .map(|v| factorial(v.len()))
        .product::<u128>();
    if total == 0 || total > RELABEL_CAP {
        return dense_relabel(section, &groups);
    }

    let group_keys: Vec<(GermId, DisjunctIdx)> = groups.keys().cloned().collect();
    let perms_per_group: Vec<Vec<Vec<u32>>> = group_keys
        .iter()
        .map(|k| permutations(&groups[k]))
        .collect();

    // Mixed-radix walk over the cartesian product of per-group permutations.
    let radices: Vec<usize> = perms_per_group.iter().map(Vec::len).collect();
    let mut counters = vec![0usize; radices.len()];
    let mut best: Option<(Vec<u8>, Section)> = None;

    loop {
        // Build the relabeling for this counter tuple.
        let mut relabel: BTreeMap<(GermId, DisjunctIdx, u32), u32> = BTreeMap::new();
        for (gi, key) in group_keys.iter().enumerate() {
            let perm = &perms_per_group[gi][counters[gi]];
            for (new_label, &old) in perm.iter().enumerate() {
                relabel.insert((key.0.clone(), key.1, old), new_label as u32);
            }
        }
        let candidate = apply(section, &relabel);
        let bytes = candidate.canonical_bytes();
        match &best {
            Some((best_bytes, _)) if *best_bytes <= bytes => {}
            _ => best = Some((bytes, candidate)),
        }

        // Increment the mixed-radix counter.
        let mut i = 0;
        loop {
            if i == counters.len() {
                return best.map(|(_, s)| s).unwrap_or_else(|| section.canonical());
            }
            counters[i] += 1;
            if counters[i] < radices[i] {
                break;
            }
            counters[i] = 0;
            i += 1;
        }
    }
}

/// The canonical `SectionId` of the isomorphism-canonical form.
#[must_use]
pub fn canonical_id(section: &Section) -> prophet_sheaf::SectionId {
    canonical_form(section).id()
}

fn apply(section: &Section, relabel: &BTreeMap<(GermId, DisjunctIdx, u32), u32>) -> Section {
    let relabel_placement = |p: &Placement| Placement {
        germ: p.germ.clone(),
        disjunct: p.disjunct,
        instance: relabel[&(p.germ.clone(), p.disjunct, p.instance)],
    };
    let relabel_ref = |r: &PortRef| PortRef {
        placement: relabel_placement(&r.placement),
        port: r.port,
    };
    let placements = section.placements().iter().map(relabel_placement).collect();
    let bonds = section
        .bonds()
        .iter()
        .map(|b| Bond {
            from: relabel_ref(&b.from),
            to: relabel_ref(&b.to),
            witness: b.witness.clone(),
        })
        .collect();
    section_from_parts(placements, bonds)
}

/// Dense fallback: relabel each group's instances by their sorted order to
/// `0..k`. Sound (a valid relabeling), but not guaranteed minimal.
fn dense_relabel(section: &Section, groups: &BTreeMap<(GermId, DisjunctIdx), Vec<u32>>) -> Section {
    let mut relabel: BTreeMap<(GermId, DisjunctIdx, u32), u32> = BTreeMap::new();
    for (key, instances) in groups {
        for (new_label, &old) in instances.iter().enumerate() {
            relabel.insert((key.0.clone(), key.1, old), new_label as u32);
        }
    }
    apply(section, &relabel)
}

fn factorial(n: usize) -> u128 {
    (1..=n as u128).product::<u128>().max(1)
}

/// All permutations of `items` (recursive; only called for small groups).
fn permutations(items: &[u32]) -> Vec<Vec<u32>> {
    if items.is_empty() {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for (i, &x) in items.iter().enumerate() {
        let mut rest = items.to_vec();
        rest.remove(i);
        for mut tail in permutations(&rest) {
            let mut perm = Vec::with_capacity(items.len());
            perm.push(x);
            perm.append(&mut tail);
            out.push(perm);
        }
    }
    out
}
