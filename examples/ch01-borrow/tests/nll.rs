use ch01_borrow::{branch_one_use, last_or_push};

#[test]
fn a_conditional_use_before_mutation_is_accepted() {
    assert_eq!(branch_one_use(false), 4);
    assert_eq!(branch_one_use(true), 4);
}

#[test]
fn last_or_push_inserts_only_when_empty() {
    let mut values = Vec::new();
    assert_eq!(last_or_push(&mut values), "");
    assert_eq!(values.len(), 1);
    values.push(String::from("kept"));
    assert_eq!(last_or_push(&mut values), "kept");
    assert_eq!(values.len(), 2);
}
