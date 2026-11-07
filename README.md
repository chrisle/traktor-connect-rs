# traktor-connect-rs

A standalone Rust port of [Christopher Le's traktor-connect](https://github.com/chrisle/traktor-connect), based on TypeScript version 1.1.5 (commit `4c9ac1edcaf4fc49f3d600885979f160bec4f5db`). Receives Traktor Pro's Ogg Vorbis broadcast stream over TCP and extracts artist, title, and encoder comments. It does not decode audio or choose which deck is on air.

## Use

```rust,no_run
use std::time::Duration;
use traktor_connect_rs::{Event, Options, TraktorConnect};

let mut connector = TraktorConnect::new(Options::default());
connector.start()?;
while let Ok(event) = connector.events().recv_timeout(Duration::from_secs(10)) {
    if let Event::Track(track) = event {
        println!("{} - {}", track.artist, track.title);
    }
}
connector.stop()?;
# Ok::<(), traktor_connect_rs::Error>(())
```

`Options` defaults to TCP port 8000, IPv4 unspecified bind address, and skipping the first nonempty track. Set `skip_first_track: false` to emit the connection's initial metadata. Set `bind_address` to loopback to restrict access, or an IPv6 address when needed. Like the original library this listener does not authenticate broadcasts or provide TLS; put network access controls at the deployment boundary when appropriate.

`start` and `stop` are idempotent. `configure(Some(port))` changes the configured port and restarts a running listener; `configure(None)` does nothing. `port()` is the configured value (including `0`), while `local_addr()` gives the actual ephemeral listening address. `is_running()` and `last_error()` expose lifecycle/fail-closed status. `stop` and `Drop` join the listener thread and close its clients; stop discards pending events so they cannot leak into a new run.

Pull events using `events().try_recv()` or `recv_timeout()`. Variants are `Ready`, `Connected`, `Disconnected`, `Track(TrackData)`, and `Error(Error)`. The optional `Logger` trait accepts `LogLevel` and a message; the default emits no logs. Pulling events is the Rust equivalent of registering the TypeScript EventEmitter listeners.

`OggDecoder::push(&[u8])` returns pages with stream serial, page sequence and payload. `VorbisDecoder::push(&[u8])` returns at most one `TrackData` per logical stream; create a new decoder when the Ogg stream serial changes. `parse_comments` is also public. `TrackData` supports serde and omits absent encoder fields.

## Compatibility and intentional differences

Preserved valid-input behavior:

- A CRLF-terminated broadcast header block receives exactly `HTTP/1.0 200 OK\r\n\r\n`, including SOURCE and PUT clients. As in the original, the request method and credentials are not authenticated.
- Case-insensitive Vorbis comment keys; the last repeated key wins; values preserve additional `=` characters. Missing artist/title become empty strings.
- Empty tracks are ignored. Dedupe uses the original MD5 of `artist + title` without a separator, globally across clients, stream serial changes and reconnects. Encoder-only changes do not re-emit tracks.
- Skip-first applies once per listener run, not once per connection. Stop/reconfiguration resets dedupe and skip-first state.
- Ogg structure version 0 is supported; CRC and audio verification are outside this metadata-only API, as in the TypeScript implementation.

Intentional corrections and limits:

- HTTP and Ogg data arriving in the same TCP read is preserved. Incomplete identification/comment headers do not read zero-filled bytes as actual input.
- Identification and comments coalesced in one Ogg payload decode immediately. Arbitrarily fragmented valid headers, lacing tables, pages and comments are supported.
- Garbage and unsupported Ogg versions resynchronize instead of pinning the decoder. Full pages are processed from large chunks without fixed-buffer truncation.
- Vorbis identification requires the `vorbis` signature; invalid framing/lengths close that connection and produce an error rather than silently retaining malformed data indefinitely.
- HTTP headers are limited to 64 KiB, Vorbis headers to 128 KiB, active clients to 64, and queued events to 256. Ogg retained bytes are bounded by a maximum legal page plus one 8 KiB input chunk; returned page vectors are proportional to supplied input.
- A full event queue stops the listener and records `last_error`, rather than silently dropping tracks or blocking shutdown. Consumers must poll events and inspect lifecycle errors. Calling `start` after that failure joins the old run, discards its pending events, and starts a fresh listener with reset dedupe/skip-first state; an explicit `stop` is not required.
- The Rust API is synchronous with one bounded-work background TCP thread, rather than Node streams/EventEmitter. Stop deterministically closes clients and clears pending events. The default bind is explicitly IPv4; select IPv6 through `Options` when required.

## Validation

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
```

Tests cover Ogg extraction and stream changes, arbitrary fragmentation, bounded malformed headers, large chunks, Vorbis comments, actual loopback SOURCE/PUT acknowledgment and metadata flow, dedupe/skip-first across reconnects, live port reconfiguration, client shutdown, and retry after a bind failure. CI runs the same checks on macOS, Windows and Linux; only locally executed results establish current validation. Linux crate checks do not add Linux support to the Now Playing desktop app.

Physical Traktor broadcasting and packaged desktop/cloud integration have not been verified by these library tests. The public source repository is [chrisle/traktor-connect-rs](https://github.com/chrisle/traktor-connect-rs). The optional [differential harness](tests/differential/README.md) compares generated metadata fixtures and loopback TCP behavior against the actual TypeScript source; its locally executed corpus passed. This establishes the documented library behavior, not physical Traktor or desktop/cloud parity.

## License

MIT. The original TypeScript library's copyright and license are preserved in [LICENSE](LICENSE). This port retains the original behavior and attribution without discontinuing the TypeScript package.
