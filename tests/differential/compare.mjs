// Run from the Now Playing monorepo; real original TS source is bundled, not mocked.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createConnection } from "node:net";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
const crate = fileURLToPath(new URL("../../", import.meta.url));
const root = resolve(crate, "../../");
const { build } = await import(
  pathToFileURL(
    join(root, "apps/desktop-rust/worker/node_modules/esbuild/lib/main.js"),
  )
);
const scratch = await mkdtemp(join(tmpdir(), "traktor-differential-"));
const hex = (bytes) => Buffer.from(bytes).toString("hex");
const norm = (t) =>
  [
    hex(Buffer.from(t.artist)),
    hex(Buffer.from(t.title)),
    t.encoder === undefined ? "-" : hex(Buffer.from(t.encoder)),
  ].join(",");
const ident = () => {
  const b = Buffer.alloc(30);
  b[0] = 1;
  b.write("vorbis", 1);
  b[11] = 2;
  b.writeUInt32LE(44100, 12);
  b[28] = 0xb8;
  b[29] = 1;
  return b;
};
const u32 = (n) => {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(n);
  return b;
};
const comments = (values) =>
  Buffer.concat([
    Buffer.from("\x03vorbis"),
    u32(7),
    Buffer.from("fixture"),
    u32(values.length),
    ...values.flatMap((v) => [u32(Buffer.byteLength(v)), Buffer.from(v)]),
    Buffer.from([1]),
  ]);
const page = (serial, sequence, payload) => {
  const b = Buffer.alloc(27);
  b.write("OggS");
  b.writeUInt32LE(serial, 14);
  b.writeUInt32LE(sequence, 18);
  const lace = [
    ...Array(Math.floor(payload.length / 255)).fill(255),
    payload.length % 255,
  ];
  b[26] = lace.length;
  b[5] = sequence === 0 ? 2 : 0;
  const result = Buffer.concat([b, Buffer.from(lace), payload]);
  let crc = 0;
  for (const byte of result) {
    crc ^= byte << 24;
    for (let bit = 0; bit < 8; bit++)
      crc = ((crc << 1) ^ (crc & 0x80000000 ? 0x04c11db7 : 0)) >>> 0;
  }
  result.writeUInt32LE(crc, 22);
  return result;
};
const stream = (serial, values) => [
  page(serial, 0, ident()),
  page(serial, 1, comments(values)),
];
const cargo = spawnSync(
  "cargo",
  ["build", "--locked", "--example", "differential"],
  { cwd: crate, stdio: "inherit" },
);
assert.equal(cargo.status, 0);
let ts, rust;
try {
  const bundle = join(scratch, "original.mjs");
  await build({
    entryPoints: [join(root, "packages/traktor-connect/src/index.ts")],
    bundle: true,
    platform: "node",
    format: "esm",
    outfile: bundle,
  });
  const { OggDecoder, VorbisDecoder, TraktorConnect } = await import(
    pathToFileURL(bundle)
  );
  const cases = [
    stream(1, ["ARTIST=Fixture", "TITLE=First", "ENCODER=test"]),
    stream(2, ["artist=é 🎧", "TITLE=東京", "ENCODER=a=b"]),
    stream(3, ["ARTIST=Old", "ArTiSt=New", "TITLE=Last", "TITLE=Winner"]),
    stream(4, ["TITLE=Only title"]),
    stream(5, ["ARTIST=Only artist"]),
    stream(6, ["ENCODER=Only encoder"]),
    stream(7, []),
    stream(8, ["ARTIST=" + "x".repeat(600), "TITLE=Long lacing table"]),
    [
      ...stream(9, ["ARTIST=A", "TITLE=One"]),
      ...stream(10, ["ARTIST=B", "TITLE=Two"]),
    ],
  ];
  const original = async (chunks) => {
    const ogg = new OggDecoder();
    let decoder = new VorbisDecoder(),
      last;
    const tracks = [];
    const errors = [];
    ogg.on("error", (e) => errors.push(e));
    ogg.on("page", (p) => {
      if (p.stream !== last) {
        last = p.stream;
        ogg.unpipe(decoder);
        decoder = new VorbisDecoder();
        decoder.on("error", (e) => errors.push(e));
        decoder.on("track", (t) => tracks.push(norm(t)));
        ogg.pipe(decoder);
      }
    });
    for (const chunk of chunks) {
      ogg.write(chunk);
      await new Promise((r) => setImmediate(r));
    }
    ogg.end();
    await new Promise((r) => setImmediate(r));
    assert.deepEqual(errors, []);
    return tracks.join(";");
  };
  const expected = await Promise.all(cases.map(original));
  const result = spawnSync(
    join(crate, "target/debug/examples/differential"),
    [],
    {
      input: cases.map((c) => c.map(hex).join(":")).join("\n") + "\n",
      encoding: "utf8",
    },
  );
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.stdout.trimEnd().split("\n"), expected);
  console.log(`PASS ${cases.length} valid-input TS/Rust decoder corpus cases`);
  const fragmented = stream(101, ["ARTIST=Fragment", "TITLE=Payload"]);
  const payloadChunks = fragmented.flatMap((p) => [
    p.subarray(0, 40),
    p.subarray(40),
  ]);
  const fragmentedExpected = await original(payloadChunks);
  const fragmentedResult = spawnSync(
    join(crate, "target/debug/examples/differential"),
    [],
    { input: payloadChunks.map(hex).join(":") + "\n", encoding: "utf8" },
  );
  assert.equal(fragmentedResult.status, 0, fragmentedResult.stderr);
  assert.equal(fragmentedResult.stdout.trimEnd(), fragmentedExpected);
  console.log(
    "PASS TS/Rust Ogg payload fragmentation with complete page headers",
  );
  const coalesced = [
    page(
      102,
      0,
      Buffer.concat([ident(), comments(["ARTIST=Coalesced", "TITLE=Headers"])]),
    ),
  ];
  const coalescedTs = await original(coalesced);
  const coalescedRust = spawnSync(
    join(crate, "target/debug/examples/differential"),
    [],
    { input: coalesced.map(hex).join(":") + "\n", encoding: "utf8" },
  );
  assert.equal(coalescedTs, "");
  assert.equal(coalescedRust.status, 0, coalescedRust.stderr);
  assert.equal(
    coalescedRust.stdout.trimEnd(),
    norm({ artist: "Coalesced", title: "Headers" }),
  );
  console.log(
    "PASS documented intentional correction: coalesced ID/comments emit immediately in Rust, TS waits for another write",
  );

  // Shared semantics: initial skip, duplicate suppression across serial/reconnect,
  // new tracks and artist+title concatenation collisions.
  ts = new TraktorConnect({ port: 0 });
  const tsTracks = [];
  ts.on("track", (t) => tsTracks.push(norm(t)));
  ts.on("error", (e) => {
    throw e;
  });
  await ts.start();
  const tsPort = ts.server.address().port;
  rust = spawn(
    join(crate, "target/debug/examples/differential"),
    ["--server"],
    { stdio: ["pipe", "pipe", "pipe"] },
  );
  let stdout = "",
    stderr = "";
  rust.stdout.setEncoding("utf8");
  rust.stdout.on("data", (d) => (stdout += d));
  rust.stderr.on("data", (d) => (stderr += d));
  const wait = async (predicate) => {
    const end = Date.now() + 4000;
    while (!predicate()) {
      assert.ok(Date.now() < end, `timeout stdout=${stdout} stderr=${stderr}`);
      await new Promise((r) => setTimeout(r, 10));
    }
  };
  await wait(() => stdout.includes("\n"));
  const rustPort = Number(stdout.match(/PORT:(\d+)/)[1]);
  const send = async (port, serial, values) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    await new Promise((r, j) => {
      socket.once("connect", r);
      socket.once("error", j);
    });
    const reply = new Promise((r, j) => {
      socket.once("data", (d) => {
        try {
          assert.equal(d.toString(), "HTTP/1.0 200 OK\r\n\r\n");
          r();
        } catch (e) {
          j(e);
        }
      });
      socket.once("error", j);
    });
    socket.write(
      "SOURCE /test HTTP/1.0\r\nContent-Type: application/ogg\r\n\r\n",
    );
    await reply;
    for (const p of stream(serial, values)) {
      socket.write(p);
      await new Promise((r) => setTimeout(r, 15));
    }
    socket.end();
    await new Promise((r) => socket.once("close", r));
    await new Promise((r) => setTimeout(r, 15));
  };
  const runs = [
    ["ARTIST=A", "TITLE=First"],
    ["ARTIST=A", "TITLE=First"],
    ["ARTIST=A", "TITLE=Second"],
    ["ARTIST=A", "TITLE=Second"],
    ["ARTIST=AB", "TITLE=C"],
    ["ARTIST=A", "TITLE=BC"],
    ["ARTIST=Last", "TITLE=Track"],
  ];
  for (let i = 0; i < runs.length; i++) {
    await send(tsPort, i + 1, runs[i]);
    await send(rustPort, i + 1, runs[i]);
  }
  await wait(
    () => stdout.split("\n").filter((l) => l.startsWith("TRACK:")).length === 3,
  );
  const rustTracks = stdout
    .split("\n")
    .filter((l) => l.startsWith("TRACK:"))
    .map((l) => l.slice(6));
  assert.deepEqual(rustTracks, tsTracks);
  assert.equal(tsTracks.length, 3);
  console.log(
    "PASS loopback TCP TS/Rust ack, skip-first, cross-reconnect/serial dedupe and concatenation collision",
  );
  rust.stdin.end("stop\n");
  await new Promise((r, j) =>
    rust.once("exit", (code) =>
      code === 0 ? r() : j(Error(`Rust exit ${code}`)),
    ),
  );
  await ts.stop();
} finally {
  rust?.kill();
  await ts?.stop();
  await rm(scratch, { recursive: true, force: true });
}
