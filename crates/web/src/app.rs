use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::{Route, Router, Routes, A};
use leptos_router::hooks::{use_location, use_navigate};
use leptos_router::path;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::bg::AmbientBg;
use crate::graph::KnowledgeGraph;
use crate::queen::QueenScene;
use crate::scene::OctopusScene;
use crate::stats::StatsDashboard;
use crate::tile::Tile;
use crate::vocab::VocabPanel;
use crate::ws::{connect, http_base, start_feeds, ws_url, Live};

/// The tabs, keyed 0–6.
const TABS: &[(&str, &str, &str)] = &[
    ("0", "live", "/"),
    ("1", "crawlers", "/crawlers"),
    ("2", "map", "/map"),
    ("3", "queen", "/queen"),
    ("4", "treasury", "/treasury"),
    ("5", "order", "/order"),
    ("6", "manual", "/man"),
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
                    <Route path=path!("/treasury") view=TreasuryPage/>
                    <Route path=path!("/order") view=OrderPage/>
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
            <a class="hdr__weights" href="https://huggingface.co/Octopuscrawl" target="_blank" rel="noreferrer">"weights ↗"</a>
            {move || { let _ = here(); () }}
        </header>
    }
}

#[component]
fn StatusBar() -> impl IntoView {
    let live = use_context::<Live>().expect("live");
    let clock = use_context::<RwSignal<String>>().expect("clock");
    let running = move || live.stats.get().map(|s| s.crawlers_running).unwrap_or(0);
    view! {
        <footer class="statusbar">
            <span class="sb__tag">"octopuscrawl"</span>
            <span class="dim">"~/octopuscrawl"</span>
            <span class="sb__sp"></span>
            <span class=move || if live.connected.get() { "dot on" } else { "dot" }></span>
            <span class="sb__cell">{move || running().to_string()}" crawling"</span>
            <span class="sb__cell dim">"queen v1"</span>
            <span class="sb__cell num">{move || clock.get()}</span>
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

    view! {
        <section class="hero">
            <p class="prompt dim">"~/octopuscrawl ❯ crawl status --live"</p>
            <h1 class="wm">"octopuscrawl"</h1>
            <p class="tag">
                "❯ pretraining the first hacker-native LLM. from scratch. on nothing but what her tentacles read."
            </p>
            <p class="statline">
                <span class="num">{move || running().to_string()}</span>" / "
                <span class="num">{move || total().to_string()}</span>" hatchlings awake · "
                <span class="num">{move || fmt_num(pages())}</span>" pages · "
                <span class="num">{move || fmt_num(tokens())}</span>" tokens · queen v1"
            </p>
        </section>

        <OctopusScene live=live/>

        <StatsDashboard live=live/>

        <DatasetCard live=live/>

        <section class="pane board">
            <div class="pane__bar">"processes"<span class="dim">" — what each hatchling is reading"</span></div>
            <table class="tbl">
                <thead>
                    <tr>
                        <th>"CRAWLER"</th><th>"S"</th><th>"HOST"</th>
                        <th class="r">"PAGES"</th><th class="r">"THREAT"</th><th>"URL"</th>
                    </tr>
                </thead>
                <tbody>
                    <For each=move || live.crawlers.get() key=|c| c.id.clone() let:c>
                        <tr>
                            <td class="name">{c.name.clone()}</td>
                            <td class=status_class(&c)>{c.status.letter().to_string()}</td>
                            <td class="dim">{host_of(&c.url)}</td>
                            <td class="r num">{c.pages_read.to_string()}</td>
                            <td class="r">
                                <span class="meter">
                                    <span class="meter__f" style=format!("width:{:.0}%", c.relevance * 100.0)></span>
                                </span>
                                <span class="num thr">{format!("{:.2}", c.relevance)}</span>
                            </td>
                            <td class="url dim">{path_of(&c.url)}</td>
                        </tr>
                    </For>
                </tbody>
            </table>
        </section>

        <section class="pane clog">
            <div class="pane__bar">"❯ tail -f queen.log"</div>
            <For each=move || live.log.get() key=|line| line.clone() let:line>
                <div class="clog__row">{line}</div>
            </For>
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
    let href = format!("{}/v1/dataset.jsonl", http_base());
    view! {
        <section class="pane dataset">
            <div class="pane__bar">
                "open dataset"<span class="dim">" — every page the queen kept, as JSONL you can download"</span>
            </div>
            <div class="dataset__row">
                <div class="ds__stat"><b class="num">{move || fmt_num(records())}</b><span class="dim">"records"</span></div>
                <div class="ds__stat"><b class="num">{move || fmt_num(tokens())}</b><span class="dim">"tokens"</span></div>
                <div class="ds__stat"><b class="num">{move || hosts().to_string()}</b><span class="dim">"hosts"</span></div>
                <div class="ds__stat"><b class="num">{move || human_bytes(bytes())}</b><span class="dim">"on disk"</span></div>
                <a class="ds__dl" href=href download="octopuscrawl-corpus.jsonl" target="_blank" rel="noreferrer">"download ↓"</a>
            </div>
            <p class="dim ds__note">
                "read-only crawl · robots.txt obeyed · near-duplicates dropped · offense + defense knowledge, never operational tooling."
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
    let fund = move || {
        live.stats
            .get()
            .map(|s| (s.next_queen.funded_sol / s.next_queen.cost_sol.max(1e-9) * 100.0).min(100.0))
            .unwrap_or(0.0)
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
                        {move || if trained() {
                            format!(
                                "{} params · trained on {} tokens from {} pages",
                                fmt_params(qparams()), fmt_num(qtokens()), fmt_num(qpages())
                            )
                        } else {
                            format!(
                                "not trained yet · corpus so far: {} real tokens from {} pages",
                                fmt_num(live_tokens()), fmt_num(live_pages())
                            )
                        }}
                    </p>
                    {move || trained().then(|| view! {
                        <a class="qv__weights" href="https://huggingface.co/Octopuscrawl" target="_blank" rel="noreferrer">"weights ↗"</a>
                    })}

                    <div class="qnext">
                        <div class="qnext__row">
                            <span>"next: v"{move || nqver().to_string()}</span>
                            <span class="dim">{move || nqstatus()}</span>
                        </div>
                        <span class="meter meter--wide">
                            <span class="meter__f" style=move || format!("width:{:.0}%", fund())></span>
                        </span>
                        <span class="dim num qnext__pct">{move || format!("{:.0}%", fund())}" funded"</span>
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
        Training => "training",
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
fn TreasuryPage() -> impl IntoView {
    let live = use_context::<Live>().expect("live");
    let loaded = move || live.treasury.get().is_some();
    let addr = move || live.treasury.get().map(|t| t.address).unwrap_or_default();
    let balance = move || live.treasury.get().map(|t| t.balance_sol).unwrap_or(0.0);
    let txs = move || live.treasury.get().map(|t| t.txs).unwrap_or_default();
    let inflow = move || txs().iter().filter(|t| t.delta_sol > 0.0).map(|t| t.delta_sol).sum::<f64>();
    let count = move || txs().len();

    view! {
        <section class="pane pad">
            <div class="pane__bar">
                "~/octopuscrawl ❯ treasury --wallet"
                <span class="dim">" · on-chain, read-only · the engine never signs or sends"</span>
            </div>
            <div class="tre__stats">
                <div class="tre__stat">
                    <span class="dim">"treasury balance"</span>
                    <b class="num">{move || if loaded() { format!("{:.4} SOL", balance()) } else { "…".into() }}</b>
                </div>
                <div class="tre__stat">
                    <span class="dim">{move || format!("inflow (last {})", count())}</span>
                    <b class="num">{move || format!("+{:.4} SOL", inflow())}</b>
                </div>
                <div class="tre__stat">
                    <span class="dim">"wallet"</span>
                    <a class="num tre__addr" href=move || format!("https://solscan.io/account/{}", addr())
                        target="_blank" rel="noreferrer">{move || short_mid(&addr())}" ↗"</a>
                </div>
            </div>
            <div class="tre__flow">
                <div class="tre__flowhd dim">"planned allocation · wires in once the token is live"</div>
                <div class="flow__row flow__split">
                    <span class="flow__k">"→ 60% compute (GPU + crawl)"</span>
                    <span class="flow__v dim">"planned"</span>
                </div>
                <div class="flow__row flow__split">
                    <span class="flow__k">"→ 40% crawler owners"</span>
                    <span class="flow__v dim">"planned · split every 12h"</span>
                </div>
            </div>
            <p class="dim tre__note">
                "The balance and the ledger below are read live from the treasury wallet on Solana — nothing here is fabricated. The 60/40 split is the plan once the memecoin's creator fees are wired to this wallet, pump.fun-style: 60% funds the GPU + crawl, 40% is split among crawler owners. octopuscrawl only reads the chain; it never signs or moves funds."
            </p>
            <div class="pane__bar tre__bar">"on-chain activity"<span class="dim">" — live from Solana"</span></div>
            <div class="tre__ledger">
                <Show
                    when=move || !txs().is_empty()
                    fallback=move || view! {
                        <div class="dim tre__empty">
                            {move || if loaded() { "no transactions on this wallet yet" } else { "reading the chain…" }}
                        </div>
                    }
                >
                    <For each=txs key=|t| t.sig.clone() let:t>
                        {
                            let pos = t.delta_sol >= 0.0;
                            let sig = t.sig.clone();
                            let href = format!("https://solscan.io/tx/{sig}");
                            view! {
                                <div class="led__row">
                                    <span class=if pos { "led__kind pos" } else { "led__kind neg" }>
                                        {if pos { "in" } else { "out" }}
                                    </span>
                                    <a class="led__memo dim" href=href target="_blank" rel="noreferrer">
                                        {short_mid(&sig)}" ↗"
                                    </a>
                                    <span class=if pos { "num led__sol pos" } else { "num led__sol neg" }>
                                        {format!("{:+.4}", t.delta_sol)}
                                    </span>
                                </div>
                            }
                        }
                    </For>
                </Show>
            </div>
        </section>
    }
}

/// Shorten a long base58 string (address / signature) as `abcd…wxyz`.
fn short_mid(s: &str) -> String {
    let n = s.chars().count();
    if n <= 12 {
        return s.to_string();
    }
    let head: String = s.chars().take(4).collect();
    let tail: String = s.chars().skip(n - 4).collect();
    format!("{head}…{tail}")
}
#[component]
fn OrderPage() -> impl IntoView {
    let url = RwSignal::new(String::new());
    let msg = RwSignal::new(String::new());
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let u = url.get();
        if u.trim().is_empty() {
            return;
        }
        msg.set(format!(
            "queued checks for {u} — robots.txt, a real page, no bot check, no adult/gambling/scam. nothing is burned; on-demand ordering opens with the token layer."
        ));
    };
    view! {
        <section class="pane pad">
            <div class="pane__bar">
                "~/octopuscrawl ❯ order --url <site>"
                <span class="dim">" · point the hatchlings at a security site you name"</span>
            </div>
            <p class="dim order__note">
                "The hatchlings read up to 25 new pages of one host within 3 hours — robots.txt obeyed, never past a bot check, security content only. Automatic checks run first; you get a public report of what the queen kept. The engine reads; it never acts on what it reads."
            </p>
            <form class="order__form" on:submit=submit>
                <span class="dim">"--url"</span>
                <input
                    class="qask__in"
                    prop:value=move || url.get()
                    on:input=move |e| url.set(event_target_value(&e))
                    placeholder="https://security.example.com"
                />
                <button class="qask__btn" type="submit">"order a crawl ❯"</button>
            </form>
            <p class="order__msg">{move || msg.get()}</p>
        </section>
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
                    <p><b>"crawl"</b>" — hatchlings drive real headless Chromium through public security pages in chapters: advisories, patches, standards, writeups, tooling. Every screen on the live page is a real page loading right now. robots.txt obeyed; never past a bot check."</p>
                    <p><b>"ingest"</b>" — every page is canonicalized, deduplicated, relevance-gated and tokenized; PII is stripped before the dataset. Her tokenizer is trained on the crawl too."</p>
                    <p><b>"map"</b>" — the dataset is indexed by project and chapter. Chapters are finite target lists, so coverage can reach 100%."</p>
                    <p><b>"train"</b>" — when a run is funded, a new queen is pretrained from random init on the dataset and nothing else, on a GPU box. Each version is bigger and has read more. Weights ship public."</p>
                    <p><b>"read-only"</b>" — both offense and defense knowledge are read, so the queen understands how a weakness works and how to defend against it. But the crawler only reads published pages; it never scans or attacks a target, and the model is not tuned into operational attack tooling. This boundary is what keeps the feed legal and sellable."</p>
                </dd>

                <dt>"CHAPTERS"</dt>
                <dd>"advisories, patches, standards, writeups, tooling, forums (locked), rfcs (locked)."</dd>

                <dt>"EXAMPLES"</dt>
                <dd class="man__ex">
                    <p>"# what the hatchlings have read so far"</p>
                    <p>"$ curl -s $INK_API/v1/stats"</p>
                    <p>"# every hatchling and the page it is on"</p>
                    <p>"$ curl -s $INK_API/v1/crawlers"</p>
                </dd>

                <dt>"FILES"</dt>
                <dd class="man__files">
                    <p><b>"/v1/live"</b>"            websocket: crawler state, frames, pages, ledger"</p>
                    <p><b>"/v1/stats"</b>"           counters, queen versions, chapters"</p>
                    <p><b>"/v1/crawlers"</b>"        every hatchling"</p>
                    <p><b>"/v1/crawlers/:id/frame.jpg"</b>"  the page it is reading"</p>
                    <p><b>"/v1/graph"</b>"           the knowledge graph — pages (nodes) and links (edges)"</p>
                    <p><b>"/v1/vocab"</b>"           the queen's trained tokenizer — size and learned subwords"</p>
                    <p><b>"/v1/dataset.jsonl"</b>"   the open dataset — every page the queen kept"</p>
                </dd>

                <dt>"SEE ALSO"</dt>
                <dd>"crawlers(1), map(1), queen(1), treasury(1), order(1)"</dd>
            </dl>
            <p class="man__foot dim">"octopuscrawl 0.1 · octopuscrawl.local · the engine only reads."</p>
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
