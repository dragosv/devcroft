# Draft issue for containers/libkrun: one unread host client stalls every vsock connection

Not filed. This is the text to send upstream if the owner decides to.

---

**Title:** vsock unix proxy: a host client that stops reading blocks all vsock
connections of the VM

**Revision:** `6151710f4ea627cf0e5dbce78e4192fb4d64c2e9`, macOS ARM64 (HVF).
Guest is Linux with virtio-vsock. Connections are host-initiated through
`VsockDevice::add_unix_port`.

**What happens.** A guest process streams data to a host client over a
port-mapped unix socket. If the host client keeps the connection open but stops
reading, every other vsock connection of the same VM also stops. That includes
new connections: the guest's `OP_RESPONSE` is not seen until the first client
reads again. In our measurement, a new connection waited 3.0 s, which was
exactly how long the first client was paused. A second VM in the same process
was unaffected.

**Why.** In `unix_proxy/unix.rs`, `sendmsg` writes the guest's data to the host
socket on the vsock queue thread:

- After `switch_to_connected`, the socket is blocking, so once the host
  socket's buffer is full, `send` blocks that thread. TX processing for the
  whole device stops with it.
- Credit is returned to the guest (`fwd_cnt = tx_cnt`) as soon as bytes enter
  the kernel buffer. The guest's credit therefore never limits it before the
  host buffer fills.

On macOS, passing `MSG_DONTWAIT` is not enough. `send(…, MSG_DONTWAIT)` on a
blocking `AF_UNIX` stream socket still blocks; we measured it with a plain
socketpair. Only `O_NONBLOCK` on the descriptor returns `EAGAIN`.

**Reproduction.**

1. In the guest, run something that writes ~64 MiB to the vsock stream.
2. On the host, connect, read nothing for 2 s, and meanwhile open a second
   connection to the same port.
3. The second connection completes only after the first resumes reading.

**A fix that worked for us** (local patch, about 160 lines, in `unix.rs` plus
two fields in `mod.rs`):

- Keep connected proxies `O_NONBLOCK`.
- On `EAGAIN` or a partial send, queue the rest per connection. Add `OUT` to
  the proxy's poll interest, and flush on writability.
- Advance `tx_cnt`, and with it the credit returned to the guest, only for
  bytes that actually reached the socket. The queue is then bounded by the
  advertised `CONN_TX_BUF_SIZE`. A guest that sends beyond its credit is reset.
- Defer a guest `SHUTDOWN` (and the close that follows both halves) until the
  queue drains, so the end of the stream is not cut off.

With this patch, the second connection is served in 1 ms while the first
client is paused, and all 64 MiB arrive intact once it reads. Our half-close,
reconnect, stop and SSH/SFTP regressions still pass. We have not run it on
Linux/KVM.
