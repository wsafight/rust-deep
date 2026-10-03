#[repr(C, packed)]
pub struct Header {
    pub tag: u8,
    pub count: u32,
}

pub fn count_ref(header: &Header) -> &u32 {
    &header.count
}
