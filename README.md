# octopuscrawl

Pretraining the first hacker-native LLM — from scratch, on nothing but what her
tentacles read. A queen octopus that gets security-savvy by **reading** published
security knowledge (CVE/NVD advisories, patch notes, OWASP/MITRE, research
writeups). The engine only reads public pages; it never attacks anything.

Built entirely in Rust.

## Workspace

| Crate | What it is |
|---|---|
| `crates/core` | Shared wire contract: data models + the `LiveMsg` WebSocket protocol. |
| `crates/crawler` | Headless-Chromium crawl engine: reads public pages, returns a screenshot + link boxes. |
| `crates/api` | Axum REST + WebSocket server; broadcasts live crawler / page / ledger events. |
| `crates/web` | Leptos (WASM) frontend: the live octopus-over-document view, crawlers board, queen, treasury, order, manual. |
| `config/domain.toml` | Every domain-specific label. Swap it to re-skin the whole app for another topic. |

## Run it

Backend — demo feed (no browser needed). REST on `:8787`, WebSocket at `/v1/live`:

```bash
cargo run -p octopuscrawl-api
```

Backend — real crawl engine (needs Chrome installed; set `OCTOPUSCRAWL_CHROME` to override the path).
On macOS, prefer [chrome-headless-shell](https://googlechromelabs.github.io/chrome-for-testing/)
unpacked at `~/.local/share/chrome-headless-shell/current/` — it's picked up automatically and,
unlike full Chrome, doesn't steal keyboard focus every time the crawler opens a page:

```bash
OCTOPUSCRAWL_CRAWL=1 cargo run -p octopuscrawl-api --features real
```

Frontend (serves on `:8080`, connects to the backend's WebSocket):

```bash
trunk serve --port 8080 crates/web/index.html
```

Open http://127.0.0.1:8080 with the backend running.

## How it works

Headless Chromium drives through public security pages in chapters: advisories,
patches, standards, writeups, tooling. Each page becomes a screenshot plus the
on-screen boxes of its links and headings; a canvas octopus is drawn over the
real page and inks the text its tentacles reach. Links are followed within an
allowlist of security hosts so coverage grows. Pages are cleaned, deduplicated
and tokenized into the queen's dataset; each funded run retrains her from scratch
and the weights ship public.

## Guardrails

Respect `robots.txt`; no paywall or bot-check bypass; strip PII before the
dataset; reject adult / gambling / scam content. The engine only reads and
documents — it never executes anything it reads, and never scans or attacks a
target.

## License

**All rights reserved — source-available, not open source.**

This repository is public so anyone can read the code and verify the project is
genuine. You may **view** it; you may **not** copy, reuse, run, modify, or
redistribute it without written permission. See [LICENSE](LICENSE).
