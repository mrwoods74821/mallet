//! Opening a map while Mallet already runs, as a double-click in Explorer does: the new
//! copy hands the map to the running one, which opens it in a tab and comes to the front,
//! and then exits instead of starting a second editor. This is what Hammer does too.
//!
//! The hand-off goes through the running copy's MCP server (`open_map` with `focus`), so
//! it needs no other channel, and copies with their own `MALLET_PROFILE` and port stay
//! apart.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

/// How long to wait for a running Mallet to answer at all.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
/// How long the running Mallet may take to open the map.
const OPEN_TIMEOUT: Duration = Duration::from_secs(180);

/// Hands `path` to a Mallet already running on `port`. Returns `true` when that Mallet
/// took the map, whether or not it could open it (it shows why in its status bar), so
/// this copy should exit.
pub fn hand_off(path: &Path, port: u16) -> bool {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) else {
        return false;
    };
    allow_foreground();
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "open_map",
            "arguments": { "path": path.display().to_string(), "focus": true },
        },
    })
    .to_string();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAccept: \
         application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let sent = stream.set_read_timeout(Some(OPEN_TIMEOUT)).is_ok() && stream.write_all(request.as_bytes()).is_ok();
    if !sent {
        return false;
    }
    let mut response = Vec::new();
    if stream.read_to_end(&mut response).is_err() && response.is_empty() {
        return false;
    }
    answered_by_mallet(&response)
}

/// Whether an HTTP response is a JSON-RPC result, which only an MCP server sends; anything
/// else on the port is not Mallet.
fn answered_by_mallet(response: &[u8]) -> bool {
    let text = String::from_utf8_lossy(response);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return false;
    }
    serde_json::from_str::<Value>(body.trim()).is_ok_and(|message| message.get("result").is_some())
}

/// Lets the running copy bring its window to the front. Windows only allows that to the
/// program the user just started, which is this one, so it passes the right on.
#[cfg(windows)]
fn allow_foreground() {
    #[link(name = "user32")]
    extern "system" {
        fn AllowSetForegroundWindow(process: u32) -> i32;
    }
    const ASFW_ANY: u32 = u32::MAX;
    // SAFETY: takes a process id and touches no memory.
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }
}

#[cfg(not(windows))]
fn allow_foreground() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_json_rpc_result_counts_as_mallet() {
        let ok =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}";
        assert!(answered_by_mallet(ok));
        assert!(!answered_by_mallet(b"HTTP/1.1 404 Not Found\r\n\r\nnot found"));
        assert!(!answered_by_mallet(b"HTTP/1.1 200 OK\r\n\r\n<html></html>"));
        assert!(!answered_by_mallet(b""));
    }

    #[test]
    fn nobody_listening_means_no_hand_off() {
        // Find a free port, then close it again.
        let port = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|listener| listener.local_addr())
            .map(|address| address.port())
            .unwrap();
        assert!(!hand_off(Path::new("map.mallet"), port));
    }
}
