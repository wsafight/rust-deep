pub fn last_or_push(values: &mut Vec<String>) -> &String {
    if let Some(last) = values.last() {
        return last;
    }
    values.push(String::new());
    values.last().unwrap()
}
