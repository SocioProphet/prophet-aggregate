//! Importer for KBpedia Knowledge Ontology (KKO) Turtle/N3 exports.
//!
//! KKO — and KBpedia's reference concepts — are **CC BY 4.0** content, not code.
//! This module is code: it extracts the `rdfs:subClassOf` hierarchy from a KKO
//! Turtle export into `(child, parent)` edges that [`crate::ResolverBuilder`]
//! ingests **at runtime**. No ontology data is vendored into the crate, so the
//! MIT bill of materials stays unambiguous; the operator supplies the `.n3` file.
//!
//! KKO's serialization is regular: a named class is a subject block introduced
//! at column 0 as `:Name a owl:Class ;`, followed by indented predicates such as
//! `rdfs:subClassOf :Parent ;`. This extractor tracks the current named subject
//! and, for every `rdfs:subClassOf` naming a class in the default namespace
//! (`:Parent`), emits an edge. Subjects that are blank nodes (`_:genid…`,
//! `owl:Restriction`s) and superclasses that are not plain named classes are
//! skipped — they are structural, not taxonomic.

use smol_str::SmolStr;

/// A local-name concept id (the part after the `:` in KKO's default namespace),
/// e.g. `Mammals`. Callers may prefix it (e.g. `kko:Mammals`) if they wish; the
/// resolver treats ids opaquely.
pub type ConceptName = SmolStr;

/// Statistics from a KKO import.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KkoStats {
    /// Number of `subClassOf` edges to named classes extracted.
    pub edges: usize,
    /// Number of `subClassOf` statements skipped (superclass was a blank node /
    /// restriction, not a plain named class).
    pub skipped_structural: usize,
    /// Number of distinct named subjects that declared at least one edge.
    pub subjects: usize,
}

/// Extract `(child, parent)` `subClassOf` edges from a KKO Turtle/N3 document.
///
/// Ids are the default-namespace local names (`:Name` → `Name`). Returned in
/// document order; deduplication and closure are the resolver's job.
#[must_use]
pub fn subclass_edges(turtle: &str) -> (Vec<(ConceptName, ConceptName)>, KkoStats) {
    let mut edges = Vec::new();
    let mut stats = KkoStats::default();
    let mut current: Option<SmolStr> = None;
    let mut subjects_with_edges = std::collections::BTreeSet::new();

    for line in turtle.lines() {
        // A subject block begins at column 0. Named subject `:Name …`.
        let starts_at_col0 = line
            .chars()
            .next()
            .is_some_and(|c| !c.is_whitespace());
        if starts_at_col0 {
            current = named_local(line.trim_start());
        }

        // subClassOf predicate (may appear on the subject line or an indented one).
        if let Some(pos) = line.find("rdfs:subClassOf") {
            let Some(subject) = current.clone() else {
                continue;
            };
            let rest = &line[pos + "rdfs:subClassOf".len()..];
            let parents = named_classes(rest);
            if parents.is_empty() {
                // Superclass was a blank node / restriction, not a named class.
                stats.skipped_structural += 1;
            }
            for parent in parents {
                if parent != subject {
                    edges.push((subject.clone(), parent));
                    subjects_with_edges.insert(subject.clone());
                }
            }
        }
    }

    stats.edges = edges.len();
    stats.subjects = subjects_with_edges.len();
    (edges, stats)
}

/// If `s` begins with a default-namespace named subject `:Name`, return `Name`.
fn named_local(s: &str) -> Option<SmolStr> {
    let rest = s.strip_prefix(':')?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(SmolStr::from(name))
    }
}

/// Collect the default-namespace named classes (`:Name`) appearing in `s`, e.g.
/// the superclasses on a `subClassOf` line. Prefixed terms like `owl:Thing` are
/// ignored (they do not begin with a bare `:`).
fn named_classes(s: &str) -> Vec<SmolStr> {
    // Stop at a blank-node restriction `[ … ]`: everything inside it (e.g.
    // `owl:onProperty :hasPart`) is structural, not a taxonomic superclass.
    let s = match s.find('[') {
        Some(i) => &s[..i],
        None => s,
    };
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            // A bare `:` (default namespace) is one not immediately preceded by a
            // name character (which would make it a prefix like `owl:`).
            let prefixed = i > 0 && {
                let p = bytes[i - 1];
                p.is_ascii_alphanumeric() || p == b'_' || p == b'-'
            };
            if !prefixed {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() {
                    let c = bytes[j];
                    if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                        j += 1;
                    } else {
                        break;
                    }
                }
                if j > start {
                    out.push(SmolStr::from(&s[start..j]));
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}
