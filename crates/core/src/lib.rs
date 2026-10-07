//! octopuscrawl-core — the shared wire contract between the crawler engine, the API
//! and the Leptos frontend. Field names form a stable contract across all three;
//! domain-specific LABELS (the "THREAT" column, chapter titles, the queen's name)
//! live in `DomainConfig`, not here, so the whole thing re-skins from one config
//! file.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Stats — the global counters shown in the hero and the status bar.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub crawlers_total: u32,
    pub crawlers_running: u32,
    pub pages_read: u64,
    pub dataset_tokens: u64,
    pub domains: u32,
    pub projects_mapped: u32,
    pub projects_target: u32,
    pub queen: QueenVersion,
    pub next_queen: QueenVersion,
    pub chapters: Vec<Chapter>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueenVersion {
    pub version: u32,
    pub status: QueenStatus,
    pub dataset_tokens: u64,
    pub dataset_pages: u64,
    pub cost_sol: f64,
    pub funded_sol: f64,
    pub params: Option<u64>,
    pub trained_at: Option<DateTime<Utc>>,
    pub weights_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueenStatus {
    Retired,
    Live,
    Funding,
    Training,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub id: String,
    pub index: u32,
    pub title: String,
    pub target: u32,
    pub covered: u32,
    pub status: ChapterStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChapterStatus {
    Done,
    Active,
    Locked,
}

// ---------------------------------------------------------------------------
// Crawler — one octopus hatchling.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Crawler {
    pub id: String,
    pub name: String,
    /// `queen` = house crawler (unowned); `owned` = spawned by a wallet.
    pub kind: CrawlerKind,
    pub owner: Option<String>,
    pub status: CrawlerStatus,
    pub url: String,
    pub title: String,
    /// The link the crawler intends to visit next, if any.
    pub target: Option<LinkBox>,
    pub pages_read: u64,
    /// Human-readable line: "diving to nvd.nist.gov", "waiting for a free browser — 1 ahead".
    pub thought: String,
    /// Domain-relevance 0.0–1.0, scored by the queen. UI labels the column via DomainConfig.
    pub relevance: f64,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Monotonic counter bumped whenever `links` changes; lets the client skip refetching.
    pub links_seq: u64,
    /// Link bounding boxes on the current page — the tentacle targets. Omitted
    /// on the wire unless the client subscribed to this crawler's frames.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkBox>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CrawlerKind {
    Queen,
    Owned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CrawlerStatus {
    /// actively reading a page (status letter `R`)
    Crawling,
    /// carrying a page back to the queen (`W`)
    Returning,
    /// waiting for a free browser (`S`)
    Idle,
    /// something went wrong (`E`)
    Error,
}

impl CrawlerStatus {
    /// The single-letter code shown in the htop `S` column.
    pub fn letter(self) -> char {
        match self {
            CrawlerStatus::Crawling => 'R',
            CrawlerStatus::Returning => 'W',
            CrawlerStatus::Idle => 'S',
            CrawlerStatus::Error => 'E',
        }
    }
}

/// What kind of thing a box on the page is — so the animation can read the
/// actual content (a heading, a line of body text) instead of page chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoxKind {
    /// An in-content hyperlink (fallback target).
    #[default]
    Link,
    /// A heading (h1–h4) — the structure of what the queen is reading.
    Head,
    /// A line/block of body text (p, li, …) — the prose that becomes tokens.
    Text,
}

/// A content box's position on the 1280×800 page screenshot — a tentacle target.
/// Boxes arrive in reading order (top-to-bottom) and prefer real content
/// (headings and body text) over navigation and footers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkBox {
    pub href: String,
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub kind: BoxKind,
}

// ---------------------------------------------------------------------------
// Frames, pages, ledger.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub crawler_id: String,
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    pub at: DateTime<Utc>,
    /// Relative URL of the JPEG: `/v1/crawlers/{id}/frame.jpg?seq=N`.
    pub url: String,
}

/// A page accepted into the dataset — a node in the web graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebNode {
    pub id: String,
    pub url: String,
    pub domain: String,
    pub title: String,
    pub chapter: Option<String>,
    pub project_id: Option<String>,
    pub tokens: u32,
    pub crawler_id: String,
    pub read_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
}

/// The live knowledge graph: every page the crawl has read (nodes) and the
/// links it actually followed between them (edges). Served at `/v1/graph` for a
/// fresh viewer; thereafter it grows from `LiveMsg::Page`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSnapshot {
    pub nodes: Vec<WebNode>,
    pub edges: Vec<Edge>,
}

/// The queen's forming vocabulary — a real BPE tokenizer trained on the crawl.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabStats {
    /// Whether a tokenizer has been trained yet (needs a minimum of pages).
    pub trained: bool,
    /// Learned vocabulary size (subword tokens).
    pub size: u32,
    /// Pages accepted into the dataset so far (post-dedup).
    pub accepted: u32,
    /// Accepted-page count at which the tokenizer next retrains.
    pub next_train_at: u32,
    /// A sample of the longer subword tokens the queen has learned to recognise.
    pub terms: Vec<String>,
}

/// Transparency metadata for the open dataset the crawl is producing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetMeta {
    pub records: u64,
    pub bytes: u64,
    pub tokens: u64,
    pub hosts: u32,
}

/// One on-chain transaction touching the treasury wallet.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreasuryTx {
    pub sig: String,
    /// Unix seconds (block time), when known.
    pub time: Option<i64>,
    /// SOL delta for the treasury wallet (+ inflow, − outflow).
    pub delta_sol: f64,
    pub ok: bool,
}

/// Live, on-chain state of the treasury wallet (read-only, from Solana RPC).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreasurySnapshot {
    pub address: String,
    pub balance_sol: f64,
    pub updated_at: Option<DateTime<Utc>>,
    /// Whether the last RPC read succeeded (false before the first poll).
    pub ok: bool,
    pub txs: Vec<TreasuryTx>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntry {
    pub id: String,
    pub at: DateTime<Utc>,
    pub kind: LedgerKind,
    pub sol: f64,
    pub usd: f64,
    pub memo: String,
    pub tx: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LedgerKind {
    /// compute cost of a crawl (browser/model seconds)
    Crawl,
    /// creator fees in
    Fees,
    /// payout to a crawler owner
    Reward,
    /// a burn (spawn / sub-agent / order)
    Burn,
}

// ---------------------------------------------------------------------------
// WebSocket protocol — `wss://…/v1/live`.
// ---------------------------------------------------------------------------

/// Server → client. Internally tagged on `type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LiveMsg {
    /// Sent once on connect: a full snapshot.
    Hello {
        stats: Stats,
        crawlers: Vec<Crawler>,
        lite: bool,
    },
    Stats { stats: Stats },
    Crawler { crawler: Crawler },
    Page { node: WebNode, edge: Option<Edge> },
    Ledger { entry: LedgerEntry },
    Frames { frames: Vec<Frame> },
    Treasury { treasury: TreasurySnapshot },
}

/// Client → server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMsg {
    Ping,
    /// Subscribe to link boxes + frame notifications for up to 64 crawlers.
    Frames {
        crawler_ids: Vec<String>,
        #[serde(default)]
        mode: String,
        #[serde(default)]
        lite: bool,
    },
}
