import socket
import threading
import time
import unittest

from smoke import line


class ControlFrames(unittest.TestCase):
    def test_pipeline_keeps_next_frame_and_restores_timeout(self):
        reader, writer = socket.socketpair()
        with reader, writer:
            reader.settimeout(2)
            writer.sendall(b'first\nsecond\n')
            self.assertEqual(line(reader), b'first\n')
            self.assertEqual(reader.gettimeout(), 2)
            self.assertEqual(line(reader), b'second\n')

    def test_exact_limit_and_oversize(self):
        reader, writer = socket.socketpair()
        with reader, writer:
            writer.sendall(b'x' * 1023 + b'\n')
            self.assertEqual(len(line(reader)), 1024)
            writer.sendall(b'x' * 1024 + b'\n')
            with self.assertRaisesRegex(ValueError, 'exceeds'):
                line(reader)

    def test_partial_eof_is_not_a_frame(self):
        reader, writer = socket.socketpair()
        with reader, writer:
            writer.sendall(b'partial')
            writer.shutdown(socket.SHUT_WR)
            with self.assertRaisesRegex(ValueError, 'closed'):
                line(reader)

    def test_dribbling_does_not_extend_deadline(self):
        reader, writer = socket.socketpair()
        with reader, writer:
            done = threading.Event()

            def dribble():
                while not done.wait(0.02):
                    writer.sendall(b'x')

            worker = threading.Thread(target=dribble)
            worker.start()
            started = time.monotonic()
            try:
                with self.assertRaises(TimeoutError):
                    line(reader, deadline=started + 0.15)
            finally:
                done.set()
                worker.join(timeout=1)
            self.assertLess(time.monotonic() - started, 0.8)


if __name__ == '__main__':
    unittest.main()
