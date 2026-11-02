use std::error::Error;
use std::fmt::{self, Display};
use std::io::Read;
use std::num::ParseIntError;

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(ParseIntError),
}

impl Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "reading limit: {error}"),
            Self::Parse(error) => write!(formatter, "parsing limit: {error}"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Parse(error) => Some(error),
        }
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ParseIntError> for ConfigError {
    fn from(error: ParseIntError) -> Self {
        Self::Parse(error)
    }
}

pub fn parse_limit(mut reader: impl Read) -> Result<usize, ConfigError> {
    let mut text = String::new();
    reader.read_to_string(&mut text)?;
    Ok(text.trim().parse()?)
}

pub type BoxError = Box<dyn Error + Send + Sync + 'static>;

pub fn load_limit(reader: impl Read) -> Result<usize, BoxError> {
    Ok(parse_limit(reader)?)
}
