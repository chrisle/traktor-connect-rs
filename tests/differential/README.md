# Original TypeScript comparison

This optional comparison is run from the Now Playing monorepo, where both the
pinned original TypeScript source and the Rust port exist. The standalone Rust
crate's ordinary Cargo tests do not require Node or that monorepo.

```sh
npm --prefix apps/desktop-rust/worker ci --ignore-scripts
node packages-rs/traktor-connect-rs/tests/differential/compare.mjs
```

The runner builds the Rust example and bundles the actual original
`packages/traktor-connect/src/index.ts` with esbuild from the connector worker.
It uses the original `OggDecoder`, `VorbisDecoder`, and `TraktorConnect`, without
mocking their parser or server. It creates synthetic metadata-only fixtures in
memory and removes the temporary JS bundle. It does not use private captures,
Traktor, audio decoding, hardware, the cloud, or the desktop adapter.

Verified against TypeScript gitlink/HEAD
`4c9ac1edcaf4fc49f3d600885979f160bec4f5db`:

- Nine Ogg/Vorbis metadata cases: normal fields, Unicode, repeated keys and case,
  embedded equals, absent artist/title/encoder, empty comments, multi-segment
  lacing and serial changes.
- Payload fragmentation with page headers complete.
- The documented intentional correction for coalesced identification and comment
  data: Rust emits immediately; the original TS decoder waits for another write.
- Real loopback TCP SOURCE header acknowledgment, default first-track skip,
  dedupe across connections/serial changes, and the original concatenated
  artist+title hash collision behavior.

Fixtures include a complete identification header and computed Ogg CRC, but do
not include setup/audio packets and do not establish audio-stream conformance.
The coalesced payload case is a decoder-specific regression exercise rather than
proof of correct general Vorbis packet/lacing interpretation.

The test driver's hex protocol is internal to this comparison, not a supported
library API. The TCP helper binds only loopback on ephemeral ports. Existing Rust
unit/TCP tests cover additional malformed-input and resource-limit corrections;
those differ deliberately from TypeScript and are not all differential cases.

This comparison establishes the listed library-level behavior only. Real
Traktor broadcasts, the Now Playing adapter and proxy registration, Windows
runtime, packaged desktop/cloud flow, and public repository/CI execution remain
separate evidence gates.
