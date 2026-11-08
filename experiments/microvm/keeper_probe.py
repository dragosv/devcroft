"""Real keeper protocol probes over the helper's fixed Unix/vsock endpoint."""
import json
import socket
import struct
import time


def send(sock, tag, value=b""):
    if not isinstance(value, bytes):
        value = json.dumps(value).encode()
    sock.sendall(struct.pack(">BI", tag, len(value)) + value)


def exact(sock, size):
    data = bytearray()
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise EOFError("keeper closed mid-frame")
        data.extend(chunk)
    return bytes(data)


def receive(sock):
    tag, size = struct.unpack(">BI", exact(sock, 5))
    if size > 1024 * 1024:
        raise ValueError("keeper response exceeded probe budget")
    payload = exact(sock, size)
    return tag, payload if tag in (6, 7) else json.loads(payload)


def connect(path):
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(8)
    sock.connect(str(path))
    return sock


def spawn(path, cmd, args=(), pty=None, env=None):
    sock = connect(path)
    try:
        send(sock, 1, {"cmd": cmd, "args": list(args), "cwd": "/workspace",
                       "env": env or {}, "pty": pty})
        tag, payload = receive(sock)
        if tag != 2:
            raise AssertionError(f"keeper spawn failed: {tag}: {payload}")
        return sock, payload["session_id"]
    except BaseException:
        sock.close()
        raise


def finish(sock):
    output = {6: bytearray(), 7: bytearray()}
    while True:
        tag, value = receive(sock)
        if tag == 10:
            return bytes(output[6]), bytes(output[7]), value
        if tag not in output:
            raise AssertionError(f"unexpected session frame: {tag}")
        output[tag].extend(value)
        if sum(map(len, output.values())) > 1024 * 1024:
            raise ValueError("keeper session output exceeded budget")


def execute(path, script, stdin=b"", env=None):
    sock, _ = spawn(path, "/bin/sh", ["-c", script], env=env)
    with sock:
        if stdin:
            send(sock, 4, stdin)
        send(sock, 5)
        return finish(sock)


def query(path):
    with connect(path) as sock:
        send(sock, 11)
        tag, value = receive(sock)
        assert tag == 12, (tag, value)
        return value


def run(path):
    result = {"status": "failed", "implementation": "src/keeper", "nono": "0.77.0"}
    phase = "exec"
    try:
        out, err, status = execute(path, "printf 'output'; printf 'error' >&2; exit 23")
        assert (out, err, status) == (b"output", b"error", {"code": 23, "signal": None}), (out, err, status)
        result["exec_stdout_stderr_exit"] = True
        phase = "stdin-eof"
        data = bytes(range(256)) * 512
        out, err, status = execute(path, "cat", stdin=data)
        assert (out, err, status) == (data, b"", {"code": 0, "signal": None}), (len(out), err, status)
        result["stdin_binary_eof_bytes"] = len(data)
        phase = "environment-cwd"
        out, err, status = execute(path,
            "printf '%s|%s|%s|%s|' \"$DEVCROFT_TASK\" \"${DEVCROFT_SSH_HOST_KEY-unset}\" "
            "\"${DEVCROFT_SHELL-unset}\" \"$PROBE_VALUE\"; pwd", env={"PROBE_VALUE": "request-value"})
        assert out == b"fixture-task|unset|unset|request-value|/workspace\n" and not err and status["code"] == 0, (out, err, status)
        result["environment_filter_overlay_cwd"] = True
        phase = "pty-resize"
        sock, _ = spawn(path, "/bin/sh", ["-c",
            "stty size; printf 'PTY_READY\\n'; read token; stty size; printf '%s\\n' \"$token\"; exit 13"],
            pty={"rows": 24, "cols": 80})
        with sock:
            initial = bytearray()
            while b"PTY_READY\r\n" not in initial:
                tag, value = receive(sock)
                assert tag == 6, (tag, value)
                initial.extend(value)
                assert len(initial) < 4096
            assert b"24 80\r\n" in initial, initial
            send(sock, 8, {"rows": 41, "cols": 132})
            send(sock, 4, b"resized-token\n")
            out, err, status = finish(sock)
            assert b"41 132\r\n" in out and b"resized-token\r\n" in out and not err, (out, err)
            assert status == {"code": 13, "signal": None}, status
        result["pty_resize"] = {"initial": [24, 80], "resized": [41, 132], "exit_code": 13}
        phase = "signals"
        result["signals"] = {}
        for name, number in [("Int", 2), ("Term", 15), ("Hup", 1)]:
            sock, session_id = spawn(path, "/bin/sleep", ["30"])
            with sock:
                assert any(s["id"] == session_id for s in query(path)["sessions"])
                send(sock, 9, name)
                out, err, status = finish(sock)
                assert status == {"code": None, "signal": number}, (name, out, err, status)
            result["signals"][name] = number
        phase = "nono-filesystem"
        out, err, status = execute(path, "printf 'allowed-write' > /workspace/probe; cat /workspace/probe")
        assert out == b"allowed-write" and not err and status["code"] == 0, (out, err, status)
        out, err, status = execute(path, "cat /private/credential")
        assert not out and b"Permission denied" in err and status["code"] != 0, (out, err, status)
        out, err, status = execute(path, "printf tamper >> /bin/readonly-data")
        assert not out and b"Permission denied" in err and status["code"] != 0, (out, err, status)
        out, err, status = execute(path, "head -c 4 /bin/busybox")
        assert out == b"\x7fELF" and not err and status["code"] == 0, (out, err, status)
        out, err, status = execute(path, "cat /bin/readonly-data")
        assert out == b"runtime-read-control\n" and not err and status["code"] == 0, (out, err, status)
        out, err, status = execute(path, "nc -l -p 12345")
        assert not out and b"Permission denied" in err and status["code"] != 0, (out, err, status)
        result["nono_inherited_policy"] = {"workspace_rw": True, "private_read_denied": True,
                                           "runtime_data_write_denied": True,
                                           "tool_and_runtime_data_read_allowed": True,
                                           "tcp_bind_denied": True}
        phase = "disconnect-cleanup"
        sock, session_id = spawn(path, "/bin/sleep", ["30"])
        sock.close()
        deadline = time.monotonic() + 7
        while True:
            live = query(path)["sessions"]
            if not any(s["id"] == session_id for s in live):
                break
            if time.monotonic() >= deadline:
                raise TimeoutError("disconnected session survived grace period")
            time.sleep(0.05)
        result["disconnect_reaps_session"] = True
        phase = "malformed-frame"
        for header in [struct.pack(">BI", 255, 0), struct.pack(">BI", 1, 16 * 1024 * 1024 + 1)]:
            with connect(path) as sock:
                sock.sendall(header)
                try:
                    assert sock.recv(1) == b"", "malformed keeper frame accepted"
                except ConnectionResetError:
                    pass
            assert "sessions" in query(path), "keeper stopped after invalid connection"
        result["unknown_oversized_frames_rejected"] = True
        result["status"] = "passed"
    except Exception as e:
        result.update(phase=phase, error=f"{type(e).__name__}: {e}")
    return result
