//! 第 5 章实战：写一个安全的树 / 图容器
//!
//! 证据生成：tools/evidence.sh ch05-project-tree
//!
//! 本章把第 1–4 章的直觉落到可运行代码上。
//! 核心问题：**树要能向上访问父节点，怎么办？**
//! 三条路，各有代价：
//!   1. `Rc<RefCell<>>` —— 共享所有权 + 运行时借用检查
//!   2. **arena + 索引** —— 零引用计数、零运行时检查（推荐）
//!   3. `unsafe` + 裸指针 —— 最快，但要自己保证不变式
//!
//! 本章用代码生成数据说明：**路线 2 通常是最优解**。

use std::cell::RefCell;
use std::rc::Rc;

// ---------- 路线 1：Rc<RefCell<Node>> ----------

pub struct RcNode {
    pub value: u64,
    pub parent: Option<Rc<RefCell<RcNode>>>,
    pub children: Vec<Rc<RefCell<RcNode>>>,
}

impl RcNode {
    pub fn new(value: u64) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            value,
            parent: None,
            children: Vec::new(),
        }))
    }

    pub fn add_child(parent: &Rc<RefCell<Self>>, value: u64) -> Rc<RefCell<Self>> {
        let child = Self::new(value);
        child.borrow_mut().parent = Some(Rc::clone(parent));
        parent.borrow_mut().children.push(Rc::clone(&child));
        child
    }
}

/// 向上遍历：每次都要 `borrow()`（运行时借用检查）
#[unsafe(no_mangle)]
pub fn rc_depth(node: &Rc<RefCell<RcNode>>) -> usize {
    let mut depth = 0;
    let mut cur = Some(Rc::clone(node));
    while let Some(n) = cur {
        cur = n.borrow().parent.clone();
        depth += 1;
    }
    depth
}

// ---------- 路线 2：arena + 索引（推荐） ----------

/// 索引是**普通整数**：`Copy`、无引用计数、无借用检查
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct NodeId(pub usize);

pub struct Arena {
    nodes: Vec<ArenaNode>,
}

struct ArenaNode {
    value: u64,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

impl Arena {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    pub fn add(&mut self, value: u64, parent: Option<NodeId>) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(ArenaNode {
            value,
            parent,
            children: Vec::new(),
        });
        if let Some(p) = parent {
            self.nodes[p.0].children.push(id);
        }
        id
    }

    pub fn value(&self, id: NodeId) -> u64 {
        self.nodes[id.0].value
    }
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0].parent
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

/// 向上遍历：**纯整数运算**，没有引用计数、没有运行时借用检查
#[unsafe(no_mangle)]
pub fn arena_depth(arena: &Arena, id: NodeId) -> usize {
    let mut depth = 0;
    let mut cur = Some(id);
    while let Some(n) = cur {
        cur = arena.parent(n);
        depth += 1;
    }
    depth
}

/// 对照：`Rc` 版本向上遍历要 clone 每个 `Rc`（引用计数 +1/-1）
#[unsafe(no_mangle)]
pub fn arena_build(n: usize) -> usize {
    let mut a = Arena::new();
    let root = a.add(0, None);
    let mut cur = root;
    for i in 0..n {
        cur = a.add(i as u64, Some(cur));
    }
    arena_depth(&a, cur)
}
