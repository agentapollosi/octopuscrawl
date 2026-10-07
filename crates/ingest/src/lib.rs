//! Ingest: turn raw crawled pages into dataset records.
//!
//! - **dedup** — SimHash over word shingles drops near-duplicate pages, so the
//!   corpus is not dominated by boilerplate-heavy near-copies.
//! - **tokenize** — a BPE tokenizer trained on the crawl itself gives real token
//!   counts (and is retrained as the corpus grows).
//! - **store** — accepted pages are appended to a JSONL dataset on disk.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;
use tokenizers::models::bpe::{BpeTrainer, BPE};
use tokenizers::models::TrainerWrapper;
use tokenizers::pre_tokenizers::whitespace::Whitespace;
use tokenizers::Tokenizer;

const DUP_DISTANCE: u32 = 4; // Hamming distance under which two pages are "the same"
const VOCAB: usize = 16_000;
const FIRST_TRAIN_AT: usize = 24;
const RETRAIN_EVERY: usize = 60;

pub struct Ingested {
    pub accepted: bool,
    pub tokens: u32,
}

/// A snapshot of the trained tokenizer, surfaced to the API/frontend.
pub struct VocabInfo {
    pub trained: bool,
    pub size: u32,
    pub accepted: u32,
    pub next_train_at: u32,
    pub terms: Vec<String>,
}

pub struct Ingest {
    dataset: PathBuf,
    tokenizer_path: PathBuf,
    dedup: Vec<u64>,
    /// Every URL already in the dataset — a page is counted once, ever, even if
    /// its content changes between reads.
    urls: std::collections::HashSet<String>,
    tok: Option<Tokenizer>,
    texts: Vec<String>,
    accepted: usize,
    last_train: usize,
}

#[derive(Serialize)]
struct Record<'a> {
    url: &'a str,
    host: &'a str,
    chapter: &'a str,
    title: &'a str,
    tokens: u32,
    text: &'a str,
}

impl Ingest {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = fs::create_dir_all(&dir);
        let dataset = dir.join("corpus.jsonl");
        let tokenizer_path = dir.join("tokenizer.json");
        // Resume across restarts: reload the trained tokenizer, rebuild the dedup
        // set from the existing corpus (so re-read seeds are not appended again),
        // and keep one record per URL — rewriting the file de-duplicated so a
        // restart never piles up copies. This keeps the dataset + counters stable
        // across redeploys instead of resetting / growing duplicates.
        let tok = Tokenizer::from_file(&tokenizer_path).ok();

        let mut dedup = Vec::new();
        let mut texts = Vec::new();
        let mut unique_lines: Vec<String> = Vec::new();
        let mut seen_urls = std::collections::HashSet::new();
        let mut had_dupes = false;
        if let Ok(body) = fs::read_to_string(&dataset) {
            for line in body.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    let url = v.get("url").and_then(|u| u.as_str()).unwrap_or("");
                    if url.is_empty() || !seen_urls.insert(url.to_string()) {
                        had_dupes = true;
                        continue;
                    }
                    let mut v = v;
                    if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
                        let cleaned = redact_emails(t);
                        if cleaned != t {
                            had_dupes = true; // forces a rewrite of the cleaned file
                            v["text"] = serde_json::Value::String(cleaned);
                        }
                    }
                    unique_lines.push(serde_json::to_string(&v).unwrap_or_else(|_| line.to_string()));
                    if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
                        dedup.push(simhash(t));
                        texts.push(t.chars().take(4000).collect());
                    }
                }
            }
        }
        if texts.len() > 500 {
            let n = texts.len() - 500;
            texts.drain(0..n);
        }
        let accepted = unique_lines.len();
        if had_dupes {
            let mut out = unique_lines.join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            let _ = fs::write(&dataset, out);
        }

        Ingest {
            dataset,
            tokenizer_path,
            dedup,
            urls: seen_urls,
            tok,
            texts,
            accepted,
            last_train: accepted,
        }
    }

    pub fn accepted(&self) -> usize {
        self.accepted
    }
    pub fn has_tokenizer(&self) -> bool {
        self.tok.is_some()
    }

    /// A snapshot of the queen's forming vocabulary: whether a tokenizer exists,
    /// its learned size, dataset progress, and a sample of the longer subword
    /// tokens it has learned (the most-frequent merges first).
    pub fn vocab_info(&self) -> VocabInfo {
        let trained = self.tok.is_some();
        let size = self.tok.as_ref().map(|t| t.get_vocab_size(false) as u32).unwrap_or(0);
        let next_train_at = if self.accepted < FIRST_TRAIN_AT {
            FIRST_TRAIN_AT as u32
        } else {
            (self.last_train + RETRAIN_EVERY) as u32
        };
        let mut terms = Vec::new();
        if let Some(t) = &self.tok {
            // longer, purely-alphabetic subwords, earliest merges first = the
            // tokens the queen learned to recognise most often.
            let mut items: Vec<(String, u32)> = t
                .get_vocab(false)
                .into_iter()
                .filter(|(k, _)| k.len() >= 4 && k.chars().all(|c| c.is_ascii_alphabetic()))
                .collect();
            items.sort_by_key(|(_, id)| *id);
            terms = items.into_iter().map(|(k, _)| k).take(64).collect();
        }
        VocabInfo {
            trained,
            size,
            accepted: self.accepted as u32,
            next_train_at,
            terms,
        }
    }

    /// Process one crawled page. Returns whether it was accepted (not a
    /// near-duplicate) and its real token count.
    pub fn process(&mut self, url: &str, host: &str, chapter: &str, title: &str, text: &str) -> Ingested {
        if self.urls.contains(url) {
            return Ingested { accepted: false, tokens: 0 };
        }
        // personal contact details never reach the dataset
        let clean = redact_emails(text);
        let text = clean.as_str();
        let h = simhash(text);
        if self.dedup.iter().any(|s| hamming(*s, h) < DUP_DISTANCE) {
            return Ingested { accepted: false, tokens: 0 };
        }
        self.dedup.push(h);
        self.urls.insert(url.to_string());

        let tokens = self.count(text) as u32;
        self.append(url, host, chapter, title, text, tokens);

        self.texts.push(text.chars().take(4000).collect());
        if self.texts.len() > 500 {
            self.texts.remove(0);
        }
        self.accepted += 1;
        if self.accepted == FIRST_TRAIN_AT || self.accepted >= self.last_train + RETRAIN_EVERY {
            self.retrain();
            self.last_train = self.accepted;
        }
        Ingested { accepted: true, tokens }
    }

    fn count(&self, text: &str) -> usize {
        match &self.tok {
            Some(t) => t
                .encode(text, false)
                .map(|e| e.len())
                .unwrap_or_else(|_| text.split_whitespace().count()),
            None => text.split_whitespace().count(),
        }
    }

    fn retrain(&mut self) {
        if let Ok(t) = train(&self.texts, VOCAB) {
            let _ = t.save(&self.tokenizer_path, false);
            self.tok = Some(t);
        }
    }

    fn append(&self, url: &str, host: &str, chapter: &str, title: &str, text: &str, tokens: u32) {
        let rec = Record { url, host, chapter, title, tokens, text };
        if let Ok(line) = serde_json::to_string(&rec) {
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.dataset) {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

/// Train a BPE tokenizer on the given texts.
pub fn train(texts: &[String], vocab: usize) -> anyhow::Result<Tokenizer> {
    let mut tok = Tokenizer::new(BPE::default());
    tok.with_pre_tokenizer(Some(Whitespace {}));
    let mut trainer: TrainerWrapper = BpeTrainer::builder()
        .vocab_size(vocab)
        .min_frequency(2)
        .show_progress(false)
        .build()
        .into();
    tok.train(&mut trainer, texts.iter().map(|s| s.as_str()))
        .map_err(|e| anyhow::anyhow!("train: {e}"))?;
    Ok(tok)
}

/// 64-bit SimHash over words — near-identical pages (re-fetches, templated pages
/// that differ by a few tokens) land within a small Hamming distance.
pub fn simhash(text: &str) -> u64 {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return fnv1a(text);
    }
    let mut v = [0i32; 64];
    for w in &words {
        let h = fnv1a(w);
        for (i, slot) in v.iter_mut().enumerate() {
            if (h >> i) & 1 == 1 {
                *slot += 1;
            } else {
                *slot -= 1;
            }
        }
    }
    let mut out = 0u64;
    for (i, slot) in v.iter().enumerate() {
        if *slot > 0 {
            out |= 1 << i;
        }
    }
    out
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

fn fnv1a(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}


/// Replace e-mail addresses with `[email]` so no personal contact detail is kept.
pub fn redact_emails(text: &str) -> String {
    fn local(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-')
    }
    fn domain(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '-')
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut start_ok = 0; // chars before this index are already in `out`
    while i < chars.len() {
        if chars[i] == '@' {
            let mut a = i;
            while a > start_ok && local(chars[a - 1]) {
                a -= 1;
            }
            let mut b = i + 1;
            while b < chars.len() && domain(chars[b]) {
                b += 1;
            }
            // trim a trailing dot ("mail me at x@y.com.")
            while b > i + 1 && chars[b - 1] == '.' {
                b -= 1;
            }
            let dom: String = chars[i + 1..b].iter().collect();
            let tld_ok = dom
                .rsplit_once('.')
                .map(|(l, t)| !l.is_empty() && t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic()))
                .unwrap_or(false);
            if a < i && tld_ok {
                out.extend(&chars[start_ok..a]);
                out.push_str("[email]");
                start_ok = b;
                i = b;
                continue;
            }
        }
        i += 1;
    }
    out.extend(&chars[start_ok..]);
    out
}

#[cfg(test)]
mod redact_tests {
    use super::redact_emails;

    #[test]
    fn redacts_addresses_only() {
        assert_eq!(redact_emails("mail secure@example.org now."), "mail [email] now.");
        assert_eq!(redact_emails("a.b+c@sub.mail.co.uk"), "[email]");
        assert_eq!(redact_emails("git@ and @user and x@y"), "git@ and @user and x@y");
        assert_eq!(redact_emails("no email here"), "no email here");
    }
}
