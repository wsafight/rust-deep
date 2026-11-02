use std::error::Error;

fn local_error() -> Result<(), Box<dyn Error>> {
    Ok(())
}

pub fn thread_safe_error() -> Result<(), Box<dyn Error + Send + Sync>> {
    local_error()?;
    Ok(())
}
