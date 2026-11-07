pub fn identification() -> Vec<u8> {
    let mut value = vec![0; 30];
    value[0] = 1;
    value[1..7].copy_from_slice(b"vorbis");
    value
}
pub fn comments(values: &[&str]) -> Vec<u8> {
    let mut value = b"\x03vorbis".to_vec();
    value.extend(7_u32.to_le_bytes());
    value.extend(b"fixture");
    value.extend((values.len() as u32).to_le_bytes());
    for comment in values {
        value.extend((comment.len() as u32).to_le_bytes());
        value.extend(comment.as_bytes());
    }
    value.push(1);
    value
}
pub fn page(serial: u32, sequence: u32, data: &[u8]) -> Vec<u8> {
    let mut header = vec![0; 27];
    header[..4].copy_from_slice(b"OggS");
    header[14..18].copy_from_slice(&serial.to_le_bytes());
    header[18..22].copy_from_slice(&sequence.to_le_bytes());
    let segments = data.len() / 255 + 1;
    assert!(segments <= 255);
    header[26] = segments as u8;
    header.extend(std::iter::repeat_n(255, data.len() / 255));
    header.push((data.len() % 255) as u8);
    header.extend(data);
    header
}
pub fn stream(serial: u32, artist: &str, title: &str) -> Vec<u8> {
    [
        page(serial, 0, &identification()),
        page(
            serial,
            1,
            &comments(&[
                &format!("ARTIST={artist}"),
                &format!("TITLE={title}"),
                "ENCODER=fixture",
            ]),
        ),
    ]
    .concat()
}
