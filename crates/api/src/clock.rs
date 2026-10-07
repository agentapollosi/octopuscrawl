//! The site's clock. The server's own system time can drift, so every timestamp
//! the site stores or shows is system time plus an offset measured against
//! Cloudflare's trace endpoint (sub-second `ts=`), re-measured every hour.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};

static OFFSET_MS: AtomicI64 = AtomicI64::new(0);

pub fn now() -> DateTime<Utc> {
    Utc::now() + chrono::Duration::milliseconds(OFFSET_MS.load(Ordering::Relaxed))
}

/// A timestamp taken from the system clock (e.g. a file's mtime), corrected.
pub fn correct(t: DateTime<Utc>) -> DateTime<Utc> {
    t + chrono::Duration::milliseconds(OFFSET_MS.load(Ordering::Relaxed))
}

async fn measure(client: &reqwest::Client) -> Option<i64> {
    let t0 = Utc::now();
    let body = client
        .get("https://www.cloudflare.com/cdn-cgi/trace")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    let t1 = Utc::now();
    let ts: f64 = body.lines().find_map(|l| l.strip_prefix("ts="))?.trim().parse().ok()?;
    let mid_ms = (t0.timestamp_millis() + t1.timestamp_millis()) / 2;
    Some((ts * 1000.0) as i64 - mid_ms)
}

pub fn spawn() {
    tokio::spawn(async move {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default();
        loop {
            if let Some(off) = measure(&client).await {
                OFFSET_MS.store(off, Ordering::Relaxed);
                if off.abs() > 2_000 {
                    println!("[clock] system clock is off by {:.1}s — correcting site timestamps", off as f64 / 1000.0);
                }
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}
