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

/// Dataset she was pretrained on: (pages, tokens) straight from the corpus file.
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

/// The QueenVersion reflecting the on-disk model (or "funding" when untrained).
pub fn queen_version() -> QueenVersion {
    let (dataset_pages, dataset_tokens) = dataset_counts();
    if let Some(info) = octopuscrawl_queen::model_info(dir()) {
        let trained_at = std::fs::metadata(dir().join("queen.safetensors"))
            .ok()
            .and_then(|m| m.modified().ok())
            .map(DateTime::<Utc>::from);
        QueenVersion {
            version: 1,
            status: QueenStatus::Live,
            dataset_tokens,
            dataset_pages,
            cost_sol: 0.0,
            funded_sol: 0.0,
            params: Some(info.params),
            trained_at,
            weights_url: Some("/v1/queen/weights".to_string()),
        }
    } else {
        QueenVersion {
            version: 1,
            status: QueenStatus::Funding,
            dataset_tokens,
            dataset_pages,
            cost_sol: 0.0,
            funded_sol: 0.0,
            params: None,
            trained_at: None,
            weights_url: None,
        }
    }
}

/// Refresh snapshot.stats.queen from the on-disk model + dataset.
pub async fn refresh_stats(state: &AppState) {
    let qv = queen_version();
    let mut s = state.snapshot.write().await;
    s.stats.queen = qv;
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
        let mut trained_records: u64 = if trained() { record_count() } else { 0 };
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
            {
                let mut s = state.snapshot.write().await;
                s.stats.queen.status = QueenStatus::Training;
            }
            let stats = state.snapshot.read().await.stats.clone();
            let _ = state.tx.send(octopuscrawl_core::LiveMsg::Stats { stats });
            println!("[queen] training on {records} records…");

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
                Ok(Ok(_)) => trained_records = records,
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
