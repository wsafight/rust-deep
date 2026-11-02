struct AsyncResource;

async fn cleanup() {}

impl Drop for AsyncResource {
    fn drop(&mut self) {
        cleanup().await;
    }
}
