#![feature(assoc_unix_epoch)]

extern crate cookie;
extern crate failure;
extern crate hyper;
extern crate serde;
extern crate serde_json;

use cookie::{Cookie, CookieJar, Key};
use failure::Error;
use hyper::header::COOKIE;
use hyper::{Body, Chunk, Request, Response, StatusCode};
use serde::Serialize;
use std::collections::HashMap;
use std::hash::Hash;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

const COOKIE_NAME: &'static str = "sse-authorization";

type Clients = Vec<Client>;
type Channels<C> = HashMap<C, Clients>;

/// Push server implementing Server-Sent Events (SSE)
///
/// Because the Server implements `Sync`, it can be stored in
/// a static variable using e.g. `lazy_static`.
pub struct Server<C> {
    channels: Mutex<Channels<C>>,
    next_id: AtomicUsize,
    cookie_key: Key,
}

impl<C: Hash + Eq + FromStr> Server<C> {
    /// Create a new SSE push-server.
    pub fn new() -> Server<C> {
        Server {
            channels: Mutex::new(HashMap::new()),
            next_id: AtomicUsize::new(0),
            cookie_key: Key::generate(),
        }
    }

    /// Push a message for the event to all clients registered on the channel.
    ///
    /// The message is first serialized and then send to all registered
    /// clients on the given channel, if any.
    ///
    /// Returns an error if the serialization fails.
    pub fn push<S: Serialize>(&self, channel: C, event: &str, message: &S) -> Result<(), Error> {
        let payload = serde_json::to_string(message)?;
        let message = format!("event: {}\ndata: {}\n\n", event, payload);

        self.send_chunk_to_channel(message, channel)?;

        Ok(())
    }

    /// Initiate a new SSE stream for the given request.
    ///
    /// The request must include a valid authorization token cookie. The
    /// channel is parsed from the last segment of the uri path. If the
    /// request cannot be parsed correctly of the auth token is expired,
    /// an appropriate http error response is returned.
    pub fn create_stream(&self, request: &Request<Body>) -> Response<Body> {
        // Extract channel from uri path (last segment)
        let path = request.uri().path();
        let channel = match path.rsplit('/').next().map(FromStr::from_str) {
            Some(Ok(channel)) => channel,
            _ => {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::empty())
                    .expect("Could not create response");
            }
        };

        // Parse request cookies and load into a cookie jar, which is
        // used for the decrytion of the cookie
        let mut cookies = request.headers()
            .get_all(COOKIE).iter()
            .filter_map(|cookies| cookies.to_str().ok())
            .flat_map(|cookies| cookies.split(';'))
            .filter(|cookie| cookie.starts_with(COOKIE_NAME))
            .map(|cookies| cookies.to_string()) // Cookies requires 'static
            .filter_map(|cookie| Cookie::parse(cookie).ok())
            .fold(CookieJar::new(), |mut jar, cookie| {
                jar.add_original(cookie);
                jar
            });

        // Decrypt auth token, parse the unix timestamp, and calculate
        // the elapsed time since its creation
        let token_time = cookies.private(&self.cookie_key)
            .get(COOKIE_NAME)
            .and_then(|cookie| cookie.value().parse().ok())
            .map(|value| SystemTime::UNIX_EPOCH + Duration::from_secs(value))
            .map(|token_time| SystemTime::now().duration_since(token_time));

        // Check if client is authorized (has a valid auth token
        // not older than 24 hours)
        let authorized = match token_time {
            Some(Ok(duration)) => duration.as_secs() < 24 * 60 * 60,
            Some(Err(_)) => true, // Token is in the future (time shift)
            None => false, // No auth cookie found
        };
        if !authorized {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("Could not create response");
        }

        let (sender, body) = Body::channel();
        self.add_client(channel, sender);

        Response::builder()
            .header("Cache-Control", "no-cache")
            .header("Content-Type", "text/event-stream")
            .body(body)
            .expect("Could not create response")
    }

    /// Create a cookie with an authorization token that will be checked
    /// in `register_client` before establishing the SSE stream.
    ///
    /// A new token can be send to the client on every request, as
    /// creating and checking the tokens is cheap. The token is valid
    /// for 24 hours after it has been generated.
    pub fn generate_auth_cookie(&self) -> Cookie {
        let unix_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("System time before unix epoch");
        let cookie_value = unix_time.as_secs().to_string();

        // Create private (encrypted and authenticated) cookie
        // with the current unix timestamp as its value using
        // the CookieJar
        let mut jar = CookieJar::new();
        jar.private(&self.cookie_key)
            .add(Cookie::new(COOKIE_NAME, cookie_value));

        jar.get(COOKIE_NAME)
            .expect("Cookie was just inserted")
            .clone() // TODO: can this clone be avoided?
    }

    /// Send hearbeat to all clients on all channels.
    ///
    /// This should be called regularly (e.g. every minute) to detect
    /// a disconnect of the underlying TCP connection.
    pub fn send_heartbeats(&self) {
        self.send_chunk_to_all_clients(":\n\n".into());
    }

    /// Remove disconnected clients.
    ///
    /// This removes all clients from all channels that have closed the
    /// conenction or are not responding to the heartbeats, which caused
    /// a TPC timeout.
    ///
    /// This function should be called regularly (e.g. together with
    /// `send_heartbeats`) to keep the memory usage low.
    pub fn remove_stale_clients(&self) {
        let mut channels = self.channels.lock().unwrap();

        channels.retain(|_, clients| {
            clients.retain(|client| {
                if let Some(first_error) = client.first_error {
                    if first_error.elapsed() > Duration::from_secs(5) {
                        println!("Removing stale client {}", client.id);
                        return false;
                    }
                }
                true
            });

            !clients.is_empty()
        });
    }

    fn add_client(&self, channel: C, sender: hyper::body::Sender) {
        self.channels
            .lock().unwrap()
            .entry(channel)
            .or_insert_with(Default::default)
            .push(Client {
                tx: sender,
                id: self.next_id.fetch_add(1, Ordering::SeqCst),
                first_error: None,
            });
    }

    fn send_chunk_to_channel(&self, chunk: String, channel: C) -> Result<(), Error> {
        let mut channels = self.channels.lock().unwrap();

        match channels.get_mut(&channel) {
            Some(clients) => {
                for client in clients.iter_mut() {
                    let chunk = Chunk::from(chunk.clone());
                    let result = client.send_chunk(chunk);
                    println!("  {}: {:?}", client.id, result.is_ok());
                }
            }
            None => {} // Currently no clients on the given channel
        }

        Ok(())
    }

    fn send_chunk_to_all_clients(&self, chunk: String) {
        let mut channels = self.channels.lock().unwrap();

        for client in channels.values_mut().flat_map(IntoIterator::into_iter) {
            let chunk = Chunk::from(chunk.clone());
            client.send_chunk(chunk).ok();
        }
    }
}

#[derive(Debug)]
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