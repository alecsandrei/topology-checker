#![cfg(unix)]

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    process::{Command, Stdio},
};

#[test]
fn serves_on_the_requested_port_and_shuts_down_on_interrupt() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_topology-checker-web"))
        .args(["--port", "0"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut banner = String::new();
    stdout.read_line(&mut banner).unwrap();
    let port: u16 = banner
        .trim()
        .rsplit(':')
        .next()
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("unexpected banner {banner:?}"));
    assert_ne!(port, 0);

    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .write_all(
            b"GET /assets/missing.js HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 404"), "{response}");

    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(child.wait().unwrap().success());
}
