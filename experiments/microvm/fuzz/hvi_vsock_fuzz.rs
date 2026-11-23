#[cfg(test)]
mod fuzz_tests {
    use super::*;
    const B: u64 = 0x4000_0000;
    struct G(u64);
    impl G {
        fn n(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn b(&mut self, k: u64) -> u64 {
            self.n() % k
        }
        fn p<T: Copy>(&mut self, v: &[T]) -> T {
            v[self.b(v.len() as u64) as usize]
        }
    }
    fn prog(d: &mut VirtioVsock, m: &GuestRam, q: u16, base: u64) {
        for (o, v) in [
            (mmio::QUEUE_SEL, u64::from(q)),
            (mmio::QUEUE_NUM, 8),
            (mmio::QUEUE_DESC_LOW, base),
            (mmio::QUEUE_DESC_HIGH, 0),
            (mmio::QUEUE_DRIVER_LOW, base + 0x1000),
            (mmio::QUEUE_DRIVER_HIGH, 0),
            (mmio::QUEUE_DEVICE_LOW, base + 0x2000),
            (mmio::QUEUE_DEVICE_HIGH, 0),
            (mmio::QUEUE_READY, 1),
        ] {
            d.mmio(m, o, true, v);
        }
    }
    fn desc(m: &GuestRam, base: u64, i: u16, a: u64, l: u32, f: u16, n: u16) {
        let d = base + u64::from(i) * 16;
        m.write_u64(d, a).unwrap();
        m.write_u32(d + 8, l).unwrap();
        m.write_u16(d + 12, f).unwrap();
        m.write_u16(d + 14, n).unwrap();
    }
    fn avail(m: &GuestRam, base: u64, head: u16) {
        let i = m.read_u16(base + 0x1002).unwrap();
        m.write_u16(base + 0x1004 + u64::from(i % 8) * 2, head).unwrap();
        m.write_u16(base + 0x1002, i.wrapping_add(1)).unwrap();
    }
    fn hdr(g: &mut G, dst: u32, len: u32, op: u16) -> Hdr {
        Hdr {
            src_cid: g.p(&[3, 3, 3, 3, 3, 2, 9]),
            dst_cid: g.p(&[2, 2, 2, 2, 2, 2, 5]),
            src_port: g.p(&[AGENT_PORT, AGENT_PORT, AGENT_PORT, AGENT_PORT, 0, u32::MAX]),
            dst_port: dst,
            len,
            typ: g.p(&[1, 1, 1, 1, 1, 2, 3]),
            op,
            flags: g.b(8) as u32,
            buf_alloc: g.p(&[0, 1, 4096, 65536, u32::MAX]),
            fwd_cnt: g.p(&[0, 0, 1, 4096, 1 << 31, u32::MAX]),
        }
    }
    const TXB: u64 = B + 0x10000;
    fn rig() -> (GuestRam, VirtioVsock) {
        let m = GuestRam::from_ranges(&[(B, 0x100000)]);
        let mut d = VirtioVsock::new();
        prog(&mut d, &m, RX_QUEUE, B);
        prog(&mut d, &m, TX_QUEUE, B + 0x4000);
        (m, d)
    }
    fn tx(d: &mut VirtioVsock, m: &GuestRam, h: Hdr, body: u32) {
        m.write(TXB, &h.to_bytes()).unwrap();
        desc(m, B + 0x4000, 0, TXB, 44, u16::from(body > 0), 1);
        desc(m, B + 0x4000, 1, TXB + 0x1000, body, 0, 0);
        avail(m, B + 0x4000, 0);
        d.mmio(m, mmio::QUEUE_NOTIFY, true, 1);
    }
    fn healthy() -> impl Fn(&mut VirtioVsock, &GuestRam) -> bool {
        |d, m| {
            d.mmio(m, mmio::STATUS, true, 0);
            for a in [B + 0x1002, B + 0x2002, B + 0x5002, B + 0x6002] {
                m.write_u16(a, 0).unwrap();
            }
            prog(d, m, RX_QUEUE, B);
            prog(d, m, TX_QUEUE, B + 0x4000);
            let (a, mut peer) = UnixStream::pair().unwrap();
            let p = d.add_conn(a).unwrap();
            d.connect(m, p);
            let h = |op, len| Hdr {
                src_cid: 3, dst_cid: 2, src_port: AGENT_PORT, dst_port: p,
                len, typ: 1, op, buf_alloc: 65536, ..Hdr::default()
            };
            tx(d, m, h(OP_RESPONSE, 0), 0);
            m.write(TXB + 0x1000, b"pong").unwrap();
            tx(d, m, h(OP_RW, 4), 4);
            let mut b = [0; 4];
            peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            peer.read_exact(&mut b).is_ok() && &b == b"pong"
        }
    }
    #[test]
    fn fuzz_control() {
        let (m, mut d) = rig();
        assert!(healthy()(&mut d, &m));
    }
    #[test]
    fn fuzz_pending_growth() {
        for (posted, op) in [(false, OP_REQUEST), (false, OP_CREDIT_REQUEST), (true, OP_REQUEST)] {
            let (m, mut d) = rig();
            let (a, _peer) = UnixStream::pair().unwrap();
            let p = d.add_conn(a).unwrap();
            d.connect(&m, p);
            let h = |op| Hdr { src_cid: 3, dst_cid: 2, src_port: AGENT_PORT, dst_port: p, typ: 1, op, buf_alloc: 65536, ..Hdr::default() };
            tx(&mut d, &m, h(OP_RESPONSE), 0);
            for _ in 0..200_000 {
                if posted {
                    desc(&m, B, 0, B + 0x20000, 4096, VIRTQ_DESC_F_WRITE, 0);
                    avail(&m, B, 0);
                }
                tx(&mut d, &m, h(op), 0);
            }
            eprintln!("PEND rx_posted={posted} op={op} packets=200000 pending={}", d.pending.len());
        }
    }
    #[test]
    fn fuzz_random() {
        let secs: u64 = std::env::var("FUZZ_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(5);
        let seed0: u64 = std::env::var("FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
        let end = Instant::now() + Duration::from_secs(secs);
        let (mut total, mut round, mut sick) = (0u64, 0u64, 0u64);
        let mut found: HashMap<String, (u64, u64)> = HashMap::new();
        while Instant::now() < end {
            let mut g = G((seed0.wrapping_mul(0x9E3779B97F4A7C15) ^ (round + 1).wrapping_mul(0xD1B54A32D192ED03)) | 1);
            let (m, mut d) = rig();
            let mut peers = vec![];
            let mut ports = vec![1u32];
            for step in 0..1000u64 {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    match g.b(12) {
                        0 if peers.len() < 6 => {
                            let (a, p) = UnixStream::pair().unwrap();
                            p.set_nonblocking(true).unwrap();
                            let port = d.add_conn(a).unwrap();
                            ports.push(port);
                            peers.push(p);
                            d.connect(&m, port);
                        }
                        1 => { let p = g.p(&ports); let n = g.b(9000) as usize; d.host_data(&m, p, &vec![1; n]); }
                        2 => { let p = g.p(&ports); match g.b(4) { 0 => { d.host_eof(&m, p); } 1 => d.host_hung_up(&m, p), 2 => d.host_gone(&m, p), _ => { d.flush_to_host(&m, p); } } }
                        3 if !peers.is_empty() => { peers.remove(0); }
                        4 | 5 => {
                            let i = g.b(8) as u16;
                            let len = g.p(&[0u32, 1, 43, 44, 45, 64, 4096, u32::MAX]);
                            let a = g.p(&[B + 0x20000, B + 0xffff0, u64::MAX - 3, 0]);
                            let f = g.p(&[2u16, 2, 2, 3, 0, 1]);
                            desc(&m, B, i, a, len, f, g.b(10) as u16);
                            avail(&m, B, if g.b(20) == 0 { g.n() as u16 } else { i });
                            d.mmio(&m, mmio::QUEUE_NOTIFY, true, 0);
                        }
                        6 if g.b(20) == 0 => {
                            let o = g.p(&[mmio::QUEUE_SEL, mmio::QUEUE_NUM, mmio::QUEUE_READY, mmio::QUEUE_DESC_LOW, mmio::QUEUE_DRIVER_HIGH, mmio::QUEUE_DEVICE_LOW, mmio::STATUS, mmio::INTERRUPT_ACK, mmio::CONFIG + 4]);
                            let v = g.p(&[0u64, 1, 7, 8, 65535, u64::from(u32::MAX), B]);
                            let w = g.b(2) == 0;
                            d.mmio(&m, o, w, v);
                        }
                        7 => {
                            let n = 1 + g.b(6) as u16;
                            let dst = g.p(&ports);
                            let op = g.b(9) as u16;
                            let h = hdr(&mut g, dst, 4096, op);
                            m.write(TXB, &h.to_bytes()).unwrap();
                            for i in 0..n {
                                let a = g.p(&[TXB, TXB, TXB + 7, B + 0xffff0, u64::MAX - 5]);
                                let l = g.p(&[0u32, 1, 43, 44, 4096, 65580, 65581, u32::MAX]);
                                let f = u16::from(i + 1 < n) | g.p(&[0u16, 0, 0, 1, 2]);
                                let nx = if g.b(8) == 0 { g.n() as u16 } else { i + 1 };
                                desc(&m, B + 0x4000, i, a, l, f, nx);
                            }
                            avail(&m, B + 0x4000, if g.b(20) == 0 { g.n() as u16 } else { 0 });
                            d.mmio(&m, mmio::QUEUE_NOTIFY, true, 1);
                        }
                        _ => {
                            let dst = if g.b(8) == 0 { g.n() as u32 } else { g.p(&ports) };
                            let body = g.p(&[0u32, 0, 1, 4096, 65536, 70000]);
                            let len = g.p(&[body, body, 0, 1, 65537, u32::MAX]);
                            let op = if g.b(10) == 0 { g.n() as u16 } else { g.b(9) as u16 };
                            let h = hdr(&mut g, dst, len, op);
                            tx(&mut d, &m, h, body);
                        }
                    }
                }));
                total += 1;
                if let Err(e) = r {
                    let s = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    found.entry(s).or_insert((round, step));
                    break;
                }
                for p in &mut peers {
                    let mut b = [0; 4096];
                    if g.b(3) == 0 { let _ = p.read(&mut b); }
                }
                for c in d.conns.values() {
                    assert!(c.to_host.len() <= OUR_BUF_ALLOC as usize, "to_host bound");
                }
            }
            if !healthy()(&mut d, &m) { sick += 1; eprintln!("SICK {round}"); }
            round += 1;
        }
        eprintln!("RAND seed0={seed0} rounds={round} actions={total} unhealthy={sick} panics={found:#?}");
    }
}
