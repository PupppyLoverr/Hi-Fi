//! IPC server for the `hifi` CLI: a Unix-domain socket on macOS/Linux and a
//! loopback TCP listener on Windows. Requests arrive on a
//! std thread per connection and are forwarded into the UI thread via a
//! std channel; the shell drains it and replies over a per-request channel.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::{Sender, channel};
use std::thread;

use crate::shell::IpcJob;
use hifi_core::IpcRequest;

#[cfg(unix)]
pub fn serve(sock_path: std::path::PathBuf, jobs: Sender<IpcJob>) {
    thread::spawn(move || {
        let _ = std::fs::remove_file(&sock_path);
        let Ok(listener) = std::os::unix::net::UnixListener::bind(&sock_path) else {
            eprintln!("hifi: cannot bind {}", sock_path.display());
            return;
        };
        for conn in listener.incoming().flatten() {
            let jobs = jobs.clone();
            thread::spawn(move || {
                if let Ok(reader) = conn.try_clone() {
                    handle(reader, conn, jobs);
                }
            });
        }
    });
}

#[cfg(not(unix))]
pub fn serve(sock_path: std::path::PathBuf, jobs: Sender<IpcJob>) {
    thread::spawn(move || {
        let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", 0)) else {
            eprintln!("hifi: cannot bind loopback IPC listener");
            return;
        };
        let Ok(addr) = listener.local_addr() else {
            return;
        };
        if std::fs::write(
            hifi_core::ipc::port_file(&sock_path),
            addr.port().to_string(),
        )
        .is_err()
        {
            eprintln!("hifi: cannot publish IPC port");
            return;
        }
        for conn in listener.incoming().flatten() {
            let jobs = jobs.clone();
            thread::spawn(move || {
                if let Ok(reader) = conn.try_clone() {
                    handle(reader, conn, jobs);
                }
            });
        }
    });
}

fn handle(reader: impl Read, mut writer: impl Write, jobs: Sender<IpcJob>) {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
        return;
    }
    let req: IpcRequest = match serde_json::from_str(line.trim()) {
        Ok(r) => r,
        Err(e) => {
            let _ = writeln!(
                writer,
                "{}",
                serde_json::to_string(&hifi_core::IpcResponse::err("?", e.to_string()))
                    .unwrap_or_default()
            );
            return;
        }
    };
    let (tx, rx) = channel();
    if jobs
        .send(IpcJob {
            request: req,
            reply: tx,
        })
        .is_err()
    {
        return;
    }
    if let Ok(resp) = rx.recv()
        && let Ok(s) = serde_json::to_string(&resp)
    {
        let _ = writeln!(writer, "{s}");
        let _ = writer.flush();
    }
}
