//! The queen's forming vocabulary — a real BPE tokenizer trained on the crawl.
//! Shows how many subword tokens she has learned, progress to the next retrain,
//! and a live cloud of the actual security subwords she now recognises.

use leptos::prelude::*;

use crate::ws::Live;

fn grouped(n: u64) -> String {
    let s = n.to_string();
    let len = s.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[component]
pub fn VocabPanel(live: Live) -> impl IntoView {
    let trained = move || live.vocab.get().map(|v| v.trained).unwrap_or(false);
    let size = move || live.vocab.get().map(|v| v.size).unwrap_or(0);
    let accepted = move || live.vocab.get().map(|v| v.accepted).unwrap_or(0);
    let next_at = move || live.vocab.get().map(|v| v.next_train_at).unwrap_or(0);
    let terms = move || live.vocab.get().map(|v| v.terms).unwrap_or_default();
    let pct = move || {
        let a = accepted() as f64;
        let n = (next_at().max(1)) as f64;
        (a / n * 100.0).clamp(0.0, 100.0)
    };

    view! {
        <section class="vocab pane">
            <div class="pane__bar">
                "the queen's vocabulary"
                <span class="dim">" — a BPE tokenizer she trains on the crawl itself"</span>
            </div>
            <div class="vocab__top">
                <div class="vocab__stat">
                    <span class="dim">"subword tokens learned"</span>
                    <b class="num">{move || grouped(size() as u64)}</b>
                </div>
                <div class="vocab__stat">
                    <span class="dim">"accepted pages"</span>
                    <b class="num">{move || grouped(accepted() as u64)}</b>
                </div>
                <div class="vocab__stat vocab__stat--wide">
                    <span class="dim">
                        {move || format!("next retrain at {} accepted pages", grouped(next_at() as u64))}
                    </span>
                    <span class="meter meter--wide">
                        <span class="meter__f" style=move || format!("width:{:.0}%", pct())></span>
                    </span>
                </div>
            </div>
            <div class="vocab__cloud">
                <Show
                    when=move || trained() && !terms().is_empty()
                    fallback=|| view! {
                        <p class="dim vocab__empty">
                            "no vocabulary yet — the queen forms her first tokenizer once she has read enough pages, then retrains as the crawl grows."
                        </p>
                    }
                >
                    <For each=terms key=|t| t.clone() let:tk>
                        <span class="tok">{tk}</span>
                    </For>
                </Show>
            </div>
        </section>
    }
}
