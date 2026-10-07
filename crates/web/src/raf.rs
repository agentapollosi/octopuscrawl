//! A requestAnimationFrame driver that stops and frees itself when the owning
//! component unmounts. The running closures live in a thread-local registry so
//! the `on_cleanup` handler only has to capture a numeric id (which is
//! `Send + Sync`, as leptos requires) to drop its loop.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use leptos::prelude::on_cleanup;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

pub fn window() -> web_sys::Window {
    web_sys::window().expect("window")
}

thread_local! {
    static NEXT: Cell<u64> = const { Cell::new(1) };
    static LOOPS: RefCell<HashMap<u64, Rc<Closure<dyn FnMut(f64)>>>> =
        RefCell::new(HashMap::new());
}

fn schedule(id: u64) {
    LOOPS.with(|m| {
        if let Some(cb) = m.borrow().get(&id) {
            let c: &Closure<dyn FnMut(f64)> = cb;
            let _ = window().request_animation_frame(c.as_ref().unchecked_ref());
        }
    });
}

/// Run `tick` once per animation frame until the current reactive owner is
/// cleaned up.
pub fn run_raf(mut tick: impl FnMut(f64) + 'static) {
    let id = NEXT.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    });

    let closure = Closure::wrap(Box::new(move |t: f64| {
        if LOOPS.with(|m| !m.borrow().contains_key(&id)) {
            return;
        }
        tick(t);
        schedule(id);
    }) as Box<dyn FnMut(f64)>);

    LOOPS.with(|m| {
        m.borrow_mut().insert(id, Rc::new(closure));
    });
    schedule(id);

    on_cleanup(move || {
        LOOPS.with(|m| {
            m.borrow_mut().remove(&id);
        });
    });
}
