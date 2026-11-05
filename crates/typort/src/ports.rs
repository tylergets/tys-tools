//! The TCP ports the current user is listening on, read from `/proc`: sockets in the LISTEN
//! state from `/proc/net/tcp{,6}`, matched to processes through `/proc/<pid>/fd`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;

/// `TCP_LISTEN` in `/proc/net/tcp`.
const LISTEN: &str = "0A";
/// How long [`Scanner`] trusts its socket owners while the listening sockets stay the same
/// (a process can still change its command line or working directory).
const OWNERS_TTL: Duration = Duration::from_secs(60);

/// A listening socket from `/proc/net/tcp{,6}`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Socket {
    addr: IpAddr,
    port: u16,
    inode: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Process {
    pub pid: u32,
    /// Short name: the script for interpreters (`vite`, `next`, `http.server`), else the program.
    pub name: String,
    pub command: Vec<String>,
    pub cwd: Option<PathBuf>,
}

impl Process {
    /// The working directory's name, unless it is `/` or the home directory.
    pub fn project(&self) -> Option<String> {
        let cwd = self.cwd.as_ref()?;
        if dirs_home().as_deref() == Some(cwd.as_path()) {
            return None;
        }
        cwd.file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }

    pub fn command_line(&self) -> String {
        self.command.join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Port {
    pub port: u16,
    /// Addresses it listens on (IPv4 and IPv6), sorted.
    pub addrs: Vec<IpAddr>,
    /// Processes holding the socket (several for forking servers). Empty if they couldn't be
    /// found, e.g. in another PID namespace.
    pub processes: Vec<Process>,
    /// The `<title>` of the page it serves, when it speaks HTTP (see [`crate::title`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl Port {
    /// The main process's name, or `unknown` if it is hidden (e.g. a hardened service).
    pub fn name(&self) -> &str {
        self.processes
            .first()
            .map_or("unknown", |p| p.name.as_str())
    }

    pub fn project(&self) -> Option<String> {
        self.processes.first().and_then(Process::project)
    }

    pub fn pids(&self) -> Vec<u32> {
        self.processes.iter().map(|p| p.pid).collect()
    }

    /// `vite · myapp`
    pub fn label(&self) -> String {
        match self.project() {
            Some(project) => format!("{} · {project}", self.name()),
            None => self.name().to_string(),
        }
    }

    /// The host to browse to: `localhost` when it listens on loopback or every interface,
    /// else its (first) address.
    pub fn host(&self) -> String {
        if self
            .addrs
            .iter()
            .any(|a| a.is_loopback() || a.is_unspecified())
        {
            return "localhost".into();
        }
        match self.addrs.first() {
            Some(IpAddr::V6(addr)) => format!("[{addr}]"),
            Some(addr) => addr.to_string(),
            None => "localhost".into(),
        }
    }

    pub fn url(&self, https: bool) -> String {
        let scheme = if https { "https" } else { "http" };
        format!("{scheme}://{}:{}/", self.host(), self.port)
    }

    /// Who can connect: `localhost only`, `all interfaces`, or the addresses.
    pub fn reach(&self) -> String {
        if self.addrs.iter().any(IpAddr::is_unspecified) {
            "all interfaces".into()
        } else if self.addrs.iter().all(IpAddr::is_loopback) {
            "localhost only".into()
        } else {
            let addrs: Vec<String> = self.addrs.iter().map(IpAddr::to_string).collect();
            addrs.join(", ")
        }
    }
}

/// Every port the current user listens on, sorted by port.
pub fn scan() -> Result<Vec<Port>> {
    Scanner::default().scan()
}

/// Scans repeatedly. Finding a socket's processes walks every process's open files, so
/// that is only redone when the listening sockets change (or after [`OWNERS_TTL`]); the
/// socket tables themselves are cheap to read.
#[derive(Default)]
pub struct Scanner {
    inodes: HashSet<u64>,
    owners: HashMap<u64, Vec<Process>>,
    checked: Option<Instant>,
}

impl Scanner {
    pub fn scan(&mut self) -> Result<Vec<Port>> {
        let uid = fs::metadata("/proc/self")
            .context("failed to read /proc/self")?
            .uid();
        let mut sockets = Vec::new();
        for (path, v6) in [("/proc/net/tcp", false), ("/proc/net/tcp6", true)] {
            match fs::read_to_string(path) {
                Ok(text) => sockets.extend(parse_net_tcp(&text, v6, uid)),
                // No IPv6 is fine; no IPv4 table means no /proc.
                Err(_) if v6 => {}
                Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
            }
        }
        let inodes: HashSet<u64> = sockets.iter().map(|s| s.inode).collect();
        let fresh = self
            .checked
            .is_some_and(|checked| checked.elapsed() < OWNERS_TTL);
        if !fresh || inodes != self.inodes {
            self.owners = socket_owners(uid, &inodes);
            self.inodes = inodes;
            self.checked = Some(Instant::now());
        }
        Ok(build_ports(sockets, &self.owners))
    }
}

fn build_ports(sockets: Vec<Socket>, owners: &HashMap<u64, Vec<Process>>) -> Vec<Port> {
    let mut ports: BTreeMap<u16, Port> = BTreeMap::new();
    for socket in sockets {
        let port = ports.entry(socket.port).or_insert_with(|| Port {
            port: socket.port,
            addrs: Vec::new(),
            processes: Vec::new(),
            title: None,
        });
        if !port.addrs.contains(&socket.addr) {
            port.addrs.push(socket.addr);
        }
        for process in owners.get(&socket.inode).into_iter().flatten() {
            if !port.processes.iter().any(|p| p.pid == process.pid) {
                port.processes.push(process.clone());
            }
        }
    }
    ports
        .into_values()
        .map(|mut port| {
            port.addrs.sort();
            // The parent (lowest pid) first: it is usually the one to name and stop.
            port.processes.sort_by_key(|p| p.pid);
            port
        })
        .collect()
}

/// Listening sockets owned by `uid` in a `/proc/net/tcp` or `tcp6` table.
fn parse_net_tcp(text: &str, v6: bool, uid: u32) -> Vec<Socket> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 || fields[3] != LISTEN || fields[7].parse::<u32>().ok()? != uid {
                return None;
            }
            let (addr, port) = fields[1].split_once(':')?;
            Some(Socket {
                addr: parse_addr(addr, v6)?,
                port: u16::from_str_radix(port, 16).ok()?,
                inode: fields[9].parse().ok()?,
            })
        })
        .collect()
}

/// The kernel prints addresses as 32-bit words in host byte order.
fn parse_addr(hex: &str, v6: bool) -> Option<IpAddr> {
    let word = |i: usize| -> Option<[u8; 4]> {
        let chunk = hex.get(i * 8..i * 8 + 8)?;
        Some(u32::from_str_radix(chunk, 16).ok()?.to_ne_bytes())
    };
    if v6 {
        if hex.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 16];
        for i in 0..4 {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&word(i)?);
        }
        let addr = Ipv6Addr::from(bytes);
        // Show IPv4-mapped addresses (`::ffff:127.0.0.1`) as IPv4.
        Some(match addr.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(addr),
        })
    } else {
        (hex.len() == 8).then(|| word(0).map(|b| IpAddr::V4(Ipv4Addr::from(b))))?
    }
}

/// The processes of `uid` holding each of `inodes`.
fn socket_owners(uid: u32, inodes: &HashSet<u64>) -> HashMap<u64, Vec<Process>> {
    let mut owners: HashMap<u64, Vec<Process>> = HashMap::new();
    if inodes.is_empty() {
        return owners;
    }
    let Ok(entries) = fs::read_dir("/proc") else {
        return owners;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let dir = entry.path();
        if fs::metadata(&dir).map(|m| m.uid()).ok() != Some(uid) {
            continue;
        }
        let Ok(fds) = fs::read_dir(dir.join("fd")) else {
            continue;
        };
        let held: Vec<u64> = fds
            .flatten()
            .filter_map(|fd| socket_inode(&fs::read_link(fd.path()).ok()?))
            .filter(|inode| inodes.contains(inode))
            .collect();
        if held.is_empty() {
            continue;
        }
        let process = read_process(pid, &dir);
        for inode in held {
            owners.entry(inode).or_default().push(process.clone());
        }
    }
    owners
}

/// `socket:[12345]` -> 12345
fn socket_inode(link: &Path) -> Option<u64> {
    link.to_str()?
        .strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

fn read_process(pid: u32, dir: &Path) -> Process {
    let comm = fs::read_to_string(dir.join("comm"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let command: Vec<String> = fs::read(dir.join("cmdline"))
        .map(|raw| {
            raw.split(|&b| b == 0)
                .filter(|arg| !arg.is_empty())
                .map(|arg| String::from_utf8_lossy(arg).into_owned())
                .collect()
        })
        .unwrap_or_default();
    Process {
        pid,
        name: process_name(&comm, &command),
        command,
        cwd: fs::read_link(dir.join("cwd")).ok(),
    }
}

const INTERPRETERS: [&str; 10] = [
    "node", "nodejs", "bun", "deno", "python", "ruby", "php", "perl", "java", "dotnet",
];
/// Interpreter subcommands skipped when looking for the script (`deno run x.ts`).
const SUBCOMMANDS: [&str; 5] = ["run", "x", "exec", "serve", "task"];
/// Script names that say nothing; the interpreter's name is shown instead.
const GENERIC: [&str; 8] = [
    "index", "main", "server", "cli", "start", "app", "run", "dev",
];

/// A short, recognizable name: the script run by an interpreter (`vite`, `next`,
/// `http.server`), else the program (from the command line, as `comm` is cut at 15 bytes).
pub fn process_name(comm: &str, command: &[String]) -> String {
    let program = command
        .first()
        .map(|argv0| clean_program(basename(argv0.split(' ').next().unwrap_or(argv0))))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| clean_program(comm));

    let base = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if !INTERPRETERS.contains(&base) {
        return program;
    }
    let mut args = command.iter().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-m" => {
                if let Some(module) = args.next() {
                    return module.clone();
                }
            }
            "-jar" => {
                if let Some(jar) = args.next() {
                    return strip_ext(basename(jar)).to_string();
                }
            }
            "-r" | "--require" | "--import" | "--loader" | "-e" | "-c" | "--eval" => {
                args.next();
            }
            arg if arg.starts_with('-') || SUBCOMMANDS.contains(&arg) => {}
            script => return script_name(script).unwrap_or(program),
        }
    }
    program
}

/// `.../node_modules/vite/bin/vite.js` -> `vite`, `node_modules/.bin/next` -> `next`,
/// `manage.py` -> `manage`; `None` for generic names like `index.js`.
fn script_name(path: &str) -> Option<String> {
    let stem = strip_ext(basename(path));
    if let Some((_, rest)) = path.rsplit_once("node_modules/") {
        let mut parts = rest.split('/');
        let package = match parts.next()? {
            scope if scope.starts_with('@') => format!("{scope}/{}", parts.next()?),
            ".bin" | ".pnpm" => String::new(),
            package => package.to_string(),
        };
        if !package.is_empty() {
            return Some(package);
        }
    }
    (!stem.is_empty() && !GENERIC.contains(&stem)).then(|| stem.to_string())
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn strip_ext(name: &str) -> &str {
    for ext in [
        ".js", ".mjs", ".cjs", ".ts", ".mts", ".py", ".rb", ".php", ".jar",
    ] {
        if let Some(stem) = name.strip_suffix(ext) {
            return stem;
        }
    }
    name
}

/// Undo Nix wrapper names: `.kdeconnectd-wrapped` -> `kdeconnectd`.
/// `comm` cuts names at 15 bytes, hence the partial suffixes.
fn clean_program(name: &str) -> String {
    let Some(name) = name.strip_prefix('.') else {
        return name.to_string();
    };
    [
        "-wrapped", "-wrappe", "-wrapp", "-wrap", "-wra", "-wr", "-w",
    ]
    .iter()
    .find_map(|suffix| name.strip_suffix(suffix))
    .unwrap_or(name)
    .to_string()
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(args: &str) -> Vec<String> {
        args.split(' ').map(String::from).collect()
    }

    #[test]
    fn parses_listen_sockets_for_the_user() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1435 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0000000000000000 100 0 0 10 0
   1: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 1111 1 0000000000000000 100 0 0 10 0
   2: 0100007F:1435 0100007F:C350 01 00000000:00000000 00:00000000 00000000  1000        0 5555 1 0000000000000000 20 4 30 10 -1";
        let sockets = parse_net_tcp(table, false, 1000);
        assert_eq!(
            sockets,
            vec![Socket {
                addr: "127.0.0.1".parse().unwrap(),
                port: 5173,
                inode: 4242
            }]
        );
    }

    #[test]
    fn parses_ipv6_addresses() {
        assert_eq!(
            parse_addr("00000000000000000000000001000000", true),
            Some("::1".parse().unwrap())
        );
        assert_eq!(
            parse_addr("00000000000000000000000000000000", true),
            Some("::".parse().unwrap())
        );
        assert_eq!(
            parse_addr("0000000000000000FFFF00000100007F", true),
            Some("127.0.0.1".parse().unwrap())
        );
    }

    #[test]
    fn names_dev_servers() {
        let name = |comm, args| process_name(comm, &cmd(args));
        assert_eq!(
            name(
                "node",
                "node /app/node_modules/vite/bin/vite.js --port 5173"
            ),
            "vite"
        );
        assert_eq!(name("node", "node /app/node_modules/.bin/next dev"), "next");
        assert_eq!(
            name("node", "node /app/node_modules/@angular/cli/bin/ng serve"),
            "@angular/cli"
        );
        assert_eq!(
            name("python3", "python3 -m http.server 8000"),
            "http.server"
        );
        assert_eq!(
            name("python3.12", "/usr/bin/python3.12 manage.py runserver"),
            "manage"
        );
        assert_eq!(name("node", "node --inspect index.js"), "node");
        assert_eq!(name("deno", "deno run -A main.ts"), "deno");
        assert_eq!(name("bun", "bun run server.ts"), "bun");
        assert_eq!(name("hugo", "hugo server -D"), "hugo");
        assert_eq!(
            name(".kdeconnectd-wr", "/nix/store/x/bin/.kdeconnectd-wrapped"),
            "kdeconnectd"
        );
        assert_eq!(name("postgres", ""), "postgres");
    }

    #[test]
    fn urls_and_reach() {
        let port = |addrs: &[&str]| Port {
            port: 3000,
            addrs: addrs.iter().map(|a| a.parse().unwrap()).collect(),
            processes: Vec::new(),
            title: None,
        };
        assert_eq!(port(&["127.0.0.1"]).url(false), "http://localhost:3000/");
        assert_eq!(port(&["127.0.0.1", "::1"]).reach(), "localhost only");
        assert_eq!(port(&["0.0.0.0"]).reach(), "all interfaces");
        assert_eq!(
            port(&["192.168.1.5"]).url(true),
            "https://192.168.1.5:3000/"
        );
        assert_eq!(port(&["fd00::5"]).url(false), "http://[fd00::5]:3000/");
    }
}
