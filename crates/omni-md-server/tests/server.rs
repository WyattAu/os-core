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
    Command::new(env!("CARGO_BIN_EXE_omni-md-server"))
        .env("OMNI_PORT", port.to_string())
        .env("OMNI_TICKS", ticks.to_string())
        .spawn()
        .ok()
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
