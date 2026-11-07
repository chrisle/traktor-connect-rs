mod common;
use common::*;
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, TcpListener, TcpStream},
    time::{Duration, Instant},
};
use traktor_connect_rs::{Event, Options, TraktorConnect};
fn server(skip_first_track: bool) -> TraktorConnect {
    let mut server = TraktorConnect::new(Options {
        port: 0,
        skip_first_track,
        bind_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
        ..Options::default()
    });
    server.start().unwrap();
    assert_eq!(
        server
            .events()
            .recv_timeout(Duration::from_secs(2))
            .unwrap(),
        Event::Ready
    );
    server
}
fn connect(server: &TraktorConnect) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr().unwrap()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
}
fn acknowledge(stream: &mut TcpStream) {
    let mut reply = [0; 19];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"HTTP/1.0 200 OK\r\n\r\n");
}
fn track(server: &TraktorConnect) -> traktor_connect_rs::TrackData {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match server
            .events()
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            Event::Track(track) => return track,
            Event::Error(error) => panic!("{error}"),
            _ => {}
        }
    }
}
fn no_track(server: &TraktorConnect) {
    let deadline = Instant::now() + Duration::from_millis(60);
    while let Ok(event) = server
        .events()
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        assert!(!matches!(event, Event::Track(_)), "unexpected {event:?}");
    }
}
#[test]
fn source_and_put_acknowledge_fragmented_headers_and_preserve_coalesced_body() {
    for method in ["SOURCE", "PUT"] {
        let mut server = server(false);
        let mut client = connect(&server);
        for byte in format!("{method} /live HTTP/1.0\r\nContent-Type: application/ogg\r\n").bytes()
        {
            client.write_all(&[byte]).unwrap();
        }
        client
            .write_all(&[b"\r\n".as_slice(), &stream(1, "Artist", "Title")].concat())
            .unwrap();
        acknowledge(&mut client);
        assert_eq!(track(&server).title, "Title");
        server.stop().unwrap();
    }
}
#[test]
fn fragmented_metadata_and_serial_changes_emit_without_duplicates() {
    let mut server = server(false);
    let mut client = connect(&server);
    client.write_all(b"SOURCE /live HTTP/1.0\r\n\r\n").unwrap();
    acknowledge(&mut client);
    for chunk in stream(1, "Artist", "First").chunks(3) {
        client.write_all(chunk).unwrap();
    }
    assert_eq!(track(&server).title, "First");
    client.write_all(&stream(2, "Artist", "First")).unwrap();
    no_track(&server);
    client.write_all(&stream(3, "Artist", "Second")).unwrap();
    assert_eq!(track(&server).title, "Second");
    server.stop().unwrap();
}
#[test]
fn skip_first_and_dedupe_survive_reconnect_but_reset_after_stop() {
    let mut server = server(true);
    let mut client = connect(&server);
    client
        .write_all(
            &[
                b"SOURCE / HTTP/1.0\r\n\r\n".as_slice(),
                &stream(1, "Artist", "First"),
            ]
            .concat(),
        )
        .unwrap();
    acknowledge(&mut client);
    no_track(&server);
    drop(client);
    let mut client = connect(&server);
    client
        .write_all(
            &[
                b"PUT / HTTP/1.0\r\n\r\n".as_slice(),
                &stream(1, "Artist", "First"),
            ]
            .concat(),
        )
        .unwrap();
    acknowledge(&mut client);
    no_track(&server);
    client.write_all(&stream(2, "Artist", "Second")).unwrap();
    assert_eq!(track(&server).title, "Second");
    server.stop().unwrap();
    server.start().unwrap();
    assert_eq!(
        server
            .events()
            .recv_timeout(Duration::from_secs(2))
            .unwrap(),
        Event::Ready
    );
    let mut client = connect(&server);
    client
        .write_all(
            &[
                b"SOURCE / HTTP/1.0\r\n\r\n".as_slice(),
                &stream(9, "Artist", "Again"),
            ]
            .concat(),
        )
        .unwrap();
    acknowledge(&mut client);
    no_track(&server);
    client.write_all(&stream(10, "Artist", "Next")).unwrap();
    assert_eq!(track(&server).title, "Next");
    server.stop().unwrap();
}
#[test]
fn runtime_port_configuration_restarts_and_closes_old_clients() {
    let mut server = server(false);
    let old = server.local_addr().unwrap();
    let mut client = connect(&server);
    client.write_all(b"SOURCE / HTTP/1.0\r\n\r\n").unwrap();
    acknowledge(&mut client);
    let reserved = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    server.configure(Some(port)).unwrap();
    assert_eq!(server.port(), port);
    assert_eq!(server.local_addr().unwrap().port(), port);
    assert_ne!(server.local_addr().unwrap(), old);
    let mut byte = [0];
    assert_eq!(client.read(&mut byte).unwrap(), 0);
    assert_eq!(
        server
            .events()
            .recv_timeout(Duration::from_secs(2))
            .unwrap(),
        Event::Ready
    );
    let mut client = connect(&server);
    client
        .write_all(
            &[
                b"SOURCE / HTTP/1.0\r\n\r\n".as_slice(),
                &stream(1, "A", "After configure"),
            ]
            .concat(),
        )
        .unwrap();
    acknowledge(&mut client);
    assert_eq!(track(&server).title, "After configure");
    server.stop().unwrap();
}
#[test]
fn startup_failure_is_retryable_and_lifecycle_is_idempotent() {
    let occupied = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = occupied.local_addr().unwrap().port();
    let mut server = TraktorConnect::new(Options {
        port,
        bind_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
        ..Options::default()
    });
    assert!(server.start().is_err());
    assert!(!server.is_running());
    drop(occupied);
    server.start().unwrap();
    let addr = server.local_addr();
    server.start().unwrap();
    assert_eq!(server.local_addr(), addr);
    server.stop().unwrap();
    server.stop().unwrap();
    assert!(!server.is_running());
    server.configure(Some(9000)).unwrap();
    assert_eq!(server.port(), 9000);
    assert!(!server.is_running());
}
#[test]
fn malformed_connection_is_closed_without_killing_listener() {
    let mut server = server(false);
    let mut client = connect(&server);
    client.write_all(&vec![b'x'; 65537]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_error = false;
    while Instant::now() < deadline {
        if let Event::Error(_) = server
            .events()
            .recv_timeout(Duration::from_millis(500))
            .unwrap()
        {
            saw_error = true;
            break;
        }
    }
    assert!(saw_error);
    assert!(server.is_running());
    let mut valid = connect(&server);
    valid
        .write_all(
            &[
                b"PUT / HTTP/1.0\r\n\r\n".as_slice(),
                &stream(1, "A", "Good"),
            ]
            .concat(),
        )
        .unwrap();
    acknowledge(&mut valid);
    assert_eq!(track(&server).title, "Good");
    server.stop().unwrap();
}

#[test]
fn empty_tracks_are_ignored_and_dedupe_is_global_between_clients() {
    let mut server = server(false);
    let mut first = connect(&server);
    first
        .write_all(&[b"SOURCE / HTTP/1.0\r\n\r\n".as_slice(), &stream(1, "", "")].concat())
        .unwrap();
    acknowledge(&mut first);
    no_track(&server);
    first.write_all(&stream(2, "Same", "Track")).unwrap();
    assert_eq!(track(&server).artist, "Same");
    let mut second = connect(&server);
    second
        .write_all(
            &[
                b"PUT / HTTP/1.0\r\n\r\n".as_slice(),
                &stream(1, "Same", "Track"),
            ]
            .concat(),
        )
        .unwrap();
    acknowledge(&mut second);
    no_track(&server);
    second.write_all(&stream(2, "Other", "Track")).unwrap();
    assert_eq!(track(&server).artist, "Other");
    server.stop().unwrap();
}
#[test]
fn event_queue_overflow_fails_closed_without_blocking_shutdown() {
    let mut server = server(false);
    let mut client = connect(&server);
    client.write_all(b"SOURCE / HTTP/1.0\r\n\r\n").unwrap();
    acknowledge(&mut client);
    let data = (0..350)
        .flat_map(|serial| stream(serial, "Artist", &format!("Track {serial}")))
        .collect::<Vec<_>>();
    let _ = client.write_all(&data);
    let deadline = Instant::now() + Duration::from_secs(2);
    while server.is_running() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!server.is_running());
    assert!(server.last_error().unwrap().0.contains("queue is full"));
    server.stop().unwrap();
}

#[test]
fn start_recovers_directly_after_queue_saturation_without_old_events() {
    let mut server = server(true);
    let mut client = connect(&server);
    client.write_all(b"SOURCE / HTTP/1.0\r\n\r\n").unwrap();
    acknowledge(&mut client);
    let data = (0..350)
        .flat_map(|serial| stream(serial, "Old Artist", &format!("Old Track {serial}")))
        .collect::<Vec<_>>();
    let _ = client.write_all(&data);
    let deadline = Instant::now() + Duration::from_secs(2);
    while server.is_running() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!server.is_running());
    assert!(server.last_error().is_some());
    // Do not drain or call stop: start itself must retire the full old queue.
    server.start().unwrap();
    assert_eq!(
        server
            .events()
            .recv_timeout(Duration::from_secs(2))
            .unwrap(),
        Event::Ready
    );
    assert!(server.is_running());
    assert!(server.last_error().is_none());
    assert!(server.events().try_recv().is_err());
    let mut client = connect(&server);
    client
        .write_all(
            &[
                b"PUT / HTTP/1.0\r\n\r\n".as_slice(),
                &stream(1, "New Artist", "Skip new first"),
            ]
            .concat(),
        )
        .unwrap();
    acknowledge(&mut client);
    no_track(&server);
    client
        .write_all(&stream(2, "New Artist", "New emitted track"))
        .unwrap();
    assert_eq!(track(&server).title, "New emitted track");
    server.stop().unwrap();
}
