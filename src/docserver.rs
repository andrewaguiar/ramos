//! `ramos doc`'s static file server: serves an already-generated docs
//! directory (`doc::generate_with`'s output — `index.html`, `docs.json`,
//! `assets/`) over plain HTTP, so a browser can be pointed at it directly
//! instead of opening `index.html` off disk (which `fetch("docs.json")`
//! needs a real origin for — see the doc generator's own note on that).
//!
//! Hand-rolled, like the rest of this crate takes no dependencies: no real
//! HTTP server crate, just enough of the protocol to answer a `GET` for a
//! file that exists. There is no routing, no templating, nothing dynamic —
//! the same "no framework" spirit `HttpServer` (the *Ramos* stdlib module,
//! a different thing entirely) documents about itself.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};

/// Serve `root`'s files on `127.0.0.1:port`. Blocks forever, one thread per
/// connection — there is no clean shutdown short of killing the process
/// (Ctrl-C), which is the whole point of a throwaway local docs server.
pub fn serve(root: &Path, port: u16) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("cannot bind 127.0.0.1:{port}: {e}"))?;
    for incoming in listener.incoming() {
        let Ok(stream) = incoming else { continue };
        let root = root.to_path_buf();
        std::thread::spawn(move || handle_connection(stream, &root));
    }
    Ok(())
}

/// One connection, one request, no keep-alive — every response closes the
/// connection, matching `HttpServer`'s own "one request per connection"
/// rule, which is plenty for a docs site nobody is benchmarking.
fn handle_connection(mut stream: TcpStream, root: &Path) {
    let Some(head) = read_request_head(&mut stream) else {
        return;
    };
    let Some((method, target)) = first_line_of(&head).and_then(|l| parse_request_line(&l)) else {
        return;
    };
    let (status, content_type, body) = if method != "GET" {
        (
            405,
            "text/plain; charset=utf-8",
            b"method not allowed\n".to_vec(),
        )
    } else {
        let path_only = target.split('?').next().unwrap_or(&target);
        match resolve(root, path_only).and_then(|file| fs::read(&file).ok().map(|b| (file, b))) {
            Some((file, bytes)) => (200, content_type_for(&file), bytes),
            None => (404, "text/plain; charset=utf-8", b"not found\n".to_vec()),
        }
    };
    println!("{method} {target} -> {status}");
    // A write failure just means the client is already gone (closed the
    // connection early, most often) — nothing to do about it here.
    let _ = respond(&mut stream, status, content_type, &body);
}

/// Reads until the blank line that ends the request's headers
/// (`"\r\n\r\n"`), the same boundary `HttpServer` reads up to on the Ramos
/// side — this server has no use for a body (it only ever serves `GET`), so
/// nothing past the headers is read. `None` on a connection that closes, or
/// sends more than 64 KiB of headers, before that line ever shows up.
fn read_request_head(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            return Some(buf[..pos].to_vec());
        }
        if buf.len() > 64 * 1024 {
            return None;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    }
}

fn first_line_of(head: &[u8]) -> Option<String> {
    let end = head.iter().position(|&b| b == b'\n').unwrap_or(head.len());
    Some(
        String::from_utf8_lossy(&head[..end])
            .trim_end_matches('\r')
            .to_string(),
    )
}

fn parse_request_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.split(' ');
    Some((parts.next()?.to_string(), parts.next()?.to_string()))
}

/// `url_path` (already stripped of any `?query`) to a file under `root` — a
/// bare `/` is `index.html`, everything else is joined as-is. Every path
/// component must be a plain name: `..`, a bare `/` in the middle, or a
/// drive prefix all fail rather than being joined, which is what keeps a
/// crafted request inside `root` instead of walking out of it.
fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let rel = url_path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let rel_path = Path::new(rel);
    if rel_path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    Some(root.join(rel_path))
}

/// `path`'s extension to the `Content-Type` it is served as — just the
/// handful `doc::generate_with` actually writes (`html`, `json`, `css`,
/// `png`), plus a couple of common ones for good measure. Anything else is
/// sent as opaque bytes rather than guessed at.
fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("json") => "application/json",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}
