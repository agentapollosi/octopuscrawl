//! In-memory store of the latest JPEG frame per crawler, served at
//! `/v1/crawlers/{id}/frame.jpg`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct FrameStore(Arc<Mutex<HashMap<String, (u64, Vec<u8>)>>>);

impl FrameStore {
    pub fn put(&self, id: &str, seq: u64, jpeg: Vec<u8>) {
        self.0.lock().unwrap().insert(id.to_string(), (seq, jpeg));
    }
    pub fn get(&self, id: &str) -> Option<(u64, Vec<u8>)> {
        self.0.lock().unwrap().get(id).cloned()
    }
}
