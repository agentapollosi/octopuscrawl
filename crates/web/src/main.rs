mod app;
mod bg;
mod graph;
mod octopus;
mod queen;
mod raf;
mod scene;
mod stats;
mod tile;
mod vocab;
mod ws;

use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <app::App/> });
}
