//! Persisted transitive-closure index: round-trips exactly, and scales — a
//! resolver built once over a large hierarchy reloads with no recomputation and
//! answers subsumption in O(1).

use prophet_aggregate::{IndexedResolver, ResolverBuilder};
use prophet_sheaf::{Iri, Subsumption};
use std::time::Instant;

fn agrees(a: &IndexedResolver, b: &IndexedResolver, pairs: &[(&str, &str)]) {
    for (g, s) in pairs {
        let ra = a.subsumes(&Iri::from(*g), &Iri::from(*s)).unwrap();
        let rb = b.subsumes(&Iri::from(*g), &Iri::from(*s)).unwrap();
        assert_eq!(
            ra.as_ref().map(|w| &w.chain),
            rb.as_ref().map(|w| &w.chain),
            "reloaded index disagreed on {s} ⊑ {g}"
        );
    }
}

#[test]
fn index_round_trips_exactly() {
    let original = ResolverBuilder::new()
        .edge("ex:SSN", "ex:FinancialPII")
        .edge("ex:SSN", "ex:GovIssuedId")
        .edge("ex:FinancialPII", "ex:PII")
        .edge("ex:GovIssuedId", "ex:PII")
        .edge("ex:PII", "ex:PersonalData")
        .strict(true)
        .build();

    let bytes = original.to_index_bytes();
    let reloaded = IndexedResolver::from_index_bytes(&bytes).unwrap();

    assert_eq!(reloaded.concept_count(), original.concept_count());
    agrees(
        &original,
        &reloaded,
        &[
            ("ex:PersonalData", "ex:SSN"), // deep transitive
            ("ex:PII", "ex:SSN"),
            ("ex:FinancialPII", "ex:SSN"),
            ("ex:SSN", "ex:PII"), // asymmetry: None
            ("ex:PII", "ex:PII"), // reflexive
        ],
    );
    // Re-serializing the reloaded index reproduces the bytes exactly.
    assert_eq!(reloaded.to_index_bytes(), bytes);
}

#[test]
fn decode_rejects_garbage() {
    assert!(IndexedResolver::from_index_bytes(b"nope").is_err());
    assert!(IndexedResolver::from_index_bytes(&[]).is_err());
    // Truncated after a valid header.
    assert!(IndexedResolver::from_index_bytes(b"PSI1").is_err());
}

#[test]
fn scales_to_a_large_hierarchy() {
    // A wide, shallow 4-ary tree of ~8000 concepts: parent(i) = (i-1)/4.
    const N: u32 = 8000;
    let mut builder = ResolverBuilder::new();
    for i in 1..N {
        let parent = (i - 1) / 4;
        builder = builder.edge(&format!("c{i}"), &format!("c{parent}"));
    }

    let t_build = Instant::now();
    let r = builder.build();
    let build_ms = t_build.elapsed().as_secs_f64() * 1e3;
    assert_eq!(r.concept_count(), N as usize);

    // Every leaf subsumes up to the root c0; O(1) membership + short witness.
    let t_query = Instant::now();
    for i in (N / 2)..N {
        let leaf = Iri::from(format!("c{i}").as_str());
        let root = Iri::from("c0");
        let w = r.subsumes(&root, &leaf).unwrap().expect("leaf ⊑ root");
        assert_eq!(w.chain.first().unwrap().as_str(), format!("c{i}"));
        assert_eq!(w.chain.last().unwrap().as_str(), "c0");
    }
    let query_us =
        t_query.elapsed().as_secs_f64() * 1e6 / f64::from(N / 2);

    // Persist and reload with no recomputation.
    let bytes = r.to_index_bytes();
    let t_load = Instant::now();
    let reloaded = IndexedResolver::from_index_bytes(&bytes).unwrap();
    let load_ms = t_load.elapsed().as_secs_f64() * 1e3;
    assert_eq!(reloaded.concept_count(), N as usize);
    assert!(reloaded
        .subsumes(&Iri::from("c0"), &Iri::from("c7999"))
        .unwrap()
        .is_some());

    eprintln!(
        "scale[{N}]: build {build_ms:.1}ms, ~{query_us:.2}µs/query, index {} bytes, reload {load_ms:.1}ms",
        bytes.len()
    );
    // Generous ceilings — the point is it stays cheap, not a hard SLA.
    assert!(build_ms < 2000.0, "closure build too slow: {build_ms}ms");
    assert!(query_us < 50.0, "queries too slow: {query_us}µs");
}
