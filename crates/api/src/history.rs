//! The dataset's growth over time: one sample a minute, kept for a week and
//! persisted so the curve survives restarts. Served at `/v1/history`.

use std::sync::Arc;
use std::time::Duration;

use octopuscrawl_core::{GraphSnapshot, HistPoint};

use crate::AppState;

const FILE: &str = "data/history.json";
const MAX_POINTS: usize = 7 * 24 * 60;

pub fn load() -> Vec<HistPoint> {
    std::fs::read_to_string(FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(h: &[HistPoint]) {
    if let Ok(body) = serde_json::to_string(h) {
        let tmp = format!("{FILE}.tmp");
        let _ = std::fs::create_dir_all("data");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, FILE);
        }
    }
}

/// With no saved history yet, rebuild the recent curve from the knowledge graph:
/// every node there is a real page with the moment it was read and its tokens,
/// so walking back from today's totals gives the true shape of recent growth.
fn backfill(g: &GraphSnapshot, pages_now: u64, tokens_now: u64) -> Vec<HistPoint> {
    let mut reads: Vec<(i64, u64)> = g.nodes.iter().map(|n| (n.read_at.timestamp(), n.tokens as u64)).collect();
    reads.sort_by_key(|r| r.0);
    let mut out = Vec::with_capacity(reads.len() + 2);
    let (mut p, mut tk) = (pages_now, tokens_now);
    out.push(HistPoint { t: crate::clock::now().timestamp(), pages: p, tokens: tk });
    for (t, tok) in reads.iter().rev() {
        out.push(HistPoint { t: *t, pages: p, tokens: tk });
        p = p.saturating_sub(1);
        tk = tk.saturating_sub(*tok);
        out.push(HistPoint { t: *t - 1, pages: p, tokens: tk });
    }
    out.reverse();
    // one point per minute is plenty
    let mut per_min: Vec<HistPoint> = Vec::new();
    for pt in out {
        match per_min.last_mut() {
            Some(last) if last.t / 60 == pt.t / 60 => *last = pt,
            _ => per_min.push(pt),
        }
    }
    per_min
}

pub fn spawn_sampler(state: Arc<AppState>) {
    tokio::spawn(async move {
        // let the crawl restore its counters from the dataset first
        tokio::time::sleep(Duration::from_secs(30)).await;
        {
            let empty = state.history.read().await.is_empty();
            if empty {
                let (pages, tokens) = {
                    let s = state.snapshot.read().await;
                    (s.stats.pages_read, s.stats.dataset_tokens)
                };
                let g = state.graph.read().await.clone();
                let h = backfill(&g, pages, tokens);
                println!("[history] rebuilt {} points from the graph's read times", h.len());
                save(&h);
                *state.history.write().await = h;
            }
        }
        loop {
            let (pages, tokens) = {
                let s = state.snapshot.read().await;
                (s.stats.pages_read, s.stats.dataset_tokens)
            };
            let snapshot = {
                let mut h = state.history.write().await;
                h.push(HistPoint { t: crate::clock::now().timestamp(), pages, tokens });
                let n = h.len();
                if n > MAX_POINTS {
                    h.drain(0..n - MAX_POINTS);
                }
                h.clone()
            };
            save(&snapshot);
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

/// The last `hours` of history, thinned to at most `max` points.
pub fn window(h: &[HistPoint], hours: i64, max: usize) -> Vec<HistPoint> {
    let since = crate::clock::now().timestamp() - hours * 3600;
    let pts: Vec<HistPoint> = h.iter().copied().filter(|p| p.t >= since).collect();
    if pts.len() <= max || max < 2 {
        return pts;
    }
    let step = pts.len() as f64 / (max - 1) as f64;
    let mut out: Vec<HistPoint> = (0..max - 1).map(|i| pts[(i as f64 * step) as usize]).collect();
    out.push(*pts.last().unwrap());
    out
}
