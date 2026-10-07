//! WebSocket client for the live feed. Parses `LiveMsg` straight into the shared
//! `octopuscrawl-core` types and pushes updates into Leptos signals, reconnecting on drop.

use futures::StreamExt;
use gloo_net::http::Request;
use gloo_net::websocket::{futures::WebSocket, Message};
use octopuscrawl_core::{
    Crawler, DatasetMeta, Edge, GraphSnapshot, LedgerEntry, LiveMsg, Stats, TreasurySnapshot,
    VocabStats, WebNode, HistPoint, SponsorView};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

/// Base HTTP origin for the API: the dev server on :8787 when the page is viewed
/// locally, otherwise the same origin the page was served from (so a hosted
/// build works behind a reverse proxy that forwards `/v1` to the API).
pub fn http_base() -> String {
    let loc = web_sys::window().expect("window").location();
    let host = loc.host().unwrap_or_default();
    if host.starts_with("localhost") || host.starts_with("127.0.0.1") {
        "http://127.0.0.1:8787".to_string()
    } else {
        let proto = loc.protocol().unwrap_or_else(|_| "https:".into());
        format!("{proto}//{host}")
    }
}

/// WebSocket URL for the live feed, matching `http_base`'s origin logic.
pub fn ws_url() -> String {
    let loc = web_sys::window().expect("window").location();
    let host = loc.host().unwrap_or_default();
    if host.starts_with("localhost") || host.starts_with("127.0.0.1") {
        "ws://127.0.0.1:8787/v1/live".to_string()
    } else {
        let wsproto = if loc.protocol().unwrap_or_default() == "https:" {
            "wss:"
        } else {
            "ws:"
        };
        format!("{wsproto}//{host}/v1/live")
    }
}

/// One point in the live time-series, for sparklines and the growth chart.
#[derive(Clone, Copy)]
pub struct Sample {
    pub tokens: u64,
    pub pages: u64,
    pub running: u32,
}

/// One entry in the live reading feed.
#[derive(Clone)]
pub struct FeedItem {
    pub id: u64,
    pub host: String,
    pub chapter: String,
    pub tokens: u32,
}

/// All live state, held as copyable signals so components can share it freely.
#[derive(Clone, Copy)]
pub struct Live {
    pub stats: RwSignal<Option<Stats>>,
    pub crawlers: RwSignal<Vec<Crawler>>,
    pub log: RwSignal<Vec<String>>,
    pub ledger: RwSignal<Vec<LedgerEntry>>,
    pub history: RwSignal<Vec<Sample>>,
    pub connected: RwSignal<bool>,
    /// Live knowledge graph — pages read (nodes) and links followed (edges).
    pub nodes: RwSignal<Vec<WebNode>>,
    pub edges: RwSignal<Vec<Edge>>,
    /// The queen's forming vocabulary and the open-dataset metadata (polled).
    pub vocab: RwSignal<Option<VocabStats>>,
    pub dataset: RwSignal<Option<DatasetMeta>>,
    /// Live on-chain treasury wallet state.
    pub treasury: RwSignal<Option<TreasurySnapshot>>,
    /// Live reading feed (newest first) for the activity ticker.
    pub feed: RwSignal<Vec<FeedItem>>,
    /// The dataset's growth over the last 24h, sampled server-side each minute.
    pub growth: RwSignal<Vec<HistPoint>>,
    /// Sponsored hatchlings and what they have added.
    pub sponsors: RwSignal<Vec<SponsorView>>,
}

impl Live {
    pub fn new() -> Self {
        Self {
            stats: RwSignal::new(None),
            crawlers: RwSignal::new(Vec::new()),
            log: RwSignal::new(Vec::new()),
            ledger: RwSignal::new(Vec::new()),
            history: RwSignal::new(Vec::new()),
            connected: RwSignal::new(false),
            nodes: RwSignal::new(Vec::new()),
            edges: RwSignal::new(Vec::new()),
            vocab: RwSignal::new(None),
            dataset: RwSignal::new(None),
            treasury: RwSignal::new(None),
            feed: RwSignal::new(Vec::new()),
            growth: RwSignal::new(Vec::new()),
            sponsors: RwSignal::new(Vec::new()),
        }
    }
}

fn next_feed_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

fn push_sample(live: &Live, s: &Stats) {
    live.history.update(|h| {
        h.push(Sample {
            tokens: s.dataset_tokens,
            pages: s.pages_read,
            running: s.crawlers_running,
        });
        let n = h.len();
        if n > 240 {
            h.drain(0..n - 240);
        }
    });
}

pub fn connect(live: Live, url: &str) {
    let url = url.to_string();
    spawn_local(async move {
        loop {
            if let Ok(ws) = WebSocket::open(&url) {
                live.connected.set(true);
                let (_write, mut read) = ws.split();
                while let Some(Ok(msg)) = read.next().await {
                    let txt = match msg {
                        Message::Text(t) => t,
                        Message::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
                    };
                    if let Ok(m) = serde_json::from_str::<LiveMsg>(&txt) {
                        apply(&live, m);
                    }
                }
                live.connected.set(false);
            }
            // Reconnect after a short pause.
            gloo_timers::future::TimeoutFuture::new(1500).await;
        }
    });
}

fn apply(live: &Live, m: LiveMsg) {
    match m {
        LiveMsg::Hello { stats, crawlers, .. } => {
            push_sample(live, &stats);
            live.stats.set(Some(stats));
            live.crawlers.set(crawlers);
        }
        LiveMsg::Stats { stats } => {
            push_sample(live, &stats);
            live.stats.set(Some(stats));
        }
        LiveMsg::Crawler { crawler } => live.crawlers.update(|v| {
            if let Some(slot) = v.iter_mut().find(|c| c.id == crawler.id) {
                *slot = crawler;
            } else {
                v.push(crawler);
            }
        }),
        LiveMsg::Page { node, edge } => {
            let line = format!(
                "{}  {}  {}k tok",
                node.read_at.format("%H:%M:%S%.3f"),
                node.url,
                node.tokens / 1000
            );
            live.log.update(|v| {
                v.insert(0, line);
                v.truncate(14);
            });
            let item = FeedItem {
                id: next_feed_id(),
                host: node.domain.clone(),
                chapter: node.chapter.clone().unwrap_or_default(),
                tokens: node.tokens,
            };
            live.feed.update(|v| {
                v.insert(0, item);
                v.truncate(12);
            });
            ingest_page(live, node, edge);
        }
        LiveMsg::Ledger { entry } => live.ledger.update(|v| {
            v.insert(0, entry);
            v.truncate(60);
        }),
        LiveMsg::Frames { .. } => {}
        LiveMsg::Treasury { treasury } => live.treasury.set(Some(treasury)),
    }
}

/// Insert a freshly-read page (and the link that led to it) into the live graph,
/// deduped by stable id and bounded so the view stays light.
fn ingest_page(live: &Live, node: WebNode, edge: Option<Edge>) {
    live.nodes.update(|ns| {
        if !ns.iter().any(|n| n.id == node.id) {
            ns.push(node);
            let n = ns.len();
            if n > 240 {
                ns.drain(0..n - 240);
            }
        }
    });
    if let Some(e) = edge {
        live.edges.update(|es| {
            if e.from != e.to && !es.iter().any(|x| x.from == e.from && x.to == e.to) {
                es.push(e);
                let n = es.len();
                if n > 480 {
                    es.drain(0..n - 480);
                }
            }
        });
    }
}

/// Fetch the initial graph snapshot, then poll the vocabulary and dataset
/// metadata (neither rides the WebSocket). Safe to call once on mount.
pub fn start_feeds(live: Live) {
    spawn_local(async move {
        if let Ok(resp) = Request::get(&format!("{}/v1/graph", http_base())).send().await {
            if let Ok(g) = resp.json::<GraphSnapshot>().await {
                live.nodes.set(g.nodes);
                live.edges.set(g.edges);
            }
        }
    });
    spawn_local(async move {
        loop {
            refresh_sponsors(live).await;
            gloo_timers::future::TimeoutFuture::new(10_000).await;
        }
    });
    spawn_local(async move {
        loop {
            if let Ok(resp) = Request::get(&format!("{}/v1/history?hours=24", http_base())).send().await {
                if let Ok(h) = resp.json::<Vec<HistPoint>>().await {
                    live.growth.set(h);
                }
            }
            gloo_timers::future::TimeoutFuture::new(60_000).await;
        }
    });
    spawn_local(async move {
        loop {
            if let Ok(resp) = Request::get(&format!("{}/v1/vocab", http_base())).send().await {
                if let Ok(v) = resp.json::<VocabStats>().await {
                    live.vocab.set(Some(v));
                }
            }
            if let Ok(resp) = Request::get(&format!("{}/v1/dataset/meta", http_base())).send().await {
                if let Ok(d) = resp.json::<DatasetMeta>().await {
                    live.dataset.set(Some(d));
                }
            }
            if let Ok(resp) = Request::get(&format!("{}/v1/treasury", http_base())).send().await {
                if let Ok(t) = resp.json::<TreasurySnapshot>().await {
                    live.treasury.set(Some(t));
                }
            }
            gloo_timers::future::TimeoutFuture::new(5000).await;
        }
    });
}

/// Re-read the sponsored hatchlings (also called right after a spawn).
pub async fn refresh_sponsors(live: Live) {
    if let Ok(resp) = Request::get(&format!("{}/v1/sponsors", http_base())).send().await {
        if let Ok(v) = resp.json::<Vec<SponsorView>>().await {
            live.sponsors.set(v);
        }
    }
}
