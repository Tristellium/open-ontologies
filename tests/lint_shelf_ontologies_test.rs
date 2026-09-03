//! F-025 tranche-1 item 4, driven on the real shelf: `lint` hardcoded
//! Turtle (`ontology.rs`), so it failed on every RDF/XML document handed to
//! it directly (as `onto_lint`, the MCP tool, does) with a parse error from
//! the wrong parser. These are 5 real, publicly-licensed ontologies from the
//! practice's ontology shelf that are distributed as RDF/XML (three of them
//! under a `.rdf` extension declaring internal DTD entities, the other two
//! staged under a `.ttl`-convention filename despite their actual
//! `application/rdf+xml` media type -- see `SHELF-MANIFEST.json`), used here
//! exactly as the F-025 tranche-1 report's "4 of our 10 shelf files" failure
//! count was measured against.
//!
//! `OntologyService::lint` is called directly (the same call `onto_lint`
//! makes), not through the CLI's own `content_as_turtle` workaround, so this
//! exercises the root-cause fix rather than the CLI-side pre-conversion that
//! already existed upstream.

use open_ontologies::ontology::OntologyService;

fn fixture(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/shelf")
        .join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
}

fn assert_lints_cleanly(name: &str, content: &str) {
    let result = OntologyService::lint(content)
        .unwrap_or_else(|e| panic!("{name}: lint returned Err (should never happen -- lint reports issues, it doesn't fail): {e}"));
    let parsed: serde_json::Value = serde_json::from_str(&result)
        .unwrap_or_else(|e| panic!("{name}: lint output was not valid JSON: {e}\n{result}"));
    assert!(
        parsed.get("issue_count").is_some(),
        "{name}: expected a lint report with issue_count, got: {result}"
    );
    // A parse failure must not silently become "an empty document, zero
    // issues" -- that would be a clean bill of health for a file lint never
    // actually read. Every one of these 5 real ontologies has classes
    // missing a label or comment somewhere, so a genuine read reports at
    // least one issue.
    let issue_count = parsed["issue_count"].as_u64().unwrap_or(0);
    assert!(
        issue_count > 0,
        "{name}: lint reported zero issues on a real, large ontology -- \
         suspicious: did it actually read the content, or silently parse an \
         empty document?"
    );
}

#[test]
fn lints_ebucore_rdfxml_cleanly() {
    assert_lints_cleanly("ebucore", &fixture("ebucore.rdf"));
}

// Note: ebu-ccdm's own lint report is legitimately issue_count: 0 (every
// class is fully labeled/commented) -- so it is checked only for "does not
// error and returns a well-formed report", not that it finds an issue, the
// way the other four fixtures are.
#[test]
fn lints_ebu_ccdm_rdfxml_does_not_error() {
    let content = fixture("ebu-ccdm.rdf");
    let result =
        OntologyService::lint(&content).expect("ebu-ccdm: lint must not error on RDF/XML");
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed.get("issue_count").is_some(), "got: {result}");
}

#[test]
fn lints_iof_core_rdfxml_cleanly() {
    assert_lints_cleanly("iof/Core", &fixture("iof/Core.rdf"));
}

#[test]
fn lints_iof_production_planning_rdfxml_cleanly() {
    assert_lints_cleanly("iof/ProductionPlanning", &fixture("iof/ProductionPlanning.rdf"));
}

#[test]
fn lints_iof_supply_chain_rdfxml_cleanly() {
    assert_lints_cleanly("iof/SupplyChain", &fixture("iof/SupplyChain.rdf"));
}
