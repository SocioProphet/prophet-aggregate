# prophet-aggregate

**Search over the manifest sheaf.**

[`prophet-sheaf`](https://github.com/SocioProphet/prophet-sheaf) answers *"is this
assembly legal?"*. This crate answers the harder question: *"what are all the
legal assemblies?"* Given a catalogue of capabilities (a manifest), a starting
point (a **seed** — the thing you want to build), and a size budget, it finds
**every closed, coherent way to complete the seed** — each one deduplicated,
deterministic, and carrying a receipt.

It is the SP-3 aggregator that `prophet-sheaf` (SP-1) was deliberately built to
unblock: SP-1 decides legality and refuses to search; SP-3 searches.

---

## What it does, in plain terms

You hand it a seed capability. It grows that seed into finished assemblies by
plugging in supporting capabilities from the catalogue — wiring an offered port
to a required one only where the two genuinely mate — until every **Required**
port is satisfied. Each finished assembly is a *closed, coherent section*. You
get back the full set of them.

Two assemblies that are the same shape with the parts merely renumbered are
**the same answer**, and are returned once. Run it twice on the same inputs and
you get the identical list, in the identical order. Every answer is re-checked
by `prophet-sheaf` before it is handed back, so the search cannot return an
illegal assembly even if the search itself had a bug.

```rust
use prophet_aggregate::{Aggregator, Bound, ResolverBuilder};

let resolver = ResolverBuilder::new().edge("ex:SSN", "ex:PII").build();
let agg = Aggregator::new(&manifest, &resolver, Policy::default(), Bound::placements(6));
let solutions = agg.complete(&[seed_placement])?;   // Vec<Solution>, each with a SectionId
```

---

## The two pieces SP-1 left for here

### 1. A fast subsumption resolver

Concept ports mate by **subsumption** (`SSN ⊑ PII`), decided by a resolver. SP-1
kept that resolver interface cheap-to-index on purpose, because search calls it
in a tight loop — a resolver doing live graph traversal per call would make
aggregation non-viable. [`IndexedResolver`] is that index: it walks the concept
hierarchy **once** at construction and precomputes every concept's ancestors, so
each subsumption check is an O(1) set lookup. The justifying witness chain is
still produced (and memoized) — an allow is never a bare "true".

No ontology data ships in the crate; you feed the resolver edges at runtime
(from KKO, a commercial ontology, or a fixture), which keeps the MIT surface
free of CC-BY content.

### 2. Isomorphism canonicalization

SP-1 orders placements by `(germ, disjunct, instance)` but does **not** collapse
assemblies that differ only by renumbering interchangeable parts — it flagged
that as a search concern. [`canonical_form`] is it: it relabels an assembly into
a normal form so that isomorphic assemblies become byte-identical and share a
`SectionId`. It does so **exactly** (minimizing over the valid relabelings), so
it never merges two assemblies that are not truly the same shape.

---

## Guarantees, and how they're tested

* **Sound.** Every returned assembly is validated closed *and* coherent by
  `prophet-sheaf`. (Enforced on every result.)
* **Complete.** Over bounded models the search finds *exactly* what an
  independent brute-force enumerator finds — no more, no fewer. (A differential
  test, three manifests including subsumption.)
* **Deduplicated.** No two results are isomorphic; canonicalization is proven
  permutation-invariant.
* **Deterministic.** Same inputs → identical, `SectionId`-ordered output.
* **Honest about outages.** If the resolver cannot answer, the search *aborts*
  with a resolver error rather than silently returning fewer completions — an
  outage is never a quiet "no".

## Bill of materials

`LICENSE` is MIT. `deny.toml` pins a permissive allowlist (MIT, Apache-2.0,
BSD-2/3, ISC, Unicode); AGPL and LGPL are denied, and CI fails on any violation.
The in-estate dependencies (`prophet-sheaf`, `prophet-truth`) are consumed as
pinned git sources, allowlisted explicitly. `cargo tree` contains zero
GPL-family licenses.

## Building

```bash
cargo test                    # soundness, completeness, determinism, dedup
cargo deny check              # license / advisory gate
cargo clippy -- -D warnings
```
