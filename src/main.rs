#[macro_use]
extern crate failure;
extern crate futures;
extern crate hyper;
#[macro_use]
extern crate lazy_static;
extern crate serde;
extern crate serde_json;
extern crate tokio;

use failure::Error;
use futures::future;
use hyper::rt::{Future, Stream};
use hyper::service::service_fn;
use hyper::{Body, Chunk, Request, Response, StatusCode};
use serde::Serialize;
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use tokio::timer::Interval;

struct Server {
    clients: Mutex<Vec<Client>>,
    next_id: AtomicUsize,
}

impl Server {
    pub fn new() -> Server {
        Server {
            clients: Mutex::new(vec![]),
            next_id: AtomicUsize::new(0),
        }
    }

    pub fn push<S: Serialize>(&self, event: &str, message: &S) -> Result<(), Error> {
        let payload = serde_json::to_string(message)?;
        let message = format!("event: {}\ndata: {}\n\n", event, payload);

        self.send_chunk_to_all_clients(message);

        Ok(())
    }

    pub fn send_heartbeats(&self) {
        self.send_chunk_to_all_clients(":\n\n".into());
    }

    pub fn add_client(&self, sender: hyper::body::Sender) {
        self.clients
            .lock().unwrap()
            .push(Client {
                tx: sender,
                id: self.next_id.fetch_add(1, Ordering::SeqCst),
                first_error: None,
            });
    }

    pub fn remove_stale_clients(&self) {
        let mut clients = self.clients.lock().unwrap();

        clients.retain(|client| {
            if let Some(first_error) = client.first_error {
                if first_error.elapsed() > Duration::from_secs(5) {
                    println!("Removing stale client {}", client.id);
                    return false;
                }
            }
            true
        });
    }

    fn send_chunk_to_all_clients(&self, chunk: String) {
        let mut clients = self.clients.lock().unwrap();
        for client in clients.iter_mut() {
            let result = client.send_chunk(Chunk::from(chunk.clone()));
            println!("  {}: {:?}", client.id, result.is_ok());
        }
    }
}

struct Client {
    tx: hyper::body::Sender,
    id: usize,
    first_error: Option<Instant>,
}

impl Client {
    fn send_chunk(&mut self, chunk: Chunk) -> Result<(), Chunk> {
        let result = self.tx.send_data(chunk);

        match (&result, self.first_error) {
            (Err(_), None) => {
                // Store time when an error was first seen
                self.first_error = Some(Instant::now());
            }
            (Ok(_), Some(_)) => {
                // Clear error when write succeeds
                self.first_error = None;
            }
            _ => {}
        }

        result
    }
}

lazy_static! {
    static ref PUSH_SERVER: Server = Server::new();
}

fn sse(req: Request<Body>) -> future::Ok<Response<Body>, hyper::Error> {
    let response = match req.uri().path() {
        "/" => {
            Response::builder()
                .body("<html>
                    <head>
                        <title>EventSource Test</title>
                        <meta charset=\"utf-8\" />
                    </head>
                    <body>
                        <ol></ol>
                        <script>
                            var evtSource = new EventSource('http://10.0.11.10:3000/events');
                            evtSource.addEventListener('update', event => {
                                var newElement = document.createElement('li');
                                var eventList = document.querySelector('ol');

                                newElement.innerHTML = JSON.parse(event.data);
                                eventList.appendChild(newElement);
                            });
                        </script>
                    </body>".into())
                .expect("Invalid header specification")
        },
        "/events" => {
            let (sender, body) = Body::channel();

            PUSH_SERVER.add_client(sender);

            Response::builder()
                .header("Cache-Control", "no-cache")
                .header("Content-Type", "text/event-stream")
                .body(body)
                .expect("Invalid SSE header specification")
        },
        _ => {
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body("Not found!".into())
                .unwrap()
        }
    };

    future::ok(response)
}

fn terminal() {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.expect("line decoding error");

        if line.is_empty() {
            continue
        }

        println!("Sending '{}' to every listener:", line);

        PUSH_SERVER.push("update", &line).unwrap();
    }
}

fn main() {
    let addr = ([0, 0, 0, 0], 3000).into();

    let server = hyper::Server::bind(&addr)
        .serve(|| service_fn(sse))
        .map_err(|e| eprintln!("server error: {}", e));

    let push_maintenance = Interval::new(Instant::now(), Duration::from_secs(5))
        .for_each(|_| {
            PUSH_SERVER.remove_stale_clients();
            PUSH_SERVER.send_heartbeats();
            future::ok(())
        })
        .map_err(|e| eprintln!("timer error: {}", e));

    thread::spawn(terminal);

    println!("Listening on http://{}", addr);
    hyper::rt::run(
        server
        .join(push_maintenance)
        .map(|_| ())
    );
}
