//! Shared builders for prophet-aggregate integration tests.
#![allow(dead_code)]

use prophet_sheaf::{
    Arity, Direction, Disjunct, DisjunctIdx, EpistemicLevel, Germ, GermId, Manifest, Placement,
    Polarity, Port, PortClass, PortType,
};

pub fn cap(ty: &str, dir: Direction, arity: Arity) -> Port {
    Port {
        ty: PortType::capability(ty),
        polarity: Polarity {
            dir,
            class: PortClass::Flow,
        },
        arity,
    }
}

pub fn concept(iri: &str, dir: Direction, arity: Arity) -> Port {
    Port {
        ty: PortType::concept(iri),
        polarity: Polarity {
            dir,
            class: PortClass::Flow,
        },
        arity,
    }
}

pub fn germ(anchor: &str, ports: Vec<Port>) -> Germ {
    germ_lvl(anchor, ports, EpistemicLevel::Empirical)
}

pub fn germ_lvl(anchor: &str, ports: Vec<Port>, level: EpistemicLevel) -> Germ {
    Germ {
        anchor: GermId::from(anchor),
        disjuncts: vec![Disjunct::new(ports)],
        epistemic: level,
    }
}

/// A dataflow manifest where `transform` is only `Speculative` (source and sink
/// are `Empirical`), so any pipeline using a transform composes down to
/// `Speculative`.
pub fn graded_manifest(transform_level: EpistemicLevel) -> Manifest {
    let mut m = Manifest::new();
    m.insert(germ_lvl(
        "source",
        vec![cap("d", Direction::Out, Arity::Required)],
        EpistemicLevel::Empirical,
    ));
    m.insert(germ_lvl(
        "sink",
        vec![cap("d", Direction::In, Arity::Required)],
        EpistemicLevel::Empirical,
    ));
    m.insert(germ_lvl(
        "transform",
        vec![
            cap("d", Direction::Out, Arity::Required),
            cap("d", Direction::In, Arity::Required),
        ],
        transform_level,
    ));
    m
}

/// Capability dataflow vocabulary. Every germ except `hub` has a Required port,
/// so any closed coherent completion is "required-justified" (no gratuitous
/// disconnected germs) — which keeps the search and a brute-force oracle in
/// agreement.
pub fn dataflow_manifest() -> Manifest {
    let mut m = Manifest::new();
    m.insert(germ("source", vec![cap("d", Direction::Out, Arity::Required)]));
    m.insert(germ("sink", vec![cap("d", Direction::In, Arity::Required)]));
    m.insert(germ(
        "transform",
        vec![
            cap("d", Direction::Out, Arity::Required),
            cap("d", Direction::In, Arity::Required),
        ],
    ));
    m
}

/// Adds a fan-out `hub` (one `Out` Multi{0,2}) to the dataflow vocabulary.
pub fn dataflow_with_hub() -> Manifest {
    let mut m = dataflow_manifest();
    m.insert(germ(
        "hub",
        vec![cap(
            "d",
            Direction::Out,
            Arity::Multi { min: 0, max: Some(2) },
        )],
    ));
    m
}

/// Concept vocabulary: a PII sink (requires ex:PII) and an SSN source (offers
/// ex:SSN). With a resolver where `SSN ⊑ PII`, the two complete each other.
pub fn concept_manifest() -> Manifest {
    let mut m = Manifest::new();
    m.insert(germ(
        "pii-sink",
        vec![concept("ex:PII", Direction::In, Arity::Required)],
    ));
    m.insert(germ(
        "ssn-source",
        vec![concept("ex:SSN", Direction::Out, Arity::Required)],
    ));
    m
}

pub fn place(germ: &str, inst: u32) -> Placement {
    Placement {
        germ: GermId::from(germ),
        disjunct: DisjunctIdx(0),
        instance: inst,
    }
}

// Canonical port indices (Out < In within a disjunct).
pub const SOURCE_OUT: u32 = 0;
pub const SINK_IN: u32 = 0;
pub const TRANSFORM_OUT: u32 = 0;
pub const TRANSFORM_IN: u32 = 1;
pub const HUB_OUT: u32 = 0;
