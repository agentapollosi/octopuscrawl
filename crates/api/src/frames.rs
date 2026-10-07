//! In-memory store of the last few JPEG frames per crawler, served at
//! `/v1/crawlers/{id}/frame.jpg?seq=N`. Keeping a short history means the page a
//! viewer is watching stays matched to its boxes even if that hatchling has
//! already moved on to its next page.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

const KEEP: usize = 3;

#[derive(Clone, Default)]
pub struct FrameStore(Arc<Mutex<HashMap<String, VecDeque<(u64, Arc<Vec<u8>>)>>>>);

impl FrameStore {
    pub fn put(&self, id: &str, seq: u64, jpeg: Vec<u8>) {
        let mut m = self.0.lock().unwrap();
        let q = m.entry(id.to_string()).or_default();
        q.push_back((seq, Arc::new(jpeg)));
        while q.len() > KEEP {
            q.pop_front();
        }
    }
    /// The frame with this `seq` if still held, otherwise the latest one.
    pub fn get(&self, id: &str, seq: Option<u64>) -> Option<(u64, Arc<Vec<u8>>)> {
        let m = self.0.lock().unwrap();
        let q = m.get(id)?;
        if let Some(s) = seq {
            if let Some(hit) = q.iter().find(|(fs, _)| *fs == s) {
                return Some(hit.clone());
            }
        }
        q.back().cloned()
    }
}
