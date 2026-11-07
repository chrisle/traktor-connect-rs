//! Incremental identification/comment header decoder. Once a stream's comments
//! have been decoded, audio pages are discarded without retaining their bytes.
use crate::{Error, TrackData};
pub const MAX_COMMENT_BYTES: usize = 131_072;
#[derive(Default)]
pub struct VorbisDecoder {
    buffer: Vec<u8>,
    identification_read: bool,
    complete: bool,
}
impl VorbisDecoder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn buffered_bytes(&self) -> usize {
        self.buffer.len()
    }
    pub fn push(&mut self, input: &[u8]) -> Result<Option<TrackData>, Error> {
        if self.complete {
            return Ok(None);
        }
        if input.len() > MAX_COMMENT_BYTES.saturating_sub(self.buffer.len()) {
            return Err(Error("Vorbis headers exceed byte limit".into()));
        }
        self.buffer.extend_from_slice(input);
        if !self.identification_read {
            if self.buffer.len() < 30 {
                return Ok(None);
            }
            if self.buffer[0] != 1 || &self.buffer[1..7] != b"vorbis" {
                return Err(Error("Expected Vorbis identification header 1".into()));
            }
            self.buffer.drain(..30);
            self.identification_read = true;
        }
        let Some(comments) = parse_packet(&self.buffer)? else {
            return Ok(None);
        };
        let track = parse_comments(&comments);
        self.complete = true;
        self.buffer.clear();
        self.buffer.shrink_to_fit();
        Ok(Some(track))
    }
}
fn read_u32(bytes: &[u8], position: usize) -> Option<usize> {
    bytes
        .get(position..position.checked_add(4)?)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
        .map(|value| value as usize)
}
fn parse_packet(bytes: &[u8]) -> Result<Option<Vec<String>>, Error> {
    if bytes.len() < 11 {
        return Ok(None);
    }
    if bytes[0] != 3 || &bytes[1..7] != b"vorbis" {
        return Err(Error("Expected Vorbis comment header 3".into()));
    }
    let vendor_size = read_u32(bytes, 7).unwrap();
    if vendor_size > MAX_COMMENT_BYTES - 15 {
        return Err(Error("Vorbis vendor exceeds byte limit".into()));
    }
    let count_position = 11 + vendor_size;
    let Some(count) = read_u32(bytes, count_position) else {
        return Ok(None);
    };
    if count > MAX_COMMENT_BYTES / 4 {
        return Err(Error("Vorbis comment count exceeds limit".into()));
    }
    let mut position = count_position + 4;
    let mut comments = Vec::new();
    for _ in 0..count {
        let Some(size) = read_u32(bytes, position) else {
            return Ok(None);
        };
        let end = position
            .checked_add(4)
            .and_then(|value| value.checked_add(size))
            .ok_or_else(|| Error("Vorbis comment size overflow".into()))?;
        if end >= MAX_COMMENT_BYTES {
            return Err(Error("Vorbis comment exceeds byte limit".into()));
        }
        let Some(value) = bytes.get(position + 4..end) else {
            return Ok(None);
        };
        comments.push(String::from_utf8_lossy(value).into_owned());
        position = end;
    }
    let Some(framing) = bytes.get(position) else {
        return Ok(None);
    };
    if *framing != 1 {
        return Err(Error("Invalid Vorbis comment framing bit".into()));
    }
    Ok(Some(comments))
}
pub fn parse_comments(comments: &[String]) -> TrackData {
    let mut track = TrackData::default();
    for comment in comments {
        let (key, value) = comment.split_once('=').unwrap_or((comment, ""));
        match key.to_lowercase().as_str() {
            "artist" => track.artist = value.into(),
            "title" => track.title = value.into(),
            "encoder" => track.encoder = Some(value.into()),
            _ => {}
        }
    }
    track
}
