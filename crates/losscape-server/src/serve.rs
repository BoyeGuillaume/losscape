//! Accepts connections, with limits `axum::serve` doesn't offer. Holding a connection open costs
//! a client nothing: one that has nothing to say is closed, and no address can take them all.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::ConnectInfo;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, watch};
use tower::ServiceExt;

use crate::auth::address_key;
use crate::config::IpNet;

#[derive(Clone, Debug)]
pub struct Limits {
    /// To start a request, and to send its headers.
    pub header_read_timeout: Duration,
    /// Without anything sent or received: between requests, or in the middle of one. Live
    /// streams send something every 15 s, and queries are limited to less than this.
    pub idle_timeout: Duration,
    /// In total. Past this, new connections wait to be accepted.
    pub max_connections: usize,
    /// Per address (IPv6: per /64). Past this, new connections are closed.
    pub max_per_address: usize,
    /// Addresses without that limit: everyone behind a reverse proxy has its address.
    pub trusted_proxies: Vec<IpNet>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            header_read_timeout: Duration::from_secs(20),
            idle_timeout: Duration::from_secs(180),
            max_connections: 4096,
            max_per_address: 512,
            trusted_proxies: Vec::new(),
        }
    }
}

/// Connections open per address.
#[derive(Default)]
struct PerAddress(Mutex<HashMap<String, usize>>);

impl PerAddress {
    fn enter(self: &Arc<Self>, key: String, max: usize) -> Option<AddressSlot> {
        let mut counts = self.0.lock().expect("not poisoned");
        let count = counts.entry(key.clone()).or_insert(0);
        if *count >= max {
            return None;
        }
        *count += 1;
        Some(AddressSlot(self.clone(), key))
    }
}

struct AddressSlot(Arc<PerAddress>, String);

impl Drop for AddressSlot {
    fn drop(&mut self) {
        let mut counts = self.0.0.lock().expect("not poisoned");
        if let Some(count) = counts.get_mut(&self.1) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.1);
            }
        }
    }
}

/// A connection that remembers when something last went through it.
struct Watched {
    stream: TcpStream,
    since: Instant,
    last_ms: Arc<AtomicU64>,
}

impl Watched {
    fn touch(&self) {
        let now = self.since.elapsed().as_millis() as u64;
        self.last_ms.store(now, Ordering::Relaxed);
    }
}

impl AsyncRead for Watched {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.stream).poll_read(cx, buf);
        if buf.filled().len() > before {
            self.touch();
        }
        poll
    }
}

impl AsyncWrite for Watched {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let poll = Pin::new(&mut self.stream).poll_write(cx, data);
        if matches!(poll, Poll::Ready(Ok(n)) if n > 0) {
            self.touch();
        }
        poll
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// Completes once nothing has gone through the connection for `limit`.
async fn until_idle(since: Instant, last_ms: Arc<AtomicU64>, limit: Duration) {
    let limit_ms = limit.as_millis() as u64;
    loop {
        let now = since.elapsed().as_millis() as u64;
        let idle = now.saturating_sub(last_ms.load(Ordering::Relaxed));
        if idle >= limit_ms {
            return;
        }
        tokio::time::sleep(Duration::from_millis(limit_ms - idle)).await;
    }
}

/// Serves `app` until `shutdown` completes and the connections open by then have finished.
pub async fn serve(
    listener: TcpListener,
    app: Router,
    limits: Limits,
    shutdown: impl Future<Output = ()>,
) {
    let mut builder = Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_read_timeout);
    builder.http2().timer(TokioTimer::new());
    let builder = Arc::new(builder);
    let slots = Arc::new(Semaphore::new(limits.max_connections));
    let per_address = Arc::new(PerAddress::default());
    let graceful = GracefulShutdown::new();
    let (stopping, stop_waiting) = watch::channel(false);
    let mut shutdown = std::pin::pin!(shutdown);

    loop {
        let slot = tokio::select! {
            () = &mut shutdown => break,
            slot = slots.clone().acquire_owned() => slot.expect("never closed"),
        };
        let (stream, peer) = tokio::select! {
            () = &mut shutdown => break,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(err) => {
                    // Out of file descriptors, most likely: it passes as connections close.
                    tracing::warn!(%err, "accepting a connection failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
        };
        let address_slot = if limits
            .trusted_proxies
            .iter()
            .any(|net| net.contains(peer.ip()))
        {
            None
        } else {
            match per_address.enter(address_key(Some(peer.ip())), limits.max_per_address) {
                Some(slot) => Some(slot),
                None => continue, // closes it
            }
        };
        stream.set_nodelay(true).ok();
        let app = app.clone();
        let service = service_fn(move |mut request: hyper::Request<Incoming>| {
            // What `into_make_service_with_connect_info` provides.
            request
                .extensions_mut()
                .insert(ConnectInfo::<SocketAddr>(peer));
            app.clone().oneshot(request)
        });
        let builder = builder.clone();
        let watcher = graceful.watcher();
        let mut stop_waiting = stop_waiting.clone();
        let (first_byte, idle) = (limits.header_read_timeout, limits.idle_timeout);
        tokio::spawn(async move {
            let _slots = (slot, address_slot);
            // Telling HTTP/1 from HTTP/2 waits for the first bytes, with no limit of its own.
            let mut byte = [0u8; 1];
            tokio::select! {
                started = tokio::time::timeout(first_byte, stream.peek(&mut byte)) => {
                    if !matches!(started, Ok(Ok(n)) if n > 0) {
                        return;
                    }
                }
                _ = stop_waiting.changed() => return,
            }
            let since = Instant::now();
            let last_ms = Arc::new(AtomicU64::new(0));
            let io = Watched {
                stream,
                since,
                last_ms: last_ms.clone(),
            };
            let connection = builder.serve_connection_with_upgrades(TokioIo::new(io), service);
            // Errors are clients going away, or being timed out: nothing to act on.
            tokio::select! {
                _ = watcher.watch(connection.into_owned()) => {}
                () = until_idle(since, last_ms, idle) => {}
            }
        });
    }
    drop(listener);
    stopping.send_replace(true);
    graceful.shutdown().await;
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::oneshot;

    use super::*;

    async fn start(
        limits: Limits,
    ) -> (SocketAddr, oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app =
            Router::new()
                .route(
                    "/",
                    get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move {
                        peer.ip().to_string()
                    }),
                )
                // What a live stream does: a little, regularly, for longer than the idle limit.
                .route(
                    "/drip",
                    get(|| async {
                        let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, io::Error>>(1);
                        tokio::spawn(async move {
                            for _ in 0..8 {
                                tokio::time::sleep(Duration::from_millis(50)).await;
                                if tx.send(Ok("x".into())).await.is_err() {
                                    return;
                                }
                            }
                        });
                        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx))
                    }),
                );
        let (stop, stopped) = oneshot::channel();
        let server = tokio::spawn(serve(listener, app, limits, async {
            stopped.await.ok();
        }));
        (addr, stop, server)
    }

    const REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";

    async fn read_all(stream: &mut TcpStream) -> String {
        let mut out = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut out))
            .await
            .expect("answered or closed in time")
            .ok();
        String::from_utf8_lossy(&out).into_owned()
    }

    #[tokio::test]
    async fn serves_requests_and_knows_who_asked() {
        let (addr, stop, server) = start(Limits::default()).await;
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(REQUEST).await.unwrap();
        let response = read_all(&mut client).await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("127.0.0.1"), "{response}");
        stop.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn closes_connections_that_dont_finish_their_headers() {
        let limits = Limits {
            header_read_timeout: Duration::from_millis(200),
            ..Limits::default()
        };
        let (addr, stop, server) = start(limits).await;
        let mut slow = TcpStream::connect(addr).await.unwrap();
        slow.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n")
            .await
            .unwrap();
        let started = Instant::now();
        read_all(&mut slow).await;
        assert!(started.elapsed() < Duration::from_secs(2));
        stop.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn idle_connections_that_never_send_anything_are_closed_too() {
        let limits = Limits {
            header_read_timeout: Duration::from_millis(200),
            ..Limits::default()
        };
        let (addr, stop, server) = start(limits).await;
        let mut idle = TcpStream::connect(addr).await.unwrap();
        let started = Instant::now();
        read_all(&mut idle).await;
        assert!(started.elapsed() < Duration::from_secs(2));
        stop.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn connections_past_the_limit_wait_for_a_free_one() {
        let limits = Limits {
            max_connections: 1,
            header_read_timeout: Duration::from_secs(30),
            ..Limits::default()
        };
        let (addr, stop, server) = start(limits).await;
        let holding = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut waiting = TcpStream::connect(addr).await.unwrap();
        waiting.write_all(REQUEST).await.unwrap();
        let mut byte = [0u8; 1];
        assert!(
            tokio::time::timeout(Duration::from_millis(200), waiting.read(&mut byte))
                .await
                .is_err(),
            "not served while the only slot is taken"
        );
        drop(holding);
        let response = read_all(&mut waiting).await;
        assert!(response.contains("127.0.0.1"), "{response}");
        stop.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn stops_when_asked_also_with_all_slots_taken() {
        let limits = Limits {
            max_connections: 1,
            header_read_timeout: Duration::from_secs(30),
            ..Limits::default()
        };
        let (addr, stop, server) = start(limits).await;
        let _holding = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("stopped in time")
            .unwrap();
    }

    #[tokio::test]
    async fn connections_kept_open_between_requests_are_closed_when_idle() {
        let limits = Limits {
            idle_timeout: Duration::from_millis(200),
            ..Limits::default()
        };
        let (addr, stop, server) = start(limits).await;
        let mut kept = TcpStream::connect(addr).await.unwrap();
        kept.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut buf = [0u8; 256];
        assert!(kept.read(&mut buf).await.unwrap() > 0, "answered");
        let started = Instant::now();
        read_all(&mut kept).await;
        assert!(started.elapsed() < Duration::from_secs(2));
        stop.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn slow_responses_arent_idle() {
        let limits = Limits {
            idle_timeout: Duration::from_millis(200),
            ..Limits::default()
        };
        let (addr, stop, server) = start(limits).await;
        let mut client = TcpStream::connect(addr).await.unwrap();
        client
            .write_all(b"GET /drip HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let response = read_all(&mut client).await;
        let (_, body) = response.split_once("\r\n\r\n").unwrap();
        assert_eq!(body.matches('x').count(), 8, "{response}");
        stop.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn one_address_cant_take_all_the_connections() {
        let limits = Limits {
            max_per_address: 2,
            ..Limits::default()
        };
        let (addr, stop, server) = start(limits).await;
        let _a = TcpStream::connect(addr).await.unwrap();
        let b = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut refused = TcpStream::connect(addr).await.unwrap();
        assert_eq!(read_all(&mut refused).await, "", "closed right away");
        drop(b);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut next = TcpStream::connect(addr).await.unwrap();
        next.write_all(REQUEST).await.unwrap();
        assert!(read_all(&mut next).await.starts_with("HTTP/1.1 200"));
        stop.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn reverse_proxies_have_no_such_limit() {
        let limits = Limits {
            max_per_address: 1,
            trusted_proxies: vec!["127.0.0.1".parse().unwrap()],
            ..Limits::default()
        };
        let (addr, stop, server) = start(limits).await;
        let _a = TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut b = TcpStream::connect(addr).await.unwrap();
        b.write_all(REQUEST).await.unwrap();
        assert!(read_all(&mut b).await.starts_with("HTTP/1.1 200"));
        stop.send(()).unwrap();
        server.await.unwrap();
    }
}
