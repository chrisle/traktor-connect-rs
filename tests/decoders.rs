mod common;
use common::*;
use traktor_connect_rs::{parse_comments, OggDecoder, VorbisDecoder, MAX_COMMENT_BYTES};
#[test]
fn extracts_original_minimal_page_fields() {
    let pages = OggDecoder::new().push(&page(12345, 0, &[0; 10])).unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].stream, 12345);
    assert_eq!(pages[0].page, 0);
    assert_eq!(pages[0].payload, vec![0; 10]);
}
#[test]
fn pages_fragmented_at_every_boundary_preserve_payloads_and_stream_changes() {
    let input = [
        page(11111, 0, &identification()),
        page(22222, 1, &comments(&["ARTIST=a", "TITLE=b"])),
    ]
    .concat();
    for split in 0..=input.len() {
        let mut decoder = OggDecoder::new();
        let mut pages = decoder.push(&input[..split]).unwrap();
        pages.extend(decoder.push(&input[split..]).unwrap());
        assert_eq!(
            pages
                .iter()
                .map(|page| (page.stream, page.page))
                .collect::<Vec<_>>(),
            vec![(11111, 0), (22222, 1)]
        );
    }
}
#[test]
fn resynchronizes_garbage_and_invalid_version_with_bounded_retention() {
    let mut decoder = OggDecoder::new();
    assert!(decoder.push(&vec![42; 1_000_000]).unwrap().is_empty());
    assert!(decoder.buffered_bytes() <= 3);
    let mut invalid = page(1, 0, &[0; 10]);
    invalid[4] = 1;
    let input = [invalid, page(2, 0, &[9; 10])].concat();
    let pages = decoder.push(&input).unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].stream, 2);
}
#[test]
fn supports_multiple_large_pages_in_one_chunk_without_truncation() {
    let data = vec![37; 60_000];
    let input = [page(1, 0, &data), page(1, 1, &data), page(2, 0, &data)].concat();
    let pages = OggDecoder::new().push(&input).unwrap();
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[2].payload, data);
}
#[test]
fn vorbis_handles_every_byte_fragment_and_discards_audio_after_metadata() {
    let input = [
        identification(),
        comments(&["ARTIST=Årtist", "title=track=extended", "ENCODER=test"]),
    ]
    .concat();
    let mut decoder = VorbisDecoder::new();
    let mut track = None;
    for byte in input {
        if let Some(value) = decoder.push(&[byte]).unwrap() {
            assert!(track.is_none());
            track = Some(value);
        }
    }
    let track = track.unwrap();
    assert_eq!(track.artist, "Årtist");
    assert_eq!(track.title, "track=extended");
    assert_eq!(track.encoder.as_deref(), Some("test"));
    assert!(decoder.push(&vec![0; 1_000_000]).unwrap().is_none());
    assert_eq!(decoder.buffered_bytes(), 0);
}
#[test]
fn identification_and_comments_coalesced_in_one_payload_emit_immediately() {
    let input = [identification(), comments(&["TITLE=coalesced"])].concat();
    assert_eq!(
        VorbisDecoder::new().push(&input).unwrap().unwrap().title,
        "coalesced"
    );
}
#[test]
fn incomplete_headers_do_not_emit_and_invalid_lengths_fail_boundedly() {
    let mut decoder = VorbisDecoder::new();
    assert!(decoder.push(&identification()).unwrap().is_none());
    let input = comments(&["TITLE=test"]);
    assert!(decoder.push(&input[..input.len() - 1]).unwrap().is_none());
    assert_eq!(decoder.push(&[1]).unwrap().unwrap().title, "test");
    let mut oversized = b"\x03vorbis".to_vec();
    oversized.extend(u32::MAX.to_le_bytes());
    let mut decoder = VorbisDecoder::new();
    decoder.push(&identification()).unwrap();
    assert!(decoder.push(&oversized).is_err());
    assert!(VorbisDecoder::new()
        .push(&vec![0; MAX_COMMENT_BYTES + 1])
        .is_err());
    assert!(VorbisDecoder::new().push(&[0; 30]).is_err());
}
#[test]
fn duplicate_case_insensitive_keys_last_win_missing_fields_and_equals_match_ts() {
    let track = parse_comments(&[
        "Artist=first".into(),
        "ARTIST=last".into(),
        "TITLE=a=b".into(),
        "encoder".into(),
    ]);
    assert_eq!(track.artist, "last");
    assert_eq!(track.title, "a=b");
    assert_eq!(track.encoder.as_deref(), Some(""));
    assert_eq!(parse_comments(&[]).artist, "");
}
#[test]
fn stream_fixture_decodes_through_both_layers() {
    let pages = OggDecoder::new()
        .push(&stream(42, "Artist", "Title"))
        .unwrap();
    let mut decoder = VorbisDecoder::new();
    let mut tracks = Vec::new();
    for page in pages {
        if let Some(track) = decoder.push(&page.payload).unwrap() {
            tracks.push(track);
        }
    }
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Title");
}

#[test]
fn comment_packet_continues_across_ogg_pages() {
    let metadata = comments(&["ARTIST=Page Artist", "TITLE=Continued"]);
    let input = [
        page(5, 0, &identification()),
        page(5, 1, &metadata[..13]),
        page(5, 2, &metadata[13..]),
    ]
    .concat();
    let mut decoder = VorbisDecoder::new();
    let mut tracks = Vec::new();
    for page in OggDecoder::new().push(&input).unwrap() {
        if let Some(track) = decoder.push(&page.payload).unwrap() {
            tracks.push(track);
        }
    }
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Continued");
}
