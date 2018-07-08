extern crate hyper_sse;

extern crate futures;
extern crate hyper;
#[macro_use]
extern crate lazy_static;
extern crate tokio;

use hyper::{Body, Request, Response};
use hyper_sse::Server;
use std::io::{self, BufRead};

lazy_static! {
    static ref PUSH_SERVER: Server<u8> = Server::new();
}

const HTML: &'static str = "\
<html>
<head>
    <title>hyper_sse demo</title>
    <meta charset=\"utf-8\" />
</head>
<body>
    <ol></ol>
    <script>
        var evtSource = new EventSource('http://[::1]:3000/push/0?%auth_token%');
        evtSource.addEventListener('update', event => {
            var newElement = document.createElement('li');
            var eventList = document.querySelector('ol');

            newElement.innerHTML = JSON.parse(event.data);
            eventList.appendChild(newElement);
        });
    </script>
</body>";

fn service_handler(req: Request<Body>) -> Response<Body> {
    match req.uri().path() {
        "/" => {
            let auth_cookie = PUSH_SERVER.generate_auth_token(Some(0)).unwrap();

            Response::builder()
                .body(HTML.replace("%auth_token%", &auth_cookie).into())
                .expect("Invalid header specification")
        },
        _ => {
            PUSH_SERVER.create_stream(&req)
        }
    }
}

fn spawn_server(server: &'static Server<u8>) {
    use futures::future;
    use hyper::rt::{Future, Stream};
    use hyper::service::service_fn_ok;
    use std::thread;
    use std::time::{Duration, Instant};
    use tokio::timer::Interval;

    let addr = ("[::1]:3000").parse().unwrap();

    let http_server = hyper::Server::bind(&addr)
        .serve(move || service_fn_ok(service_handler))
        .map_err(|e| panic!("Push server failed: {}", e));

    let maintenance = Interval::new(Instant::now(), Duration::from_secs(45))
        .for_each(move |_| {
            server.remove_stale_clients();
            server.send_heartbeats();
            future::ok(())
        })
        .map_err(|e| panic!("Push maintenance failed: {}", e));

    thread::spawn(move || {
        hyper::rt::run(
            http_server
            .join(maintenance)
            .map(|_| ())
        );
    });
}

fn main() {
    spawn_server(&PUSH_SERVER);

    println!("Open to following page in your browser: http://[::1]:3000/");
    println!("Enter push message and press enter:");

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();

        PUSH_SERVER.push(0, "update", &line).ok();
    }
}
