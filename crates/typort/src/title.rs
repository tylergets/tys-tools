//! The `<title>` of the page a port serves: a plain HTTP/1.0 GET with short timeouts,
//! following one redirect. Ports that don't speak HTTP just give no title.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use crate::ports::Port;

const TIMEOUT: Duration = Duration::from_millis(400);
/// Enough for the `<head>` of any page.
const MAX_BYTES: u64 = 64 * 1024;
const MAX_CHARS: usize = 80;
/// How long a fetched title is reused. The key includes the pids, so a restarted server is
/// fetched again right away.
const CACHE_TTL: Duration = Duration::from_secs(300);
/// How long "no title" is reused: the server may still be starting up.
const MISS_TTL: Duration = Duration::from_secs(10);

/// Remembers titles per port and processes, so the tray doesn't fetch every page on every
/// scan.
#[derive(Default)]
pub struct Cache {
    entries: HashMap<(u16, Vec<u32>), (Instant, Option<String>)>,
}

impl Cache {
    /// Set `title` on each port (skipping `https_ports`), fetching stale ones in parallel.
    pub fn fill(&mut self, ports: &mut [Port], https_ports: &[u16]) {
        let now = Instant::now();
        self.entries
            .retain(|_, (fetched, title)| {
                let ttl = if title.is_some() { CACHE_TTL } else { MISS_TTL };
                now.duration_since(*fetched) < ttl
            });

        let stale: Vec<(usize, SocketAddr)> = ports
            .iter()
            .enumerate()
            .filter(|(_, p)| !https_ports.contains(&p.port))
            .filter(|(_, p)| !self.entries.contains_key(&key(p)))
            .filter_map(|(i, p)| Some((i, connect_addr(p)?)))
            .collect();
        let fetched: Vec<(usize, Option<String>)> = thread::scope(|scope| {
            let handles: Vec<_> = stale
                .iter()
                .map(|&(i, addr)| scope.spawn(move || (i, fetch(addr))))
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        for (i, title) in fetched {
            self.entries.insert(key(&ports[i]), (now, title));
        }

        for port in ports.iter_mut() {
            port.title = self
                .entries
                .get(&key(port))
                .and_then(|(_, title)| title.clone());
        }
    }
}

fn key(port: &Port) -> (u16, Vec<u32>) {
    (port.port, port.pids())
}

/// Where to connect: loopback for wildcard binds, else the bound address.
fn connect_addr(port: &Port) -> Option<SocketAddr> {
    let ip = port
        .addrs
        .iter()
        .map(|addr| match addr {
            IpAddr::V4(a) if a.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(a) if a.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
            addr => *addr,
        })
        // Prefer IPv4 loopback, which every dev server answers on.
        .min_by_key(|addr| (!addr.is_loopback(), addr.is_ipv6()))?;
    Some(SocketAddr::new(ip, port.port))
}

fn fetch(addr: SocketAddr) -> Option<String> {
    let mut path = "/".to_string();
    for _ in 0..2 {
        let response = get(addr, &path)?;
        match redirect(&response) {
            Some(location) => path = location,
            None => return parse_title(&response),
        }
    }
    None
}

fn get(addr: SocketAddr, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    let host = match addr.ip() {
        IpAddr::V4(ip) if ip.is_loopback() => "localhost".to_string(),
        IpAddr::V6(ip) if ip.is_loopback() => "localhost".to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
        IpAddr::V4(ip) => ip.to_string(),
    };
    let request = format!(
        "GET {path} HTTP/1.0\r\nHost: {host}:{}\r\nUser-Agent: typort\r\n\
         Accept: text/html\r\nConnection: close\r\n\r\n",
        addr.port()
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut body = Vec::new();
    // A read timeout part way through still leaves what arrived.
    let _ = stream.take(MAX_BYTES).read_to_end(&mut body);
    let text = String::from_utf8_lossy(&body).into_owned();
    text.starts_with("HTTP/").then_some(text)
}

/// The path of a same-server redirect (`Location: /app/` or `http://localhost:5173/app/`).
fn redirect(response: &str) -> Option<String> {
    let status: u16 = response.split_whitespace().nth(1)?.parse().ok()?;
    if !(300..400).contains(&status) {
        return None;
    }
    let (head, _) = response.split_once("\r\n\r\n")?;
    let location = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("location")
            .then(|| value.trim())
    })?;
    if location.starts_with('/') && !location.starts_with("//") {
        return Some(location.to_string());
    }
    let rest = location.strip_prefix("http://")?;
    let host = rest.split('/').next()?;
    if !host.starts_with("localhost") && !host.starts_with("127.0.0.1") {
        return None;
    }
    Some(format!(
        "/{}",
        rest.split_once('/').map_or("", |(_, path)| path)
    ))
}

fn parse_title(response: &str) -> Option<String> {
    let lower = response.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    let title = decode_entities(&response[start..end]);
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return None;
    }
    Some(if title.chars().count() > MAX_CHARS {
        let cut: String = title.chars().take(MAX_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        title
    })
}

fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_titles() {
        let page =
            "HTTP/1.1 200 OK\r\n\r\n<html><head><TITLE data-x=1>\n  Vite &amp; React\n</title>";
        assert_eq!(parse_title(page).as_deref(), Some("Vite & React"));
        assert_eq!(parse_title("HTTP/1.1 200 OK\r\n\r\n<title></title>"), None);
        assert_eq!(parse_title("HTTP/1.1 200 OK\r\n\r\n{\"ok\":true}"), None);
    }

    #[test]
    fn follows_local_redirects_only() {
        let response =
            |location: &str| format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\n\r\n");
        assert_eq!(redirect(&response("/app/")).as_deref(), Some("/app/"));
        assert_eq!(
            redirect(&response("http://localhost:5173/login")).as_deref(),
            Some("/login")
        );
        assert_eq!(redirect(&response("https://example.com/")), None);
        assert_eq!(redirect(&response("//evil.test/")), None);
        assert_eq!(redirect("HTTP/1.1 200 OK\r\n\r\n"), None);
    }

    #[test]
    fn fetches_from_a_local_server() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for location in [Some("/home"), None] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let reply = match location {
                    Some(to) => format!("HTTP/1.0 301 Moved\r\nLocation: {to}\r\n\r\n"),
                    None => "HTTP/1.0 200 OK\r\n\r\n<title>My App</title>".into(),
                };
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });
        assert_eq!(fetch(addr).as_deref(), Some("My App"));
        server.join().unwrap();
    }
}
