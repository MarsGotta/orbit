//! A fake Sentry on 127.0.0.1 that keeps every request the real SDK sends
//! to it. Nothing that uses it ever reaches sentry.io.
#![allow(dead_code)]
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

pub struct FakeSentry {
    port: u16,
    received: Arc<Mutex<Vec<String>>>,
}

impl FakeSentry {
    /// Listens on an ephemeral port of the loopback until the process ends.
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let received = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&received);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let kept = Arc::clone(&kept);
                std::thread::spawn(move || {
                    let _ = answer(stream, &kept);
                });
            }
        });
        Ok(Self { port, received })
    }

    /// The DSN of project 1 with key `public`, plain HTTP on the loopback.
    pub fn dsn(&self) -> String {
        format!("http://public@127.0.0.1:{}/1", self.port)
    }

    /// Every request received so far, headers and body, as text.
    pub fn requests(&self) -> Vec<String> {
        self.received
            .lock()
            .map(|received| received.clone())
            .unwrap_or_default()
    }

    /// The JSON items of every envelope received (headers lines included).
    pub fn items(&self) -> Vec<serde_json::Value> {
        self.requests()
            .iter()
            .filter_map(|request| {
                request
                    .split_once("\r\n\r\n")
                    .map(|(_, body)| body.to_owned())
            })
            .flat_map(|body| {
                body.lines()
                    .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The events received, in order: each item whose header says `event`.
    pub fn events(&self) -> Vec<serde_json::Value> {
        let items = self.items();
        items
            .windows(2)
            .filter(|pair| pair[0].get("type").and_then(|kind| kind.as_str()) == Some("event"))
            .map(|pair| pair[1].clone())
            .collect()
    }
}

/// Reads one request (headers and a `Content-Length` body), keeps it, and
/// answers 200 before closing.
fn answer(stream: TcpStream, kept: &Mutex<Vec<String>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut head = String::new();
    let mut length = 0_usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
        let end = line == "\r\n";
        head.push_str(&line);
        if end {
            break;
        }
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body)?;
    head.push_str(&String::from_utf8_lossy(&body));
    if let Ok(mut received) = kept.lock() {
        received.push(head);
    }
    let mut stream = stream;
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")?;
    stream.flush()
}
