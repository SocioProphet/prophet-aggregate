//! Bounded enumeration of closed, coherent completions of a seed assembly.
//!
//! This is the search SP-1 refused to do. Given a manifest, a resolver, a policy,
//! and a **seed** (one or more placements you want to build around), it finds
//! every way to complete the seed into a **closed, coherent section** by adding
//! supporting germs from the manifest and bonding their ports — up to a size
//! bound, deduplicated up to isomorphism, in deterministic order.
//!
//! The search is a backtracking one. At each step it takes the first still-open
//! `Required` port and enumerates every legal way to satisfy it: bond it to an
//! existing compatible open port, or introduce a new placement that offers one.
//! Coherence is checked at every step (mid-construction validation is exactly
//! what SP-1's coherent/closed split exists for), so illegal branches are pruned
//! immediately. States already visited — identified by their isomorphism-
//! canonical id — are skipped, which both bounds the work and collapses the
//! bond-order permutations that would otherwise explode the tree.
//!
//! Every solution is validated closed *and* coherent by `prophet-sheaf` before it
//! is recorded, so the enumerator cannot emit an illegal assembly even if the
//! search logic has a bug.

use crate::canon::{canonical_form, canonical_id};
use prophet_sheaf::{
    can_mate, section_from_parts, Bond, CoherenceError, Direction, DisjunctIdx, EpistemicLevel,
    GermId, Manifest, MatingRefusal, Placement, Policy, Port, PortRef, ResolverError, Section,
    SectionId, Subsumption,
};
use std::collections::{BTreeMap, BTreeSet};

/// A size bound on the search. Without one, cycles make the space infinite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bound {
    /// The maximum number of placements a solution (or any partial state) may
    /// contain.
    pub max_placements: usize,
}

impl Bound {
    /// A bound of `n` placements.
    #[must_use]
    pub fn placements(n: usize) -> Self {
        Bound { max_placements: n }
    }
}

/// A completed assembly: a closed, coherent, isomorphism-canonical section and
/// its receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct Solution {
    /// The isomorphism-canonical section.
    pub section: Section,
    /// Its content identity.
    pub id: SectionId,
    /// The assembly's composed epistemic standing: the **meet** of its germs'
    /// declared levels (from `prophet-truth`, via `prophet-sheaf`). Composition
    /// degrades — an assembly is no better grounded than its weakest germ — and
    /// `Rejected` is absorbing, so any assembly resting on a rejected germ is
    /// itself `Rejected`.
    pub epistemic: EpistemicLevel,
}

/// Why a search could not run to completion.
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateError {
    /// A seed placement names a germ or disjunct absent from the manifest.
    UnknownSeed {
        /// The offending placement.
        placement: Placement,
    },
    /// The subsumption resolver could not answer. The search is abandoned rather
    /// than silently returning fewer solutions — an outage is not a "no".
    Resolver(ResolverError),
}

impl core::fmt::Display for AggregateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AggregateError::UnknownSeed { placement } => {
                write!(f, "unknown seed placement {placement:?}")
            }
            AggregateError::Resolver(e) => write!(f, "resolver unavailable during search: {e}"),
        }
    }
}

impl std::error::Error for AggregateError {}

/// The aggregator: a manifest, a resolver, a policy, and a bound.
pub struct Aggregator<'a> {
    manifest: &'a Manifest,
    resolver: &'a dyn Subsumption,
    policy: Policy,
    bound: Bound,
}

impl<'a> Aggregator<'a> {
    /// Build an aggregator.
    #[must_use]
    pub fn new(
        manifest: &'a Manifest,
        resolver: &'a dyn Subsumption,
        policy: Policy,
        bound: Bound,
    ) -> Self {
        Aggregator {
            manifest,
            resolver,
            policy,
            bound,
        }
    }

    /// Enumerate every closed, coherent completion of `seed`, deduplicated up to
    /// isomorphism and returned in deterministic order (by `SectionId`).
    pub fn complete(&self, seed: &[Placement]) -> Result<Vec<Solution>, AggregateError> {
        for p in seed {
            if self.resolve_placement_ok(p).is_none() {
                return Err(AggregateError::UnknownSeed {
                    placement: p.clone(),
                });
            }
        }

        let mut solutions: BTreeMap<SectionId, Section> = BTreeMap::new();
        let mut visited: BTreeSet<SectionId> = BTreeSet::new();
        self.expand(seed.to_vec(), Vec::new(), &mut solutions, &mut visited)?;

        Ok(solutions
            .into_iter()
            .map(|(id, section)| {
                let epistemic = self.composed_epistemic(&section);
                Solution {
                    section,
                    id,
                    epistemic,
                }
            })
            .collect())
    }

    /// The meet of the declared epistemic levels of every germ placed in the
    /// section — the composition rule from `prophet-truth`: an assembly is no
    /// more grounded than its weakest part, and inherits `Rejected` absorbingly.
    fn composed_epistemic(&self, section: &Section) -> EpistemicLevel {
        let mut level = EpistemicLevel::Proved; // identity for meet (top)
        for p in section.placements() {
            if let Some(germ) = self.manifest.get(&p.germ) {
                level = level.meet(germ.epistemic);
            }
        }
        level
    }

    fn expand(
        &self,
        placements: Vec<Placement>,
        bonds: Vec<Bond>,
        solutions: &mut BTreeMap<SectionId, Section>,
        visited: &mut BTreeSet<SectionId>,
    ) -> Result<(), AggregateError> {
        let section = section_from_parts(placements.clone(), bonds.clone());
        let cid = canonical_id(&section);
        if !visited.insert(cid) {
            return Ok(()); // already explored this state (up to isomorphism)
        }

        // Prune incoherent branches; abort the whole search on a resolver outage.
        match section.is_coherent(self.manifest, &self.policy, self.resolver) {
            Ok(_) => {}
            Err(CoherenceError::Resolver(e)) => return Err(AggregateError::Resolver(e)),
            Err(CoherenceError::Incoherent(_)) => return Ok(()),
        }

        // A closed, coherent section is a solution — record and stop growing it.
        let target = match section.is_closed(self.manifest) {
            Ok(()) => {
                solutions.insert(cid, canonical_form(&section));
                return Ok(());
            }
            Err(unsatisfied) => unsatisfied.into_iter().next().unwrap(),
        };

        let tref = PortRef {
            placement: target.placement.clone(),
            port: target.port,
        };
        let tport = self.resolve(&tref).expect("target port resolves");

        // Option A: bond the target to an existing compatible open port.
        for pref in self.existing_ports(&section) {
            if pref == tref {
                continue;
            }
            let Some(pport) = self.resolve(&pref) else {
                continue;
            };
            match can_mate(&tport, &pport, self.resolver) {
                Ok(witness) => {
                    if self.has_capacity(&bonds, &tref, &tport)
                        && self.has_capacity(&bonds, &pref, &pport)
                        && !bonded(&bonds, &tref, &pref)
                    {
                        let bond = oriented_bond(&tref, &tport, &pref, witness);
                        let mut nb = bonds.clone();
                        nb.push(bond);
                        self.expand(placements.clone(), nb, solutions, visited)?;
                    }
                }
                Err(MatingRefusal::ResolverUnavailable { error }) => {
                    return Err(AggregateError::Resolver(error));
                }
                Err(_) => {}
            }
        }

        // Option B: introduce a new placement that offers a mating port.
        let within_instance_cap = match self.policy.max_instances {
            Some(m) => (placements.len() as u32) < m,
            None => true,
        };
        if placements.len() < self.bound.max_placements && within_instance_cap {
            for (gid, germ) in self.manifest.iter() {
                for (di, disjunct) in germ.disjuncts.iter().enumerate() {
                    let fresh = next_instance(&placements, gid, di as u32);
                    for (j, pport) in disjunct.ports().iter().enumerate() {
                        match can_mate(&tport, pport, self.resolver) {
                            Ok(witness) => {
                                if !self.has_capacity(&bonds, &tref, &tport) {
                                    continue;
                                }
                                let placement = Placement {
                                    germ: gid.clone(),
                                    disjunct: DisjunctIdx(di as u32),
                                    instance: fresh,
                                };
                                let pref = PortRef {
                                    placement: placement.clone(),
                                    port: j as u32,
                                };
                                let bond = oriented_bond(&tref, &tport, &pref, witness);
                                let mut np = placements.clone();
                                np.push(placement);
                                let mut nb = bonds.clone();
                                nb.push(bond);
                                self.expand(np, nb, solutions, visited)?;
                            }
                            Err(MatingRefusal::ResolverUnavailable { error }) => {
                                return Err(AggregateError::Resolver(error));
                            }
                            Err(_) => {}
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn resolve(&self, r: &PortRef) -> Option<Port> {
        let germ = self.manifest.get(&r.placement.germ)?;
        let disjunct = germ.disjunct(r.placement.disjunct)?;
        disjunct.ports().get(r.port as usize).cloned()
    }

    fn resolve_placement_ok(&self, p: &Placement) -> Option<()> {
        self.manifest.get(&p.germ)?.disjunct(p.disjunct).map(|_| ())
    }

    /// Every port of every placement currently in the section, canonical order.
    fn existing_ports(&self, section: &Section) -> Vec<PortRef> {
        let mut out = Vec::new();
        for p in section.placements() {
            if let Some(germ) = self.manifest.get(&p.germ) {
                if let Some(d) = germ.disjunct(p.disjunct) {
                    for j in 0..d.ports().len() as u32 {
                        out.push(PortRef {
                            placement: p.clone(),
                            port: j,
                        });
                    }
                }
            }
        }
        out
    }

    fn has_capacity(&self, bonds: &[Bond], r: &PortRef, port: &Port) -> bool {
        match port.arity.max_bonds() {
            Some(max) => bond_count(bonds, r) < max,
            None => true,
        }
    }
}

fn bond_count(bonds: &[Bond], r: &PortRef) -> u32 {
    bonds.iter().filter(|b| &b.from == r || &b.to == r).count() as u32
}

fn bonded(bonds: &[Bond], a: &PortRef, b: &PortRef) -> bool {
    bonds
        .iter()
        .any(|x| (&x.from == a && &x.to == b) || (&x.from == b && &x.to == a))
}

/// Orient a bond Out→In given the target port `tport` and its `tref`.
fn oriented_bond(
    tref: &PortRef,
    tport: &Port,
    pref: &PortRef,
    witness: prophet_sheaf::MatingWitness,
) -> Bond {
    if tport.polarity.dir == Direction::Out {
        Bond {
            from: tref.clone(),
            to: pref.clone(),
            witness,
        }
    } else {
        Bond {
            from: pref.clone(),
            to: tref.clone(),
            witness,
        }
    }
}

fn next_instance(placements: &[Placement], germ: &GermId, disjunct: u32) -> u32 {
    placements
        .iter()
        .filter(|p| &p.germ == germ && p.disjunct == DisjunctIdx(disjunct))
        .map(|p| p.instance + 1)
        .max()
        .unwrap_or(0)
}
