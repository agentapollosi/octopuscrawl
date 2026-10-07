//! octopuscrawl-api — Axum REST + WebSocket server.
//!
//! Milestone 0: serves the live contract from an in-memory demo feed. The demo
//! loop is swapped for the real chromiumoxide crawler engine later; the wire
//! shape (`LiveMsg`) and the broadcast fan-out stay the same.

mod corpus;
mod demo;
mod frames;
mod treasury;
#[cfg(feature = "real")]
mod crawl;
#[cfg(feature = "real")]
mod brain;

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use octopuscrawl_core::*;
use serde::Deserialize;
use tokio::sync::{broadcast, RwLock};
use tower_http::cors::CorsLayer;

use corpus::Corpus;
use frames::FrameStore;

struct Snapshot {
    stats: Stats,
    crawlers: Vec<Crawler>,
}

struct AppState {
    tx: broadcast::Sender<LiveMsg>,
    snapshot: RwLock<Snapshot>,
    frames: FrameStore,
    corpus: Corpus,
    graph: RwLock<GraphSnapshot>,
    vocab: RwLock<VocabStats>,
    treasury: RwLock<TreasurySnapshot>,
}

/// On-disk dataset the crawl produces (relative to the server's working dir).
const DATASET_PATH: &str = "data/corpus.jsonl";
/// Persisted knowledge graph, so the map survives restarts.
const GRAPH_PATH: &str = "data/graph.json";

fn load_graph() -> GraphSnapshot {
    std::fs::read_to_string(GRAPH_PATH)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Periodically flush the live graph to disk so it survives restarts.
fn spawn_graph_saver(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let g = state.graph.read().await.clone();
            if g.nodes.is_empty() {
                continue;
            }
            if let Ok(s) = serde_json::to_string(&g) {
                let _ = tokio::fs::create_dir_all("data").await;
                let _ = tokio::fs::write(GRAPH_PATH, s).await;
            }
        }
    });
}

#[tokio::main]
async fn main() {
    let crawlers = demo::seed_crawlers(24);
    let stats = demo::seed_stats(&crawlers);
    let (tx, _rx) = broadcast::channel::<LiveMsg>(1024);
    let state = Arc::new(AppState {
        tx,
        snapshot: RwLock::new(Snapshot { stats, crawlers }),
        frames: FrameStore::default(),
        corpus: Corpus::default(),
        graph: RwLock::new(load_graph()),
        vocab: RwLock::new(VocabStats::default()),
        treasury: RwLock::new(TreasurySnapshot::default()),
    });

    spawn_engine(state.clone());
    spawn_graph_saver(state.clone());
    treasury::spawn_poller(state.clone());

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/stats", get(get_stats))
        .route("/v1/crawlers", get(get_crawlers))
        .route("/v1/crawlers/{id}/frame.jpg", get(get_frame))
        .route("/v1/queen/ask", get(queen_ask))
        .route("/v1/queen/generate", get(queen_generate))
        .route("/v1/queen/weights", get(queen_weights))
        .route("/v1/graph", get(get_graph))
        .route("/v1/vocab", get(get_vocab))
        .route("/v1/dataset.jsonl", get(get_dataset))
        .route("/v1/dataset/meta", get(get_dataset_meta))
        .route("/v1/treasury", get(get_treasury))
        .route("/v1/live", get(ws_upgrade))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = "127.0.0.1:8787";
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    println!("octopuscrawl-api listening on http://{addr}");
    axum::serve(listener, app).await.expect("serve");
}

async fn get_stats(State(state): State<Arc<AppState>>) -> Json<Stats> {
    Json(state.snapshot.read().await.stats.clone())
}

async fn get_graph(State(state): State<Arc<AppState>>) -> Json<GraphSnapshot> {
    Json(state.graph.read().await.clone())
}

async fn get_vocab(State(state): State<Arc<AppState>>) -> Json<VocabStats> {
    Json(state.vocab.read().await.clone())
}

async fn get_treasury(State(state): State<Arc<AppState>>) -> Json<TreasurySnapshot> {
    Json(state.treasury.read().await.clone())
}

async fn get_dataset_meta(State(_state): State<Arc<AppState>>) -> Json<DatasetMeta> {
    let mut meta = DatasetMeta::default();
    let mut hosts = std::collections::HashSet::new();
    if let Ok(body) = tokio::fs::read_to_string(DATASET_PATH).await {
        meta.bytes = body.len() as u64;
        for line in body.lines() {
            if line.trim().is_empty() {
                continue;
            }
            meta.records += 1;
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                meta.tokens += v.get("tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                if let Some(h) = v.get("host").and_then(|h| h.as_str()) {
                    hosts.insert(h.to_string());
                }
            }
        }
    }
    meta.hosts = hosts.len() as u32;
    Json(meta)
}

async fn get_dataset(State(_state): State<Arc<AppState>>) -> Response {
    match tokio::fs::read(DATASET_PATH).await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"octopuscrawl-corpus.jsonl\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "dataset not ready yet").into_response(),
    }
}

async fn get_crawlers(State(state): State<Arc<AppState>>) -> Json<Vec<Crawler>> {
    Json(state.snapshot.read().await.crawlers.clone())
}

async fn get_frame(Path(id): Path<String>, State(state): State<Arc<AppState>>) -> Response {
    match state.frames.get(&id) {
        Some((_seq, jpeg)) => (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            jpeg,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct AskQuery {
    q: Option<String>,
}

/// Ask the queen — honest retrieval over what the crawl has read so far.
async fn queen_ask(
    Query(aq): Query<AskQuery>,
    State(state): State<Arc<AppState>>,
) -> Json<corpus::AskResponse> {
    Json(state.corpus.ask(aq.q.as_deref().unwrap_or("")))
}

#[derive(Deserialize)]
struct GenQuery {
    prompt: Option<String>,
    n: Option<usize>,
    temp: Option<f64>,
}

/// The queen writes — free generation from her from-scratch model.
async fn queen_generate(Query(gq): Query<GenQuery>, State(_state): State<Arc<AppState>>) -> Response {
    let _ = &gq;
    #[cfg(feature = "real")]
    {
        let prompt = gq.prompt.unwrap_or_default();
        let n = gq.n.unwrap_or(200).clamp(16, 400);
        let temp = gq.temp.unwrap_or(0.8).clamp(0.2, 1.5);
        return match brain::generate(prompt, n, temp).await {
            Ok(text) => Json(serde_json::json!({ "text": text })).into_response(),
            Err(e) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "text": format!("the queen is still learning to write — {e}") })),
            )
                .into_response(),
        };
    }
    #[cfg(not(feature = "real"))]
    {
        Json(serde_json::json!({ "text": "the queen is not wired in this build" })).into_response()
    }
}

/// Download the queen's actual weights (safetensors).
async fn queen_weights(State(_state): State<Arc<AppState>>) -> Response {
    match tokio::fs::read("data/queen.safetensors").await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"octopuscrawl-queen-v1.safetensors\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "the queen has no trained weights yet").into_response(),
    }
}

/// Start the live data source: the real crawl engine when built with
/// `--features real` and `OCTOPUSCRAWL_CRAWL=1`, otherwise the demo feed.
fn spawn_engine(state: Arc<AppState>) {
    #[cfg(feature = "real")]
    {
        if std::env::var("OCTOPUSCRAWL_CRAWL").as_deref() == Ok("1") {
            tokio::spawn(crawl::run(state.clone()));
            brain::spawn_trainer(state.clone());
            tokio::spawn(stats_broadcaster(state));
            return;
        }
    }
    tokio::spawn(run_demo(state));
}

/// The real engine updates the snapshot in place; push it to live viewers on a
/// steady cadence so counters and the queen's status update without a reload.
async fn stats_broadcaster(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(Duration::from_millis(2000)).await;
        let stats = {
            let mut snap = state.snapshot.write().await;
            snap.stats.updated_at = chrono::Utc::now();
            snap.stats.clone()
        };
        let _ = state.tx.send(LiveMsg::Stats { stats });
    }
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = state.tx.subscribe();

    // Send the opening snapshot.
    let hello = {
        let snap = state.snapshot.read().await;
        LiveMsg::Hello {
            stats: snap.stats.clone(),
            crawlers: snap.crawlers.clone(),
            lite: true,
        }
    };
    if send_json(&mut sink, &hello).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(m) => {
                    if send_json(&mut sink, &m).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Client fell behind — resend a fresh snapshot and carry on.
                    let hello = {
                        let snap = state.snapshot.read().await;
                        LiveMsg::Hello {
                            stats: snap.stats.clone(),
                            crawlers: snap.crawlers.clone(),
                            lite: true,
                        }
                    };
                    if send_json(&mut sink, &hello).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Text(_))) => {
                    // ClientMsg (ping / frames subscription) parsed here later.
                }
                Some(Ok(Message::Binary(_))) => {}
                Some(Err(_)) => break,
            },
        }
    }
}

async fn send_json(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    msg: &LiveMsg,
) -> Result<(), ()> {
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}

/// Advance the demo world and broadcast the changes.
async fn run_demo(state: Arc<AppState>) {
    let mut r = demo::Lcg::new(0xDEADBEEF);
    let mut ticks: u64 = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(700)).await;
        ticks += 1;

        let n = { state.snapshot.read().await.crawlers.len() };
        if n == 0 {
            continue;
        }
        let idx = r.below(n);

        let (crawler_msg, extra) = {
            let mut snap = state.snapshot.write().await;
            let produced = demo::tick_crawler(&mut snap.crawlers[idx], &mut r);
            let crawler = snap.crawlers[idx].clone();
            // Refresh derived counters.
            snap.stats.crawlers_running = snap
                .crawlers
                .iter()
                .filter(|c| c.status == CrawlerStatus::Crawling)
                .count() as u32;
            if produced.is_some() {
                snap.stats.pages_read += 1;
                snap.stats.dataset_tokens += produced.as_ref().unwrap().0.tokens as u64;
            }
            (crawler, produced)
        };

        let _ = state.tx.send(LiveMsg::Crawler { crawler: crawler_msg });
        if let Some((node, entry)) = extra {
            let _ = state.tx.send(LiveMsg::Page { node, edge: None });
            let _ = state.tx.send(LiveMsg::Ledger { entry });
        }

        // Broadcast stats roughly every 2s (every 3 ticks).
        if ticks % 3 == 0 {
            let stats = {
                let mut snap = state.snapshot.write().await;
                snap.stats.updated_at = chrono::Utc::now();
                snap.stats.clone()
            };
            let _ = state.tx.send(LiveMsg::Stats { stats });
        }
    }
}
