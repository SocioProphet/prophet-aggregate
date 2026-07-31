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
    let t_build = std::time::Instant::now();
    let (builder, _) = ResolverBuilder::new().load_kko(&text);
    let r = builder.build();
    let build_ms = t_build.elapsed().as_secs_f64() * 1e3;

    println!("KKO import from {path}");
    println!("  subClassOf edges : {}", stats.edges);
    println!("  skipped (structural): {}", stats.skipped_structural);
    println!("  named subjects   : {}", stats.subjects);
    println!("  concepts (nodes) : {}", r.concept_count());
    println!("  closure build    : {build_ms:.0} ms");

    // Query benchmark over a sample of edges.
    let sample: Vec<_> = edges.iter().step_by((edges.len() / 20000).max(1)).collect();
    let t_q = std::time::Instant::now();
    for (child, parent) in &sample {
        let _ = r
            .subsumes(&Iri::from(parent.as_str()), &Iri::from(child.as_str()))
            .unwrap();
    }
    let per_q_us = t_q.elapsed().as_secs_f64() * 1e6 / sample.len() as f64;
    println!("  subsumption query: {per_q_us:.2} µs (n={})", sample.len());

    // Persisted index size and reload time.
    let t_ser = std::time::Instant::now();
    let index = r.to_index_bytes();
    let ser_ms = t_ser.elapsed().as_secs_f64() * 1e3;
    let t_load = std::time::Instant::now();
    let reloaded = prophet_aggregate::IndexedResolver::from_index_bytes(&index).unwrap();
    let load_ms = t_load.elapsed().as_secs_f64() * 1e3;
    assert_eq!(reloaded.concept_count(), r.concept_count());
    println!(
        "  persisted index  : {:.1} MB (serialize {ser_ms:.0} ms, reload {load_ms:.0} ms, zero recompute)",
        index.len() as f64 / 1024.0 / 1024.0
    );

    // Optionally write the persisted index to a file: `--emit-index <path>`.
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--emit-index") {
        if let Some(out) = args.get(pos + 1) {
            std::fs::write(out, &index).expect("write index");
            println!("  wrote index → {out}");
        }
    }

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

    // Demonstrate a genuine transitive (multi-hop) subsumption, data-driven and
    // O(n): index one parent per child, then find a child whose parent also has
    // a parent (a 3+ node chain).
    let mut parent_of = std::collections::HashMap::new();
    for (child, parent) in &edges {
        parent_of.entry(child.as_str()).or_insert(parent.as_str());
    }
    for (child, parent) in &edges {
        if let Some(grand) = parent_of.get(parent.as_str()) {
            if child.as_str() != *grand {
                if let Some(w) = r
                    .subsumes(&Iri::from(*grand), &Iri::from(child.as_str()))
                    .unwrap()
                {
                    if w.chain.len() >= 3 {
                        let short: Vec<_> = w
                            .chain
                            .iter()
                            .map(|i| i.as_str().rsplit('/').next().unwrap_or(i.as_str()))
                            .collect();
                        println!(
                            "  transitive example ({} hops): {:?}",
                            w.chain.len() - 1,
                            short
                        );
                        break;
                    }
                }
            }
        }
    }
    assert_eq!(confirmed, edges.len(), "resolver failed to confirm an edge");
}
