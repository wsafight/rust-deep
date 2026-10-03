struct Envelope {
    payload: String,
}

impl Drop for Envelope {
    fn drop(&mut self) {}
}

pub fn take_payload(envelope: Envelope) -> String {
    envelope.payload
}
