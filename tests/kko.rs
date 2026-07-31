//! KKO importer: extracts the subClassOf hierarchy from a KKO-format Turtle
//! export and feeds it to the resolver. Tested on a synthetic snippet in the
//! real KKO shape (the actual CC-BY ontology ships in the HellGraph layer, not
//! here).

use prophet_aggregate::{kko, ResolverBuilder};
use prophet_sheaf::{Iri, Subsumption};

const SNIPPET: &str = "\
# a KKO-shaped export
:Mammals a owl:Class ;
  rdfs:subClassOf :Animals ;
  owl:disjointWith _:g1 .

:Animals a owl:Class ;
  rdfs:subClassOf :LivingThings .

:Dog a owl:Class ;
  rdfs:subClassOf :Mammals , :Pets .

_:g1 a owl:Class ;
  rdfs:subClassOf _:g2 .

:Robot a owl:Class ;
  rdfs:subClassOf [ owl:onProperty :hasPart ] .
";

#[test]
fn extracts_named_subclass_edges() {
    let (edges, stats) = kko::subclass_edges(SNIPPET);
    let as_pairs: Vec<(String, String)> = edges
        .iter()
        .map(|(c, p)| (c.to_string(), p.to_string()))
        .collect();
    assert_eq!(
        as_pairs,
        vec![
            ("Mammals".into(), "Animals".into()),
            ("Animals".into(), "LivingThings".into()),
            ("Dog".into(), "Mammals".into()),
            ("Dog".into(), "Pets".into()),
        ]
    );
    assert_eq!(stats.edges, 4);
    assert_eq!(stats.subjects, 3);
    // Robot's superclass is a restriction (blank node), not a named class.
    assert_eq!(stats.skipped_structural, 1);
}

#[test]
fn imported_hierarchy_answers_transitive_subsumption() {
    let (builder, _) = ResolverBuilder::new().load_kko(SNIPPET);
    let r = builder.build();
    // Dog ⊑ Mammals ⊑ Animals ⊑ LivingThings.
    assert!(r
        .subsumes(&Iri::from("LivingThings"), &Iri::from("Dog"))
        .unwrap()
        .is_some());
    // Multi-parent: Dog ⊑ Pets.
    assert!(r.subsumes(&Iri::from("Pets"), &Iri::from("Dog")).unwrap().is_some());
    // Asymmetry holds.
    assert!(r.subsumes(&Iri::from("Dog"), &Iri::from("Animals")).unwrap().is_none());
}
