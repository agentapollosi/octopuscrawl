//! Train the queen on the crawl corpus, then print a few samples.
//! usage: queen-train [corpus.jsonl] [steps] [outdir]

use anyhow::Result;
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let corpus = args.get(1).cloned().unwrap_or_else(|| "data/corpus.jsonl".into());
    let steps: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1500);
    let outdir = args.get(3).cloned().unwrap_or_else(|| "data".into());

    // gather the "text" field from each JSONL record into one training string
    let raw = std::fs::read_to_string(&corpus)?;
    let mut text = String::new();
    for line in raw.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
                text.push_str(t);
                text.push_str("\n\n");
            }
        }
    }
    println!("[queen] corpus: {} chars from {}", text.len(), corpus);

    if steps > 0 {
        let t0 = std::time::Instant::now();
        let rep = octopuscrawl_queen::train(&text, steps, Path::new(&outdir))?;
        println!(
            "[queen] trained {} steps · final loss {:.4} · ~{} params · vocab {} · {:.1}s",
            rep.steps,
            rep.final_loss,
            rep.params,
            rep.vocab,
            t0.elapsed().as_secs_f32()
        );
    } else {
        println!("[queen] steps=0 → generate only from existing model");
    }

    let q = octopuscrawl_queen::Queen::load(Path::new(&outdir))?;
    for prompt in ["The vulnerability", "An attacker can", "SQL injection is"] {
        let out = q.generate(prompt, 200, 0.8)?;
        println!("\n=== prompt: {prompt:?} ===\n{out}");
    }
    Ok(())
}
