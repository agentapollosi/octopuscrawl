//! Live reading feed: a ticker of what the hatchlings are reading, newest first,
//! each row sliding in as the page lands.

use leptos::prelude::*;

use crate::graph::chapter_color;
use crate::ws::Live;

fn fmt_tok(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

#[component]
pub fn ActivityFeed(live: Live) -> impl IntoView {
    view! {
        <section class="pane feed">
            <div class="pane__bar">
                "live feed"
                <span class="dim">" — what the hatchlings are reading, as it happens"</span>
            </div>
            <div class="feed__list">
                <Show
                    when=move || !live.feed.get().is_empty()
                    fallback=|| view! { <div class="dim feed__empty">"waiting for the first read…"</div> }
                >
                    <For each=move || live.feed.get() key=|f| f.id let:f>
                        <div class="feed__row">
                            <span class="feed__dot" style=format!("background:{}", chapter_color(&f.chapter))></span>
                            <span class="feed__host">{f.host.clone()}</span>
                            <span class="feed__chap dim">{f.chapter.clone()}</span>
                            <span class="feed__tok num">"+"{fmt_tok(f.tokens)}" tok"</span>
                        </div>
                    </For>
                </Show>
            </div>
        </section>
    }
}
