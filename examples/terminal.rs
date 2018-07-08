extern crate hyper_sse;

#[macro_use]
extern crate lazy_static;

use hyper_sse::Server;
use std::io::{self, BufRead};

lazy_static! {
    static ref PUSH_SERVER: Server<u64> = Server::new();
}

fn main() {
    let addr = ("[::1]:3000").parse().unwrap();
    PUSH_SERVER.spawn(addr);

    let auth_token = PUSH_SERVER.generate_auth_token(Some(0)).unwrap()

    println!("Use the following command to connect to the SSE push server:");
    println!("  curl http://[::1]:3000/push/0?{}", auth_token);
    println!("Enter push message and press enter:");

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();

        PUSH_SERVER.push(0, "update", &line).ok();
    }
}
