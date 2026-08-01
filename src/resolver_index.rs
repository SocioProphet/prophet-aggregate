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
use std::sync::RwLock;
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
            witness_cache: RwLock::new(BTreeMap::new()),
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
    witness_cache: RwLock<BTreeMap<(Iri, Iri), SubsumptionWitness>>,
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
        if let Some(w) = self.witness_cache.read().unwrap().get(&(general.clone(), specific.clone())) {
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
            .write().unwrap()
            .insert((general.clone(), specific.clone()), witness.clone());
        witness
    }
}

/// Failure to decode a persisted resolver index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexDecodeError {
    /// The byte string ended mid-field.
    Truncated,
    /// A concept index referenced a node out of range.
    BadNodeIndex,
    /// A length-prefixed string was not valid UTF-8.
    BadUtf8,
    /// The format magic/version did not match.
    BadHeader,
}

impl core::fmt::Display for IndexDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            IndexDecodeError::Truncated => "persisted index truncated",
            IndexDecodeError::BadNodeIndex => "persisted index referenced an out-of-range node",
            IndexDecodeError::BadUtf8 => "persisted index string was not UTF-8",
            IndexDecodeError::BadHeader => "persisted index header mismatch",
        })
    }
}

impl std::error::Error for IndexDecodeError {}

const INDEX_MAGIC: &[u8; 4] = b"PSI1"; // prophet-sheaf index, v1

impl IndexedResolver {
    /// Serialize the **precomputed** index (nodes, parent edges, and the full
    /// ancestor closure) to a compact, deterministic byte string. Concepts are
    /// interned to indices, so the encoding is small and stable.
    ///
    /// This is the "persisted transitive-closure index": build the resolver once
    /// over a large ontology, persist it (e.g. in HellGraph), and later
    /// [`from_index_bytes`](IndexedResolver::from_index_bytes) reloads it with
    /// **no recomputation** of the closure.
    #[must_use]
    pub fn to_index_bytes(&self) -> Vec<u8> {
        let nodes: Vec<&Iri> = self.nodes.iter().collect();
        let idx: BTreeMap<&Iri, u32> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, i as u32))
            .collect();

        let mut out = Vec::new();
        out.extend_from_slice(INDEX_MAGIC);
        out.push(self.strict as u8);
        put_u32(&mut out, nodes.len() as u32);
        for n in &nodes {
            put_str(&mut out, n.as_str());
        }
        // Parent edges, in node order.
        for node in &nodes {
            let ps = self.parents.get(*node);
            let count = ps.map_or(0, BTreeSet::len);
            put_u32(&mut out, count as u32);
            if let Some(ps) = ps {
                for p in ps {
                    put_u32(&mut out, idx[p]);
                }
            }
        }
        // Ancestor closure, in node order.
        for node in &nodes {
            let empty = BTreeSet::new();
            let anc = self.ancestors.get(*node).unwrap_or(&empty);
            put_u32(&mut out, anc.len() as u32);
            for a in anc {
                put_u32(&mut out, idx[a]);
            }
        }
        out
    }

    /// Reload a resolver from [`to_index_bytes`](IndexedResolver::to_index_bytes)
    /// with no closure recomputation.
    pub fn from_index_bytes(bytes: &[u8]) -> Result<Self, IndexDecodeError> {
        let mut cur = bytes;
        let header = take_idx(&mut cur, 4)?;
        if header != INDEX_MAGIC {
            return Err(IndexDecodeError::BadHeader);
        }
        let strict = get_u8_idx(&mut cur)? != 0;
        let n = get_u32_idx(&mut cur)? as usize;
        let mut nodes_vec: Vec<Iri> = Vec::with_capacity(n.min(1 << 20));
        for _ in 0..n {
            nodes_vec.push(get_str_idx(&mut cur)?);
        }
        let node_at = |i: u32| -> Result<Iri, IndexDecodeError> {
            nodes_vec
                .get(i as usize)
                .cloned()
                .ok_or(IndexDecodeError::BadNodeIndex)
        };

        let mut parents: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();
        for node in &nodes_vec {
            let count = get_u32_idx(&mut cur)? as usize;
            let mut set = BTreeSet::new();
            for _ in 0..count {
                set.insert(node_at(get_u32_idx(&mut cur)?)?);
            }
            if !set.is_empty() {
                parents.insert(node.clone(), set);
            }
        }
        let mut ancestors: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();
        for node in &nodes_vec {
            let count = get_u32_idx(&mut cur)? as usize;
            let mut set = BTreeSet::new();
            for _ in 0..count {
                set.insert(node_at(get_u32_idx(&mut cur)?)?);
            }
            ancestors.insert(node.clone(), set);
        }

        Ok(IndexedResolver {
            parents,
            ancestors,
            nodes: nodes_vec.into_iter().collect(),
            strict,
            witness_cache: RwLock::new(BTreeMap::new()),
        })
    }
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}
fn take_idx<'a>(input: &mut &'a [u8], n: usize) -> Result<&'a [u8], IndexDecodeError> {
    if input.len() < n {
        return Err(IndexDecodeError::Truncated);
    }
    let (head, tail) = input.split_at(n);
    *input = tail;
    Ok(head)
}
fn get_u8_idx(input: &mut &[u8]) -> Result<u8, IndexDecodeError> {
    Ok(take_idx(input, 1)?[0])
}
fn get_u32_idx(input: &mut &[u8]) -> Result<u32, IndexDecodeError> {
    let b = take_idx(input, 4)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}
fn get_str_idx(input: &mut &[u8]) -> Result<Iri, IndexDecodeError> {
    let len = get_u32_idx(input)? as usize;
    let b = take_idx(input, len)?;
    core::str::from_utf8(b)
        .map(Iri::from)
        .map_err(|_| IndexDecodeError::BadUtf8)
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
