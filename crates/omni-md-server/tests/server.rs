//! Integration test: spawns the real server binary on a scratch port,
//! seeds synthetic ticks through the whole data plane, and probes the
//! endpoints over raw HTTP (omni-service pattern, no extra deps).
//! Keeps `main` inside the coverage gate.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn http_get(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok()?;
    Some(buf)
}

fn spawn(port: u16, ticks: usize) -> Option<Child> {
    spawn_opts(port, ticks, None)
}

fn spawn_opts(port: u16, ticks: usize, udp: Option<u16>) -> Option<Child> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_omni-md-server"));
    cmd.env("OMNI_PORT", port.to_string())
        .env("OMNI_TICKS", ticks.to_string());
    if let Some(u) = udp {
        cmd.env("OMNI_UDP_PORT", u.to_string());
    }
    cmd.spawn().ok()
}

fn wait_listening(port: u16) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn body_of(response: &str) -> &str {
    match response.split_once("\r\n\r\n") {
        Some((_, body)) => body,
        None => "",
    }
}

#[test]
fn the_data_plane_serves_what_the_pipeline_stored() {
    let port: u16 = 19090 + u16::try_from(std::process::id() % 1000).unwrap_or(0);
    let mut child = spawn(port, 8);
    assert!(child.is_some(), "server spawns");
    let Some(child) = child.as_mut() else { return };

    assert!(wait_listening(port), "server listens on {port}");
    let early_exit = child.try_wait();
    let still_running = early_exit.as_ref().is_ok_and(std::option::Option::is_none);
    assert!(still_running, "server stays up");

    // Collect everything FIRST, then kill the child: a failing assert
    // must never leave the child holding the harness's stdio pipe
    // (cargo waits on pipe EOF and the test hangs).
    let health = http_get(port, "/healthz");
    let stats = http_get(port, "/stats");
    let latest = http_get(port, "/ticks/latest/3");
    let over = http_get(port, "/ticks/latest/999");

    let _ = child.kill();
    let _ = child.wait();

    // Liveness.
    assert!(health.is_some(), "healthz answers");
    let health_page = health.unwrap_or_default();
    assert!(
        body_of(&health_page).contains("ok"),
        "healthz body: {health_page}"
    );

    // The seeded plane: 8 synthetic ticks through encode -> decode ->
    // fan-out -> segment, all queryable.
    assert!(stats.is_some(), "stats answers");
    let stats_page = stats.unwrap_or_default();
    assert!(
        body_of(&stats_page).contains("\"records\":8"),
        "stats reports the 8 seeded records: {}",
        body_of(&stats_page)
    );

    // Latest-3 endpoint: exactly 3 JSON records, the newest last.
    assert!(latest.is_some(), "latest answers");
    let latest_page = latest.unwrap_or_default();
    let latest_body = body_of(&latest_page);
    assert!(latest_body.starts_with('['), "JSON array: {latest_page}");
    // Record delimiters are `},{` — field commas inside each record
    // must not count.
    let records = latest_body.matches("},{").count() + 1;
    assert_eq!(records, 3, "three records; body was: {latest_body:?}");
    assert!(
        latest_body.contains("\"ts_ns\":1700000000000000700"),
        "newest tick present: {latest_page}"
    );

    // Over-ask clamps to the segment's count.
    assert!(over.is_some(), "over-ask answers");
    let over_page = over.unwrap_or_default();
    let over_body = body_of(&over_page);
    let over_records = over_body.matches("},{").count() + 1;
    assert_eq!(over_records, 8, "clamped to all 8 records: {over_page}");
}

/// The real feed socket: one SBE frame per datagram travels the same
/// pipeline seam as the synthetic seed (`frame_to_tick` -> fan-out ->
/// segment) and surfaces through the endpoints.
#[test]
fn the_udp_feed_persists_through_the_pipeline() {
    use omni_md::{FRAME_LEN, TICK_SCHEMA_ID, TICK_TEMPLATE_ID};
    use omni_sbe::SbeEncoder;

    let port: u16 = 19590 + u16::try_from(std::process::id() % 1000).unwrap_or(0);
    let udp: u16 = 19790 + u16::try_from(std::process::id() % 1000).unwrap_or(0);
    let mut child = spawn_opts(port, 4, Some(udp));
    assert!(child.is_some(), "server spawns");
    let Some(child) = child.as_mut() else { return };
    assert!(wait_listening(port), "server listens on {port}");

    // Encode one tick to the wire exactly as the plane emits it.
    let mut frame = [0u8; FRAME_LEN];
    let mut enc = SbeEncoder::new(&mut frame);
    enc.write_header(32, TICK_TEMPLATE_ID, TICK_SCHEMA_ID, 0);
    assert!(enc.put_u32(8, 7) && enc.put_u32(12, 0));
    assert!(enc.put_u64(16, 1_000) && enc.put_u64(24, 2_000));
    assert!(enc.put_u64(32, 1_700_000_100_000_000_000));

    let socket = std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0));
    assert!(socket.is_ok(), "udp bind");
    let Some(socket) = socket.ok() else { return };
    // The encoder's output must decode: the same seam the seeder uses.
    assert!(
        omni_md::frame_to_tick(&frame).is_some(),
        "the encoded frame fails the plane's own decode"
    );
    let sent = socket.send_to(&frame, (std::net::Ipv4Addr::LOCALHOST, udp));
    assert!(sent.is_ok(), "udp frame send");
    // One malformed datagram: counted bad, never crashing the plane.
    let junk = socket.send_to(b"junk", (std::net::Ipv4Addr::LOCALHOST, udp));
    assert!(junk.is_ok(), "udp junk send");

    // Poll /stats until the frame lands (or fail on timeout).
    let deadline = Instant::now() + Duration::from_secs(10);
    let stats = loop {
        let page = http_get(port, "/stats").unwrap_or_default();
        let body = body_of(&page);
        if body.contains("\"udp_ticks\":1") && body.contains("\"udp_bad\":1") {
            break body.to_string();
        }
        assert!(Instant::now() < deadline, "udp tick never landed: {body}");
        std::thread::sleep(Duration::from_millis(100));
    };

    // The fed tick is the newest record — poll inside the loop's
    // lifetime, before the child dies.
    let deadline2 = Instant::now() + Duration::from_secs(5);
    let latest_body = loop {
        let page = http_get(port, "/ticks/latest/1").unwrap_or_default();
        let body = body_of(&page);
        if body.contains("\"ts_ns\":1700000100000000000") {
            break body.to_string();
        }
        assert!(Instant::now() < deadline2, "fed tick not served: {page}");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        latest_body.contains("\"instrument\":7"),
        "the fed tick's fields survive: {latest_body}"
    );

    let _ = child.kill();
    let _ = child.wait();
    assert!(stats.contains("\"records\":5"), "seeded 4 + fed 1: {stats}");
}
