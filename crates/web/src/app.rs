use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::{Route, Router, Routes, A};
use leptos_router::hooks::{use_location, use_navigate};
use leptos_router::path;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::bg::AmbientBg;
use crate::feed::ActivityFeed;
use crate::graph::KnowledgeGraph;
use crate::queen::QueenScene;
use crate::scene::OctopusScene;
use crate::stats::StatsDashboard;
use crate::swarm::SwarmRadar;
use crate::tile::Tile;
use crate::vocab::VocabPanel;
use crate::ws::{connect, http_base, start_feeds, ws_url, Live};

/// The tabs, keyed 0–6.
const TABS: &[(&str, &str, &str)] = &[
    ("0", "live", "/"),
    ("1", "crawlers", "/crawlers"),
    ("2", "map", "/map"),
    ("3", "queen", "/queen"),
    ("4", "manual", "/man"),
];

#[component]
pub fn App() -> impl IntoView {
    let live = Live::new();
    connect(live, &ws_url());
    start_feeds(live);
    provide_context(live);

    let clock = RwSignal::new(now_hms());
    spawn_local(async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(1000).await;
            clock.set(now_hms());
        }
    });
    provide_context(clock);

    view! {
        <AmbientBg/>
        <Router>
            <Header/>
            <main class="view">
                <Routes fallback=|| view! { <div class="pane pad">"404 — off the trail"</div> }>
                    <Route path=path!("/") view=LivePage/>
                    <Route path=path!("/crawlers") view=CrawlersPage/>
                    <Route path=path!("/map") view=MapPage/>
                    <Route path=path!("/queen") view=QueenPage/>
                    <Route path=path!("/man") view=ManPage/>
                </Routes>
            </main>
            <StatusBar/>
        </Router>
    }
}

#[component]
fn Header() -> impl IntoView {
    let live = use_context::<Live>().expect("live");
    let loc = use_location();
    let here = move || loc.pathname.get();

    // keyboard: digits 0–5 jump between tabs (ignored while typing)
    let nav = use_navigate();
    Effect::new(move |_| {
        let nav = nav.clone();
        let cb = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
            if e.ctrl_key() || e.meta_key() || e.alt_key() {
                return;
            }
            if let Some(t) = e.target() {
                if let Ok(el) = t.dyn_into::<web_sys::HtmlElement>() {
                    let tag = el.tag_name();
                    if tag == "INPUT" || tag == "TEXTAREA" || el.is_content_editable() {
                        return;
                    }
                }
            }
            if let Some((_, _, pathname)) = TABS.iter().find(|(k, _, _)| *k == e.key()) {
                nav(pathname, Default::default());
            }
        });
        if let Some(w) = web_sys::window() {
            let _ = w.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
        }
        cb.forget();
    });

    let pages = move || live.stats.get().map(|s| s.pages_read).unwrap_or(0);
    let tokens = move || live.stats.get().map(|s| s.dataset_tokens).unwrap_or(0);

    view! {
        <header class="hdr">
            <A href="/" attr:class="brand">
                <img class="brand__mark" src="/mark.svg" alt="" />
                "octopuscrawl"
            </A>
            <nav class="tabs">
                {TABS.iter().map(|(k, label, pathname)| {
                    let pathname = *pathname;
                    let k = *k;
                    let label = *label;
                    view! {
                        <A href=pathname attr:class="tab">
                            <span class="tab__k">{k}":"</span>{label}
                        </A>
                    }
                }).collect_view()}
            </nav>
            <span class="hdr__sp"></span>
            <span class="hdr__stat"><b class="num">{move || fmt_num(pages())}</b>" pages"</span>
            <span class="hdr__stat dim"><b class="num">{move || fmt_num(tokens())}</b>" tokens"</span>
            <a class="hdr__weights" href="https://github.com/agentapollosi/octopuscrawl" target="_blank" rel="noreferrer">"source ↗"</a>
            {move || { let _ = here(); () }}
        </header>
    }
}

#[component]
fn StatusBar() -> impl IntoView {
    let live = use_context::<Live>().expect("live");
    let clock = use_context::<RwSignal<String>>().expect("clock");
    let running = move || live.stats.get().map(|s| s.crawlers_running).unwrap_or(0);
    let qver = move || live.stats.get().map(|s| s.queen.version).unwrap_or(0);
    view! {
        <footer class="statusbar">
            <span class="sb__tag">"octopuscrawl"</span>
            <span class="dim sb__path">"~/octopuscrawl"</span>
            <span class="sb__sp"></span>
            <span class=move || if live.connected.get() { "dot on" } else { "dot" }></span>
            <span class="sb__cell">{move || running().to_string()}" crawling"</span>
            <span class="sb__cell dim">"queen v"{move || qver().to_string()}</span>
            <span class="sb__cell num sb__clock">{move || clock.get()}</span>
        </footer>
    }
}

#[component]
fn LivePage() -> impl IntoView {
    let live = use_context::<Live>().expect("live");

    let pages = move || live.stats.get().map(|s| s.pages_read).unwrap_or(0);
    let tokens = move || live.stats.get().map(|s| s.dataset_tokens).unwrap_or(0);
    let running = move || live.stats.get().map(|s| s.crawlers_running).unwrap_or(0);
    let total = move || live.stats.get().map(|s| s.crawlers_total).unwrap_or(0);
    let qver = move || live.stats.get().map(|s| s.queen.version).unwrap_or(0);

    view! {
        <section class="hero">
            <p class="hero__eyebrow">
                <span class=move || if live.connected.get() { "dot on" } else { "dot" }></span>
                <span class="hero__cmd">"~/octopuscrawl ❯ crawl status --live"</span>
            </p>
            <h1 class="hero__h">
                "Pretraining a hacker-native LLM "<em>"from scratch"</em>", on nothing but what her tentacles read."
            </h1>
            <p class="hero__lede">
                "Her hatchlings read the public security web in real headless Chrome — advisories, patches, standards, writeups, tooling, RFCs. Every new page lands in her dataset, and every 250 pages the next queen trains itself. The engine only reads."
            </p>
            <div class="hero__row">
                <div class="hero__chips">
                    <span class="hchip"><b class="num">{move || format!("{}/{}", running(), total())}</b>" hatchlings reading"</span>
                    <span class="hchip"><b class="num">{move || fmt_num(pages())}</b>" pages"</span>
                    <span class="hchip"><b class="num">{move || fmt_num(tokens())}</b>" tokens"</span>
                    <span class="hchip hchip--q">"queen "<b class="num">"v"{move || qver().to_string()}</b></span>
                </div>
                <div class="hero__cta">
                    <A href="/queen" attr:class="btn btn--pri">"ask the queen →"</A>
                    <A href="/map" attr:class="btn">"explore the map"</A>
                </div>
            </div>
        </section>

        <OctopusScene live=live/>

        <LivePace live=live/>

        <StatsDashboard live=live/>

        <SwarmRadar live=live/>

        <DatasetCard live=live/>

        <section class="pane board">
            <div class="pane__bar">"processes"<span class="dim">" — what each hatchling is reading"</span></div>
            <div class="tbl__scroll">
            <table class="tbl">
                <thead>
                    <tr>
                        <th>"CRAWLER"</th><th>"S"</th><th>"HOST"</th>
                        <th class="r">"NEW PAGES"</th><th>"CHAPTER"</th><th>"URL"</th>
                    </tr>
                </thead>
                <tbody>
                    <For each=move || live.crawlers.get() key=|c| c.id.clone() let:c>
                        <tr>
                            <td class="name">{c.name.clone()}</td>
                            <td class=status_class(&c)>{c.status.letter().to_string()}</td>
                            <td class="dim">{host_of(&c.url)}</td>
                            <td class="r num">{c.pages_read.to_string()}</td>
                            <td>
                                <span class="chapcell">
                                    <i style=format!("background:{}", crate::graph::chapter_color(chapter_of_host(&host_of(&c.url))))></i>
                                    {chapter_of_host(&host_of(&c.url))}
                                </span>
                            </td>
                            <td class="url dim">{path_of(&c.url)}</td>
                        </tr>
                    </For>
                </tbody>
            </table>
            </div>
        </section>

        <ActivityFeed live=live/>
    }
}

/// Live reading pace — tokens/min and pages/min, derived from the time-series.
#[component]
fn LivePace(live: Live) -> impl IntoView {
    // pace over the last ~10 minutes, from the server's minute-by-minute record
    // plus the live counters as the newest point
    let pace = move || {
        let (now, tok_now, pg_now) = live
            .stats
            .get()
            .map(|s| (s.updated_at.timestamp() as f64, s.dataset_tokens as f64, s.pages_read as f64))
            .unwrap_or((0.0, 0.0, 0.0));
        live.growth.with(|g| {
            let since = now - 600.0;
            let base = g.iter().find(|p| p.t as f64 >= since).or_else(|| g.last());
            match base {
                Some(b) if now - b.t as f64 > 30.0 => {
                    let mins = (now - b.t as f64) / 60.0;
                    (
                        ((tok_now - b.tokens as f64).max(0.0) / mins) as u64,
                        ((pg_now - b.pages as f64).max(0.0) / mins).round() as u64,
                    )
                }
                _ => (0, 0),
            }
        })
    };
    let tpm = move || pace().0;
    let ppm = move || pace().1;
    view! {
        <section class="pane pace">
            <span class="pace__live"><span class="dot on"></span>"reading now"<span class="dim">" · last 10 min"</span></span>
            <span class="pace__cell">
                <b class="num">{move || fmt_num(tpm())}</b><span class="dim">" tokens/min"</span>
            </span>
            <span class="pace__cell">
                <b class="num">{move || ppm().to_string()}</b><span class="dim">" pages/min"</span>
            </span>
        </section>
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Filt {
    All,
    Crawling,
    Returning,
    Idle,
}
impl Filt {
    fn keep(self, s: octopuscrawl_core::CrawlerStatus) -> bool {
        use octopuscrawl_core::CrawlerStatus::*;
        match self {
            Filt::All => true,
            Filt::Crawling => s == Crawling,
            Filt::Returning => s == Returning,
            Filt::Idle => s == Idle,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Filt::All => "all",
            Filt::Crawling => "crawling",
            Filt::Returning => "returning",
            Filt::Idle => "idle",
        }
    }
}

#[component]
fn CrawlersPage() -> impl IntoView {
    let live = use_context::<Live>().expect("live");
    let filter = RwSignal::new(Filt::All);
    let ids = move || {
        live.crawlers.with(|cs| {
            cs.iter()
                .filter(|c| filter.get().keep(c.status))
                .map(|c| c.id.clone())
                .collect::<Vec<_>>()
        })
    };
    view! {
        <section class="pane">
            <div class="pane__bar tiles__bar">
                <span>"~/octopuscrawl ❯ crawlers watch --all"</span>
                <span class="tiles__filters">
                    {[Filt::All, Filt::Crawling, Filt::Returning, Filt::Idle].into_iter().map(|fo| {
                        let count = move || live.crawlers.with(|cs| cs.iter().filter(|c| fo.keep(c.status)).count());
                        view! {
                            <button class="chip" class:on=move || filter.get() == fo on:click=move |_| filter.set(fo)>
                                {fo.label()}" "<span class="num">{move || count().to_string()}</span>
                            </button>
                        }
                    }).collect_view()}
                </span>
            </div>
            <div class="tiles">
                <For each=ids key=|id| id.clone() let:id>
                    <Tile id=id/>
                </For>
            </div>
        </section>
    }
}
#[component]
fn MapPage() -> impl IntoView {
    let live = use_context::<Live>().expect("live");
    view! {
        <section class="pane mapwrap">
            <div class="pane__bar">
                "~/octopuscrawl ❯ map --graph"
                <span class="dim">" · every page the queen read, and the real links she followed between them"</span>
            </div>
            <KnowledgeGraph live=live/>
        </section>
    }
}

#[component]
fn DatasetCard(live: Live) -> impl IntoView {
    let records = move || live.dataset.get().map(|d| d.records).unwrap_or(0);
    let tokens = move || live.dataset.get().map(|d| d.tokens).unwrap_or(0);
    let hosts = move || live.dataset.get().map(|d| d.hosts).unwrap_or(0);
    let bytes = move || live.dataset.get().map(|d| d.bytes).unwrap_or(0);
    view! {
        <section class="pane dataset">
            <div class="pane__bar">
                "the dataset"<span class="dim">" — every page the queen has read, cleaned and tokenized"</span>
            </div>
            <div class="dataset__row">
                <div class="ds__stat"><b class="num">{move || fmt_num(records())}</b><span class="dim">"pages"</span></div>
                <div class="ds__stat"><b class="num">{move || fmt_num(tokens())}</b><span class="dim">"tokens"</span></div>
                <div class="ds__stat"><b class="num">{move || hosts().to_string()}</b><span class="dim">"hosts"</span></div>
                <div class="ds__stat"><b class="num">{move || human_bytes(bytes())}</b><span class="dim">"on disk"</span></div>
            </div>
            <p class="dim ds__note">
                "read-only crawl · robots.txt obeyed · near-duplicates dropped · offense + defense knowledge, never operational tooling. the corpus is the queen's alone — not distributed."
            </p>
        </section>
    }
}

#[component]
fn QueenPage() -> impl IntoView {
    let live = use_context::<Live>().expect("live");

    let qver = move || live.stats.get().map(|s| s.queen.version).unwrap_or(0);
    let qparams = move || live.stats.get().and_then(|s| s.queen.params).unwrap_or(0);
    let qtokens = move || live.stats.get().map(|s| s.queen.dataset_tokens).unwrap_or(0);
    let qpages = move || live.stats.get().map(|s| s.queen.dataset_pages).unwrap_or(0);
    let trained = move || qparams() > 0;
    let live_tokens = move || live.stats.get().map(|s| s.dataset_tokens).unwrap_or(0);
    let live_pages = move || live.stats.get().map(|s| s.pages_read).unwrap_or(0);
    let nqver = move || live.stats.get().map(|s| s.next_queen.version).unwrap_or(0);
    let nqstatus =
        move || live.stats.get().map(|s| queen_status(s.next_queen.status)).unwrap_or("");
    let nq_training = move || {
        live.stats
            .get()
            .map(|s| s.next_queen.status == octopuscrawl_core::QueenStatus::Training)
            .unwrap_or(false)
    };
    // new pages read since the live version was trained, out of what the next needs
    let nq_need = move || {
        live.stats
            .get()
            .map(|s| s.next_queen.dataset_pages.saturating_sub(s.queen.dataset_pages))
            .unwrap_or(0)
    };
    let nq_done = move || live_pages().saturating_sub(qpages()).min(nq_need());
    let nq_pct = move || {
        if nq_training() {
            100.0
        } else if nq_need() == 0 {
            0.0
        } else {
            nq_done() as f64 / nq_need() as f64 * 100.0
        }
    };
    let chapters = move || live.stats.get().map(|s| s.chapters).unwrap_or_default();

    let chat = RwSignal::new(Vec::<Msg>::new());
    let input = RwSignal::new(String::new());
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let q = input.get();
        if q.trim().is_empty() {
            return;
        }
        let id = js_sys::Date::now() as u64;
        chat.update(|c| {
            c.insert(0, Msg { id, q: q.clone(), a: "…reading what I know".to_string() });
            c.truncate(16);
        });
        input.set(String::new());
        // ask the queen — she answers only from what she has read
        leptos::task::spawn_local(async move {
            let enc = js_sys::encode_uri_component(&q).as_string().unwrap_or_default();
            let url = format!("{}/v1/queen/ask?q={enc}", http_base());
            let answer = match gloo_net::http::Request::get(&url).send().await {
                Ok(resp) => match resp.text().await {
                    Ok(t) => serde_json::from_str::<serde_json::Value>(&t)
                        .ok()
                        .and_then(|v| v.get("answer").and_then(|a| a.as_str()).map(str::to_string))
                        .unwrap_or_else(|| "could not read the queen's reply".to_string()),
                    Err(_) => "could not read the queen's reply".to_string(),
                },
                Err(_) => "the queen is offline — start the backend".to_string(),
            };
            chat.update(|c| {
                if let Some(m) = c.iter_mut().find(|m| m.id == id) {
                    m.a = answer;
                }
            });
        });
    };

    view! {
        <section class="queen">
            <div class="queen__map pane">
                <div class="pane__bar">
                    "~/octopuscrawl ❯ queen status --train"
                    <span class="dim">" · dataset = the crawl, nothing else"</span>
                </div>
                <QueenScene live=live/>
            </div>

            <aside class="queen__side pane">
                <div class="pane__bar">"the queen"</div>
                <div class="qside">
                    <div class="qv">
                        <span class="qv__name">"queen v"{move || qver().to_string()}</span>
                        <span class="qv__live">{move || if trained() { "live" } else { "collecting" }}</span>
                    </div>
                    <p class="qv__meta dim">
                        {move || if trained() && qpages() > 0 {
                            format!(
                                "{} params · trained on {} tokens from {} pages",
                                fmt_params(qparams()), fmt_num(qtokens()), fmt_num(qpages())
                            )
                        } else if trained() {
                            format!("{} params · retraining on the cleaned dataset", fmt_params(qparams()))
                        } else {
                            format!(
                                "not trained yet · corpus so far: {} real tokens from {} pages",
                                fmt_num(live_tokens()), fmt_num(live_pages())
                            )
                        }}
                    </p>
                    {move || trained().then(|| view! {
                        <a class="qv__weights" href="https://github.com/agentapollosi/octopuscrawl" target="_blank" rel="noreferrer">"source ↗"</a>
                    })}

                    <div class="qnext" class:qnext--training=nq_training>
                        <div class="qnext__row">
                            <span>"next: v"{move || nqver().to_string()}</span>
                            <span class="qnext__st">{move || nqstatus()}</span>
                        </div>
                        <span class="meter meter--wide">
                            <span class="meter__f" style=move || format!("width:{:.1}%", nq_pct())></span>
                        </span>
                        <span class="dim num qnext__pct">
                            {move || if nq_training() {
                                format!("pretraining from scratch on {} pages", fmt_num(live_pages()))
                            } else {
                                format!(
                                    "{} / {} new pages · trains itself when full",
                                    fmt_num(nq_done()), fmt_num(nq_need())
                                )
                            }}
                        </span>
                    </div>

                    <div class="qchaps">
                        <div class="qchaps__hd dim">"chapters"</div>
                        <For each=chapters key=|c| c.id.clone() let:ch>
                            <div class="qchap">
                                <span class="qchap__n">
                                    <span class="dim">{ch.index.to_string()}" "</span>{ch.title.clone()}
                                </span>
                                <span class="meter">
                                    <span class="meter__f" style=format!("width:{:.0}%", pct(ch.covered, ch.target))></span>
                                </span>
                                <span class="qchap__v num dim">
                                    {if matches!(ch.status, octopuscrawl_core::ChapterStatus::Locked) {
                                        "locked".to_string()
                                    } else {
                                        format!("{}/{}", ch.covered, ch.target)
                                    }}
                                </span>
                            </div>
                        </For>
                    </div>
                </div>
            </aside>

            <div class="queen__dock pane">
                <form class="qask" on:submit=submit>
                    <span class="dim">"queen ❯"</span>
                    <input
                        class="qask__in"
                        prop:value=move || input.get()
                        on:input=move |e| input.set(event_target_value(&e))
                        placeholder="ask anything security"
                    />
                    <button class="qask__btn" type="submit">"ask"</button>
                </form>
                <div class="qlog">
                    <For each=move || chat.get() key=|m| (m.id, m.a.clone()) let:m>
                        <div class="qlog__q">"❯ "{m.q.clone()}</div>
                        <div class="qlog__a dim">{m.a.clone()}</div>
                    </For>
                </div>
            </div>
        </section>

        <QueenWrites live=live/>

        <VocabPanel live=live/>
    }
}

#[component]
fn QueenWrites(live: Live) -> impl IntoView {
    let trained = move || live.stats.get().and_then(|s| s.queen.params).unwrap_or(0) > 0;
    let prompt = RwSignal::new("The vulnerability".to_string());
    let out = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let write = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let p = prompt.get();
        if p.trim().is_empty() || busy.get() {
            return;
        }
        busy.set(true);
        out.set("…the queen is writing".to_string());
        spawn_local(async move {
            let enc = js_sys::encode_uri_component(&p).as_string().unwrap_or_default();
            let url = format!("{}/v1/queen/generate?prompt={enc}&n=240", http_base());
            let text = match gloo_net::http::Request::get(&url).send().await {
                Ok(r) => match r.text().await {
                    Ok(t) => serde_json::from_str::<serde_json::Value>(&t)
                        .ok()
                        .and_then(|v| v.get("text").and_then(|a| a.as_str()).map(str::to_string))
                        .unwrap_or_else(|| "…".to_string()),
                    Err(_) => "…".to_string(),
                },
                Err(_) => "the queen is offline — start the backend".to_string(),
            };
            out.set(text);
            busy.set(false);
        });
    };
    view! {
        <section class="writes pane">
            <div class="pane__bar">
                "the queen writes"
                <span class="dim">" — free generation from her from-scratch model, trained only on the crawl"</span>
            </div>
            <Show
                when=move || trained()
                fallback=|| view! {
                    <p class="dim writes__empty">
                        "not pretrained yet — once the hatchlings have read enough, the queen trains a small model from scratch (random init) on the corpus and starts writing here. crude at first, better as she reads more."
                    </p>
                }
            >
                <form class="writes__form" on:submit=write>
                    <span class="dim">"seed ❯"</span>
                    <input
                        class="qask__in"
                        prop:value=move || prompt.get()
                        on:input=move |e| prompt.set(event_target_value(&e))
                        placeholder="The vulnerability"
                    />
                    <button class="qask__btn" type="submit">"write ❯"</button>
                </form>
                <pre class="writes__out">{move || out.get()}</pre>
            </Show>
        </section>
    }
}

#[derive(Clone)]
struct Msg {
    id: u64,
    q: String,
    a: String,
}

fn queen_status(s: octopuscrawl_core::QueenStatus) -> &'static str {
    use octopuscrawl_core::QueenStatus::*;
    match s {
        Retired => "retired",
        Live => "live",
        Funding => "funding",
        Collecting => "collecting",
        Training => "training now",
    }
}

fn fmt_params(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{}M", n / 1_000_000)
    } else if n == 0 {
        "—".to_string()
    } else {
        fmt_num(n)
    }
}

fn pct(cov: u32, tgt: u32) -> f64 {
    if tgt == 0 {
        0.0
    } else {
        (cov as f64 / tgt as f64 * 100.0).min(100.0)
    }
}

#[component]
fn ManPage() -> impl IntoView {
    view! {
        <section class="pane pad man">
            <div class="pane__bar">"OCTOPUSCRAWL(1)                    octopuscrawl commands manual                    OCTOPUSCRAWL(1)"</div>
            <dl class="man__dl">
                <dt>"NAME"</dt>
                <dd>"octopuscrawl — crawls the security web and pretrains a hacker-native LLM on it, from scratch. the engine only reads."</dd>

                <dt>"SYNOPSIS"</dt>
                <dd class="man__syn">"octopuscrawl → crawl → tokenize → pretrain → queen v"<i>"n+1"</i></dd>

                <dt>"DESCRIPTION"</dt>
                <dd>
                    <p><b>"crawl"</b>" — hatchlings drive real headless Chromium through public security pages in chapters: advisories, patches, standards, writeups, tooling, rfcs. It finds pages the way a careful crawler should: every link on each page it reads, CISA's catalog of known exploited vulnerabilities (each CVE → its NVD record), and the sitemaps sites publish for crawlers. Two tabs read in parallel, rotating across sites so no host is hit more than once every few seconds, content pages first, the least-covered chapter first. Every screen is a real capture of a real page; between fetches the swarm replays its recent reads. robots.txt obeyed; never past a bot check."</p>
                    <p><b>"ingest"</b>" — every page is deduplicated (one record per URL, SimHash for near-copies), e-mail addresses are redacted, and the rest is tokenized. Her tokenizer is trained on the crawl too. A page that is already in the dataset is never counted twice."</p>
                    <p><b>"map"</b>" — every page lands in a chapter, and every link she followed becomes an edge in the knowledge graph. Each chapter has a target, so coverage can reach 100%."</p>
                    <p><b>"train"</b>" — every 250 new pages, the next queen is pretrained from random init on the dataset and nothing else. Each version has read more than the one before; the page count she trained on is recorded when her run ends. The weights stay private — the source is public so anyone can verify how she is made."</p>
                    <p><b>"read-only"</b>" — both offense and defense knowledge are read, so the queen understands how a weakness works and how to defend against it. But the crawler only reads published pages; it never scans or attacks a target, and the model is not tuned into operational attack tooling. This boundary is not negotiable."</p>
                </dd>

                <dt>"CHAPTERS"</dt>
                <dd>"advisories, patches, standards, writeups, tooling, rfcs. forums (locked)."</dd>

                <dt>"EXAMPLES"</dt>
                <dd class="man__ex">
                    <p>"# what the hatchlings have read so far"</p>
                    <p>"$ curl -s https://octopuscrawl.net/v1/stats"</p>
                    <p>"# every hatchling and the page it is on"</p>
                    <p>"$ curl -s https://octopuscrawl.net/v1/crawlers"</p>
                </dd>

                <dt>"FILES"</dt>
                <dd class="man__files">
                    <p><b>"/v1/live"</b>"            websocket: crawler state, frames, pages read"</p>
                    <p><b>"/v1/stats"</b>"           counters, queen versions, chapters"</p>
                    <p><b>"/v1/crawlers"</b>"        every hatchling"</p>
                    <p><b>"/v1/crawlers/:id/frame.jpg"</b>"  the page it is reading"</p>
                    <p><b>"/v1/graph"</b>"           the knowledge graph — pages (nodes) and links (edges)"</p>
                    <p><b>"/v1/vocab"</b>"           the queen's trained tokenizer — size and learned subwords"</p>
                    <p><b>"/v1/queen/ask"</b>"       ask the queen — answers only from what she has read"</p>
                </dd>

                <dt>"SEE ALSO"</dt>
                <dd>"crawlers(1), map(1), queen(1)"</dd>
            </dl>
            <p class="man__foot dim">"octopuscrawl 0.1 · octopuscrawl.net · the engine only reads."</p>
        </section>
    }
}

fn ledger_kind(k: octopuscrawl_core::LedgerKind) -> &'static str {
    use octopuscrawl_core::LedgerKind::*;
    match k {
        Crawl => "crawl",
        Fees => "fees",
        Reward => "reward",
        Burn => "burn",
    }
}

// -- helpers ----------------------------------------------------------------

fn status_class(c: &octopuscrawl_core::Crawler) -> &'static str {
    match c.status {
        octopuscrawl_core::CrawlerStatus::Crawling => "s s-r",
        octopuscrawl_core::CrawlerStatus::Returning => "s s-w",
        octopuscrawl_core::CrawlerStatus::Idle => "s s-s",
        octopuscrawl_core::CrawlerStatus::Error => "s s-e",
    }
}

fn host_of(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn path_of(url: &str) -> String {
    let after = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    match after.find('/') {
        Some(i) => after[i..].to_string(),
        None => "/".to_string(),
    }
}

fn fmt_num(n: u64) -> String {
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

fn human_bytes(n: u64) -> String {
    if n >= 1_048_576 {
        format!("{:.1} MB", n as f64 / 1_048_576.0)
    } else if n >= 1024 {
        format!("{:.0} KB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

fn now_hms() -> String {
    let d = js_sys::Date::new_0();
    format!(
        "{:02}:{:02}:{:02} UTC",
        d.get_utc_hours() as u32,
        d.get_utc_minutes() as u32,
        d.get_utc_seconds() as u32
    )
}

/// Host → chapter, mirroring the crawl's own mapping.
fn chapter_of_host(host: &str) -> &'static str {
    if host.is_empty() {
        "—"
    } else if host.contains("nvd.nist") || host.contains("cisa") || host.contains("cve") {
        "advisories"
    } else if host.contains("ubuntu")
        || host.contains("redhat")
        || host.contains("debian")
        || host.contains("suse")
        || host.contains("msrc")
    {
        "patches"
    } else if host.contains("nmap")
        || host.contains("zaproxy")
        || host.contains("wireshark")
        || host.contains("metasploit")
        || host.contains("kali")
    {
        "tooling"
    } else if host.contains("portswigger") || host.contains("sans") {
        "writeups"
    } else if host.contains("rfc-editor") || host.contains("ietf") {
        "rfcs"
    } else if host.contains("owasp") || host.contains("mitre") {
        "standards"
    } else {
        "writeups"
    }
}
