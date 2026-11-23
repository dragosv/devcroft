use super::device::Vsock;
use super::defs::TsiFlags;
use crate::legacy::DummyIrqChip;
use crate::virtio::{DeviceQueue, InterruptTransport, Queue, VirtioDevice};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use utils::eventfd::EventFd;
use vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};

const RXD: u64 = 0x1000;
const RXA: u64 = 0x3000;
const RXU: u64 = 0x4000;
const TXD: u64 = 0x6000;
const TXA: u64 = 0x8000;
const TXU: u64 = 0x9000;
const RXB: u64 = 0x10000; // 128 * 4096
const TXB: u64 = 0x100000;
const N: u16 = 128;
const PORT: u32 = 7000;

fn fixed() -> bool { std::env::var("FUZZ_FIXED").is_ok() }
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u64) as usize]
    }
}

struct Rig {
    mem: GuestMemoryMmap,
    dev: Vsock,
    sock: PathBuf,
    tx: u16,
    rx: u16,
    used_seen: u16,
}

fn rig(tag: &str, mem_len: usize) -> Rig {
    let sock = PathBuf::from(format!("/tmp/dvf-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let mem = GuestMemoryMmap::from_ranges(&[(GuestAddress(0), mem_len)]).unwrap();
    let mk = |d, a, u| {
        let mut q = Queue::new(256);
        q.size = N;
        q.ready = true;
        q.desc_table = GuestAddress(d);
        q.avail_ring = GuestAddress(a);
        q.used_ring = GuestAddress(u);
        DeviceQueue::new(q, Arc::new(EventFd::new(utils::eventfd::EFD_NONBLOCK).unwrap()))
    };
    let mut map = HashMap::new();
    map.insert(PORT, (sock.clone(), true));
    let mut dev = Vsock::new(3, None, Some(map), TsiFlags::empty()).unwrap();
    let irq = InterruptTransport::new(DummyIrqChip::new().into(), "fuzz".into()).unwrap();
    dev.activate(
        mem.clone(),
        irq,
        vec![mk(RXD, RXA, RXU), mk(TXD, TXA, TXU), mk(0xa000, 0xb000, 0xc000)],
    )
    .unwrap();
    let t = Instant::now();
    while UnixStream::connect(&sock).is_err() {
        assert!(t.elapsed() < Duration::from_secs(5), "listener");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut r = Rig { mem, dev, sock, tx: 0, rx: 0, used_seen: 0 };
    for i in 0..N {
        r.post_rx(i);
    }
    r
}

fn hdr(src: u32, dst: u32, len: u32, typ: u16, op: u16, flags: u32, alloc: u32, fwd: u32) -> Vec<u8> {
    let mut b = vec![];
    b.extend(3u64.to_le_bytes());
    b.extend(2u64.to_le_bytes());
    for v in [src, dst, len] {
        b.extend(v.to_le_bytes());
    }
    b.extend(typ.to_le_bytes());
    b.extend(op.to_le_bytes());
    for v in [flags, alloc, fwd] {
        b.extend(v.to_le_bytes());
    }
    b
}

impl Rig {
    fn desc(&self, t: u64, i: u16, addr: u64, len: u32, flags: u16, next: u16) {
        let a = t + i as u64 * 16;
        self.mem.write_obj(addr, GuestAddress(a)).unwrap();
        self.mem.write_obj(len, GuestAddress(a + 8)).unwrap();
        self.mem.write_obj(flags, GuestAddress(a + 12)).unwrap();
        self.mem.write_obj(next, GuestAddress(a + 14)).unwrap();
    }
    fn post_rx(&mut self, i: u16) {
        self.desc(RXD, i, RXB + i as u64 * 4096, 4096, 2, 0);
        self.mem.write_obj(i, GuestAddress(RXA + 4 + (self.rx % N) as u64 * 2)).unwrap();
        self.rx = self.rx.wrapping_add(1);
        self.mem.write_obj(self.rx, GuestAddress(RXA + 2)).unwrap();
    }
    fn kick(&mut self, head: u16) {
        self.mem.write_obj(head, GuestAddress(TXA + 4 + (self.tx % N) as u64 * 2)).unwrap();
        self.tx = self.tx.wrapping_add(1);
        self.mem.write_obj(self.tx, GuestAddress(TXA + 2)).unwrap();
        self.dev.process_stream_tx();
        if self.dev.muxer.has_pending_rx() {
            self.dev.process_stream_rx();
        }
    }
    fn send(&mut self, h: &[u8], body: usize) {
        self.mem.write_slice(h, GuestAddress(TXB)).unwrap();
        if body == 0 {
            self.desc(TXD, 0, TXB, 44, 0, 0);
        } else {
            self.desc(TXD, 0, TXB, 44, 1, 1);
            self.desc(TXD, 1, TXB + 0x1000, body as u32, 0, 0);
        }
        self.kick(0);
    }
    /// drain used RX; returns (src_port, dst_port, op) of each
    fn drain(&mut self) -> Vec<(u32, u32, u16)> {
        let mut out = vec![];
        let idx: u16 = self.mem.read_obj(GuestAddress(RXU + 2)).unwrap();
        while self.used_seen != idx {
            let e = RXU + 4 + (self.used_seen % N) as u64 * 8;
            let id: u32 = self.mem.read_obj(GuestAddress(e)).unwrap();
            let a = RXB + (id as u64) * 4096;
            let s: u32 = self.mem.read_obj(GuestAddress(a + 16)).unwrap();
            let d: u32 = self.mem.read_obj(GuestAddress(a + 20)).unwrap();
            let o: u16 = self.mem.read_obj(GuestAddress(a + 30)).unwrap();
            out.push((s, d, o));
            self.used_seen = self.used_seen.wrapping_add(1);
            self.post_rx(id as u16);
        }
        out
    }
    /// Positive control: answer every REQUEST, ours included, until ours is served.
    fn healthy(&mut self) -> bool {
        let mut c = match UnixStream::connect(&self.sock) { Ok(c) => c, Err(e) => { eprintln!("DIAG connect {e}"); return false } };
        c.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(3) {
            for x in self.drain() {
                if x.2 == 1 {
                    self.send(&hdr(PORT, x.0, 0, 1, 2, 0, 65536, 0), 0);
                    self.mem.write_slice(b"pong", GuestAddress(TXB + 0x1000)).unwrap();
                    self.send(&hdr(PORT, x.0, 4, 1, 5, 0, 65536, 0), 4);
                }
            }
            // a real driver notifies the RX queue after posting buffers
            self.dev.process_stream_rx();
            let mut b = [0; 4];
            if c.read_exact(&mut b).is_ok() { if &b != b"pong" { eprintln!("DIAG bytes {b:?}") } return &b == b"pong" }
        }
        let rd = |a| self.mem.read_obj::<u16>(GuestAddress(a)).unwrap();
        eprintln!("DIAG rx avail={} used={} tx avail={} used={}", rd(RXA + 2), rd(RXU + 2), rd(TXA + 2), rd(TXU + 2));
        false
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.sock);
    }
}

fn quiet<R>(f: impl FnOnce() -> R) -> Result<R, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|e| {
        e.downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    })
}

#[test]
fn f0_control() {
    assert!(rig("ctl", 16 << 20).healthy());
}

#[test]
fn f1_acceptor_ops() {
    for op in 0..9u16 {
        for typ in [1u16, 3] {
            let mut r = rig(&format!("acc{op}-{typ}"), 16 << 20);
            let body = if op == 5 { 8 } else { 0 };
            let out = quiet(|| r.send(&hdr(PORT, 620, body, typ, op, 3, 65536, 0), body as usize));
            let after = quiet(|| r.healthy());
            eprintln!("ACC type={typ} op={op}: {:?} after={after:?}", out.err());
        }
    }
}

#[test]
fn f2_chain_amplification() {
    // Header says len=1; 100 descriptors each cover the same 16 MiB.
    let mut r = rig("amp", 32 << 20);
    r.mem.write_slice(&hdr(PORT, 999, 1, 1, 5, 0, 0, 0), GuestAddress(TXB)).unwrap();
    r.desc(TXD, 0, TXB, 44, 1, 1);
    for i in 1..=100u16 {
        r.desc(TXD, i, 0x1000000, 16 << 20, if i == 100 { 0 } else { 1 }, i + 1);
    }
    let t = Instant::now();
    r.kick(0);
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    eprintln!("AMP declared=1 guest=32MiB took={:?} maxrss={} MiB healthy={}", t.elapsed(), u.ru_maxrss >> 20, r.healthy());
}

#[test]
fn f3_random() {
    let secs: u64 = std::env::var("FUZZ_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
    let seed0: u64 = std::env::var("FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let end = Instant::now() + Duration::from_secs(secs);
    let (mut total, mut round, mut sick) = (0u64, 0u64, 0u64);
    let mut found: HashMap<String, (u64, u64)> = HashMap::new();
    while Instant::now() < end {
        let seed = seed0.wrapping_mul(0x9E3779B97F4A7C15) ^ (round + 1).wrapping_mul(0xD1B54A32D192ED03);
        let mut g = Rng(seed | 1);
        let mut r = rig(&format!("r{seed0}-{round}"), 16 << 20);
        let mut clients = vec![];
        let mut ports = vec![1u32];
        for step in 0..400u64 {
            if g.below(40) == 0 && clients.len() < 6 {
                if let Ok(mut c) = UnixStream::connect(&r.sock) {
                    c.set_nonblocking(true).unwrap();
                    let _ = c.write(&[7; 3000]);
                    clients.push(c);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            if g.below(30) == 0 && !clients.is_empty() {
                clients.remove(0);
            }
            if g.below(3) != 0 {
                for x in r.drain() {
                    if x.2 == 1 {
                        ports.push(x.0);
                    }
                }
            }
            for c in &mut clients {
                let mut b = [0; 4096];
                if g.below(2) == 0 {
                    let _ = c.read(&mut b);
                }
            }
            let src = g.pick(&[PORT, PORT, PORT, 1, 0, u32::MAX]);
            let dst = if g.below(8) == 0 { g.next() as u32 } else { g.pick(&ports) };
            let body = g.pick(&[0usize, 0, 1, 44, 4096, 65536, 70000]);
            let len = g.pick(&[body as u32, body as u32, 0, 1, 65536, 65537, u32::MAX]);
            let typ = g.pick(&[1u16, 1, 1, 1, 3, 2, 0]);
            let op = if g.below(10) == 0 { g.next() as u16 } else { g.below(8) as u16 };
            if !fixed() && (dst == 620 || typ == 3) {
                continue; // the acceptor finding is covered by f1
            }
            let h = hdr(src, dst, len, typ, op, g.next() as u32 & 7,
                g.pick(&[0, 1, 4096, 65536, u32::MAX]), g.pick(&[0, 1, 4096, 1 << 31, u32::MAX]));
            let shape = g.below(8);
            let out = quiet(|| match shape {
                0..=4 => r.send(&h, body),
                _ => {
                    r.mem.write_slice(&h, GuestAddress(TXB)).unwrap();
                    let n = 1 + g.below(6) as u16;
                    for i in 0..n {
                        let addr = match g.below(6) {
                            0 => u64::MAX - g.below(64),
                            1 => (16 << 20) - g.below(64),
                            _ => TXB + if i == 0 { 0 } else { g.below(8192) },
                        };
                        let dl = g.pick(&[0u32, 1, 43, 44, 45, 64, 4096, 65580, u32::MAX]);
                        let mut f = if i + 1 < n { 1 } else { 0 };
                        if g.below(10) == 0 { f |= 2 }
                        if g.below(20) == 0 { f |= 1 }
                        let nx = if g.below(10) == 0 { g.next() as u16 } else { i + 1 };
                        r.desc(TXD, i, addr, dl, f, nx);
                    }
                    let head = if g.below(20) == 0 { g.below(N as u64) as u16 } else { 0 };
                    r.kick(head);
                }
            });
            total += 1;
            if let Err(e) = out {
                found.entry(e).or_insert((round, step));
                break;
            }
        }
        // every host client gone; the device must still serve a new one
        drop(clients);
        match quiet(|| r.healthy()) {
            Ok(true) => {}
            Ok(false) => { sick += 1; eprintln!("SICK round={round}"); }
            Err(e) => { found.entry(format!("post:{e}")).or_insert((round, 400)); }
        }
        round += 1;
    }
    eprintln!("RAND seed0={seed0} rounds={round} packets={total} unhealthy={sick} panics={found:#?}");
}

/// RW before RESPONSE, to a host client that does not read.
#[test]
fn f4_rw_before_response() {
    for respond in [true, false] {
        let mut r = rig(&format!("pre{respond}"), 16 << 20);
        r.drain();
        let c = UnixStream::connect(&r.sock).unwrap();
        let t = Instant::now();
        let port = loop {
            assert!(t.elapsed() < Duration::from_secs(5));
            if let Some(x) = r.drain().iter().rev().find(|x| x.2 == 1) { break x.0 }
            std::thread::sleep(Duration::from_millis(2));
        };
        if respond { r.send(&hdr(PORT, port, 0, 1, 2, 0, 65536, 0), 0); }
        let (tx, rx) = std::sync::mpsc::channel();
        let h = std::thread::spawn(move || {
            for _ in 0..16 { r.send(&hdr(PORT, port, 65536, 1, 5, 0, 65536, 0), 65536); }
            let _ = tx.send(());
            r
        });
        let done = rx.recv_timeout(Duration::from_secs(5)).is_ok();
        eprintln!("PRE respond_first={respond} device_finished_in_5s={done}");
        drop(c);
        let mut r = h.join().unwrap();
        eprintln!("PRE healthy_after_client_gone={}", r.healthy());
    }
}
