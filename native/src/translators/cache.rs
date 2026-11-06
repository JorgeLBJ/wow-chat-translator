//! LRU cache in front of a translator. Failures are not cached so they can be
//! retried. Port of `cache.js`.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use crate::core::pipeline::{Translation, Translator};

pub const CACHE_MAX: usize = 1000;

struct Lru {
    map: HashMap<String, Translation>,
    /// Keys from least to most recently used.
    order: VecDeque<String>,
}

impl Lru {
    fn touch(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos).unwrap();
            self.order.push_back(k);
        }
    }
}

pub struct CachedTranslator {
    inner: Box<dyn Translator + Send + Sync>,
    max: usize,
    lru: Mutex<Lru>,
}

impl CachedTranslator {
    pub fn new(inner: Box<dyn Translator + Send + Sync>, max: usize) -> Self {
        Self { inner, max, lru: Mutex::new(Lru { map: HashMap::new(), order: VecDeque::new() }) }
    }
}

impl Translator for CachedTranslator {
    fn translate(&self, text: &str, target: &str) -> Result<Translation, String> {
        let key = format!("{target}\0{}", text.trim().to_lowercase());
        {
            let mut lru = self.lru.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(hit) = lru.map.get(&key).cloned() {
                lru.touch(&key);
                return Ok(hit);
            }
        }
        // The lock is not held while translating, so other texts are not blocked.
        let result = self.inner.translate(text, target)?;
        let mut lru = self.lru.lock().unwrap_or_else(|e| e.into_inner());
        if lru.map.insert(key.clone(), result.clone()).is_some() {
            lru.touch(&key);
        } else {
            lru.order.push_back(key);
        }
        while lru.map.len() > self.max {
            match lru.order.pop_front() {
                Some(oldest) => {
                    lru.map.remove(&oldest);
                }
                None => break,
            }
        }
        Ok(result)
    }
}
