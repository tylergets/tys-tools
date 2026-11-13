//! Optional quote commands run only after the screen is locked, with bounded output
//! and runtime. Neither a failed command nor a slow one can delay locking/unlocking.
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

pub fn spawn(argv: Vec<String>) -> Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || match run(&argv) {
        Ok(text) => {
            let _ = sender.send(text);
        }
        Err(err) => eprintln!("tylock: quote command: {err:#}"),
    });
    receiver
}

fn run(argv: &[String]) -> anyhow::Result<String> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let result = (|| -> anyhow::Result<String> {
        let mut stdout = child.stdout.take().unwrap();
        let fd = stdout.as_raw_fd();
        // SAFETY: stdout owns this live descriptor; only its nonblocking flag changes.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            anyhow::ensure!(
                flags != -1 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) != -1,
                "cannot make quote output nonblocking"
            );
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut bytes = Vec::new();
        loop {
            anyhow::ensure!(Instant::now() < deadline, "timed out after 3 seconds");
            let mut buffer = [0; 1024];
            match stdout.read(&mut buffer) {
                Ok(0) => {
                    if let Some(status) = child.try_wait()? {
                        anyhow::ensure!(status.success(), "exited with {status}");
                        return Ok(String::from_utf8(bytes)?.trim().to_owned());
                    }
                }
                Ok(n) => {
                    bytes.extend_from_slice(&buffer[..n]);
                    anyhow::ensure!(bytes.len() <= 16 * 1024, "output exceeds 16 KiB");
                    continue;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err.into()),
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    if result.is_err() {
        // SAFETY: the child's process group was created above; kill its helpers too.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_output_and_failed_commands() {
        assert_eq!(
            run(&[
                "sh".into(),
                "-c".into(),
                "printf 'quote\\n— author\\n'".into()
            ])
            .unwrap(),
            "quote\n— author"
        );
        assert!(run(&["sh".into(), "-c".into(), "printf hidden; exit 1".into()]).is_err());
    }

    #[test]
    fn unbounded_output_is_rejected() {
        let error = run(&[
            "sh".into(),
            "-c".into(),
            "while :; do printf 'too much output'; done".into(),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("16 KiB"));
    }

    #[test]
    fn hung_command_times_out() {
        let start = Instant::now();
        assert!(run(&["sh".into(), "-c".into(), "sleep 30".into()])
            .unwrap_err()
            .to_string()
            .contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(6));
    }
}
