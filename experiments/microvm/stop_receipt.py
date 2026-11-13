"""Experimental cooperative stop correlation; no production supervisor or attestation."""
from dataclasses import dataclass, asdict
import json
from pathlib import Path
import re
import secrets
import subprocess

TOKEN = re.compile(r'[0-9a-f]{64}')


@dataclass(frozen=True)
class HelperIdentity:
    pid: int
    start_abstime: int

    def __post_init__(self):
        if type(self.pid) is not int or type(self.start_abstime) is not int or min(self.pid, self.start_abstime) <= 0:
            raise ValueError('positive native helper PID and creation identity required')


def owned_identity(process, observer, evidence=None):
    """Query only the owned, unreaped child through the existing native observer."""
    if process.poll() is not None:
        raise RuntimeError('owned helper exited before identity query')
    result = subprocess.run([str(observer)], input=f'{process.pid}\n'.encode(),
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5, check=True)
    rows = [json.loads(line) for line in result.stdout.splitlines()]
    if evidence is not None:
        Path(evidence).write_bytes(result.stdout)
    if len(rows) != 2 or rows[0].get('event') != 'calibration':
        raise RuntimeError('native process identity observer did not calibrate')
    records = rows[1].get('processes', [])
    if len(records) != 1 or records[0].get('pid') != process.pid or 'error' in records[0]:
        raise RuntimeError('native process identity unavailable')
    identity = HelperIdentity(process.pid, records[0]['start_abstime'])
    if process.poll() is not None:
        raise RuntimeError('owned helper exited during identity query')
    return identity


class StopReceipt:
    def __init__(self, identity, generation, nonce=None):
        if not isinstance(identity, HelperIdentity) or type(generation) is not int or not 1 <= generation <= 1000001:
            raise ValueError('owned helper identity and bounded guest boot generation required')
        self.identity, self.generation = identity, generation
        self.nonce = secrets.token_hex(32) if nonce is None else nonce
        if not isinstance(self.nonce, str) or TOKEN.fullmatch(self.nonce) is None:
            raise ValueError('64 lowercase hexadecimal stop nonce required')
        self.intent = False
        self.acknowledged = False

    def request(self, live_identity):
        if live_identity != self.identity:
            raise ValueError('helper identity changed before stop request')
        if self.intent:
            raise ValueError('stop intent already recorded for this receipt')
        self.intent = True
        return f'SHUTDOWN {self.generation} {self.nonce}\n'.encode()

    def accept_ack(self, identity, packet):
        expected = f'SYNCED {self.generation} {self.nonce}\n'.encode()
        if (not self.intent or self.acknowledged or identity != self.identity
                or not isinstance(packet, bytes) or len(packet) > 160 or packet != expected):
            return False
        self.acknowledged = True
        return True

    def classify(self, identity, exit_code, forced=False):
        if identity != self.identity:
            return 'identity-mismatch'
        if type(exit_code) is not int:
            raise ValueError('a reaped child exit code is required')
        if forced:
            return 'forced-stop'
        return 'orderly' if self.intent and self.acknowledged and exit_code == 0 else 'crash'

    def record(self):
        return {'helper_identity': asdict(self.identity), 'guest_generation': self.generation,
                'nonce': self.nonce, 'stop_intent': self.intent, 'sync_acknowledged': self.acknowledged,
                'qualification': 'cooperative fixture sync receipt; not companion service-stop/unmount, guest attestation or supervisor recovery'}
