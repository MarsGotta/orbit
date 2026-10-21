//! Socket admission remains independent of topic subscription capacity.
use super::*;
use orbit_core::{
    PROTOCOL_VERSION,
    protocol::{Command as ProtocolCommand, Request, Response},
};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
};

fn request_id(number: usize) -> String {
    format!("00000000-0000-4000-8000-{number:012x}")
}

fn wire_request(id: String, body: ProtocolCommand) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&Request {
        v: PROTOCOL_VERSION,
        id,
        op: None,
        body,
    })
    .unwrap();
    bytes.push(b'\n');
    bytes
}

#[test]
fn sixty_fifth_simultaneous_connection_receives_busy_response() {
    let directory = tempfile::Builder::new()
        .prefix("orbit-ipc-busy-")
        .tempdir_in("/tmp")
        .unwrap();
    let _daemon = start(directory.path());
    let mut occupied = Vec::new();
    for _ in 0..64 {
        let mut stream = UnixStream::connect(directory.path().join("orbit.sock")).unwrap();
        stream.write_all(b"{").unwrap();
        occupied.push(stream);
    }
    thread::sleep(Duration::from_millis(250));

    let expected_id = request_id(900);
    let mut extra = UnixStream::connect(directory.path().join("orbit.sock")).unwrap();
    extra
        .write_all(&wire_request(expected_id.clone(), ProtocolCommand::Status))
        .unwrap();
    extra
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    let mut line = String::new();
    assert!(BufReader::new(extra).read_line(&mut line).unwrap() > 0);
    let response: Response = serde_json::from_str(&line).unwrap();

    assert_eq!(response.id, expected_id);
    assert!(!response.ok);
    assert_eq!(
        response.refusal,
        Some(orbit_core::refusal::Refusal::TooMany {
            resource: orbit_core::refusal::Resource::IpcRequest,
            count: 65,
            max: 64,
        })
    );
    assert_eq!(
        response.error.as_deref(),
        response
            .refusal
            .as_ref()
            .map(ToString::to_string)
            .as_deref()
    );
    assert!(response.status.is_none());
    drop(occupied);
}
