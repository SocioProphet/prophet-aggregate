//! The indexed subsumption resolver: correctness vs a naive climber, witness
//! chains, transitivity, strict-mode unknowns, and outage-free operation.

use prophet_aggregate::{IndexedResolver, ResolverBuilder};
use prophet_sheaf::{Iri, ResolverError, Subsumption};

#[test]
fn reflexive_and_direct_and_transitive() {
    let r = ResolverBuilder::new()
        .edge("ex:SSN", "ex:FinancialPII")
        .edge("ex:FinancialPII", "ex:PII")
        .edge("ex:PII", "ex:PersonalData")
        .build();

    // Reflexive.
    assert!(r.subsumes(&Iri::from("ex:PII"), &Iri::from("ex:PII")).unwrap().is_some());
    // Direct.
    assert!(r.subsumes(&Iri::from("ex:PII"), &Iri::from("ex:FinancialPII")).unwrap().is_some());
    // Transitive (two and three hops).
    assert!(r.subsumes(&Iri::from("ex:PII"), &Iri::from("ex:SSN")).unwrap().is_some());
    assert!(r
        .subsumes(&Iri::from("ex:PersonalData"), &Iri::from("ex:SSN"))
        .unwrap()
        .is_some());
    // Asymmetric: general is not subsumed by specific.
    assert!(r.subsumes(&Iri::from("ex:SSN"), &Iri::from("ex:PII")).unwrap().is_none());
    // Unrelated.
    let r2 = ResolverBuilder::new().edge("ex:A", "ex:B").edge("ex:C", "ex:D").build();
    assert!(r2.subsumes(&Iri::from("ex:B"), &Iri::from("ex:C")).unwrap().is_none());
}

#[test]
fn witness_carries_the_shortest_path() {
    let r = ResolverBuilder::new()
        .edge("ex:SSN", "ex:FinancialPII")
        .edge("ex:FinancialPII", "ex:PII")
        .build();
    let w = r
        .subsumes(&Iri::from("ex:PII"), &Iri::from("ex:SSN"))
        .unwrap()
        .unwrap();
    let chain: Vec<_> = w.chain.iter().map(|i| i.as_str().to_string()).collect();
    assert_eq!(chain, vec!["ex:SSN", "ex:FinancialPII", "ex:PII"]);
}

#[test]
fn multi_parent_dag_is_handled() {
    // SSN is both FinancialPII and GovIssuedId; both roll up to PII.
    let r = ResolverBuilder::new()
        .edge("ex:SSN", "ex:FinancialPII")
        .edge("ex:SSN", "ex:GovIssuedId")
        .edge("ex:FinancialPII", "ex:PII")
        .edge("ex:GovIssuedId", "ex:PII")
        .build();
    assert!(r.subsumes(&Iri::from("ex:PII"), &Iri::from("ex:SSN")).unwrap().is_some());
    assert!(r
        .subsumes(&Iri::from("ex:GovIssuedId"), &Iri::from("ex:SSN"))
        .unwrap()
        .is_some());
}

#[test]
fn strict_mode_reports_unknown_concepts_as_error_not_denial() {
    let r = ResolverBuilder::new().edge("ex:SSN", "ex:PII").strict(true).build();
    // A concept the hierarchy has never heard of is an error, not a silent "no".
    match r.subsumes(&Iri::from("ex:PII"), &Iri::from("ex:Unheard")) {
        Err(ResolverError::UnknownConcept { iri }) => assert_eq!(iri.as_str(), "ex:Unheard"),
        other => panic!("expected UnknownConcept, got {other:?}"),
    }
}

#[test]
fn loads_edges_from_a_text_export() {
    // A tab/comma export (KKO subClassOf-style), with a comment, a blank line,
    // and one malformed row.
    let export = "\
# subClassOf export
ex:SSN\tex:FinancialPII
ex:FinancialPII,ex:PII

malformed-row-without-separator
ex:PII\tex:PersonalData
";
    let (builder, skipped) = ResolverBuilder::new().load_edges(export);
    let r = builder.build();
    assert_eq!(skipped, vec![5], "the malformed row should be reported");
    // Transitive closure spans both tab- and comma-separated edges.
    assert!(r
        .subsumes(&Iri::from("ex:PersonalData"), &Iri::from("ex:SSN"))
        .unwrap()
        .is_some());
    assert_eq!(r.concept_count(), 4);
}

#[test]
fn lenient_mode_treats_unknowns_as_isolated_nodes() {
    let r = ResolverBuilder::new().edge("ex:SSN", "ex:PII").build();
    // Reflexive still holds for an unseen concept; unrelated returns a clean no.
    assert!(r.subsumes(&Iri::from("ex:X"), &Iri::from("ex:X")).unwrap().is_some());
    assert!(r.subsumes(&Iri::from("ex:PII"), &Iri::from("ex:X")).unwrap().is_none());
}

#[test]
fn resolver_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<IndexedResolver>();
}

#[test]
fn concurrent_subsumes_is_consistent() {
    use std::sync::Arc;
    use std::thread;

    let r = Arc::new(
        ResolverBuilder::new()
            .edge("ex:SSN", "ex:FinancialPII")
            .edge("ex:FinancialPII", "ex:PII")
            .build(),
    );
    let mut handles = Vec::new();
    for _ in 0..16 {
        let r = Arc::clone(&r);
        handles.push(thread::spawn(move || {
            for _ in 0..500 {
                let w = r
                    .subsumes(&Iri::from("ex:PII"), &Iri::from("ex:SSN"))
                    .unwrap()
                    .expect("SSN ⊑ PII");
                assert_eq!(w.chain.first().unwrap().as_str(), "ex:SSN");
                assert_eq!(w.chain.last().unwrap().as_str(), "ex:PII");
                assert!(r.subsumes(&Iri::from("ex:SSN"), &Iri::from("ex:PII")).unwrap().is_none());
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}
