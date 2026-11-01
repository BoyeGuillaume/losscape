//! In-memory buffer that `log()` appends to. Drained into one batch every ~100 ms.
//!
//! Columnar and allocation-free on the hot path: metric names are interned once, and a
//! logged point is four `Vec` pushes.

use std::collections::HashMap;

use losscape_core::proto::v0::ScalarColumns;
use serde_json::{Map, Value};

#[derive(Default)]
pub struct Buffer {
    /// Interned metric names: global index -> name, and back.
    names: Vec<String>,
    index: HashMap<String, u32>,
    /// Latest value per global index, and the indexes logged since the last batch:
    /// the run summary tracks the last logged value of every metric.
    last: Vec<f64>,
    dirty: Vec<bool>,
    dirty_list: Vec<u32>,

    key_idx: Vec<u32>,
    step: Vec<u64>,
    time_ms: Vec<i64>,
    value: Vec<f64>,

    summary: Map<String, Value>,
    config: Map<String, Value>,
}

/// What one batch carries, before it gets a seq.
#[derive(Debug, Default, PartialEq)]
pub struct Drained {
    pub scalars: ScalarColumns,
    pub summary: Map<String, Value>,
    pub config: Map<String, Value>,
}

impl Buffer {
    pub fn push(&mut self, key: &str, step: u64, time_ms: i64, value: f64) {
        let idx = match self.index.get(key) {
            Some(&idx) => idx,
            None => self.intern(key),
        };
        self.key_idx.push(idx);
        self.step.push(step);
        self.time_ms.push(time_ms);
        self.value.push(value);

        let i = idx as usize;
        self.last[i] = value;
        if !self.dirty[i] {
            self.dirty[i] = true;
            self.dirty_list.push(idx);
        }
        // A logged value replaces an explicit summary value set earlier in this batch.
        if !self.summary.is_empty() {
            self.summary.remove(key);
        }
    }

    /// Like [`Buffer::push`], without updating the run summary (system metrics).
    pub fn push_untracked(&mut self, key: &str, step: u64, time_ms: i64, value: f64) {
        let idx = match self.index.get(key) {
            Some(&idx) => idx,
            None => self.intern(key),
        };
        self.key_idx.push(idx);
        self.step.push(step);
        self.time_ms.push(time_ms);
        self.value.push(value);
    }

    fn intern(&mut self, key: &str) -> u32 {
        let idx = self.names.len() as u32;
        self.names.push(key.to_owned());
        self.index.insert(key.to_owned(), idx);
        self.last.push(f64::NAN);
        self.dirty.push(false);
        idx
    }

    /// Number of buffered points.
    pub fn rows(&self) -> usize {
        self.value.len()
    }

    /// Drops points pushed after `rows` (used to undo a partially failed `log()` call).
    pub fn truncate(&mut self, rows: usize) {
        self.key_idx.truncate(rows);
        self.step.truncate(rows);
        self.time_ms.truncate(rows);
        self.value.truncate(rows);
    }

    pub fn update_summary(&mut self, update: Map<String, Value>) {
        for key in update.keys() {
            // An explicit value replaces a logged one from earlier in this batch.
            if let Some(&idx) = self.index.get(key)
                && self.dirty[idx as usize]
            {
                self.dirty[idx as usize] = false;
                self.dirty_list.retain(|&i| i != idx);
            }
        }
        self.summary.extend(update);
    }

    pub fn update_config(&mut self, update: Map<String, Value>) {
        self.config.extend(update);
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty() && self.summary.is_empty() && self.config.is_empty()
    }

    /// Takes everything buffered, with metric names remapped to a per-batch dictionary.
    pub fn drain(&mut self) -> Drained {
        let mut local = vec![u32::MAX; self.names.len()];
        let mut keys = Vec::new();
        let key_idx = self
            .key_idx
            .iter()
            .map(|&global| {
                let slot = &mut local[global as usize];
                if *slot == u32::MAX {
                    *slot = keys.len() as u32;
                    keys.push(self.names[global as usize].clone());
                }
                *slot
            })
            .collect();
        self.key_idx.clear();

        let mut summary = std::mem::take(&mut self.summary);
        for idx in self.dirty_list.drain(..) {
            let i = idx as usize;
            self.dirty[i] = false;
            // JSON has no NaN/Inf; serde_json writes them as null.
            summary.insert(self.names[i].clone(), Value::from(self.last[i]));
        }

        Drained {
            scalars: ScalarColumns {
                keys,
                key_idx,
                step: std::mem::take(&mut self.step),
                time_ms: std::mem::take(&mut self.time_ms),
                value: std::mem::take(&mut self.value),
            },
            summary,
            config: std::mem::take(&mut self.config),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn drain_remaps_keys_per_batch() {
        let mut b = Buffer::default();
        b.push("a", 0, 10, 1.0);
        b.push("b", 0, 10, 2.0);
        b.drain();

        b.push("b", 1, 11, 3.0);
        b.push("b", 2, 12, 4.0);
        let d = b.drain();
        assert_eq!(d.scalars.keys, ["b"], "only keys used in this batch");
        assert_eq!(d.scalars.key_idx, [0, 0]);
        assert_eq!(d.scalars.step, [1, 2]);
        assert_eq!(d.scalars.value, [3.0, 4.0]);
        assert_eq!(
            d.summary,
            obj(json!({"b": 4.0})),
            "summary holds the last value"
        );
        assert!(b.is_empty());
    }

    #[test]
    fn last_write_wins_between_log_and_explicit_summary() {
        let mut b = Buffer::default();
        b.push("acc", 0, 0, 0.5);
        b.update_summary(obj(json!({"acc": 0.9})));
        assert_eq!(b.drain().summary, obj(json!({"acc": 0.9})));

        b.update_summary(obj(json!({"acc": 0.9})));
        b.push("acc", 1, 0, 0.6);
        assert_eq!(b.drain().summary, obj(json!({"acc": 0.6})));
    }

    #[test]
    fn untracked_points_stay_out_of_the_summary() {
        let mut b = Buffer::default();
        b.push_untracked("system/cpu", 1, 0, 50.0);
        b.push("loss", 0, 0, 1.0);
        let d = b.drain();
        assert_eq!(d.scalars.value, [50.0, 1.0]);
        assert_eq!(d.summary, obj(json!({"loss": 1.0})));
    }

    #[test]
    fn truncate_undoes_points() {
        let mut b = Buffer::default();
        b.push("a", 0, 0, 1.0);
        b.push("a", 1, 0, 2.0);
        b.truncate(1);
        assert_eq!(b.drain().scalars.value, [1.0]);
    }

    #[test]
    fn config_updates_merge() {
        let mut b = Buffer::default();
        b.update_config(obj(json!({"lr": 0.1, "bs": 32})));
        b.update_config(obj(json!({"lr": 0.01})));
        assert_eq!(b.drain().config, obj(json!({"lr": 0.01, "bs": 32})));
    }
}
