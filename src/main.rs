extern crate futures;
extern crate hyper;
#[macro_use]
extern crate lazy_static;
extern crate tokio;

use futures::future;
use hyper::rt::{Future, Stream};
use hyper::service::service_fn;
use hyper::{Body, Chunk, Request, Response, Server, StatusCode};
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use tokio::timer::Interval;

struct Client {
    tx: hyper::body::Sender,
    id: usize,
}

impl Client {
    fn send_chunk(&mut self, chunk: Chunk) -> Result<(), Chunk> {
        self.tx.send_data(chunk)
    }
}

static CLIENT_ID: AtomicUsize = AtomicUsize::new(0);

lazy_static! {
    static ref CLIENTS: Mutex<Vec<Client>> = Mutex::new(vec![]);
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

                                newElement.innerHTML = event.data;
                                eventList.appendChild(newElement);
                            });
                        </script>
                    </body>".into())
                .expect("Invalid header specification")
        },
        "/events" => {
            let (sender, body) = Body::channel();

            CLIENTS
                .lock()
                .expect("RwLock poisoned")
                .push(Client {
                    tx: sender,
                    id: CLIENT_ID.fetch_add(1, Ordering::SeqCst),
                });

            Response::builder()
                .header("Cache-Control", "no-cache")
                .header("Content-Type", "text/event-stream")
                .body(body)
                .expect("Invalid header specification")
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
        } else if line == "drop" {
            CLIENTS.write().unwrap().clear();
            continue
        }

        println!("Sending '{}' to every listener:", line);

        let mut clients = CLIENTS.lock().unwrap();
        for client in clients.iter_mut() {
            let chunk = format!("event: update\ndata: {}\n\n", line);
            let result = client.send_chunk(Chunk::from(chunk));
            println!("  {}: {:?}", client.id, result.is_ok());
        }
    }
}

fn send_heartbeats(_instant: Instant) -> impl Future<Item = (), Error = tokio::timer::Error> {
    let mut clients = CLIENTS.lock().unwrap();

    for client in clients.iter_mut() {
        println!("Sending heartbeat to client {}", client.id);

        let chunk = Chunk::from(": heatbeat\n\n");
        client.send_chunk(chunk).ok();
    }

    future::ok(())
}

fn main() {
    let addr = ([0, 0, 0, 0], 3000).into();

    let server = Server::bind(&addr)
        .serve(|| service_fn(sse))
        .map_err(|e| eprintln!("server error: {}", e));

    let heartbeat = Interval::new(Instant::now(), Duration::from_secs(5))
        .for_each(send_heartbeats)
        .map_err(|e| eprintln!("timer error: {}", e));

    thread::spawn(terminal);

    println!("Listening on http://{}", addr);
    hyper::rt::run(
        server
        .join(heartbeat)
        .map(|_| ())
    );
}
