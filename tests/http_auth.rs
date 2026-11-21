//! The pieces of authentication that only a real socket can show.
//!
//! `tests/api_shapes.rs` proves the client asks the transport to install a
//! token. It cannot show that the token becomes an `Authorization` header,
//! because that header is added inside `HttpTransport` from the token it holds.
//! So this starts a one-shot HTTP server on a loopback port, points a real
//! `HttpTransport` at it, and reads the request the transport actually sent.
//!
//! Nothing here touches the network: the listener is on 127.0.0.1, and it is
//! not reachable from outside the process.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use limonada::api::{ApiError, HttpTransport, Transport};

/// Start a one-shot server that answers the next request with `status` and
/// `body`, and reports the request head (request line plus headers) back.
///
/// Returns the base URL to point a transport at and the channel the head
/// arrives on, so the test can assert on what was sent rather than only that
/// something was.
fn serve_once(status: u16, body: &'static str) -> (String, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let address = listener.local_addr().expect("local address");
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept one connection");
        let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));

        // Read the request line and headers. The body is deliberately not
        // asserted on here; it is what `api_shapes.rs` covers.
        let mut head = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            head.push_str(&line);
        }

        // Drain the body if one was announced, so the client is not left writing
        // into a closed socket.
        let length = head
            .lines()
            .filter_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::to_string)
            })
            .next()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if length > 0 {
            let mut sink = vec![0u8; length];
            let _ = reader.read_exact(&mut sink);
        }

        let reason = match status {
            200 => "OK",
            401 => "Unauthorized",
            _ => "Error",
        };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();

        let _ = tx.send(head);
    });

    (format!("http://{address}"), rx)
}

/// The transport sends the installed token as `Authorization: Bearer <token>`.
///
/// This is the mechanism the whole signed-in path rests on: a live lemmy.ml
/// 0.19.20 reads the token from that header, and answers an unauthenticated
/// `post/like` with HTTP 401 `{"error":"incorrect_login"}`.
#[test]
fn the_session_token_is_sent_as_a_bearer_header() {
    let (base, head) = serve_once(200, r#"{"success":true}"#);
    let transport = HttpTransport::new(&base).expect("a loopback transport");

    transport.set_auth(Some("a-session-token".into()));
    transport
        .get("/api/v3/user/validate_auth", &[])
        .expect("the request should be answered");

    let head = head
        .recv_timeout(Duration::from_secs(5))
        .expect("the server should have seen a request");
    assert!(
        head.starts_with("GET /api/v3/user/validate_auth"),
        "unexpected request line: {head:?}"
    );
    assert!(
        head.to_ascii_lowercase()
            .contains("authorization: bearer a-session-token"),
        "the token must travel as a Bearer header; the request was: {head:?}"
    );
}

/// With no token installed, no `Authorization` header is sent at all.
///
/// Not merely cosmetic: an empty Bearer header is rejected by the server, so an
/// anonymous browse must be genuinely anonymous.
#[test]
fn an_anonymous_request_sends_no_authorization_header() {
    let (base, head) = serve_once(200, r#"{"site_view":{"site":{"id":1,"name":"x"}}}"#);
    let transport = HttpTransport::new(&base).expect("a loopback transport");

    transport
        .get("/api/v3/site", &[])
        .expect("the request should be answered");

    let head = head
        .recv_timeout(Duration::from_secs(5))
        .expect("the server should have seen a request");
    assert!(
        !head.to_ascii_lowercase().contains("authorization"),
        "an anonymous request must not carry a token: {head:?}"
    );
}

/// Signing out clears the header too.
#[test]
fn clearing_the_token_stops_sending_the_header() {
    let (base, head) = serve_once(200, r#"{"site_view":{"site":{"id":1,"name":"x"}}}"#);
    let transport = HttpTransport::new(&base).expect("a loopback transport");

    transport.set_auth(Some("a-session-token".into()));
    transport.set_auth(None);
    assert_eq!(transport.auth(), None);

    transport.get("/api/v3/site", &[]).expect("answered");
    let head = head
        .recv_timeout(Duration::from_secs(5))
        .expect("a request");
    assert!(
        !head.to_ascii_lowercase().contains("authorization"),
        "a cleared token must not still be sent: {head:?}"
    );
}

/// HTTP 401 becomes [`ApiError::Unauthorized`], not a bare [`ApiError::Status`].
///
/// The distinction is what lets the interface offer "sign in again" instead of
/// "retry", and what the app matches on to forget a token the instance has
/// stopped accepting.
#[test]
fn a_401_is_reported_as_unauthorized() {
    let (base, _head) = serve_once(401, r#"{"error":"incorrect_login"}"#);
    let transport = HttpTransport::new(&base).expect("a loopback transport");

    let err = transport
        .post(
            "/api/v3/post/like",
            &serde_json::json!({"post_id": 1, "score": 1}),
        )
        .expect_err("a 401 must fail");

    match err {
        ApiError::Unauthorized { detail } => assert!(
            detail.contains("incorrect_login"),
            "the server's own reason should be kept; got {detail:?}"
        ),
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}
