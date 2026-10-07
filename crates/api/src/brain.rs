//! The queen's model: her trained-model status, text generation, and the
//! background (re)training loop. Behind the `real` feature (pulls in candle).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use octopuscrawl_core::{QueenStatus, QueenVersion};

use crate::AppState;

const DIR: &str = "data";
const TRAIN_STEPS: usize = 2500;
const FIRST_AT: u64 = 120; // accepted records before her first training
const RETRAIN_DELTA: u64 = 250; // extra records before a retrain

fn dir() -> &'static Path {
    Path::new(DIR)
}

pub fn trained() -> bool {
    octopuscrawl_queen::exists(dir())
}

/// What the dataset holds right now: (pages, tokens) straight from the corpus file.
fn dataset_counts() -> (u64, u64) {
    let mut pages = 0u64;
    let mut tokens = 0u64;
    if let Ok(body) = std::fs::read_to_string(dir().join("corpus.jsonl")) {
        for line in body.lines() {
            if line.trim().is_empty() {
                continue;
            }
            pages += 1;
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                tokens += v.get("tokens").and_then(|t| t.as_u64()).unwrap_or(0);
            }
        }
    }
    (pages, tokens)
}

/// Snapshot of exactly what a trained version read, written when its run finishes —
/// so "trained on N pages" is what she actually saw, not what the crawl has now.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
struct Trained {
    version: u32,
    pages: u64,
    tokens: u64,
}

fn read_trained() -> Option<Trained> {
    serde_json::from_str(&std::fs::read_to_string(dir().join("queen.trained.json")).ok()?).ok()
}

fn write_trained(t: Trained) {
    if let Ok(body) = serde_json::to_string(&t) {
        let _ = std::fs::write(dir().join("queen.trained.json"), body);
    }
}

/// The live version, reflecting the on-disk model (or "collecting" when untrained).
pub fn queen_version() -> QueenVersion {
    let meta = read_trained();
    if let Some(info) = octopuscrawl_queen::model_info(dir()) {
        let trained_at = std::fs::metadata(dir().join("queen.safetensors"))
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| crate::clock::correct(DateTime::<Utc>::from(t)));
        // a model trained before snapshots existed claims no page count (0)
        QueenVersion {
            version: meta.map(|m| m.version).unwrap_or(1),
            status: QueenStatus::Live,
            dataset_tokens: meta.map(|m| m.tokens).unwrap_or(0),
            dataset_pages: meta.map(|m| m.pages).unwrap_or(0),
            cost_sol: 0.0,
            funded_sol: 0.0,
            params: Some(info.params),
            trained_at,
            weights_url: None,
        }
    } else {
        // no verifiable model yet — nothing is claimed until a run finishes
        QueenVersion {
            version: 0,
            status: QueenStatus::Collecting,
            dataset_tokens: 0,
            dataset_pages: 0,
            cost_sol: 0.0,
            funded_sol: 0.0,
            params: None,
            trained_at: None,
            weights_url: None,
        }
    }
}

/// The next version: trains once the crawl has read RETRAIN_DELTA more pages than
/// the live one saw (or FIRST_AT for the very first run). Progress is live data.
pub fn next_queen(cur: &QueenVersion, training: bool) -> QueenVersion {
    let (pages_now, tokens_now) = dataset_counts();
    let target_pages = if cur.version == 0 { FIRST_AT } else { cur.dataset_pages + RETRAIN_DELTA };
    let avg = if pages_now > 0 { tokens_now as f64 / pages_now as f64 } else { 0.0 };
    QueenVersion {
        version: cur.version + 1,
        status: if training { QueenStatus::Training } else { QueenStatus::Collecting },
        dataset_tokens: (target_pages as f64 * avg) as u64,
        dataset_pages: target_pages,
        cost_sol: 0.0,
        funded_sol: 0.0,
        params: None,
        trained_at: None,
        weights_url: None,
    }
}

/// Refresh snapshot.stats.queen + next_queen from the on-disk model + dataset.
pub async fn refresh_stats(state: &AppState) {
    let qv = queen_version();
    let nq = next_queen(&qv, false);
    let mut s = state.snapshot.write().await;
    s.stats.queen = qv;
    s.stats.next_queen = nq;
}

/// Generate text continuing `prompt` (blocking CPU work, off the async runtime).
pub async fn generate(prompt: String, n: usize, temp: f64) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || {
        let q = octopuscrawl_queen::Queen::load(dir())?;
        q.generate(&prompt, n, temp)
    })
    .await?
}

fn record_count() -> u64 {
    std::fs::read_to_string(dir().join("corpus.jsonl"))
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count() as u64)
        .unwrap_or(0)
}

/// Background (re)training: train once there is enough corpus, then retrain as
/// it grows. Conservative so it does not fight the crawler for the CPU.
pub fn spawn_trainer(state: Arc<AppState>) {
    // publish the current model's status on boot
    {
        let state = state.clone();
        tokio::spawn(async move {
            refresh_stats(&state).await;
        });
    }
    tokio::spawn(async move {
        // resume from what the live version actually read (0 if unknown → retrain
        // soon on the clean dataset so the numbers become verifiable)
        let mut trained_records: u64 = read_trained().map(|m| m.pages).unwrap_or(0);
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let records = record_count();
            let due = if trained() {
                records >= trained_records + RETRAIN_DELTA
            } else {
                records >= FIRST_AT
            };
            if !due {
                continue;
            }
            // the live version keeps answering; the NEXT one is what trains
            let (pages_at, tokens_at) = dataset_counts();
            let next_version = queen_version().version + 1;
            {
                let mut s = state.snapshot.write().await;
                let cur = s.stats.queen.clone();
                s.stats.next_queen = next_queen(&cur, true);
            }
            let stats = state.snapshot.read().await.stats.clone();
            let _ = state.tx.send(octopuscrawl_core::LiveMsg::Stats { stats });
            println!("[queen] training v{next_version} on {pages_at} pages…");

            let res = tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
                let text = octopuscrawl_queen::corpus_text(&dir().join("corpus.jsonl"))?;
                let rep = octopuscrawl_queen::train(&text, TRAIN_STEPS, dir())?;
                println!(
                    "[queen] trained: {} params · loss {:.3} · vocab {}",
                    rep.params, rep.final_loss, rep.vocab
                );
                Ok(rep.params)
            })
            .await;

            match res {
                Ok(Ok(_)) => {
                    write_trained(Trained { version: next_version, pages: pages_at, tokens: tokens_at });
                    trained_records = pages_at;
                }
                other => eprintln!("[queen] training failed: {other:?}"),
            }
            refresh_stats(&state).await;
            let stats = state.snapshot.read().await.stats.clone();
            let _ = state.tx.send(octopuscrawl_core::LiveMsg::Stats { stats });

            // never retrain back-to-back
            tokio::time::sleep(Duration::from_secs(1800)).await;
        }
    });
}
