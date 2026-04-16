//! 跳表索引模块 - SkipList Index Module
//!
//! 本模块实现了基于跳表的时间索引，用于快速时间范围查询。
//!
//! ## 跳表简介
//!
//! 跳表是一种概率数据结构，提供与平衡树相当的查询效率：
//! - 查询: O(log n) 平均
//! - 插入: O(log n) 平均
//! - 范围查询: O(log n + k)，k 为结果数量
//!
//! ## 为什么选择跳表
//!
//! 相比红黑树/B+树，跳表有以下优势：
//! 1. 实现简单，易于理解和维护
//! 2. 范围查询高效，天然有序
//! 3. 并发友好，锁粒度细
//! 4. 内存占用可控
//!
//! ## 在 TSDB 中的应用
//!
//! 跳表用于存储时间戳到数据块偏移量的映射：
//! ```text
//! Timestamp → [block_offset_1, block_offset_2, ...]
//! ```
//!
//! 同一时间戳可能对应多个数据块（不同字段），因此使用 Vec<u64> 存储。



/// 时间戳类型（微秒精度）
type Timestamp = i64;

/// 跳表节点（公共接口版本）
#[derive(Debug, Clone)]
pub struct SkipListNode {
    /// 时间戳键
    pub key: Timestamp,
    /// 数据块偏移量列表
    pub block_offsets: Vec<u64>,
    /// 各层前向指针索引
    pub forward: Vec<usize>,
}

/// 跳表 - Skip List
///
/// 基于概率的多层链表结构，支持高效的范围查询。
///
/// ## 结构示意
///
/// ```text
/// Level 3:  HEAD ──────────────────────────────> NULL
/// Level 2:  HEAD ───────> Node2 ───────────────> NULL
/// Level 1:  HEAD -> Node1 -> Node2 -> Node3 ───> NULL
/// Level 0:  HEAD -> Node1 -> Node2 -> Node3 ───> NULL
/// ```
///
/// ## 参数
///
/// - `nodes`: 所有节点存储（Arena 分配）
/// - `head`: 头节点索引
/// - `max_level`: 最大层数（通常为 16）
/// - `len`: 数据节点数量
/// - `rng_state`: 随机数生成器状态
pub struct SkipList {
    /// 节点存储（索引式访问）
    nodes: Vec<SkipNode>,
    /// 头节点索引
    head: usize,
    /// 最大层数
    max_level: usize,
    /// 数据节点数量
    len: usize,
    /// 随机数生成器状态（线性同余）
    rng_state: u64,
}

/// 跳表节点（内部实现）
#[derive(Debug, Clone)]
struct SkipNode {
    /// 时间戳键
    key: Timestamp,
    /// 数据块偏移量列表
    block_offsets: Vec<u64>,
    /// 各层前向指针（索引）
    forward: Vec<Option<usize>>,
    /// 是否为哨兵节点
    is_sentinel: bool,
}

impl SkipList {
    /// 创建新的跳表
    ///
    /// # 参数
    ///
    /// - `max_level`: 最大层数，通常设为 16
    ///
    /// # 返回值
    ///
    /// 空的跳表实例
    ///
    /// # 层数选择
    ///
    /// 层数越多，查询越快，但内存占用越大。
    /// 经验值：
    /// - 100 万节点：16 层
    /// - 1000 万节点：20 层
    pub fn new(max_level: usize) -> Self {
        // 创建哨兵节点（头节点）
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
            rng_state: 42,  // 固定种子，保证可重复性
        }
    }

    /// 插入键值对
    ///
    /// 如果键已存在，将值追加到该键的偏移量列表中。
    ///
    /// # 参数
    ///
    /// - `key`: 时间戳键
    /// - `block_offset`: 数据块偏移量
    ///
    /// # 算法
    ///
    /// 1. 从最高层开始，找到每层的插入位置
    /// 2. 检查是否已存在相同键
    /// 3. 如果存在，追加偏移量
    /// 4. 如果不存在，随机生成层数并插入新节点
    pub fn insert(&mut self, key: Timestamp, block_offset: u64) {
        // 记录每层的更新位置
        let mut update = vec![self.head; self.max_level];
        let mut current = self.head;

        // 从最高层向下查找插入位置
        for level in (0..self.max_level).rev() {
            while let Some(next) = self.nodes[current].forward[level] {
                if self.nodes[next].key >= key {
                    break;
                }
                current = next;
            }
            update[level] = current;
        }

        // 检查是否已存在相同键
        if let Some(next) = self.nodes[current].forward[0] {
            if self.nodes[next].key == key && !self.nodes[next].is_sentinel {
                // 键已存在，追加偏移量
                self.nodes[next].block_offsets.push(block_offset);
                return;
            }
        }

        // 生成随机层数
        let new_level = self.random_level();
        let new_idx = self.nodes.len();

        // 创建新节点
        let mut new_node = SkipNode {
            key,
            block_offsets: vec![block_offset],
            forward: vec![None; self.max_level],
            is_sentinel: false,
        };

        // 更新各层指针
        for (level, update_idx) in update.iter().enumerate().take(new_level) {
            new_node.forward[level] = self.nodes[*update_idx].forward[level];
            self.nodes[*update_idx].forward[level] = Some(new_idx);
        }

        self.nodes.push(new_node);
        self.len += 1;
    }

    /// 范围查询
    ///
    /// 查询指定时间范围内的所有键值对。
    ///
    /// # 参数
    ///
    /// - `start`: 起始时间戳（包含）
    /// - `end`: 结束时间戳（包含）
    ///
    /// # 返回值
    ///
    /// 键值对列表，按键排序
    ///
    /// # 算法
    ///
    /// 1. 从最高层开始，定位到第一个 >= start 的节点
    /// 2. 从该节点开始，沿最底层遍历直到 > end
    /// 3. 收集所有符合条件的节点
    pub fn range_query(&self, start: Timestamp, end: Timestamp) -> Vec<(Timestamp, Vec<u64>)> {
        let mut results = Vec::new();
        let mut current = self.head;

        // 从最高层开始定位
        for level in (0..self.max_level).rev() {
            while let Some(next) = self.nodes[current].forward[level] {
                if self.nodes[next].key >= start {
                    break;
                }
                current = next;
            }
        }

        // 移动到第一个 >= start 的节点
        current = self.nodes[current].forward[0].unwrap_or(self.head);

        // 沿最底层遍历
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

    /// 获取数据节点数量
    pub fn len(&self) -> usize {
        self.len
    }

    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 序列化为二进制格式
    ///
    /// 将跳表序列化为紧凑的二进制格式，用于持久化存储。
    ///
    /// # 格式
    ///
    /// ```text
    /// [node_count:4B] [node_1] [node_2] ... [node_N]
    ///
    /// 每个节点:
    /// [key:8B] [offset_count:4B] [offset_1:8B] [offset_2:8B] ...
    /// ```
    ///
    /// # 返回值
    ///
    /// 序列化后的二进制数据
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // 过滤出数据节点（排除哨兵）
        let data_nodes: Vec<&SkipNode> = self.nodes.iter().filter(|n| !n.is_sentinel).collect();

        // 写入节点数量
        buf.extend_from_slice(&(data_nodes.len() as u32).to_le_bytes());

        // 写入每个节点
        for node in &data_nodes {
            // 写入键
            buf.extend_from_slice(&node.key.to_le_bytes());
            // 写入偏移量数量
            buf.extend_from_slice(&(node.block_offsets.len() as u32).to_le_bytes());
            // 写入每个偏移量
            for &offset in &node.block_offsets {
                buf.extend_from_slice(&offset.to_le_bytes());
            }
        }

        buf
    }

    /// 从二进制格式反序列化
    ///
    /// 从持久化存储恢复跳表。
    ///
    /// # 参数
    ///
    /// - `data`: 二进制数据
    ///
    /// # 返回值
    ///
    /// 解析成功返回 `Some(SkipList)`，失败返回 `None`
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < 4 { return None; }

        // 读取节点数量
        let node_count = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;

        // 创建空跳表
        let mut sl = Self::new(16);
        let mut offset = 4;

        // 逐个恢复节点
        for _ in 0..node_count {
            if offset + 12 > data.len() { return None; }

            // 读取键
            let key = i64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            offset += 8;

            // 读取偏移量数量
            let off_count = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
            offset += 4;

            // 读取每个偏移量
            for j in 0..off_count {
                if offset + 8 > data.len() { return None; }
                let block_offset = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
                offset += 8;

                if j == 0 {
                    // 第一个偏移量使用 insert 创建节点
                    sl.insert(key, block_offset);
                } else {
                    // 后续偏移量直接追加到已有节点
                    if let Some(node) = sl.nodes.iter_mut().find(|n| n.key == key && !n.is_sentinel) {
                        node.block_offsets.push(block_offset);
                    }
                }
            }
        }

        Some(sl)
    }

    /// 生成随机层数
    ///
    /// 使用线性同余生成器（LCG）生成伪随机数。
    /// 层数服从几何分布，期望值为 1/(1-p)，p=0.25。
    ///
    /// # 返回值
    ///
    /// 随机层数，范围 [1, max_level]
    fn random_level(&mut self) -> usize {
        let mut level = 1;

        // LCG 随机数生成
        self.rng_state = self.rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);

        // 以 25% 概率提升层数
        while level < self.max_level && (self.rng_state >> 33).is_multiple_of(4) {
            level += 1;
        }

        level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试基本插入和查询
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

    /// 测试重复键
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

    /// 测试空范围
    #[test]
    fn test_empty_range() {
        let mut sl = SkipList::new(16);
        sl.insert(100, 1);
        sl.insert(200, 2);

        let results = sl.range_query(300, 400);
        assert!(results.is_empty());
    }
}
