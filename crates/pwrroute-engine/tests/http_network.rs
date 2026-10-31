//! Real HTTP on loopback, with no public service dependency.
#![cfg(feature = "http")]

use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use pwrroute_engine::{HttpNetwork, Link, ShortLinks};

struct Server {
    base: String,
    asked: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let asked = Arc::new(Mutex::new(Vec::new()));
        let (stop_server, requests) = (Arc::clone(&stop), Arc::clone(&asked));
        let thread = thread::spawn(move || {
            let mut clients = Vec::new();
            while !stop_server.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let requests = Arc::clone(&requests);
                        clients.push(thread::spawn(move || {
                            // Windows hands out accepted sockets non-blocking
                            // like the listener, so a request line that has not
                            // arrived yet reads as WouldBlock and the request
                            // is dropped unrecorded. Block, under the timeout.
                            stream.set_nonblocking(false).unwrap();
                            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                            let mut reader = BufReader::new(&mut stream);
                            let mut line = String::new();
                            if reader.read_line(&mut line).is_err() { return; }
                            let parts: Vec<_> = line.split_whitespace().collect();
                            if parts.len() < 2 { return; }
                            let (method, path) = (parts[0].to_owned(), parts[1].to_owned());
                            requests.lock().unwrap().push(format!("{method} {path}"));
                            loop {
                                let mut header = String::new();
                                if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" { break; }
                            }
                            let (status, location) = match path.as_str() {
                                "/head-get" if method == "HEAD" => (405, None),
                                "/head-get" => (302, Some("/landing".to_owned())),
                                "/redirect" => (302, Some("/landing".to_owned())),
                                "/error" => (503, None),
                                "/slow" => {
                                    thread::sleep(Duration::from_millis(1000));
                                    (200, None)
                                }
                                path if path.starts_with("/chain/") => {
                                    let n: u32 = path.trim_start_matches("/chain/").parse().unwrap();
                                    (302, Some(format!("/chain/{}", n + 1)))
                                }
                                _ => (200, None),
                            };
                            let location = location.map(|s| format!("Location: {s}\r\n")).unwrap_or_default();
                            // A GET body is deliberately advertised but never sent. Expansion
                            // needs only headers and must not wait for or store a page body.
                            let _ = write!(stream, "HTTP/1.1 {status} Test\r\n{location}Content-Length: 100000\r\nConnection: close\r\n\r\n");
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(e) => panic!("loopback accept failed: {e}"),
                }
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        Self {
            base,
            asked,
            stop,
            thread: Some(thread),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn real_http_preserves_location_and_retries_head_with_get_without_reading_bodies() {
    let server = Server::new();
    let network = HttpNetwork::new();
    let links = ShortLinks::only(["127.0.0.1"]);
    let start = server.url("/redirect");
    assert_eq!(
        links.expand(&start, &network),
        Link {
            url: server.url("/landing"),
            original_url: start,
        }
    );
    assert_eq!(server.asked(), ["HEAD /redirect", "HEAD /landing"]);
    let start = server.url("/head-get");
    assert_eq!(links.expand(&start, &network).url, server.url("/landing"));
    assert_eq!(
        &server.asked()[2..],
        ["HEAD /head-get", "GET /head-get", "GET /landing"]
    );
}

#[test]
fn real_http_stops_before_requesting_the_third_redirect_target_on_both_attempts() {
    let server = Server::new();
    let links = ShortLinks::only(["127.0.0.1"]);
    let start = server.url("/chain/0");
    assert_eq!(
        links.expand(&start, &HttpNetwork::new()).url,
        server.url("/chain/3")
    );
    assert_eq!(
        server.asked(),
        [
            "HEAD /chain/0",
            "HEAD /chain/1",
            "HEAD /chain/2",
            "GET /chain/0",
            "GET /chain/1",
            "GET /chain/2"
        ]
    );
}

#[test]
fn real_http_down_slow_and_offline_leave_the_short_link_usable() {
    let server = Server::new();
    let links = ShortLinks::only(["127.0.0.1"]);
    let network = HttpNetwork::new();
    let error = server.url("/error");
    assert_eq!(links.expand(&error, &network), Link::new(error));
    let slow = server.url("/slow");
    let started = Instant::now();
    assert_eq!(links.expand(&slow, &network), Link::new(slow));
    // Two independent 750 ms budgets; generous scheduling tolerance, still
    // HEAD and GET must both time out (a successful HEAD would skip GET).
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(
        server.asked(),
        ["HEAD /error", "GET /error", "HEAD /slow", "GET /slow"]
    );
    let offline = server.url("/offline");
    drop(server);
    assert_eq!(links.expand(&offline, &network), Link::new(offline));
}
