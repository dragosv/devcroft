# Draft issue for Eugeny/russh: per-channel backpressure on incoming data

Not filed. This is the text to send upstream if the owner decides to.
Until upstream offers this, devcroft carries the change as `vendor/russh`
(`devcroft-russh`). Everything below the line is the issue; the reproducer
was run on russh 0.65.1 on 2026-11-21.

---

**Title:** Server: a channel whose data the application can't take yet stalls the whole connection (window credit is granted on receipt)

## What happens

We run a russh server that executes commands for editors and other tools, and each session channel's data goes to its child's stdin. When a child stops reading (`sleep 100` with something piped into it, or a build that's busy), we have nowhere to put the next packet, and every option is bad:

- wait for the pipe before taking the next message. The channel's queue fills, and then the whole connection stops: other channels, new channel opens, even `send_ping`;
- keep draining and queue the data ourselves. The connection stays up, but the queue has no bound, because the client keeps getting window;
- block on the pipe inline in the handler, which is the first option plus stalled runtime threads.

With the reproducer below, on 0.65.1, a client that floods one channel for 2 s either kills the connection after 5 MiB (the first option: no second channel, no echo, no ping within 10 s) or makes the server hold about 1.95 GiB (the second). 5 runs out of 5 each.

The usual fix would be to stop granting window for that channel until the child catches up, but the server grants it before the application sees the data, so the application never gets a say.

## Versions

Measured on 0.65.1 (rustc 1.95.0, macOS 15.7, tokio multi-thread with 4 workers). We first hit it on 0.62.5. The code involved is the same on `main` (56a9998).

## How it happens

Line numbers are at 56a9998.

1. On `CHANNEL_DATA`, the server debits the window and sends `CHANNEL_WINDOW_ADJUST` once it drops under half the target, all before `Handler::data` runs or the data reaches the `Channel` ([server/encrypted.rs#L1490-L1498](https://github.com/Eugeny/russh/blob/56a999871aa134072e4f2a7395fe2de313870e21/russh/src/server/encrypted.rs#L1490-L1498), then [#L1518](https://github.com/Eugeny/russh/blob/56a999871aa134072e4f2a7395fe2de313870e21/russh/src/server/encrypted.rs#L1518)). So the peer gets new credit whether or not the bytes went anywhere.
2. `Handler::adjust_window` can change the target, but `target_window_size` is one value per session. Lowering it slows every channel, not the one that's behind.

## Why draining `Channel::wait()` doesn't help

I read #730 and the answer there before writing this. The reproducer drains `Channel::wait()` the whole time in `buffer` mode, and that's the mode where the server ends up holding nearly 2 GiB: draining moves the unbounded buffer from russh into our code, it doesn't bound it. The only bound available is to stop draining, which is `block` mode, and that takes the connection down with it. #770 runs into the same thing from the client side and asks at the end whether per-channel windowing would be considered. This is a concrete case for it.

## Reproducing

A russh server forwards the first session channel to a child's stdin and echoes on every other channel. The client floods the first channel with `make_writer()`, which respects the window, waits 2 s, then opens a second channel, sends `hello` on it and waits for the echo, then calls `send_ping()`. Each step gets 10 s. Times are wall clock (`std::time::Instant`).

| mode | child | server side | sent in 2 s | buffered by the server | 2nd channel open | echo on it | `send_ping` |
|---|---|---|---|---|---|---|---|
| `control` | `cat > /dev/null` | awaits the pipe | ~2.2 GiB | 0 | ~1 ms | ~1 ms | ~1 ms |
| `block` | `sleep 60` | awaits the pipe | 5 MiB | 0 | no answer | no answer | no answer |
| `buffer` | `sleep 60` | drains into a queue | ~1.95 GiB | ~1.95 GiB | ~1 ms | ~1 ms | ~1 ms |

<details>
<summary>Reproducer (russh 0.65.1, tokio, anyhow, rand 0.10)</summary>

```toml
[dependencies]
russh = "=0.65.1"
tokio = { version = "1", features = ["full"] }
anyhow = "1"
rand = "0.10"
```

```rust
//! A server forwards the first session channel's data to a child's stdin,
//! draining `Channel::wait()` as #730 recommends. The other channels echo.
//! The client floods the first channel, then checks whether the rest of the
//! connection still answers.
//!
//! Modes:
//!   control - the child reads its stdin (`cat > /dev/null`)
//!   block   - the child never reads (`sleep 60`); the drain task awaits the pipe
//!   buffer  - the child never reads; the drain task queues without a limit
//!
//! Run: cargo run --release -- <mode>

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::keys::ssh_key::Algorithm;
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::server::{self, Auth, ChannelOpenHandle, Msg, Session, VerifiedPublicKey};
use russh::{client, Channel, ChannelMsg};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Control,
    Block,
    Buffer,
}

static SENT: AtomicUsize = AtomicUsize::new(0);
static BUFFERED: AtomicUsize = AtomicUsize::new(0);

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let mode = match std::env::args().nth(1).as_deref() {
        Some("control") => Mode::Control,
        Some("block") => Mode::Block,
        Some("buffer") => Mode::Buffer,
        _ => panic!("usage: russh-repro control|block|buffer"),
    };
    let addr = start_server(mode).await;
    let session = connect(addr).await;

    // Flood the first channel through the window-respecting writer.
    let sink = session.channel_open_session().await.unwrap();
    let mut writer = sink.make_writer();
    tokio::spawn(async move {
        let chunk = vec![0u8; 32 * 1024];
        while writer.write_all(&chunk).await.is_ok() {
            SENT.fetch_add(chunk.len(), Ordering::Relaxed);
        }
    });
    tokio::time::sleep(Duration::from_secs(2)).await;

    let limit = Duration::from_secs(10);
    let t = Instant::now();
    let opened = timeout(limit, session.channel_open_session()).await;
    let open_time = t.elapsed();
    let open_ok = matches!(opened, Ok(Ok(_)));

    let echo_time = match opened {
        Ok(Ok(mut echo)) => {
            let t = Instant::now();
            let ok = timeout(limit, async {
                echo.data(&b"hello"[..]).await.unwrap();
                while let Some(msg) = echo.wait().await {
                    if let ChannelMsg::Data { .. } = msg {
                        return;
                    }
                }
            })
            .await
            .is_ok();
            ok.then(|| t.elapsed())
        }
        _ => None,
    };

    let t = Instant::now();
    let ping = timeout(limit, session.send_ping()).await.is_ok();
    let ping_time = t.elapsed();

    let show = |ok: bool, d: Duration| {
        if ok {
            format!("{d:.2?}")
        } else {
            format!("no answer in {limit:?}")
        }
    };
    println!(
        "sent {} MiB | server buffered {} MiB | open 2nd channel: {} | echo on it: {} | ping: {}",
        SENT.load(Ordering::Relaxed) >> 20,
        BUFFERED.load(Ordering::Relaxed) >> 20,
        show(open_ok, open_time),
        echo_time.map_or(format!("no answer in {limit:?}"), |d| format!("{d:.2?}")),
        show(ping, ping_time),
    );
    std::process::exit(0);
}

async fn sink(mut channel: Channel<Msg>, mode: Mode) {
    let program = if mode == Mode::Control {
        "cat > /dev/null"
    } else {
        "sleep 60"
    };
    let mut child = Command::new("sh")
        .args(["-c", program])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();

    if mode == Mode::Buffer {
        // Keep draining no matter what; the queue is the only place left.
        let queue = Arc::new(Mutex::new(VecDeque::<Vec<u8>>::new()));
        let q = queue.clone();
        tokio::spawn(async move {
            loop {
                let next = q.lock().unwrap().pop_front();
                match next {
                    Some(buf) => {
                        BUFFERED.fetch_sub(buf.len(), Ordering::Relaxed);
                        if stdin.write_all(&buf).await.is_err() {
                            return;
                        }
                    }
                    None => tokio::time::sleep(Duration::from_millis(5)).await,
                }
            }
        });
        while let Some(msg) = channel.wait().await {
            if let ChannelMsg::Data { data } = msg {
                BUFFERED.fetch_add(data.len(), Ordering::Relaxed);
                queue.lock().unwrap().push_back(data.to_vec());
            }
        }
    } else {
        // Drain promptly, but wait for the pipe before taking the next message.
        while let Some(msg) = channel.wait().await {
            if let ChannelMsg::Data { data } = msg {
                if stdin.write_all(&data).await.is_err() {
                    return;
                }
            }
        }
    }
    drop(child);
}

async fn echo(mut channel: Channel<Msg>) {
    while let Some(msg) = channel.wait().await {
        if let ChannelMsg::Data { data } = msg {
            let _ = channel.data(&data[..]).await;
        }
    }
}

async fn start_server(mode: Mode) -> SocketAddr {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let config = Arc::new(server::Config {
        keys: vec![key],
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let config = config.clone();
            let handler = Server { mode, opened: 0 };
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    addr
}

async fn connect(addr: SocketAddr) -> client::Handle<Client> {
    let config = Arc::new(client::Config::default());
    let mut session = client::connect(config, addr, Client).await.unwrap();
    let key = Arc::new(PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap());
    let auth = session
        .authenticate_publickey("user", PrivateKeyWithHashAlg::new(key, None))
        .await
        .unwrap();
    assert!(auth.success());
    session
}

struct Server {
    mode: Mode,
    opened: usize,
}

impl server::Handler for Server {
    type Error = anyhow::Error;

    async fn auth_publickey(&mut self, _: &str, _: VerifiedPublicKey<'_>) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        self.opened += 1;
        if self.opened == 1 {
            tokio::spawn(sink(channel, self.mode));
        } else {
            tokio::spawn(echo(channel));
        }
        Ok(())
    }
}

struct Client;

impl client::Handler for Client {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, _: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }
}
```

</details>

## Proposed API

An opt-in, per channel, so nothing changes for anyone who doesn't ask for it:

```rust
// when the channel is set up
session.manual_receive_window(channel)?;

// later, from whatever writes the data out (can be a blocking thread)
handle.consumed(channel, n as u32)?;
```

For such a channel the server only debits on receipt and sends `CHANNEL_WINDOW_ADJUST` from credits returned through `consumed`, which are coalesced until the session loop picks them up. A peer that sends past the window it was given is treated as a protocol error. Other channels keep today's behaviour.

We've been running this as a fork of 0.62.5 (about 12 files, including some changes outside this issue). With it, a command that never reads its stdin no longer holds up its connection, other channels or other connections. A stock OpenSSH client still moves 16 MiB each way through one session, and the library's own tests pass.

We'd much rather use an upstream API than keep a fork. If this direction works for you, I can send a PR against `main` with just this change and a regression test, and reshape the API however you'd prefer.
