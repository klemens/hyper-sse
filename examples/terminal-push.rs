extern crate hyper_sse;

extern crate futures;
extern crate hyper;
#[macro_use]
extern crate lazy_static;
extern crate tokio;

use futures::future;
use hyper::rt::{Future, Stream};
use hyper::service::service_fn;
use hyper::{Body,  Request, Response};
use hyper_sse::*;
use std::io::{self, BufRead};
use std::thread;
use std::time::{Duration, Instant};
use tokio::timer::Interval;

lazy_static! {
    static ref PUSH_SERVER: Server<u64> = Server::new();
}

fn sse(req: Request<Body>) -> future::Ok<Response<Body>, hyper::Error> {
    let response = match req.uri().path() {
        "/" => {
            let auth_cookie = PUSH_SERVER.generate_auth_cookie();

            Response::builder()
                .header("Set-Cookie", auth_cookie.to_string().as_str())
                .body("<html>
                    <head>
                        <title>EventSource Test</title>
                        <meta charset=\"utf-8\" />
                    </head>
                    <body>
                        <ol></ol>
                        <script>
                            var evtSource = new EventSource('http://10.0.11.10:3000/events/0');
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
        _ => {
            PUSH_SERVER.create_stream(&req)
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

        PUSH_SERVER.push(0, "update", &line).ok();
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
