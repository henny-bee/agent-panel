//! One app, many windows: launching again opens a window in the running app,
//! so every window shows the same live sessions instead of a second copy.
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::Path,
};

/// Ask a running app for a new window; true when one answered.
pub(crate) fn hand_off(socket: &Path) -> bool {
    UnixStream::connect(socket)
        .and_then(|mut stream| stream.write_all(b"window\n"))
        .is_ok()
}

/// Listen for later launches, one window request each.
pub(crate) fn listen(socket: &Path) -> Option<async_channel::Receiver<()>> {
    // No app answered `hand_off`, so a socket file here is left from a crash.
    // shortcut: two launches in the same instant can both get here; the later
    // one wins the socket. A lock file if that ever shows up.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)
        .map_err(|error| eprintln!("Cannot listen for new windows: {error}"))
        .ok()?;
    let (sender, requests) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("new-windows".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(stream).read_line(&mut line).is_ok()
                    && line.trim() == "window"
                    && sender.send_blocking(()).is_err()
                {
                    break;
                }
            }
        })
        .ok()?;
    Some(requests)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_launch_reaches_the_first_and_a_stale_socket_is_replaced() {
        let dir = std::env::temp_dir().join(format!("monocode-instance-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("app.sock");
        std::fs::write(&socket, b"").unwrap();
        assert!(!hand_off(&socket), "nothing listens on a stale file");
        let requests = listen(&socket).unwrap();
        assert!(hand_off(&socket));
        assert_eq!(requests.recv_blocking(), Ok(()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
