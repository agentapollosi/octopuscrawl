//! Smoke test: read a couple of real public security pages and report what the
//! octopus would see — screenshot size, title, and the first few link boxes.
//!
//! Run: cargo run -p octopuscrawl-crawler --bin probe

use octopuscrawl_crawler::{robots_allows, Engine};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let urls = [
        "https://owasp.org/www-project-top-ten/",
        "https://www.rfc-editor.org/rfc/rfc8446",
    ];

    println!("launching headless chrome…");
    let engine = Engine::launch().await?;
    let out_dir = std::env::temp_dir();

    for url in urls {
        println!("\n=== {url}");
        if !robots_allows(url).await {
            println!("  robots.txt disallows — skipping");
            continue;
        }
        match engine.read_page(url).await {
            Ok(p) => {
                let fname = out_dir.join(format!(
                    "octopuscrawl-probe-{}.jpg",
                    url.replace(|c: char| !c.is_alphanumeric(), "_")
                ));
                std::fs::write(&fname, &p.jpeg)?;
                println!("  title: {}", p.title);
                println!("  jpeg: {} bytes -> {}", p.jpeg.len(), fname.display());
                println!("  link/heading boxes: {}", p.links.len());
                for b in p.links.iter().take(6) {
                    println!(
                        "    [{:4.0},{:4.0} {:4.0}x{:3.0}] {}",
                        b.x,
                        b.y,
                        b.w,
                        b.h,
                        b.text.chars().take(50).collect::<String>()
                    );
                }
            }
            Err(e) => println!("  error: {e:#}"),
        }
    }

    Ok(())
}
