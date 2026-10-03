struct Inspector<'a>(&'a str);

impl Drop for Inspector<'_> {
    fn drop(&mut self) {
        std::hint::black_box(self.0);
    }
}

pub fn rejected() {
    let inspector;
    let value = String::from("short lived");
    inspector = Inspector(&value);
    std::hint::black_box(&inspector);
}
