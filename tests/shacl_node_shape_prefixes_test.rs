//! `sh:prefixes` and `sh:declare` written on the node shape itself.
//!
//! Authors commonly put `sh:prefixes` on the node shape rather than on each
//! `sh:sparql` node. The node-shape complement did not whitelist either
//! predicate, so a shape carrying one was reported as an unevaluated
//! constraint: the prefixed SPARQL constraint ran and fired with its exact
//! message and severity, and the report still read `conforms: null`. A false
//! undetermined on a run that missed nothing. The shape's own `sh:prefixes`
//! is now read for every `sh:sparql` constraint on it that names none of its
//! own, and neither predicate is reported as skipped.
//!
//! The false clean stays impossible: a prefix that nothing declares still
//! leaves the constraint unexecuted and the verdict null.

use open_ontologies::graph::GraphStore;
use open_ontologies::shacl::ShaclValidator;
use std::sync::Arc;

fn store() -> Arc<GraphStore> {
    let store = Arc::new(GraphStore::new());
    store
        .load_turtle(
            r#"
            @prefix ex: <http://example.org/> .
            ex:big   a ex:Widget ; ex:size 500 .
            ex:small a ex:Widget ; ex:size 10 .
        "#,
            None,
        )
        .unwrap();
    store
}

fn report(shapes: &str) -> serde_json::Value {
    serde_json::from_str(&ShaclValidator::validate(&store(), shapes).unwrap()).unwrap()
}

fn skipped_names(r: &serde_json::Value, needle: &str) -> bool {
    r["skipped_constraints"]
        .as_array()
        .map(|a| {
            a.iter()
                .any(|e| e["constraint"].as_str().unwrap_or("").contains(needle))
        })
        .unwrap_or(false)
}

#[test]
fn node_shape_sh_prefixes_does_not_force_conforms_null() {
    // The VERIFY-ENGINE-SHACL probe shape, reduced: a cross-entity threshold
    // with a prefixed sh:select, declarations reached through the node
    // shape's own sh:prefixes.
    let shapes = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:S a sh:NodeShape ; sh:targetClass ex:Widget ;
          sh:prefixes [ sh:declare [ sh:prefix "ex" ; sh:namespace "http://example.org/" ] ] ;
          sh:sparql [
            sh:select """
              SELECT $this WHERE {
                $this ex:size ?v .
                ?other a ex:Widget ; ex:size ?ov .
                FILTER(?other != $this && ?v > ?ov * 10)
              }
            """ ;
            sh:message "more than ten times another widget" ;
            sh:severity sh:Info ] .
    "#;
    let r = report(shapes);
    assert_eq!(r["conforms"], serde_json::json!(false), "{r}");
    assert_eq!(r["violation_count"], 1, "{r}");
    assert_eq!(r["violations"][0]["message"], "more than ten times another widget", "{r}");
    assert_eq!(r["violations"][0]["severity"], "Info", "{r}");
    assert!(!skipped_names(&r, "prefixes"), "sh:prefixes is not a constraint: {r}");
    assert!(!skipped_names(&r, "declare"), "sh:declare is not a constraint: {r}");
}

#[test]
fn node_shape_sh_declare_on_the_shape_itself_is_not_a_constraint() {
    // The shape holds its own declarations and points sh:prefixes at itself.
    let shapes = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:S a sh:NodeShape ; sh:targetClass ex:Widget ;
          sh:declare [ sh:prefix "p" ; sh:namespace "http://example.org/" ] ;
          sh:prefixes ex:S ;
          sh:sparql [ sh:message "oversized" ;
            sh:select "SELECT $this WHERE { $this p:size ?s . FILTER(?s > 100) }" ] .
    "#;
    let r = report(shapes);
    assert_eq!(r["conforms"], serde_json::json!(false), "{r}");
    assert_eq!(r["violation_count"], 1, "{r}");
    assert!(!skipped_names(&r, "declare"), "{r}");
    assert!(!skipped_names(&r, "prefixes"), "{r}");
}

#[test]
fn node_shape_sh_prefixes_selects_the_declarations_that_apply() {
    // Two declaration sets bind `p` to different namespaces. The whole-graph
    // merge refuses that as ambiguous; the node shape's own sh:prefixes says
    // which set it means, so the constraint runs and fires.
    let shapes = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        @prefix other: <http://other.example/> .
        ex:S a sh:NodeShape ; sh:targetClass ex:Widget ; sh:prefixes ex:decls ;
          sh:sparql [ sh:message "oversized" ;
            sh:select "SELECT $this WHERE { $this p:size ?s . FILTER(?s > 100) }" ] .
        ex:decls    sh:declare [ sh:prefix "p" ; sh:namespace "http://example.org/" ] .
        other:decls sh:declare [ sh:prefix "p" ; sh:namespace "http://unrelated.example/" ] .
    "#;
    let r = report(shapes);
    assert_eq!(r["conforms"], serde_json::json!(false), "{r}");
    assert_eq!(r["violation_count"], 1, "{r}");
}

#[test]
fn an_undeclared_prefix_still_leaves_conformance_undetermined() {
    // Must never: a constraint that could not run reads as a pass or a fail.
    let shapes = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        ex:S a sh:NodeShape ; sh:targetClass ex:Widget ;
          sh:prefixes [ sh:declare [ sh:prefix "ex" ; sh:namespace "http://example.org/" ] ] ;
          sh:sparql [ sh:message "oversized" ;
            sh:select "SELECT $this WHERE { $this q:size ?s . FILTER(?s > 100) }" ] .
    "#;
    let r = report(shapes);
    assert_eq!(r["conforms"], serde_json::Value::Null, "{r}");
    assert!(skipped_names(&r, "sparql"), "the unexecuted constraint must be named: {r}");
}
