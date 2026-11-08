mod support;
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn provider_starts_and_serves_without_python_or_path_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let port_file = tmp.path().join("port");
    let mut command = Command::new(support::binary("orbit-dev"));
    command
        .env_clear()
        .env("PATH", "")
        .env("FAKE_LOG", tmp.path().join("requests.jsonl"))
        .env("FAKE_PORT_FILE", &port_file)
        .args(["fake-openai-server", "0"])
        .stdout(Stdio::null());
    #[cfg(windows)]
    {
        if let Some(v) = std::env::var_os("SYSTEMROOT") {
            command.env("SYSTEMROOT", v);
        }
    }
    let mut child = Guard(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !port_file.exists() {
        assert!(Instant::now() < deadline, "provider startup timeout");
        assert!(child.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    let port = fs::read_to_string(port_file)
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "GET /v1/models HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"));
    assert!(reply.contains("fake-model"));
}
