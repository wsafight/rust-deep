use ch05_project_tree::{Arena, RcNode, arena_depth, rc_depth};

#[test]
fn arena_tracks_parent_chain_and_values() {
    let mut arena = Arena::default();
    let root = arena.add(10, None);
    let child = arena.add(20, Some(root));
    let leaf = arena.add(30, Some(child));

    assert_eq!(arena.value(leaf), 30);
    assert_eq!(arena.parent(leaf), Some(child));
    assert_eq!(arena_depth(&arena, leaf), 3);
}

#[test]
fn rc_tree_tracks_parent_chain() {
    let root = RcNode::new(1);
    let child = RcNode::add_child(&root, 2);
    let leaf = RcNode::add_child(&child, 3);

    assert_eq!(rc_depth(&leaf), 3);
    assert_eq!(root.borrow().children.len(), 1);
}
