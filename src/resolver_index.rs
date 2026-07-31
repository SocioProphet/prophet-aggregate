//! A precomputed-transitive-closure [`Subsumption`] resolver.
//!
//! SP-1 kept the [`Subsumption`] trait batch-friendly and cheap to memoize on
//! purpose: SP-3 calls `can_mate` — and therefore `subsumes` — in a hot
//! enumeration loop, and a resolver doing live graph traversal per call would
//! make aggregation non-viable. This is the index SP-1 deferred here.
//!
//! Construction walks the concept hierarchy once and stores, for every concept,
//! the full set of its ancestors (its transitive closure). A `subsumes` query is
//! then an **O(1) set membership test**. The justifying witness chain is
//! reconstructed by a short, deterministic climb — and memoized — so even that
//! is paid at most once per (general, specific) pair.
//!
//! The hierarchy is a DAG (multi-parent concepts are allowed). Nothing here
//! ships ontology data; edges are supplied by the caller (from KKO, a commercial
//! ontology, or a test fixture), keeping the MIT surface free of CC-BY content.

use prophet_sheaf::{Iri, ResolverError, Subsumption, SubsumptionWitness};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Builder for an [`IndexedResolver`]: accumulate `child ⊑ parent` edges, then
/// [`build`](ResolverBuilder::build) to precompute the closure.
#[derive(Debug, Default, Clone)]
pub struct ResolverBuilder {
    // child -> set of direct parents.
    parents: BTreeMap<Iri, BTreeSet<Iri>>,
    nodes: BTreeSet<Iri>,
    strict: bool,
}

impl ResolverBuilder {
    /// A new, empty builder.
    #[must_use]
    pub fn new() -> Self {
        ResolverBuilder::default()
    }

    /// In strict mode, a query naming a concept that is not in the hierarchy
    /// yields [`ResolverError::UnknownConcept`] rather than being treated as an
    /// isolated node. Off by default (lenient: unseen concepts subsume only
    /// themselves).
    #[must_use]
    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Add a `child ⊑ parent` edge (child is the more specific concept).
    #[must_use]
    pub fn edge(mut self, child: &str, parent: &str) -> Self {
        let c = Iri::from(child);
        let p = Iri::from(parent);
        self.nodes.insert(c.clone());
        self.nodes.insert(p.clone());
        self.parents.entry(c).or_default().insert(p);
        self
    }

    /// Load `child ⊑ parent` edges from a simple text export — one edge per line,
    /// `child<sep>parent`, where `<sep>` is a tab or comma. Blank lines and lines
    /// beginning with `#` are ignored.
    ///
    /// This is how a real ontology (an exported KKO / KBpedia `subClassOf`
    /// relation, or a commercial hierarchy) is fed in **at runtime** — the data
    /// stays out of the crate, so the MIT surface never carries CC-BY content.
    /// Lines that do not parse are skipped and their 1-based numbers returned, so
    /// a malformed export is visible rather than silently partial.
    #[must_use]
    pub fn load_edges(mut self, text: &str) -> (Self, Vec<usize>) {
        let mut skipped = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = if line.contains('\t') {
                line.splitn(2, '\t').collect()
            } else {
                line.splitn(2, ',').collect()
            };
            match parts.as_slice() {
                [child, parent] if !child.trim().is_empty() && !parent.trim().is_empty() => {
                    self = self.edge(child.trim(), parent.trim());
                }
                _ => skipped.push(i + 1),
            }
        }
        (self, skipped)
    }

    /// Load a KKO (KBpedia Knowledge Ontology) Turtle/N3 export, adding every
    /// `rdfs:subClassOf` edge among named classes. The ontology *data* is
    /// supplied by the caller at runtime (it is CC BY 4.0 and lives in the
    /// HellGraph layer, not this MIT crate); this only parses it. Returns the
    /// builder and import statistics.
    #[must_use]
    pub fn load_kko(self, turtle: &str) -> (Self, crate::kko::KkoStats) {
        let (edges, stats) = crate::kko::subclass_edges(turtle);
        let mut b = self;
        for (child, parent) in edges {
            b = b.edge(child.as_str(), parent.as_str());
        }
        (b, stats)
    }

    /// Precompute the transitive closure and return the resolver.
    #[must_use]
    pub fn build(self) -> IndexedResolver {
        let mut ancestors: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();
        for node in &self.nodes {
            // BFS over parent edges; collect every reachable ancestor.
            let mut seen = BTreeSet::new();
            let mut queue = VecDeque::new();
            queue.push_back(node.clone());
            while let Some(cur) = queue.pop_front() {
                if let Some(ps) = self.parents.get(&cur) {
                    for p in ps {
                        if seen.insert(p.clone()) {
                            queue.push_back(p.clone());
                        }
                    }
                }
            }
            ancestors.insert(node.clone(), seen);
        }
        IndexedResolver {
            parents: self.parents,
            ancestors,
            nodes: self.nodes,
            strict: self.strict,
            witness_cache: RefCell::new(BTreeMap::new()),
        }
    }
}

/// A [`Subsumption`] resolver backed by a precomputed ancestor closure.
#[derive(Debug)]
pub struct IndexedResolver {
    parents: BTreeMap<Iri, BTreeSet<Iri>>,
    ancestors: BTreeMap<Iri, BTreeSet<Iri>>,
    nodes: BTreeSet<Iri>,
    strict: bool,
    witness_cache: RefCell<BTreeMap<(Iri, Iri), SubsumptionWitness>>,
}

impl IndexedResolver {
    /// The number of concepts in the hierarchy.
    #[must_use]
    pub fn concept_count(&self) -> usize {
        self.nodes.len()
    }

    fn known(&self, iri: &Iri) -> bool {
        self.nodes.contains(iri)
    }

    /// Reconstruct the shortest `specific → general` path, choosing parents in
    /// sorted order for determinism. Assumes `general` is an ancestor of
    /// `specific` (or equal).
    fn witness_path(&self, general: &Iri, specific: &Iri) -> SubsumptionWitness {
        if let Some(w) = self.witness_cache.borrow().get(&(general.clone(), specific.clone())) {
            return w.clone();
        }
        // BFS from specific to general with predecessor pointers.
        let mut pred: BTreeMap<Iri, Iri> = BTreeMap::new();
        let mut seen: BTreeSet<Iri> = BTreeSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(specific.clone());
        seen.insert(specific.clone());
        while let Some(cur) = queue.pop_front() {
            if &cur == general {
                break;
            }
            if let Some(ps) = self.parents.get(&cur) {
                for p in ps {
                    // BTreeSet iterates in sorted order → deterministic path.
                    if seen.insert(p.clone()) {
                        pred.insert(p.clone(), cur.clone());
                        queue.push_back(p.clone());
                    }
                }
            }
        }
        // Walk predecessors back from general to specific, then reverse.
        let mut chain = vec![general.clone()];
        let mut cur = general.clone();
        while &cur != specific {
            match pred.get(&cur) {
                Some(prev) => {
                    chain.push(prev.clone());
                    cur = prev.clone();
                }
                None => break, // should not happen when general is an ancestor
            }
        }
        chain.reverse(); // now specific → general
        let witness = SubsumptionWitness { chain };
        self.witness_cache
            .borrow_mut()
            .insert((general.clone(), specific.clone()), witness.clone());
        witness
    }
}

impl Subsumption for IndexedResolver {
    fn subsumes(
        &self,
        general: &Iri,
        specific: &Iri,
    ) -> Result<Option<SubsumptionWitness>, ResolverError> {
        if self.strict {
            if !self.known(general) {
                return Err(ResolverError::UnknownConcept {
                    iri: general.clone(),
                });
            }
            if !self.known(specific) {
                return Err(ResolverError::UnknownConcept {
                    iri: specific.clone(),
                });
            }
        }
        // Reflexive: every concept subsumes itself.
        if general == specific {
            return Ok(Some(SubsumptionWitness {
                chain: vec![specific.clone()],
            }));
        }
        // O(1) decision via the precomputed closure.
        let subsumed = self
            .ancestors
            .get(specific)
            .is_some_and(|a| a.contains(general));
        if subsumed {
            Ok(Some(self.witness_path(general, specific)))
        } else {
            Ok(None)
        }
    }
}
