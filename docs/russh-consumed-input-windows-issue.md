# Draft issue for Eugeny/russh: per-channel backpressure on incoming data

Not filed. This is the text to send upstream if the owner decides to.
Until upstream offers this, devcroft carries the change as `vendor/russh`
(`devcroft-russh`). Everything below the line is the issue; the reproducer
was run on russh 0.65.1 on 2026-11-21.

---

**Title:** Server: backpressure on one receive channel stalls other channels on the same connection

## What happens

When a server application stops consuming data from one session channel, operations on other channels on the same connection stop completing. Continuously draining the channel avoids the stall, but an application queue grows while the child process is not reading.

We run a russh server that executes commands for editors and other tools. Each session channel forwards incoming data to a child process's stdin. We tested two ways to handle a child that does not read its stdin (`sleep 60`):

- Await the stdin write before reading the next channel message. The channel's queue fills, and opening another channel and completing `send_ping()` each time out after 10 seconds. The echo test is skipped because the second channel did not open.
- Continue reading channel messages into an application queue while a separate task writes to stdin. Other channels remain responsive, but the queue grows because russh continues granting receive-window credit.

We need to defer receive-window credit for the slow channel until its downstream writer accepts the data, while allowing other channels to progress. The current automatic credit replenishment happens before delivery to the application.

## Versions

Measured on russh 0.65.1 (rustc 1.95.0, Tokio 1.54.1 with 4 worker threads, release build) on macOS. The code involved is the same on `main` at commit `56a9998`.

## How it happens

Line numbers are at 56a9998.

1. On `CHANNEL_DATA`, the server debits the window and sends `CHANNEL_WINDOW_ADJUST` once it drops under half the target, all before `Handler::data` runs or the data reaches the `Channel` ([server/encrypted.rs#L1490-L1498](https://github.com/Eugeny/russh/blob/56a999871aa134072e4f2a7395fe2de313870e21/russh/src/server/encrypted.rs#L1490-L1498), then [#L1518](https://github.com/Eugeny/russh/blob/56a999871aa134072e4f2a7395fe2de313870e21/russh/src/server/encrypted.rs#L1518)). The peer therefore receives new credit before the downstream writer accepts those bytes.
2. If the application awaits a blocked stdin write, it stops reading channel messages. The bounded channel queue fills, and the session loop awaits `chan.send(ChannelMsg::Data { ... }).await` ([#L1513-L1516](https://github.com/Eugeny/russh/blob/56a999871aa134072e4f2a7395fe2de313870e21/russh/src/server/encrypted.rs#L1513-L1516)). While that send is pending, the loop cannot process other channels or reply to requests on the connection. This suspends the session task; it does not require blocking a Tokio worker thread.
3. `Handler::adjust_window` can change the target, but `target_window_size` is one value per session. Changing it affects subsequent window adjustments across channels, so it does not provide independent credit control for the slow channel.

OpenSSH returns credit later. In `channels.c`, `local_consumed` grows only after data is written to the channel's output descriptor ([channels.c#L2444](https://github.com/openssh/openssh-portable/blob/1ec60f13ce4bfca18f2269b2696a2d1b6995b6e3/channels.c#L2444)), and `CHANNEL_WINDOW_ADJUST` sends only that amount ([#L2544-L2561](https://github.com/openssh/openssh-portable/blob/1ec60f13ce4bfca18f2269b2696a2d1b6995b6e3/channels.c#L2544-L2561)). A child that stops reading stops the credit for its own channel, and the rest of the connection keeps working.

## Why draining `Channel::wait()` does not bound application buffering

[#730](https://github.com/Eugeny/russh/pull/730) proposed sending `CHANNEL_WINDOW_ADJUST` only after consumption, and it was closed with the advice to drain `Channel::wait()`. The question asked there about per-channel flow control like OpenSSH's was not answered. This request differs from #730 in one way: it is opt-in per channel, and the default behavior doesn't change.

Draining avoids filling russh's channel queue. The reproducer does that in `buffer` mode, but the downstream process still does not read, so the application queue grows instead. In the `buffer` run, 1,947.9 MiB was queued against the default 2 MiB window, nearly 1,000 times the window in 2 seconds. Credit does not depend on consumption at any point.

The application has three options when a channel's downstream stops reading, and each one fails:

- Wait for the writer (`block` mode): the whole connection stalls.
- Queue without a limit (`buffer` mode): memory grows without a bound.
- Queue with a limit and close the channel when it is full: this breaks legitimate slow consumers, such as a build reading stdin slowly or `cat > file` on a slow disk. An OpenSSH client talking to `sshd` doesn't hit this.

[#770](https://github.com/Eugeny/russh/issues/770) discusses related client-side backpressure and asks about per-channel windowing.

## Reproducing

A russh server forwards the first session channel to a child process's stdin and echoes on every other channel. The client continuously writes to the first channel through `make_writer()`, which respects the receive window. After approximately 2 seconds, it samples the counters, then attempts to open a second channel, verify an exact `hello` echo, and complete `send_ping()`. Each attempted operation has its own 10-second timeout. If opening the channel fails, the echo test reports `not attempted`.

Expected behavior: when the first child stops reading, buffering remains bounded and the sender waits for receive-window credit on that channel. Opening another channel, exchanging data on it, and pinging the connection still succeed.

We ran each mode 5 times on 2026-11-21 and got the same outcome every time. The following table shows one run per mode:

| Mode | Child | Server action | Writer-accepted payload (MiB) | Application-queued payload (MiB) | Second channel open | Exact echo | `send_ping()` completion |
|---|---|---|---|---|---|---|---|
| `control` | `cat > /dev/null` | Awaits stdin write | 2,126.4 | 0.0 | 1.02 ms | 1.52 ms | 1.26 ms |
| `block` | `sleep 60` | Awaits stdin write | 5.1 | 0.0 | Timeout after 10 s | Not attempted | Timeout after 10 s |
| `buffer` | `sleep 60` | Drains into a queue | 1,950.3 | 1,947.9 | 1.22 ms | 1.50 ms | 1.14 ms |

The counters are sampled at 2 seconds, before the probes. "Writer-accepted" counts bytes accepted by the client's `write_all` calls, not bytes the child read. "Application-queued" counts bytes in the server's own queue only, so it isn't process memory. In `block` mode, 5.1 MiB is about the 2 MiB window plus 100 queued messages of up to 32 KiB, the default `channel_buffer_size` and maximum packet size.

To run the reproducer, use Rust and Cargo on a system with a POSIX `sh`, `cat`, and `sleep`. The `buffer` mode intentionally retains data and can use several GiB of memory.

1. Create a standalone project outside the devcroft workspace:

   ```sh
   cargo new --bin russh-repro
   cd russh-repro
   ```

2. Add the dependencies:

   ```sh
   cargo add russh@=0.65.1 anyhow rand@0.10 tokio --features tokio/full
   ```

3. Replace `src/main.rs` with the Rust code in the following "Reproducer" section.
4. Run each mode separately. The `block` run takes about 22 seconds, because two of its probes wait for their 10-second timeouts.

   ```sh
   cargo run --release -- control
   cargo run --release -- block
   cargo run --release -- buffer
   ```

5. Compare the three operation results with the table. The output distinguishes `ok`, `error`, `timeout`, and `not attempted`.

For example, the `block` run in the table printed:

```text
sample at 2.00s: writer accepted 5341184 bytes | application queued 0 bytes | open: timeout (10.00s) | exact echo: not attempted (second channel did not open) | send_ping: timeout (10.00s)
```

<details>
<summary>Reproducer (russh 0.65.1, tokio, anyhow, rand 0.10)</summary>

```rust
//! A server forwards the first session channel's data to a child's stdin,
//! reading channel messages with `Channel::wait()`. The other channels echo.
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
use std::fmt::Display;
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::keys::ssh_key::Algorithm;
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::server::{self, Auth, ChannelOpenHandle, Msg, Session, VerifiedPublicKey};
use russh::{Channel, ChannelMsg, client};
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
    let sample_start = Instant::now();
    let flood = tokio::spawn(async move {
        let chunk = vec![0u8; 32 * 1024];
        while writer.write_all(&chunk).await.is_ok() {
            SENT.fetch_add(chunk.len(), Ordering::Relaxed);
        }
    });
    tokio::time::sleep(Duration::from_secs(2)).await;
    let sample_time = sample_start.elapsed();
    let accepted_at_sample = SENT.load(Ordering::Relaxed);
    let queued_at_sample = BUFFERED.load(Ordering::Relaxed);

    let limit = Duration::from_secs(10);
    let t = Instant::now();
    let opened = timeout(limit, session.channel_open_session()).await;
    let open_time = t.elapsed();
    let open_status = describe(&opened, open_time);

    let echo_status = match opened {
        Ok(Ok(mut echo)) => {
            let t = Instant::now();
            let result = timeout(limit, async {
                let expected = b"hello";
                let mut received = Vec::new();
                echo.data(&expected[..]).await?;
                while let Some(msg) = echo.wait().await {
                    match msg {
                        ChannelMsg::Data { data } => {
                            received.extend_from_slice(&data);
                            anyhow::ensure!(
                                expected.starts_with(&received),
                                "unexpected echo payload"
                            );
                            if received == expected {
                                return Ok::<(), anyhow::Error>(());
                            }
                        }
                        ChannelMsg::Eof | ChannelMsg::Close => {
                            anyhow::bail!("channel closed before the complete echo");
                        }
                        _ => {}
                    }
                }
                anyhow::bail!("channel ended before the complete echo");
            })
            .await;
            describe(&result, t.elapsed())
        }
        _ => "not attempted (second channel did not open)".to_owned(),
    };

    let t = Instant::now();
    let ping = timeout(limit, async {
        session.send_ping().await?;
        anyhow::ensure!(!session.is_closed(), "session closed during ping");
        Ok::<(), anyhow::Error>(())
    })
    .await;
    let ping_status = describe(&ping, t.elapsed());
    flood.abort();
    let _ = flood.await;
    println!(
        "sample at {sample_time:.2?}: writer accepted {} bytes | application queued {} bytes | open: {open_status} | exact echo: {echo_status} | send_ping: {ping_status}",
        accepted_at_sample, queued_at_sample,
    );
}

fn describe<T, E: Display>(
    result: &Result<Result<T, E>, tokio::time::error::Elapsed>,
    elapsed: Duration,
) -> String {
    match result {
        Ok(Ok(_)) => format!("ok ({elapsed:.2?})"),
        Ok(Err(error)) => format!("error ({elapsed:.2?}): {error}"),
        Err(_) => format!("timeout ({elapsed:.2?})"),
    }
}

async fn sink(mut channel: Channel<Msg>, mode: Mode) {
    let program = if mode == Mode::Control {
        "exec cat > /dev/null"
    } else {
        "exec sleep 60"
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
        // Continue draining while another task waits for stdin to accept data.
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

    async fn auth_publickey(
        &mut self,
        _: &str,
        _: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
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

Add an opt-in mode per channel, preserving the current automatic receive-window behavior by default. The following is proposed API pseudocode; these methods are not available in upstream russh:

```rust
// Enable manual credit before accepting data on this channel.
session.manual_receive_window(channel)?;

// Return credit after a successful downstream write of n bytes.
handle.consumed(channel, n as u32)?;
```

For a manual-credit channel, the server debits the receive window on receipt and replenishes it only from credits returned through `consumed`. In our stdin-forwarding case, `consumed` means that the downstream pipe accepted the bytes in a successful write. It does not mean that the child process has read or processed them. Removing bytes from an application queue alone does not return credit.

The application returns credit once per successfully written byte, including partial writes. Credit notifications can originate outside the session task, including from a blocking writer thread, and are coalesced until the session loop processes them. Channels using automatic credit retain their current behavior.

Two details need a decision:

- **Delivery must not block the session loop.** Manual credit bounds what the peer can send, but the session loop still awaits `chan.send(...)` (L1513). If the channel's window is larger than its queue (`channel_buffer_size` times the packet size, 100 × 32 KiB by default), an application that stops reading can still stall the connection. Either the manual window must stay within the queue's capacity, or delivery for manual channels must use `try_send` with a backlog bounded by the window.
- **A peer that exceeds the window.** russh ignores the excess today, citing RFC 4254 §5.2. Under manual credit, ignoring it would drop bytes, so we propose disconnecting. OpenSSH allows 10% over the window and then disconnects with "peer ignored channel window" ([channels.c#L3635-L3651](https://github.com/openssh/openssh-portable/blob/1ec60f13ce4bfca18f2269b2696a2d1b6995b6e3/channels.c#L3635-L3651)).

We run this design in a fork based on 0.62.5. With it, a command that doesn't read stdin leaves other channels responsive, a stock OpenSSH client transfers 16 MiB in each direction through one session, and russh's own tests pass. These are fork results, separate from the upstream 0.65.1 runs.

Other shapes would also work for us, for example non-blocking delivery in the session loop with a per-channel backlog, or a per-channel version of `adjust_window`.

Would you consider an opt-in API for consumption-based receive-window credit per channel? If this direction is acceptable, we can submit a focused PR against `main` with a regression test and adapt the API to your preferred design.
