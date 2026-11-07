//! Rust port of Christopher Le's MIT-licensed traktor-connect TypeScript library.
mod ogg;
mod server;
mod vorbis;
pub use ogg::{OggDecoder, OggPage, MAX_OGG_PAGE_BYTES};
use serde::{Deserialize, Serialize};
pub use server::{Event, LogLevel, Logger, Options, TraktorConnect};
pub use vorbis::{parse_comments, VorbisDecoder, MAX_COMMENT_BYTES};
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackData {
    pub artist: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoder: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self(value.to_string())
    }
}
