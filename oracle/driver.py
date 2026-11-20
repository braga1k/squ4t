#!/usr/bin/env python3
"""Records what the original store.py does, so the Rust port can be compared with it.

    python3 -X utf8 oracle/driver.py <path to store.py> oracle/golden/ledger.jsonl

store.py is loaded as it is, never edited. Every step goes through its own run():
validation, the operation, and the file it leaves on disk.

store.py needs fcntl and a directory fsync, which only exist on POSIX. On another
system this script stands in for those two calls and nothing else; CI runs the
real thing on Linux and requires the same bytes as the committed file.
"""
import datetime as dt
import importlib.util
import json
import os
import sys
import tempfile
from pathlib import Path

SEED = 0x5157_4154  # "SQAT"
SESSIONS = 900
BASE = 1_800_000_000


def load_store(path):
    shimmed = False
    try:
        import fcntl  # noqa: F401
    except ImportError:
        import types
        stub = types.ModuleType('fcntl')
        stub.LOCK_EX = 2
        stub.flock = lambda handle, mode: None
        sys.modules['fcntl'] = stub
        shimmed = True
    if not hasattr(os, 'O_DIRECTORY'):
        # run() opens the directory only to fsync it; hand back a token for that call alone.
        token = -77
        os.O_DIRECTORY = 1 << 40
        real_open, real_fsync, real_close = os.open, os.fsync, os.close
        os.open = lambda p, flags, *a, **k: token if flags == os.O_DIRECTORY else real_open(p, flags, *a, **k)
        os.fsync = lambda fd: None if fd == token else None  # durability is not what is recorded
        os.close = lambda fd: None if fd == token else real_close(fd)
        # Text mode on Windows writes \r\n; on POSIX store.py writes \n.
        real_fdopen = os.fdopen
        os.fdopen = lambda fd, mode='r', *a, **k: real_fdopen(fd, mode, *a, **dict(k, newline='\n'))
        shimmed = True
    spec = importlib.util.spec_from_file_location('store', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module, shimmed


class Random:
    """SplitMix64: the same numbers on every Python version and platform."""

    def __init__(self, seed):
        self.state = seed & 0xFFFFFFFFFFFFFFFF

    def next(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
        return z ^ (z >> 31)

    def below(self, n):
        return self.next() % n

    def between(self, low, high):
        return low + self.below(high - low + 1)

    def chance(self, percent):
        return self.below(100) < percent

    def pick(self, items):
        return items[self.below(len(items))]


def ledger(**changes):
    base = dict(version=1, interval=60, suggested=20, paused=False, due=False,
                next_at=BASE + 3600, remaining=3600, days={'2027-01-15': 40}, last=None)
    base.update(changes)
    return json.dumps(base, sort_keys=True) + '\n'


def without(key):
    base = json.loads(ledger())
    del base[key]
    return json.dumps(base)


# Ledgers a person, another program or a broken disk could leave behind.
HOSTILE = [
    '', ' ', '{broken', '[]', 'null', '42', '"text"', '{}',
    '{"version": 42}', '{"version": "1"}', '{"version": 2}',
    ledger(), ledger().strip(), ledger().replace(', ', ',').replace(': ', ':'),
    '﻿' + ledger(), ledger() + 'trailing',
    ledger(version=1.0), ledger(version=True), ledger(version=None),
    ledger(interval=0), ledger(interval=1441), ledger(interval=60.0), ledger(interval=True), ledger(interval='60'),
    ledger(interval=1), ledger(interval=1440),
    ledger(suggested=0), ledger(suggested=1001), ledger(suggested=1.5), ledger(suggested=1000),
    ledger(paused=1), ledger(paused=None), ledger(due=0), ledger(due='false'),
    ledger(paused=True, remaining=12.75), ledger(paused=True, remaining=0), ledger(paused=True, due=True),
    ledger(next_at=-1), ledger(next_at=-0.0), ledger(next_at=1e13), ledger(next_at=10_000_000_000_001),
    ledger(next_at=1e13 + 1), ledger(next_at=True), ledger(next_at='soon'), ledger(next_at=None),
    ledger(next_at=BASE + 0.1), ledger(next_at=0),
    ledger(remaining=-1), ledger(remaining=1e13), ledger(remaining=2e13), ledger(remaining=0.5), ledger(remaining=[]),
    ledger().replace('"remaining": 3600', '"remaining": NaN'),
    ledger().replace('"remaining": 3600', '"remaining": Infinity'),
    ledger().replace('"remaining": 3600', '"remaining": 1e999'),
    ledger().replace('"remaining": 3600', '"remaining": 36e2'),
    ledger().replace('"remaining": 3600', '"remaining": 3.6E+3'),
    ledger().replace('"remaining": 3600', '"remaining": 1e-7'),
    ledger().replace('"remaining": 3600', '"remaining": 0.0001'),
    ledger().replace('"remaining": 3600', '"remaining": 123456789.123456789'),
    ledger().replace('"interval": 60', '"interval": 100000000000000000000000000'),
    ledger().replace('"interval": 60', '"interval": -0'),
    ledger().replace('"interval": 60', '"interval": 060'),
    ledger().replace('"interval": 60', '"interval": 60, "interval": 45'),
    ledger().replace('"interval": 60', '"interval": 45, "interval": 6000'),
    ledger(days=[]), ledger(days=None), ledger(days={}), ledger(days={'<img src=x>': 20}),
    ledger(days={'2027-1-15': 1}), ledger(days={'2027-02-29': 1}), ledger(days={'2028-02-29': 1}),
    ledger(days={'1900-02-29': 1}), ledger(days={'2000-02-29': 1}),
    ledger(days={'0000-01-01': 1}), ledger(days={'0001-01-01': 1}), ledger(days={'9999-12-31': 1}),
    ledger(days={'20270115': 1}), ledger(days={'2027-01-15T00:00:00': 1}), ledger(days={'2027-W02-5': 1}),
    ledger(days={' 2027-01-15': 1}), ledger(days={'2027-13-01': 1}), ledger(days={'2027-04-31': 1}),
    ledger(days={'2027-01-15': -1}), ledger(days={'2027-01-15': 1.0}), ledger(days={'2027-01-15': True}),
    ledger(days={'2027-01-15': 0}), ledger(days={'2027-01-15': 9_000_000_000_000}),
    ledger(days={'2027-01-15': 9_000_000_000_001}),
    ledger(days={'2027-01-14': 4_500_000_000_000, '2027-01-15': 4_500_000_000_000}),
    ledger(days={'2027-01-14': 4_500_000_000_000, '2027-01-15': 4_500_000_000_001}),
    ledger(days={'2027-01-15': 8_999_999_999_990}),
    ledger(days={'2026-12-%02d' % d: d for d in range(1, 12)}),
    ledger(last={'day': '2027-01-15', 'count': 20}), ledger(last={'day': '2027-01-15', 'count': 40}),
    ledger(last={'day': '2027-01-15', 'count': 41}), ledger(last={'day': '2027-01-14', 'count': 20}),
    ledger(last={'day': '2027-01-15', 'count': 0}), ledger(last={'day': '2027-01-15', 'count': 1001}),
    ledger(last={'day': '2027-01-15', 'count': 1.0}), ledger(last={'day': '2027-01-15'}), ledger(last={'count': 20}),
    ledger(last={'day': 5, 'count': 20}), ledger(last={'day': ['2027-01-15'], 'count': 20}),
    ledger(last={'day': None, 'count': 20}), ledger(last=[]), ledger(last=False), ledger(last=0), ledger(last='x'),
    ledger(last={'day': '2027-01-15', 'count': 20, 'note': 'kept until the next entry'}),
    ledger(days={'2027-01-15': 1000, '2027-01-14': 5}, last={'day': '2027-01-14', 'count': 5}),
    ledger(note='an extra field survives'), ledger(extra={'b': [1, 2.0, None, True, {'z': 1, 'a': 'é"\\\n\t\u0001'}], 'a': 1e16}),
    ledger(extra='café \U0001F3CB  '), ledger(extra=1e22), ledger(extra=-1.5e-10), ledger(extra=5e-324),
    ledger(extra=123456789012345678901234567890), ledger(extra=-0.0), ledger(extra=2**63), ledger(extra=-2**63),
    ledger().replace('"version": 1', '"version": 1, "extra": "\\ud83c\\udfcb"'),
    ledger().replace('"version": 1', '"version": 1, "extra": "\\ud800"'),
    ledger().replace('"version": 1', '"version": 1, "extra": "\\u00e9\\/\\b\\f"'),
    ledger().replace('"version": 1', '"version": 1, "extra": -Infinity'),
    ledger().replace('"version": 1', '"version": 1, "extra": [1,]'),
    ledger().replace('"version": 1', '"version": 1, "extra": "tab\there"'),
    ledger().replace('"version": 1', '"version": 1, "extra": 1.'),
    ledger().replace('"version": 1', '"version": 1, "extra": .5'),
    ledger().replace('"version": 1', '"version": 1, "extra": +1'),
    ledger().replace('"version": 1', '"version": 1, "extra": 1E5'),
    ledger().replace('"version": 1', "\"version\": 1, 'extra': 1"),
    ledger().replace('{', '{\r\n\t ').replace('}', '\n}'),
    ledger().replace('{', '{\x0c', 1), ledger().replace('{', '{ ', 1),
    ledger(extra='[' * 5)[:-1] + ' ' * 1_999_800 + '\n',
    ledger(extra='x' * 2_000_100),
] + [without(key) for key in ('version', 'interval', 'suggested', 'paused', 'due', 'next_at', 'remaining', 'days', 'last')]



class Padded:
    """A ledger made large with trailing spaces, written down as its size rather than its text."""

    def __init__(self, size):
        self.size = size
        self.text = ledger().rstrip().ljust(size - 1) + chr(10)
        assert len(self.text.encode('utf-8')) == size


# store.py refuses a ledger above 2,000,000 bytes; one byte either side of that.
HOSTILE += [Padded(2_000_000), Padded(2_000_001)]

PROBE = [('status', []), ('tick', []), ('add', ['20']), ('undo', []), ('pause', []), ('snooze', []),
         ('resume', []), ('settings', ['45', '15']), ('settings', ['45', '10']), ('add', ['1000'])]

BAD_NUMBERS = ['0', '-1', '1001', '1441', '<img src=x>', '', ' ', '1.5', '0x10', '1e3', 'ten', '--5', '5-', '1__0',
               '_5', '5_', '99999999999999999999999999', 'None', '١٢']
ODD_NUMBERS = ['1_0', ' 7 ', '+5', '007', '\t12\n', '1_000', '+1_0']


def day(now):
    return dt.datetime.fromtimestamp(now, dt.timezone.utc).date().isoformat()


def number(rng, low, high):
    roll = rng.below(100)
    if roll < 4:
        return rng.pick(BAD_NUMBERS)
    if roll < 7:
        return rng.pick(ODD_NUMBERS)
    if roll < 12:
        return str(rng.pick([low, high]))
    return str(rng.between(low, high))


def text_of(initial):
    return initial.text if isinstance(initial, Padded) else initial


def play(store, directory, initial, steps):
    path = Path(directory) / 'ledger.json'
    if initial is not None:
        path.write_text(text_of(initial), encoding='utf-8', newline='')
    recorded, previous = [], text_of(initial)
    for action, args, now, today in steps:
        entry = dict(action=action, args=args, now=now, today=today)
        try:
            entry['result'] = store.run(directory, action, args, now, today)
        except Exception as error:  # main() turns the ones it expects into exit 1; any other ends the process too
            entry['error'] = type(error).__name__
        try:
            current = path.read_bytes().decode('utf-8') if path.exists() else None
        except UnicodeDecodeError:
            current = None
        if current != previous:
            entry['ledger'] = current
        previous = current
        leftovers = sorted(p.name for p in Path(directory).iterdir() if p.name not in ('ledger.json', 'ledger.lock'))
        if leftovers:
            entry['leftovers'] = leftovers
        recorded.append(entry)
        yield entry
    return recorded


def session(rng, store, directory, initial):
    fractional = rng.chance(45)
    mixed = rng.chance(12)
    now = BASE + rng.below(400 * 86400)
    if fractional:
        now += rng.below(1_000_000) / 1_000_000
    remaining = None
    for _ in range(rng.between(1, 34)):
        roll = rng.below(100)
        if roll < 34:
            action, args = 'tick', []
        elif roll < 55:
            action, args = 'add', [number(rng, 1, 1000)]
        elif roll < 63:
            action, args = 'undo', []
        elif roll < 70:
            action, args = 'pause', []
        elif roll < 77:
            action, args = 'resume', []
        elif roll < 83:
            action, args = 'snooze', []
        elif roll < 93:
            action, args = 'settings', [number(rng, 1, 1440), number(rng, 1, 1000)]
        else:
            action, args = 'status', []
        roll = rng.below(100)
        if roll < 14:
            step = 0
        elif roll < 40:
            step = rng.between(1, 30)
        elif roll < 68 and remaining is not None:
            step = remaining + rng.pick([-2, -1, 0, 0, 1, 2])  # around the deadline
        elif roll < 82:
            step = rng.between(31, 7200)
        elif roll < 93:
            step = rng.between(7201, 3 * 86400)  # suspended, or switched off
        elif roll < 97:
            step = -rng.between(1, 7200)  # the clock was set back
        else:
            step = rng.between(3 * 86400, 60 * 86400)
        now += step
        if fractional and rng.chance(60):
            now = float(int(now)) + rng.below(1_000_000) / 1_000_000
        if mixed and rng.chance(30):
            now = int(now) if isinstance(now, float) else float(now)
        if now < 0:
            now = 0
        entry = yield action, args, now, day(now)
        result = entry.get('result') if entry else None
        remaining = result['remaining'] if result else None


def main():
    store_path, out_path = sys.argv[1], sys.argv[2]
    store, shimmed = load_store(store_path)
    rng = Random(SEED)
    cases = steps_total = errors = 0
    with open(out_path, 'w', encoding='utf-8', newline='\n') as out:
        def emit(initial, entries):
            nonlocal cases, steps_total, errors
            cases += 1
            steps_total += len(entries)
            errors += sum('error' in e for e in entries)
            if isinstance(initial, Padded):
                initial = dict(padded_to=initial.size)
            out.write(json.dumps(dict(id=cases, initial=initial, steps=entries), ensure_ascii=False, sort_keys=True) + '\n')

        # 1. Every hostile ledger, met by every action.
        for initial in HOSTILE:
            for index, (action, args) in enumerate(PROBE):
                with tempfile.TemporaryDirectory() as directory:
                    now = BASE + 100 + index
                    plan = [(action, args, now, '2027-01-15'), ('status', [], now + 1, '2027-01-15'),
                            ('tick', [], BASE + 7200.5, '2027-01-15'), ('add', ['7'], BASE + 7300, '2027-01-16'),
                            ('undo', [], BASE + 7301, '2027-01-16')]
                    emit(initial, list(play(store, directory, initial, plan)))

        # 2. Random lives of a ledger, from nothing or from a hostile start.
        for _ in range(SESSIONS):
            initial = rng.pick(HOSTILE) if rng.chance(10) else None
            if isinstance(initial, Padded):
                initial = None
            with tempfile.TemporaryDirectory() as directory:
                script = session(rng, store, directory, initial)
                entries, entry = [], None
                player_path = Path(directory) / 'ledger.json'
                if initial is not None:
                    player_path.write_text(initial, encoding='utf-8', newline='')
                previous = initial
                while True:
                    try:
                        action, args, now, today = script.send(entry)
                    except StopIteration:
                        break
                    entry = dict(action=action, args=args, now=now, today=today)
                    try:
                        entry['result'] = store.run(directory, action, args, now, today)
                    except Exception as error:
                        entry['error'] = type(error).__name__
                    current = player_path.read_bytes().decode('utf-8') if player_path.exists() else None
                    if current != previous:
                        entry['ledger'] = current
                    previous = current
                    entries.append(entry)
                emit(initial, entries)
    print(f'oracle: {cases} cases, {steps_total} steps, {errors} refused; '
          f'{"POSIX calls stood in for" if shimmed else "store.py unassisted"}', file=sys.stderr)


if __name__ == '__main__':
    main()
