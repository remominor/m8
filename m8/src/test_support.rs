//! Loopback HTTP/game fixtures. No model servers, audio devices or extra dependencies.
use crate::config::Config;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

// Keep mocks independent of the user's deployment settings and private endpoints.
pub const CONFIG: &str = r#"
[mate]
name = "Kevin"
persona = "examples/kevin"
voice_server = "http://127.0.0.1:0"
voice = "soldier"
voice_sample_rate = 24000
voice_seed = 42
voice_guidance_scale = 4.0
[mic]
barge_in = true
mute_while_speaking = true
vad_threshold = 0.5
end_of_speech_silence = 0.1
save_recordings = false
[stt]
url = "http://127.0.0.1:0/v1/audio/transcriptions"
model = "parakeet"
[kev]
url = "http://127.0.0.1:0/v1/systemone"
model = "clef-flash-artemis"
[llm]
url = "http://127.0.0.1:0/v1/chat/completions"
model = "gemma4-12b-mtp-vision"
temperature = 1.0
[models]
silero_vad = "models/silero_vad.onnx"
[game]
port = 0
"#;

pub fn config() -> Config {
    toml::from_str(CONFIG).unwrap()
}

pub struct TestDir(pub PathBuf);
impl TestDir {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "m8-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}
impl Request {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

pub struct MockHttp {
    pub url: String,
    pub requests: mpsc::Receiver<Request>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl MockHttp {
    pub fn new(mut handler: impl FnMut(&Request, &mut TcpStream) + Send + 'static) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (tx, requests) = mpsc::channel();
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("mock accept: {e}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap().to_string();
                let path = parts.next().unwrap().to_string();
                let mut headers = HashMap::new();
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    let (key, value) = line.split_once(':').unwrap();
                    headers.insert(key.to_ascii_lowercase(), value.trim().to_string());
                }
                let len = headers
                    .get("content-length")
                    .map(|s| s.parse().unwrap())
                    .unwrap_or(0);
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let request = Request {
                    method,
                    path,
                    headers,
                    body,
                };
                tx.send(request.clone()).unwrap();
                handler(&request, &mut stream);
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    pub fn request(&self) -> Request {
        self.requests.recv_timeout(Duration::from_secs(3)).unwrap()
    }
}
impl Drop for MockHttp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let result = self.thread.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}

pub fn reply(stream: &mut TcpStream, status: u16, headers: &[(&str, &str)], body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 {status} Mock\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    )
    .unwrap();
    for (key, value) in headers {
        write!(stream, "{key}: {value}\r\n").unwrap();
    }
    stream.write_all(b"\r\n").unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
}

pub const PCM_HEADERS: &[(&str, &str)] =
    &[("X-Sample-Rate", "24000"), ("X-Sample-Format", "s16le")];

pub fn chunked_head(stream: &mut TcpStream) {
    stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nTransfer-Encoding: chunked\r\nX-Sample-Rate: 24000\r\nX-Sample-Format: s16le\r\n\r\n").unwrap();
}
pub fn chunk(stream: &mut TcpStream, bytes: &[u8]) {
    write!(stream, "{:x}\r\n", bytes.len()).unwrap();
    stream.write_all(bytes).unwrap();
    stream.write_all(b"\r\n").unwrap();
    stream.flush().unwrap();
}

pub fn frame(peer: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut header = [0; 5];
    peer.read_exact(&mut header).unwrap();
    let mut body = vec![0; u32::from_be_bytes(header[1..].try_into().unwrap()) as usize];
    peer.read_exact(&mut body).unwrap();
    (header[0], body)
}
