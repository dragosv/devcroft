"""Strict input contract for the experimental Alpine setup recipe."""
import hashlib
import json
from pathlib import Path
import re

SHA = re.compile(r'[0-9a-f]{64}')
APK = re.compile(r'[A-Za-z0-9][A-Za-z0-9+_.-]*\.apk')
ORIGIN = 'https://dl-cdn.alpinelinux.org/alpine/v3.23/main/aarch64/'


def digest(path):
    with Path(path).open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def validate(value):
    if not isinstance(value, dict) or set(value) != {'schema', 'architecture', 'mechanism', 'base_sha256',
                                                   'source_sha256', 'manager_packages', 'packages'}:
        raise ValueError('unexpected recipe fields')
    if type(value['schema']) is not int or value['schema'] != 1 or value['architecture'] != 'aarch64-linux' or value['mechanism'] != 'alpine-apk-offline':
        raise ValueError('unsupported recipe schema/architecture/mechanism')
    if not all(isinstance(value[key], str) and SHA.fullmatch(value[key]) for key in ('base_sha256', 'source_sha256')):
        raise ValueError('recipe base/source must have fixed SHA-256 inputs')
    known = {}
    for group in ('manager_packages', 'packages'):
        items = value[group]
        if not isinstance(items, list) or not 1 <= len(items) <= 32:
            raise ValueError('bounded nonempty APK list required')
        seen = set()
        for item in items:
            if not isinstance(item, dict) or set(item) != {'file', 'url', 'sha256'}:
                raise ValueError('unexpected APK fields')
            file = item['file']
            if not isinstance(file, str) or not APK.fullmatch(file) or file in seen:
                raise ValueError('invalid or duplicate APK filename')
            if item['url'] != ORIGIN + file or not isinstance(item['sha256'], str) or not SHA.fullmatch(item['sha256']):
                raise ValueError('APK requires an exact official URL and SHA-256')
            if file in known and known[file] != item['sha256']:
                raise ValueError('conflicting APK identities')
            seen.add(file)
            known[file] = item['sha256']
    return value


def encoded(value):
    return (json.dumps(validate(value), sort_keys=True, separators=(',', ':')) + '\n').encode()


def load(path):
    data = Path(path).read_bytes()
    if len(data) > 65536:
        raise ValueError('recipe exceeds 64 KiB')
    return validate(json.loads(data))


def verify_inputs(value, folder):
    for item in validate(value)['manager_packages'] + value['packages']:
        file = Path(folder) / item['file']
        if file.is_symlink() or not file.is_file() or digest(file) != item['sha256']:
            raise ValueError(f'APK content mismatch: {item["file"]}')
