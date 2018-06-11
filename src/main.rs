extern crate futures;
extern crate hyper;
#[macro_use]
extern crate lazy_static;

use futures::future;
use hyper::rt::{Future, Stream};
use hyper::service::service_fn;
use hyper::{Body, Chunk, Request, Response, Server, StatusCode};
use std::io::{self, BufRead};
use std::sync::RwLock;
use std::thread;
use std::time::Duration;

lazy_static! {
    static ref CLIENTS: RwLock<Vec<hyper::body::Sender>> = RwLock::new(vec![]);
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
                .write()
                .expect("RwLock poisoned")
                .push(sender);

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

        let mut clients = CLIENTS.write().unwrap();
        for (i, client) in clients.iter_mut().enumerate() {
            let chunk = format!("event: update\ndata: {}\n\n", line);
            let result = client.send_data(Chunk::from(chunk));
            println!("  {}: {:?}", i, result);
        }
    }
}

fn heartbeat() {
    loop {
        thread::sleep(Duration::from_secs(10));
        let mut clients = CLIENTS.write().unwrap();
        for client in clients.iter_mut() {
            let chunk = format!(": heatbeat\n\n");
            client.send_data(Chunk::from(chunk)).ok();
        }
    }
}

fn main() {
    let addr = ([0, 0, 0, 0], 3000).into();

    let server = Server::bind(&addr)
        .serve(|| service_fn(sse))
        .map_err(|e| eprintln!("server error: {}", e));

    thread::spawn(terminal);
    thread::spawn(heartbeat);

    println!("Listening on http://{}", addr);
    hyper::rt::run(server);
}
