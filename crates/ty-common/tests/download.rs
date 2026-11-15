#![cfg(feature = "download")]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use ty_common::download::{self, Asset, Progress};

const SHA: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ty-download-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("model.bin")
    }
    fn partial(&self) -> PathBuf {
        self.0.join("model.bin.part")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// A real HTTP endpoint exercises interruption and Range semantics, not a mocked
// downloader. Each response closes the connection, including truncated bodies.
fn server(replies: Vec<(Option<&'static str>, &'static str)>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/model", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        for (range, response) in replies {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "expected HTTP request");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut headers = Vec::new();
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                headers.push(line.trim().to_lowercase());
            }
            assert_eq!(
                headers.iter().find_map(|h| h.strip_prefix("range: ")),
                range
            );
            assert!(headers.iter().any(|h| h == "accept-encoding: identity"));
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (url, worker)
}

fn fetch(
    dir: &Directory,
    url: &str,
    progress: &Progress,
    cancel: &AtomicBool,
    report: impl Fn(u64),
) -> anyhow::Result<PathBuf> {
    download::fetch(
        &Asset {
            url,
            path: &dir.path(),
            bytes: 5,
            sha256: SHA,
        },
        progress,
        cancel,
        report,
    )
}

const FULL: &str = "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
const REST: &str = "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 2-4/5\r\nContent-Length: 3\r\nConnection: close\r\n\r\nllo";

#[test]
fn interrupted_transfer_resumes_and_publishes_only_verified_bytes() {
    let dir = Directory::new();
    let (url, worker) = server(vec![
        (
            None,
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhe",
        ),
        (Some("bytes=2-"), REST),
    ]);
    let progress = Progress::default();
    let cancel = AtomicBool::new(false);
    assert!(fetch(&dir, &url, &progress, &cancel, |_| {}).is_err());
    assert_eq!(fs::read(dir.partial()).unwrap(), b"he");
    assert!(!dir.path().exists());
    assert_eq!(progress.done(), 2);
    fetch(&dir, &url, &progress, &cancel, |_| {}).unwrap();
    worker.join().unwrap();
    assert_eq!(fs::read(dir.path()).unwrap(), b"hello");
    assert!(!dir.partial().exists());
    assert_eq!(progress.fraction(), 1.0);
}

#[test]
fn ignored_range_restarts_instead_of_appending() {
    let dir = Directory::new();
    fs::write(dir.partial(), b"he").unwrap();
    let (url, worker) = server(vec![(Some("bytes=2-"), FULL)]);
    fetch(
        &dir,
        &url,
        &Progress::default(),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    worker.join().unwrap();
    assert_eq!(fs::read(dir.path()).unwrap(), b"hello");
}

#[test]
fn invalid_range_responses_preserve_the_resumable_prefix() {
    for response in [
        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 1-3/5\r\nContent-Length: 3\r\n\r\nell",
        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 2-4/6\r\nContent-Length: 3\r\n\r\nllo",
        "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\n\r\nllo",
        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 2-4/5\r\nContent-Length: 4\r\n\r\nllox",
        "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\r\n",
    ] {
        let dir = Directory::new();
        fs::write(dir.partial(), b"he").unwrap();
        let (url, worker) = server(vec![(Some("bytes=2-"), response)]);
        assert!(fetch(&dir, &url, &Progress::default(), &AtomicBool::new(false), |_| {}).is_err());
        worker.join().unwrap();
        assert!(!dir.path().exists());
        assert_eq!(fs::read(dir.partial()).unwrap(), b"he");
    }
}

#[test]
fn corrupt_or_oversized_partial_is_replaced_with_a_verified_fresh_download() {
    for partial in [b"xxxxx".as_slice(), b"oversized"] {
        let dir = Directory::new();
        fs::write(dir.partial(), partial).unwrap();
        let (url, worker) = server(vec![(None, FULL)]);
        fetch(
            &dir,
            &url,
            &Progress::default(),
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        worker.join().unwrap();
        assert_eq!(fs::read(dir.path()).unwrap(), b"hello");
    }
    let dir = Directory::new();
    let bad = "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nxxxxx";
    let (url, worker) = server(vec![(None, bad), (None, bad)]);
    let err = fetch(
        &dir,
        &url,
        &Progress::default(),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap_err();
    worker.join().unwrap();
    assert!(err.to_string().contains("checksum mismatch"));
    assert!(!dir.path().exists());
    assert!(!dir.partial().exists());
}

#[test]
fn cancellation_keeps_completed_partial_for_offline_verification_on_retry() {
    let dir = Directory::new();
    let (url, worker) = server(vec![(None, FULL)]);
    let cancel = AtomicBool::new(false);
    let err = fetch(&dir, &url, &Progress::default(), &cancel, |done| {
        if done == 5 {
            cancel.store(true, Ordering::Relaxed);
        }
    })
    .unwrap_err();
    worker.join().unwrap();
    assert!(download::is_cancelled(&err));
    assert!(!dir.path().exists());
    assert_eq!(fs::read(dir.partial()).unwrap(), b"hello");
    // The server has exited: a fully downloaded .part needs no HTTP request.
    fetch(
        &dir,
        &url,
        &Progress::default(),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(fs::read(dir.path()).unwrap(), b"hello");
}

#[test]
fn cross_process_lock_wait_is_cancellable_and_retry_reuses_the_published_file() {
    let dir = Directory::new();
    let (url, worker) = server(vec![(None, FULL)]);
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    thread::scope(|scope| {
        let dir = &dir;
        let url = &url;
        let first = scope.spawn(move || {
            fetch(
                dir,
                url,
                &Progress::default(),
                &AtomicBool::new(false),
                |done| {
                    if done == 5 {
                        entered_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    }
                },
            )
            .unwrap();
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // A second process must wait for the first writer and honor cancellation.
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "download_child", "--ignored"])
            .env("TY_DOWNLOAD_TEST_DIR", &dir.0)
            .env("TY_DOWNLOAD_TEST_URL", url)
            .spawn()
            .unwrap();
        let status = child.wait().unwrap();
        release_tx.send(()).unwrap();
        first.join().unwrap();
        assert!(status.success());
    });
    worker.join().unwrap();
    fetch(
        &dir,
        &url,
        &Progress::default(),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(fs::read(dir.path()).unwrap(), b"hello");
}

#[test]
#[ignore = "subprocess for the cross-process download test"]
fn download_child() {
    let path = PathBuf::from(std::env::var_os("TY_DOWNLOAD_TEST_DIR").unwrap()).join("model.bin");
    let url = std::env::var("TY_DOWNLOAD_TEST_URL").unwrap();
    let cancel = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(Duration::from_millis(200));
            cancel.store(true, Ordering::Relaxed);
        });
        let err = download::fetch(
            &Asset {
                url: &url,
                path: &path,
                bytes: 5,
                sha256: SHA,
            },
            &Progress::default(),
            &cancel,
            |_| {},
        )
        .unwrap_err();
        assert!(download::is_cancelled(&err));
    });
    assert!(!path.exists());
}
