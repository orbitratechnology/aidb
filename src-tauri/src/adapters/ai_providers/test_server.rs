//! A one-request-per-connection HTTP/1.1 server for adapter tests: it records each request and
//! answers with a scripted status, headers and body chunks (sent with chunked encoding, so
//! streams arrive in pieces as they would from a provider).

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use url::Url;

#[derive(Debug, Clone)]
pub struct Captured {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Captured {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("request body is JSON")
    }
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub chunks: Vec<Vec<u8>>,
}

impl Reply {
    /// A server-sent event stream, one chunk per event.
    pub fn sse(events: &[&str]) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            chunks: events
                .iter()
                .map(|e| format!("{e}\n\n").into_bytes())
                .collect(),
        }
    }

    pub fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            chunks: vec![body.as_bytes().to_vec()],
        }
    }

    /// Reads the request, then closes the connection without answering.
    pub fn hang_up() -> Self {
        Self {
            status: 0,
            headers: Vec::new(),
            chunks: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

pub struct TestServer {
    pub url: Url,
    requests: mpsc::UnboundedReceiver<Captured>,
}

impl TestServer {
    /// Serves `replies` in order, one per connection.
    pub async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, requests) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            for reply in replies {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let Some(captured) = read_request(&mut socket).await else {
                    return;
                };
                let _ = sender.send(captured);
                if reply.status == 0 {
                    drop(socket);
                    continue;
                }
                let mut head = format!("HTTP/1.1 {} Test\r\n", reply.status);
                for (name, value) in &reply.headers {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                head.push_str("transfer-encoding: chunked\r\nconnection: close\r\n\r\n");
                if socket.write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                for chunk in &reply.chunks {
                    let framed = [
                        format!("{:x}\r\n", chunk.len()).into_bytes(),
                        chunk.clone(),
                        b"\r\n".to_vec(),
                    ]
                    .concat();
                    if socket.write_all(&framed).await.is_err() {
                        break;
                    }
                    let _ = socket.flush().await;
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
                let _ = socket.shutdown().await;
            }
        });
        Self {
            url: Url::parse(&format!("http://{address}/v1")).expect("url"),
            requests,
        }
    }

    /// The next request the server received.
    pub async fn request(&mut self) -> Captured {
        self.requests.recv().await.expect("a request arrived")
    }
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<Captured> {
    let mut buffer = Vec::new();
    let mut piece = [0_u8; 4096];
    let head_end = loop {
        let read = socket.read(&mut piece).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&piece[..read]);
        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_owned();
    let path = request_line.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[head_end..].to_vec();
    while body.len() < length {
        let read = socket.read(&mut piece).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&piece[..read]);
    }
    Some(Captured {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}
