use std::collections::BTreeMap;

type Timestamp = i64;

#[derive(Debug, Clone)]
pub struct SkipListNode {
    pub key: Timestamp,
    pub block_offsets: Vec<u64>,
    pub forward: Vec<usize>,
}

pub struct SkipList {
    nodes: Vec<SkipNode>,
    head: usize,
    max_level: usize,
    len: usize,
    rng_state: u64,
}

#[derive(Debug, Clone)]
struct SkipNode {
    key: Timestamp,
    block_offsets: Vec<u64>,
    forward: Vec<Option<usize>>,
    is_sentinel: bool,
}

impl SkipList {
    pub fn new(max_level: usize) -> Self {
        let sentinel = SkipNode {
            key: i64::MIN,
            block_offsets: Vec::new(),
            forward: vec![None; max_level],
            is_sentinel: true,
        };
        Self {
            nodes: vec![sentinel],
            head: 0,
            max_level,
            len: 0,
            rng_state: 42,
        }
    }

    pub fn insert(&mut self, key: Timestamp, block_offset: u64) {
        let mut update = vec![self.head; self.max_level];
        let mut current = self.head;

        for level in (0..self.max_level).rev() {
            while let Some(next) = self.nodes[current].forward[level] {
                if self.nodes[next].key >= key {
                    break;
                }
                current = next;
            }
            update[level] = current;
        }

        if let Some(next) = self.nodes[current].forward[0] {
            if self.nodes[next].key == key && !self.nodes[next].is_sentinel {
                self.nodes[next].block_offsets.push(block_offset);
                return;
            }
        }

        let new_level = self.random_level();
        let new_idx = self.nodes.len();
        let mut new_node = SkipNode {
            key,
            block_offsets: vec![block_offset],
            forward: vec![None; self.max_level],
            is_sentinel: false,
        };

        for level in 0..new_level {
            new_node.forward[level] = self.nodes[update[level]].forward[level];
            self.nodes[update[level]].forward[level] = Some(new_idx);
        }

        self.nodes.push(new_node);
        self.len += 1;
    }

    pub fn range_query(&self, start: Timestamp, end: Timestamp) -> Vec<(Timestamp, Vec<u64>)> {
        let mut results = Vec::new();
        let mut current = self.head;

        for level in (0..self.max_level).rev() {
            while let Some(next) = self.nodes[current].forward[level] {
                if self.nodes[next].key >= start {
                    break;
                }
                current = next;
            }
        }

        current = self.nodes[current].forward[0].unwrap_or(self.head);

        while current < self.nodes.len() && !self.nodes[current].is_sentinel {
            let node = &self.nodes[current];
            if node.key > end {
                break;
            }
            if node.key >= start {
                results.push((node.key, node.block_offsets.clone()));
            }
            current = node.forward[0].unwrap_or(self.head);
        }

        results
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn random_level(&mut self) -> usize {
        let mut level = 1;
        self.rng_state = self.rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        while level < self.max_level && (self.rng_state >> 33) % 4 == 0 {
            level += 1;
        }
        level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_query() {
        let mut sl = SkipList::new(16);
        sl.insert(100, 1);
        sl.insert(200, 2);
        sl.insert(300, 3);
        sl.insert(400, 4);

        let results = sl.range_query(150, 350);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 200);
        assert_eq!(results[1].0, 300);
    }

    #[test]
    fn test_duplicate_key() {
        let mut sl = SkipList::new(16);
        sl.insert(100, 1);
        sl.insert(100, 2);
        sl.insert(100, 3);

        let results = sl.range_query(100, 100);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.len(), 3);
    }

    #[test]
    fn test_empty_range() {
        let mut sl = SkipList::new(16);
        sl.insert(100, 1);
        sl.insert(200, 2);

        let results = sl.range_query(300, 400);
        assert!(results.is_empty());
    }
}
