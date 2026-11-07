use crate::{Error, OggDecoder, TrackData, VorbisDecoder};
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
const MAX_HTTP_HEADER: usize = 65_536;
const MAX_CLIENTS: usize = 64;
const EVENT_CAPACITY: usize = 256;
const RESPONSE: &[u8] = b"HTTP/1.0 200 OK\r\n\r\n";
#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
pub trait Logger: Send + Sync {
    fn log(&self, level: LogLevel, message: &str);
}
#[derive(Clone)]
pub struct Options {
    pub port: u16,
    pub skip_first_track: bool,
    pub bind_address: IpAddr,
    pub logger: Option<Arc<dyn Logger>>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            port: 8000,
            skip_first_track: true,
            bind_address: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            logger: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Track(TrackData),
    Connected,
    Disconnected,
    Error(Error),
    Ready,
}
/// Owns a background TCP event loop. Events are pulled from a bounded channel;
/// an unconsumed full channel fails closed instead of silently dropping tracks.
pub struct TraktorConnect {
    options: Options,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    events: Receiver<Event>,
    sender: SyncSender<Event>,
    local_address: Option<SocketAddr>,
    failure: Arc<Mutex<Option<Error>>>,
}
impl TraktorConnect {
    pub fn new(options: Options) -> Self {
        let (sender, events) = mpsc::sync_channel(EVENT_CAPACITY);
        Self {
            options,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
            events,
            sender,
            local_address: None,
            failure: Arc::new(Mutex::new(None)),
        }
    }
    pub fn port(&self) -> u16 {
        self.options.port
    }
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.local_address
    }
    pub fn is_running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }
    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }
    pub fn last_error(&self) -> Option<Error> {
        self.failure.lock().ok().and_then(|failure| failure.clone())
    }
    pub fn start(&mut self) -> Result<(), Error> {
        if self.is_running() {
            return Ok(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // The previous run may have stopped because its receiver filled. Join
        // first so no old producer can race this drain, then discard its queued
        // events before the next run emits Ready. This also clears stale bind
        // errors on a successful retry without requiring stop() beforehand.
        while self.events.try_recv().is_ok() {}
        self.local_address = None;
        let listener = match TcpListener::bind(SocketAddr::new(
            self.options.bind_address,
            self.options.port,
        )) {
            Ok(listener) => listener,
            Err(error) => {
                let error = Error::from(error);
                let _ = self.sender.try_send(Event::Error(error.clone()));
                return Err(error);
            }
        };
        listener.set_nonblocking(true)?;
        self.local_address = Some(listener.local_addr()?);
        self.stop.store(false, Ordering::Release);
        if let Ok(mut failure) = self.failure.lock() {
            *failure = None;
        }
        let state = Runtime {
            stop: self.stop.clone(),
            sender: self.sender.clone(),
            failure: self.failure.clone(),
            options: self.options.clone(),
        };
        self.thread = Some(thread::spawn(move || serve(listener, state)));
        Ok(())
    }
    pub fn stop(&mut self) -> Result<(), Error> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| Error("Traktor listener thread panicked".into()))?;
        }
        self.local_address = None;
        // No event from the old listener may be mistaken for a new run.
        while self.events.try_recv().is_ok() {}
        Ok(())
    }
    /// Like the TS API, only a changed port triggers reconfiguration/restart.
    pub fn configure(&mut self, port: Option<u16>) -> Result<(), Error> {
        if let Some(port) = port {
            if self.options.port != port {
                let running = self.is_running();
                self.stop()?;
                self.options.port = port;
                if running {
                    self.start()?;
                }
            }
        }
        Ok(())
    }
}
impl Drop for TraktorConnect {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
struct Runtime {
    stop: Arc<AtomicBool>,
    sender: SyncSender<Event>,
    failure: Arc<Mutex<Option<Error>>>,
    options: Options,
}
impl Runtime {
    fn log(&self, level: LogLevel, message: &str) {
        if let Some(logger) = &self.options.logger {
            logger.log(level, message);
        }
    }
    fn emit(&self, event: Event) -> bool {
        match self.sender.try_send(event) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                let error = Error("Traktor event queue is full; listener stopped".into());
                self.log(LogLevel::Error, &error.0);
                if let Ok(mut failure) = self.failure.lock() {
                    *failure = Some(error);
                }
                self.stop.store(true, Ordering::Release);
                false
            }
            Err(TrySendError::Disconnected(_)) => {
                self.stop.store(true, Ordering::Release);
                false
            }
        }
    }
}
struct Client {
    socket: TcpStream,
    header: Vec<u8>,
    header_complete: bool,
    reply_sent: usize,
    ogg: OggDecoder,
    vorbis: VorbisDecoder,
    last_stream: Option<u32>,
}
impl Client {
    fn new(socket: TcpStream) -> Result<Self, Error> {
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            header: Vec::new(),
            header_complete: false,
            reply_sent: 0,
            ogg: OggDecoder::new(),
            vorbis: VorbisDecoder::new(),
            last_stream: None,
        })
    }
    fn data(&mut self, bytes: &[u8]) -> Result<Vec<TrackData>, Error> {
        let mut offset = 0;
        if !self.header_complete {
            for byte in bytes {
                if self.header.len() == MAX_HTTP_HEADER {
                    return Err(Error("Broadcast headers exceed byte limit".into()));
                }
                self.header.push(*byte);
                offset += 1;
                if self.header.ends_with(b"\r\n\r\n") {
                    self.header_complete = true;
                    self.header.clear();
                    self.header.shrink_to_fit();
                    break;
                }
            }
            if !self.header_complete {
                return Ok(Vec::new());
            }
        }
        let mut tracks = Vec::new();
        for page in self.ogg.push(&bytes[offset..])? {
            if self.last_stream != Some(page.stream) {
                self.last_stream = Some(page.stream);
                self.vorbis = VorbisDecoder::new();
            }
            if let Some(track) = self.vorbis.push(&page.payload)? {
                tracks.push(track);
            }
        }
        Ok(tracks)
    }
    fn flush_reply(&mut self) -> Result<(), Error> {
        if self.header_complete && self.reply_sent < RESPONSE.len() {
            match self.socket.write(&RESPONSE[self.reply_sent..]) {
                Ok(0) => {
                    return Err(Error(
                        "Broadcast socket closed while acknowledging headers".into(),
                    ))
                }
                Ok(written) => self.reply_sent += written,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}
fn serve(listener: TcpListener, state: Runtime) {
    state.log(LogLevel::Info, "Traktor listener started");
    if !state.emit(Event::Ready) {
        return;
    }
    let mut clients = Vec::<Client>::new();
    let mut last_hash = None;
    while !state.stop.load(Ordering::Acquire) {
        // Bound accepts and per-client reads per turn so shutdown cannot starve.
        for _ in 0..MAX_CLIENTS {
            match listener.accept() {
                Ok((socket, _)) => {
                    if clients.len() == MAX_CLIENTS {
                        drop(socket);
                        state.emit(Event::Error(Error("Traktor client limit reached".into())));
                        continue;
                    }
                    match Client::new(socket) {
                        Ok(client) => {
                            clients.push(client);
                            state.emit(Event::Connected);
                        }
                        Err(error) => {
                            state.emit(Event::Error(error));
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    state.emit(Event::Error(error.into()));
                    break;
                }
            }
        }
        let mut index = 0;
        while index < clients.len() {
            let mut closed = false;
            let client = &mut clients[index];
            if let Err(error) = client.flush_reply() {
                state.emit(Event::Error(error));
                closed = true;
            }
            let mut bytes = [0_u8; 8192];
            for _ in 0..8 {
                if closed {
                    break;
                }
                match client.socket.read(&mut bytes) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(length) => match client.data(&bytes[..length]) {
                        Ok(tracks) => {
                            for track in tracks {
                                if track.artist.is_empty() && track.title.is_empty() {
                                    continue;
                                }
                                // Keep legacy MD5 concatenation semantics, including
                                // dedupe across stream serials and reconnects.
                                let hash = md5::compute(format!("{}{}", track.artist, track.title));
                                if last_hash == Some(hash) {
                                    continue;
                                }
                                let skip = state.options.skip_first_track && last_hash.is_none();
                                last_hash = Some(hash);
                                if !skip {
                                    state.emit(Event::Track(track));
                                }
                            }
                        }
                        Err(error) => {
                            state.emit(Event::Error(error));
                            closed = true;
                        }
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => {
                        state.emit(Event::Error(error.into()));
                        closed = true;
                    }
                }
            }
            if closed {
                clients.swap_remove(index);
                state.emit(Event::Disconnected);
            } else {
                index += 1;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    drop(clients);
    state.log(LogLevel::Debug, "Traktor listener stopped");
}
