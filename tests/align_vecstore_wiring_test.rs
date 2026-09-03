//! F-025 tranche-1 item 1: `AlignmentEngine::new_with_vecstore` has no
//! production callers.
//!
//! `align.rs`'s embedding signal is real and correctly implemented -- but
//! every production construction site (`align` CLI verb, `align-feedback`
//! CLI verb, `onto_align` MCP tool, `onto_align_feedback` MCP tool) builds
//! the engine via `AlignmentEngine::new`, which hardcodes `vecstore: None`.
//! `new_with_vecstore` exists, is `#[cfg(feature = "embeddings")]`-correct,
//! and has zero callers anywhere in the binary or the server -- so
//! `embedding_sim` is 0.0 on every reachable path in a full-feature build,
//! and the HNSW cross-lingual prefilter it feeds is likewise permanently
//! dead.
//!
//! This test drives the real CLI `align` verb (`Commands::Align` in
//! `main.rs`, the exact production call site) end to end: pre-populate the
//! on-disk vecstore with a real embedding pair whose cosine similarity is
//! high but whose LABELS share no tokens at all (English/French, "Dog"
//! /"Chien" -- deliberately unrelated as strings). Under the label-only
//! pre-filter (`label_sim < 0.75`), and with the embeddings feature
//! providing the `CROSS_LINGUAL_EMBED_MIN` bypass at `embedding_sim >=
//! 0.80`, this pair can ONLY reach the candidate list if the CLI's
//! `AlignmentEngine` actually received the real vecstore -- i.e. only once
//! the dead-code defect is fixed. Pre-fix, `vecstore: None` means
//! `embedding_sim` is computed as 0.0 for every pair (align.rs's own
//! internal fallback), the pair never clears the pre-filter, and it is
//! silently ABSENT from `candidates` -- not merely low-scoring.

#![cfg(feature = "embeddings")]

use open_ontologies::state::StateDb;
use open_ontologies::vecstore::VecStore;
use std::process::Command;

const SOURCE_TTL: &str = r#"
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix ex: <http://ex.org/> .
    ex:Dog a owl:Class ; rdfs:label "Dog" .
"#;

const TARGET_TTL: &str = r#"
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix other: <http://other.org/> .
    other:Chien a owl:Class ; rdfs:label "Chien" .
"#;

const SOURCE_IRI: &str = "http://ex.org/Dog";
const TARGET_IRI: &str = "http://other.org/Chien";

fn write(dir: &std::path::Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p.to_str().unwrap().to_string()
}

/// The regression itself, driven through the actual CLI binary: a
/// cross-lingual pair with near-identical (deliberately hand-crafted,
/// cosine ~1.0) embedding vectors, but zero shared label tokens, must
/// surface as an alignment candidate carrying a nonzero
/// `embedding_similarity` signal -- proving `AlignmentEngine::new_with_vecstore`
/// is reached from `Commands::Align`, not `AlignmentEngine::new`.
#[test]
fn cli_align_wires_the_real_vecstore_into_the_production_call_site() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Populate the on-disk vecstore exactly where `setup()` in main.rs will
    // look for it (`<data_dir>/open-ontologies.db`), then drop every handle
    // before spawning the CLI so nothing holds the sqlite file open.
    {
        let db_path = data_dir.join("open-ontologies.db");
        let db = StateDb::open(&db_path).unwrap();
        let mut vs = VecStore::new(db);
        let vec_a: Vec<f32> = vec![1.0, 0.2, 0.05, 0.0];
        let vec_b: Vec<f32> = vec![0.98, 0.22, 0.04, 0.01]; // cosine ~1.0 with vec_a
        vs.upsert(SOURCE_IRI, &vec_a, &[]);
        vs.upsert(TARGET_IRI, &vec_b, &[]);
        vs.persist().expect("persisting the pre-seeded vectors must succeed");
    }

    let source_path = write(dir.path(), "source.ttl", SOURCE_TTL);
    let target_path = write(dir.path(), "target.ttl", TARGET_TTL);

    let out = Command::new(env!("CARGO_BIN_EXE_open-ontologies"))
        .arg("--data-dir")
        .arg(&data_dir)
        .args([
            "align",
            &source_path,
            &target_path,
            "--min-confidence",
            "0.01",
            "--dry-run",
        ])
        .output()
        .expect("binary should run");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "align failed.\nstdout: {stdout}\nstderr: {stderr}");

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("bad JSON: {e}\n{stdout}"));
    let candidates = parsed["candidates"].as_array().cloned().unwrap_or_default();

    let pair = candidates.iter().find(|c| {
        c["source_iri"].as_str() == Some(SOURCE_IRI) && c["target_iri"].as_str() == Some(TARGET_IRI)
    });

    assert!(
        pair.is_some(),
        "Dog/Chien has no shared label tokens and can only clear the \
         pre-filter via the embedding cross-lingual bypass -- its absence \
         means the CLI's AlignmentEngine did not receive the real vecstore. \
         candidates: {candidates:#?}"
    );
    let embedding_sim = pair.unwrap()["signals"]["embedding_similarity"]
        .as_f64()
        .expect("embeddings-feature builds must emit an embedding_similarity signal");
    assert!(
        embedding_sim > 0.8,
        "expected a high embedding_similarity from the pre-seeded near-identical \
         vectors, got {embedding_sim} -- AlignmentEngine::new (vecstore: None) \
         would score this 0.0 on every pair"
    );
}
