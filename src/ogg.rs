//! Incremental Ogg page extraction. CRC/audio decoding is intentionally absent,
//! matching the TypeScript metadata-only decoder.
use crate::Error;
pub const MAX_OGG_PAGE_BYTES: usize = 27 + 255 + 255 * 255;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OggPage {
    pub stream: u32,
    pub page: u32,
    pub payload: Vec<u8>,
}
#[derive(Default)]
pub struct OggDecoder {
    buffer: Vec<u8>,
}
impl OggDecoder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn buffered_bytes(&self) -> usize {
        self.buffer.len()
    }
    pub fn push(&mut self, input: &[u8]) -> Result<Vec<OggPage>, Error> {
        let mut pages = Vec::new();
        for chunk in input.chunks(8192) {
            self.buffer.extend_from_slice(chunk);
            loop {
                let Some(position) = self.buffer.windows(4).position(|bytes| bytes == b"OggS")
                else {
                    let keep = self.buffer.len().min(3);
                    self.buffer.drain(..self.buffer.len() - keep);
                    break;
                };
                if position > 0 {
                    self.buffer.drain(..position);
                }
                if self.buffer.len() < 27 {
                    break;
                }
                if self.buffer[4] != 0 {
                    self.buffer.drain(..4);
                    continue;
                }
                let segments = self.buffer[26] as usize;
                if self.buffer.len() < 27 + segments {
                    break;
                }
                let payload_size: usize = self.buffer[27..27 + segments]
                    .iter()
                    .map(|value| *value as usize)
                    .sum();
                let size = 27 + segments + payload_size;
                if size > MAX_OGG_PAGE_BYTES {
                    return Err(Error("Ogg page exceeds format limit".into()));
                }
                if self.buffer.len() < size {
                    break;
                }
                let stream = u32::from_le_bytes(self.buffer[14..18].try_into().unwrap());
                let page = u32::from_le_bytes(self.buffer[18..22].try_into().unwrap());
                let payload = self.buffer[27 + segments..size].to_vec();
                self.buffer.drain(..size);
                pages.push(OggPage {
                    stream,
                    page,
                    payload,
                });
            }
        }
        Ok(pages)
    }
}
