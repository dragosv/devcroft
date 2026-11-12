#!/usr/bin/env python3
"""A full-duplex Unix-socket ProxyCommand with stdin EOF as a half-close.

Keep the directions on separate threads, as the product SSH proxy does. A
single blocking read/write loop can stall both directions during large bursts.
This fixture handles bytes only; SSH itself supplies authentication/encryption.
"""
import os
import socket
import sys
import threading


def main():
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.connect(sys.argv[1])

    def send():
        try:
            while chunk := os.read(0, 65536):
                connection.sendall(chunk)
        except OSError:
            pass
        finally:
            try:
                connection.shutdown(socket.SHUT_WR)
            except OSError:
                pass

    threading.Thread(target=send, daemon=True).start()
    try:
        while chunk := connection.recv(65536):
            remaining = memoryview(chunk)
            while remaining:
                remaining = remaining[os.write(1, remaining):]
    except (BrokenPipeError, ConnectionError):
        pass
    finally:
        try:
            connection.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        connection.close()


if __name__ == '__main__':
    main()
