//! Local IPC server (tokio). Authenticates peers by `SO_PEERCRED`, bounds request size, time,
//! rate and concurrency, and forwards to the engine thread over a bounded queue.

use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{oneshot, watch, Semaphore};
use zcore_domain::protocol::{parse_request, ErrorCode, Response, MAX_REQUEST_BYTES};
use zcore_engine::config::Ipc;

use crate::engine::Msg;
use crate::ratelimit::RateLimiter;
use crate::util::monotonic_ms;

pub const MAX_CONNECTIONS: usize = 32;

#[derive(Clone)]
pub struct Shared {
    pub tx: SyncSender<Msg>,
    pub ipc: Ipc,
    pub limiter: Arc<Mutex<RateLimiter>>,
    pub conns: Arc<Semaphore>,
}

async fn write_resp(s: &mut UnixStream, r: &Response) {
    let mut line = serde_json::to_vec(r).unwrap_or_default();
    line.push(b'\n');
    let _ = tokio::time::timeout(Duration::from_secs(2), s.write_all(&line)).await;
}

pub async fn handle_conn(mut stream: UnixStream, sh: Shared) {
    let uid = match stream.peer_cred() {
        Ok(c) => c.uid(),
        Err(_) => return,
    };
    if !sh
        .limiter
        .lock()
        .map(|mut l| l.allow(uid, monotonic_ms()))
        .unwrap_or(false)
    {
        write_resp(
            &mut stream,
            &Response::err("", ErrorCode::RateLimited, "too many requests"),
        )
        .await;
        return;
    }
    let mut buf = Vec::new();
    let read = {
        let mut r = BufReader::new((&mut stream).take(MAX_REQUEST_BYTES as u64 + 1));
        tokio::time::timeout(Duration::from_secs(5), r.read_until(b'\n', &mut buf)).await
    };
    match read {
        Ok(Ok(_)) => {}
        _ => {
            write_resp(
                &mut stream,
                &Response::err("", ErrorCode::Timeout, "no complete request line"),
            )
            .await;
            return;
        }
    }
    if buf.len() > MAX_REQUEST_BYTES {
        write_resp(
            &mut stream,
            &Response::err(
                "",
                ErrorCode::TooLarge,
                format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
            ),
        )
        .await;
        return;
    }
    let line = String::from_utf8_lossy(&buf);
    let req = match parse_request(line.trim_end()) {
        Ok(r) => r,
        Err((id, code, msg)) => {
            write_resp(&mut stream, &Response::err(&id, code, msg)).await;
            return;
        }
    };
    let id = req.id.clone();
    let (tx, rx) = oneshot::channel();
    let resp = match sh.tx.try_send(Msg::Request {
        uid,
        req,
        reply: tx,
    }) {
        Err(TrySendError::Full(_)) => {
            Response::err(&id, ErrorCode::Busy, "engine queue full; retry later")
        }
        Err(TrySendError::Disconnected(_)) => {
            Response::err(&id, ErrorCode::ShuttingDown, "core is stopping")
        }
        Ok(()) => match tokio::time::timeout(Duration::from_millis(sh.ipc.request_timeout_ms), rx)
            .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Response::err(&id, ErrorCode::ShuttingDown, "core is stopping"),
            Err(_) => Response::err(
                &id,
                ErrorCode::Timeout,
                "the engine did not answer in time; the operation may still complete (check audit)",
            ),
        },
    };
    write_resp(&mut stream, &resp).await;
}

pub async fn serve(listener: UnixListener, sh: Shared, mut stop: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            r = listener.accept() => {
                let Ok((stream, _)) = r else { continue };
                let Ok(permit) = sh.conns.clone().try_acquire_owned() else {
                    let mut s = stream;
                    write_resp(&mut s, &Response::err("", ErrorCode::Busy, "too many connections")).await;
                    continue;
                };
                let sh = sh.clone();
                tokio::spawn(async move {
                    handle_conn(stream, sh).await;
                    drop(permit);
                });
            }
            _ = stop.changed() => break,
        }
    }
}
