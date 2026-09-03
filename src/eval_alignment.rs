//! OAEI-style alignment evaluation harness (#31).
//!
//! Given a reference alignment (gold standard) and a computed alignment
//! (e.g. from `onto_align`), compute precision / recall / F1 against the
//! reference. Mirrors the [OAEI](http://oaei.ontologymatching.org/) scoring
//! convention: alignments are sets of `(source_iri, target_iri, relation)`
//! triples; an entry matches the reference iff all three components match.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct AlignmentEntry {
    /// Accepts `onto_align`'s own field name (`source_iri`) as well as this
    /// evaluator's documented one, so raw aligner output can be fed
    /// straight into `onto_eval_alignment` without a manual rename
    /// (F-025 tranche-1 item 3).
    #[serde(alias = "source_iri")]
    pub source: String,
    #[serde(alias = "target_iri")]
    pub target: String,
    /// One of `"equivalent"`, `"subsumes"`, `"subsumed_by"`. Defaults to
    /// `"equivalent"` on deserialise if missing. Also accepts, and
    /// normalises to the vocabulary above, the OWL/SKOS/RDFS strings
    /// `onto_align` actually emits (`owl:equivalentClass`,
    /// `skos:exactMatch`, `rdfs:subClassOf`, `skos:closeMatch`) -- without
    /// this the two interfaces disagree twice over (field names, then
    /// vocabulary) and every correct alignment scores TP=0 against a
    /// reference written in this evaluator's own documented vocabulary
    /// (F-025 tranche-1 item 3).
    #[serde(default = "default_relation", deserialize_with = "deserialize_relation")]
    pub relation: String,
}

fn default_relation() -> String {
    "equivalent".to_string()
}

/// Normalize an alignment relation string to this evaluator's canonical
/// vocabulary (`equivalent` / `subsumes` / `subsumed_by` / `close_match`).
/// Passes the evaluator's own documented vocabulary through unchanged (a
/// hand-authored reference keeps working exactly as before); maps
/// `onto_align`'s emitted vocabulary onto it; passes any other string
/// through unchanged rather than guessing.
pub fn normalize_relation(raw: &str) -> String {
    match raw {
        "owl:equivalentClass" | "skos:exactMatch" => "equivalent".to_string(),
        // `<source> rdfs:subClassOf <target>` means source is the subclass,
        // i.e. from the (source, target) pair's perspective source is
        // subsumed BY target.
        "rdfs:subClassOf" => "subsumed_by".to_string(),
        "skos:closeMatch" => "close_match".to_string(),
        other => other.to_string(),
    }
}

fn deserialize_relation<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    Ok(normalize_relation(&raw))
}

#[derive(Clone, Debug, Serialize)]
pub struct EvaluationReport {
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub true_positive: usize,
    pub false_positive: usize,
    pub false_negative: usize,
    pub reference_size: usize,
    pub computed_size: usize,
}

/// Evaluate `computed` against `reference`. Entries are matched on
/// `(source, target, relation)` exact equality.
pub fn evaluate(reference: &[AlignmentEntry], computed: &[AlignmentEntry]) -> EvaluationReport {
    let ref_set: BTreeSet<&AlignmentEntry> = reference.iter().collect();
    let comp_set: BTreeSet<&AlignmentEntry> = computed.iter().collect();

    let tp = ref_set.intersection(&comp_set).count();
    let fp = comp_set.len() - tp;
    let fn_ = ref_set.len() - tp;

    let precision = if computed.is_empty() {
        0.0
    } else {
        tp as f64 / computed.len() as f64
    };
    let recall = if reference.is_empty() {
        0.0
    } else {
        tp as f64 / reference.len() as f64
    };
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };

    EvaluationReport {
        precision,
        recall,
        f1,
        true_positive: tp,
        false_positive: fp,
        false_negative: fn_,
        reference_size: reference.len(),
        computed_size: computed.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(s: &str, t: &str) -> AlignmentEntry {
        AlignmentEntry {
            source: s.to_string(),
            target: t.to_string(),
            relation: "equivalent".to_string(),
        }
    }

    #[test]
    fn perfect_match_gives_f1_1() {
        let r = vec![e("A", "X"), e("B", "Y")];
        let c = vec![e("A", "X"), e("B", "Y")];
        let report = evaluate(&r, &c);
        assert_eq!(report.precision, 1.0);
        assert_eq!(report.recall, 1.0);
        assert_eq!(report.f1, 1.0);
        assert_eq!(report.true_positive, 2);
    }

    #[test]
    fn complete_miss_gives_f1_0() {
        let r = vec![e("A", "X")];
        let c = vec![e("B", "Y")];
        let report = evaluate(&r, &c);
        assert_eq!(report.precision, 0.0);
        assert_eq!(report.recall, 0.0);
        assert_eq!(report.f1, 0.0);
        assert_eq!(report.false_positive, 1);
        assert_eq!(report.false_negative, 1);
    }

    #[test]
    fn partial_overlap_gives_intermediate_f1() {
        let r = vec![e("A", "X"), e("B", "Y"), e("C", "Z")];
        let c = vec![e("A", "X"), e("D", "W")];
        let report = evaluate(&r, &c);
        // P = 1/2 = 0.5, R = 1/3 ≈ 0.333, F1 = 2*0.5*0.333/(0.5+0.333) = 0.4
        assert!((report.precision - 0.5).abs() < 1e-9);
        assert!((report.recall - 1.0/3.0).abs() < 1e-9);
        assert!((report.f1 - 0.4).abs() < 1e-3);
    }

    #[test]
    fn empty_computed_gives_zero_precision_zero_recall() {
        let r = vec![e("A", "X")];
        let c: Vec<AlignmentEntry> = vec![];
        let report = evaluate(&r, &c);
        assert_eq!(report.precision, 0.0);
        assert_eq!(report.recall, 0.0);
        assert_eq!(report.false_negative, 1);
    }

    #[test]
    fn relation_mismatch_counts_as_miss() {
        let r = vec![e("A", "X")];
        let mut c = vec![e("A", "X")];
        c[0].relation = "subsumes".to_string();
        let report = evaluate(&r, &c);
        assert_eq!(report.precision, 0.0);
        assert_eq!(report.recall, 0.0);
        assert_eq!(report.true_positive, 0);
    }

    // ─── F-025 tranche-1 item 3: onto_align output is not consumable by ───
    // ─── onto_eval_alignment — two stacked interface mismatches ───────────
    //
    // The aligner (align.rs) emits `source_iri`/`target_iri` field names and
    // an `owl:`/`skos:`/`rdfs:`-prefixed relation vocabulary
    // (owl:equivalentClass, skos:exactMatch, rdfs:subClassOf,
    // skos:closeMatch). This evaluator's `AlignmentEntry` requires
    // `source`/`target` and the vocabulary documented on `relation`
    // (equivalent/subsumes/subsumed_by). Raw aligner output fed straight
    // into `onto_eval_alignment` therefore either fails to deserialise at
    // all (wrong field names) or, once the fields are renamed by hand,
    // silently scores every correct pair as a miss (disjoint relation
    // vocabularies mean `(source, target, relation)` exact-equality never
    // matches, so a perfect alignment scores P=R=F1=0 — a broken ruler, not
    // a broken engine).

    #[test]
    fn deserialises_the_aligners_field_names() {
        // Raw `onto_align` candidate shape: source_iri/target_iri, plus
        // extra fields (confidence, signals, applied, requires_review) the
        // evaluator has never heard of and must ignore.
        let raw = r#"{
            "source_iri": "http://ex.org/Dog",
            "target_iri": "http://other.org/Canine",
            "relation": "owl:equivalentClass",
            "confidence": 0.91,
            "signals": {"label_similarity": 0.9},
            "applied": true,
            "requires_review": false
        }"#;
        let entry: AlignmentEntry = serde_json::from_str(raw)
            .expect("onto_align's own field names must deserialise, not just the evaluator's");
        assert_eq!(entry.source, "http://ex.org/Dog");
        assert_eq!(entry.target, "http://other.org/Canine");
    }

    #[test]
    fn owl_equivalent_class_shaped_output_scores_a_true_positive() {
        // A perfect one-pair alignment, expressed exactly as `onto_align`
        // emits it (source_iri/target_iri, owl:equivalentClass), evaluated
        // against a reference hand-authored in the evaluator's own
        // documented vocabulary (source/target, "equivalent"). Must score
        // TP>0 -- pre-fix this was the reproduced P=R=F1=0 defect even
        // though the pair is exactly right.
        let reference = vec![e("http://ex.org/Dog", "http://other.org/Canine")];
        let computed_raw = r#"[{
            "source_iri": "http://ex.org/Dog",
            "target_iri": "http://other.org/Canine",
            "relation": "owl:equivalentClass",
            "confidence": 0.91,
            "applied": true,
            "requires_review": false
        }]"#;
        let computed: Vec<AlignmentEntry> =
            serde_json::from_str(computed_raw).expect("aligner-shaped computed_json must parse");
        let report = evaluate(&reference, &computed);
        assert!(
            report.true_positive > 0,
            "owl:equivalentClass-shaped output must score TP>0 against an \
             'equivalent' reference, got: {:?}",
            report
        );
        assert_eq!(report.precision, 1.0);
        assert_eq!(report.recall, 1.0);
    }

    #[test]
    fn subclassof_normalises_to_subsumed_by() {
        // `<source> rdfs:subClassOf <target>` means source is the subclass,
        // i.e. source is subsumed BY target.
        let raw = r#"{"source": "A", "target": "X", "relation": "rdfs:subClassOf"}"#;
        let entry: AlignmentEntry = serde_json::from_str(raw).unwrap();
        assert_eq!(entry.relation, "subsumed_by");
    }

    #[test]
    fn the_evaluators_own_vocabulary_still_passes_through_unchanged() {
        // Hand-authored references in the evaluator's own documented
        // vocabulary must keep working exactly as before (the evaluator is
        // string-agnostic by design; this pins that non-regression).
        let raw = r#"{"source": "A", "target": "X", "relation": "subsumes"}"#;
        let entry: AlignmentEntry = serde_json::from_str(raw).unwrap();
        assert_eq!(entry.relation, "subsumes");
    }
}
