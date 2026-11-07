//! Test-only differential driver: hex-encoded valid fixture chunks on stdin.
use std::io::{self, BufRead, Write};
use traktor_connect_rs::{Event, OggDecoder, Options, TraktorConnect, VorbisDecoder};
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn track(track: traktor_connect_rs::TrackData) -> String {
    format!(
        "{},{},{}",
        hex(track.artist.as_bytes()),
        hex(track.title.as_bytes()),
        track
            .encoder
            .map(|s| hex(s.as_bytes()))
            .unwrap_or_else(|| "-".into())
    )
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--server") {
        let mut server = TraktorConnect::new(Options {
            port: 0,
            bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
            ..Options::default()
        });
        server.start()?;
        println!("PORT:{}", server.local_addr().unwrap().port());
        io::stdout().flush()?;
        let (stop, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = io::stdin().lock().lines().next();
            let _ = stop.send(());
        });
        while receive.try_recv().is_err() {
            match server
                .events()
                .recv_timeout(std::time::Duration::from_millis(20))
            {
                Ok(Event::Track(value)) => {
                    println!("TRACK:{}", track(value));
                    io::stdout().flush()?;
                }
                Ok(Event::Error(error)) => return Err(error.into()),
                _ => {}
            }
        }
        server.stop()?;
        return Ok(());
    }
    for line in io::stdin().lock().lines() {
        let mut ogg = OggDecoder::new();
        let mut vorbis = VorbisDecoder::new();
        let mut serial = None;
        let mut tracks = Vec::new();
        for chunk in line?.split(':') {
            let bytes = (0..chunk.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&chunk[i..i + 2], 16))
                .collect::<Result<Vec<_>, _>>()?;
            for page in ogg.push(&bytes)? {
                if serial != Some(page.stream) {
                    serial = Some(page.stream);
                    vorbis = VorbisDecoder::new();
                }
                if let Some(value) = vorbis.push(&page.payload)? {
                    tracks.push(track(value));
                }
            }
        }
        println!("{}", tracks.join(";"));
        io::stdout().flush()?;
    }
    Ok(())
}
