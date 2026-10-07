//! The queen's reading, searchable. Until a model is trained, "ask the queen"
//! is honest retrieval over the pages the crawl has actually read: it ranks the
//! corpus by the question's terms and returns the closest pages with snippets.

use std::sync::{Arc, Mutex};

use serde::Serialize;

#[derive(Clone)]
pub struct Doc {
    pub url: String,
    pub title: String,
    pub host: String,
    pub chapter: String,
    pub text: String,
}

#[derive(Clone, Default)]
pub struct Corpus(Arc<Mutex<Vec<Doc>>>);

#[derive(Serialize)]
pub struct Source {
    pub title: String,
    pub url: String,
    pub host: String,
    pub snippet: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskResponse {
    pub answer: String,
    pub sources: Vec<Source>,
    pub pages_read: usize,
}

impl Corpus {
    pub fn add(&self, d: Doc) {
        let mut g = self.0.lock().unwrap();
        if g.iter().any(|x| x.url == d.url) {
            return;
        }
        g.push(d);
        if g.len() > 400 {
            let n = g.len() - 400;
            g.drain(0..n);
        }
    }

    /// Number of distinct hosts read so far — the real "domains" counter.
    pub fn host_count(&self) -> usize {
        let g = self.0.lock().unwrap();
        let mut seen = std::collections::HashSet::new();
        for d in g.iter() {
            seen.insert(d.host.as_str());
        }
        seen.len()
    }

    pub fn ask(&self, q: &str) -> AskResponse {
        let terms: Vec<String> = q
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2)
            .map(|w| w.to_string())
            .collect();

        let g = self.0.lock().unwrap();
        let pages_read = g.len();

        if pages_read == 0 {
            return AskResponse {
                answer: "the queen has not read anything yet — start the crawl engine.".into(),
                sources: vec![],
                pages_read,
            };
        }
        if terms.is_empty() {
            return AskResponse {
                answer: format!("ask me about something I have read across {pages_read} pages."),
                sources: vec![],
                pages_read,
            };
        }

        let mut scored: Vec<(f64, &Doc)> = g
            .iter()
            .map(|d| {
                let tl = d.title.to_lowercase();
                let tx = d.text.to_lowercase();
                let mut s = 0.0;
                for t in &terms {
                    s += tl.matches(t.as_str()).count() as f64 * 4.0;
                    s += tx.matches(t.as_str()).count() as f64;
                }
                (s, d)
            })
            .filter(|(s, _)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        if scored.is_empty() {
            return AskResponse {
                answer: format!(
                    "nothing in the {pages_read} pages I have read matches that yet — I am still small."
                ),
                sources: vec![],
                pages_read,
            };
        }

        let sources: Vec<Source> = scored
            .iter()
            .take(4)
            .map(|(_, d)| Source {
                title: d.title.clone(),
                url: d.url.clone(),
                host: d.host.clone(),
                snippet: snippet(&d.text, &terms),
            })
            .collect();
        let top = scored[0].1;
        let answer = format!(
            "from what I have read ({} pages): closest is \u{201c}{}\u{201d} on {} — {}",
            pages_read,
            top.title,
            top.host,
            snippet(&top.text, &terms)
        );
        AskResponse {
            answer,
            sources,
            pages_read,
        }
    }
}

fn snippet(text: &str, terms: &[String]) -> String {
    let lower = text.to_lowercase();
    let hit = terms.iter().filter_map(|t| lower.find(t.as_str())).min();
    let mut start = hit.map(|p| p.saturating_sub(60)).unwrap_or(0);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    let s: String = text[start..].chars().take(220).collect();
    format!("…{}…", s.trim())
}
