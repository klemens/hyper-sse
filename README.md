# hyper-sse

[![crates.io version](https://img.shields.io/crates/v/hyper-sse.svg)](https://crates.io/crates/hyper-sse)
[![docs.rs version](https://docs.rs/hyper-sse/badge.svg)](https://docs.rs/hyper-sse)

Simple Server-Sent Events (SSE) library for hyper and Rocket. Requires
a nightly compiler until Rust 1.28 is stable.

## Example

```toml
[dependencies]
hyper-sse = "0.1"
lazy_static = "1"
```

```rust
extern crate hyper_sse;
#[macro_use] extern crate lazy_static;

use hyper_sse::Server;
use std::io::BufRead;

lazy_static! {
    static ref SSE: Server<u8> = Server::new();
}

fn main() {
    SSE.spawn("[::1]:3000".parse().unwrap());

    // Use SSE.generate_auth_cookie() to generate auth tokens

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();

        SSE.push(0, "update", &line).ok();
    }
}
```

## License

Licensed under either of

 * Apache License, Version 2.0, (http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license (http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
