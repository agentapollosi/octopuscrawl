//! Smoke test for the ingest layer: dedup + BPE token counts.
//! Run: cargo run -p octopuscrawl-ingest --bin itest

use octopuscrawl_ingest::{hamming, simhash, train, Ingest};

fn main() -> anyhow::Result<()> {
    let samples = [
        "Cross-site scripting (XSS) is a web security vulnerability that lets an attacker compromise user interactions with a vulnerable application.",
        "SQL injection is a web security vulnerability that lets an attacker interfere with the queries an application makes to its database.",
        "Cross site scripting XSS is a web security vulnerability allowing attackers to compromise interactions with a vulnerable application.",
        "The Transport Layer Security protocol provides confidentiality and integrity for data in transit between two communicating applications.",
    ];

    // dedup: sample 0 and sample 2 are near-duplicates
    let h0 = simhash(samples[0]);
    let h2 = simhash(samples[2]);
    let h1 = simhash(samples[1]);
    println!("simhash dist(xss, xss-paraphrase) = {}", hamming(h0, h2));
    println!("simhash dist(xss, sqli)           = {}", hamming(h0, h1));

    // train a tiny BPE on the samples and count tokens
    let texts: Vec<String> = samples.iter().map(|s| s.to_string()).collect();
    let tok = train(&texts, 500)?;
    for s in &samples[..2] {
        let n = tok.encode(*s, false).map(|e| e.len()).unwrap_or(0);
        println!("{n:3} tokens  |  {}", &s[..s.len().min(60)]);
    }

    // end-to-end ingest into a temp dir
    let dir = std::env::temp_dir().join("octopuscrawl-ingest-test");
    let _ = std::fs::remove_dir_all(&dir);
    let mut ing = Ingest::new(&dir);
    for (i, s) in samples.iter().enumerate() {
        let r = ing.process(&format!("https://x/{i}"), "x", "writeups", "t", s);
        println!("page {i}: accepted={} tokens={}", r.accepted, r.tokens);
    }
    println!("accepted total = {} (expected 3, one near-dup dropped)", ing.accepted());
    println!("dataset at {}", dir.join("corpus.jsonl").display());
    Ok(())
}
