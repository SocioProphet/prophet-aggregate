//! Import a KKO Turtle/N3 export and report the extracted hierarchy.
//!
//! `cargo run --example kko_report -- path/to/kko.n3`
//!
//! The KKO data itself is CC BY 4.0 and is not part of this crate; point this at
//! an operator-supplied export (it ships bundled in the HellGraph layer).

use prophet_aggregate::ResolverBuilder;
use prophet_sheaf::{Iri, Subsumption};

fn main() {
    let path = std::env::args().nth(1).expect("usage: kko_report <kko.n3>");
    let text = std::fs::read_to_string(&path).expect("read kko file");

    let (edges, stats) = prophet_aggregate::kko::subclass_edges(&text);
    let (builder, _) = ResolverBuilder::new().load_kko(&text);
    let r = builder.build();

    println!("KKO import from {path}");
    println!("  subClassOf edges : {}", stats.edges);
    println!("  skipped (structural): {}", stats.skipped_structural);
    println!("  named subjects   : {}", stats.subjects);
    println!("  concepts (nodes) : {}", r.concept_count());

    // Verify every extracted edge is confirmed by the built resolver.
    let mut confirmed = 0usize;
    for (child, parent) in &edges {
        if r
            .subsumes(&Iri::from(parent.as_str()), &Iri::from(child.as_str()))
            .unwrap()
            .is_some()
        {
            confirmed += 1;
        }
    }
    println!("  edges confirmed by resolver: {confirmed}/{}", edges.len());

    // Demonstrate a genuine transitive (multi-hop) subsumption, data-driven.
    let mut demonstrated = false;
    for (a, b) in &edges {
        for (b2, c) in &edges {
            if b == b2 && a != c && b != c {
                if let Some(w) = r
                    .subsumes(&Iri::from(c.as_str()), &Iri::from(a.as_str()))
                    .unwrap()
                {
                    if w.chain.len() >= 3 {
                        let path: Vec<_> = w.chain.iter().map(|i| i.as_str()).collect();
                        println!("  transitive example: {} ⊑ {}  via {:?}", a, c, path);
                        demonstrated = true;
                        break;
                    }
                }
            }
        }
        if demonstrated {
            break;
        }
    }
    assert_eq!(confirmed, edges.len(), "resolver failed to confirm an edge");
}
