//! # prophet-aggregate
//!
//! **Search over the manifest sheaf.** `prophet-sheaf` decides whether a *given*
//! assembly is legal; this crate *finds* the legal assemblies. Given a manifest,
//! a subsumption resolver, a policy, and a **seed** (the capability you want to
//! build around), it enumerates every closed, coherent section that completes the
//! seed — deduplicated up to isomorphism, in deterministic order, each with a
//! `SectionId` receipt.
//!
//! This is the SP-3 aggregator that SP-1 was built to unblock. It supplies the
//! two things SP-1 deferred here:
//!
//! * **[`IndexedResolver`]** — the precomputed-transitive-closure resolver SP-1's
//!   §5.1.2 promised "lands with SP-3", so `can_mate` is O(1) in the search loop.
//! * **Isomorphism canonicalization** ([`canonical_form`]) — SP-1 sorts by
//!   `(germ, disjunct, instance)` but does not normalize *up to* relabeling
//!   interchangeable instances; that normalization lives here, so isomorphic
//!   solutions collapse to one.
//!
//! ```no_run
//! use prophet_aggregate::{Aggregator, Bound, ResolverBuilder};
//! # use prophet_sheaf::{Manifest, Placement, Policy};
//! # fn demo(manifest: &Manifest, seed: &[Placement]) {
//! let resolver = ResolverBuilder::new().edge("ex:SSN", "ex:PII").build();
//! let agg = Aggregator::new(manifest, &resolver, Policy::default(), Bound::placements(6));
//! let solutions = agg.complete(seed).unwrap();
//! # let _ = solutions;
//! # }
//! ```

mod canon;
mod enumerate;
pub mod kko;
mod resolver_index;

pub use canon::{canonical_form, canonical_id};
pub use enumerate::{Aggregator, AggregateError, Bound, Solution};
pub use kko::KkoStats;
pub use resolver_index::{IndexDecodeError, IndexedResolver, ResolverBuilder};

// Re-export the prophet-sheaf surface callers need so they can drive the
// aggregator without depending on prophet-sheaf by name.
pub use prophet_sheaf::{
    Manifest, Placement, Policy, Section, SectionId, Subsumption,
};
