# The oracle

What the original programs do, recorded so the Rust port can be compared with them rather than
with anyone's reading of them.

| Recording | Source | Recorded by | Compared by |
| --- | --- | --- | --- |
| `golden/ledger.jsonl.gz` | `store.py` of [omarchy-squat-break](https://github.com/braga1k/omarchy-squat-break) at `bff0771`, unmodified | `driver.py` | `crates/core/tests/oracle.rs` |
| `golden/motion.json` | `src/motion.cs` of [1nstall](https://github.com/braga1k/1nstall) at `4e4dc64`, unmodified (SHA-256 in `motion/SOURCE.sha256`) | `motion/record.ps1` | `crates/core/tests/motion.rs` |

## The ledger

`driver.py` loads `store.py` and calls its own `run()` for every step, then records the answer,
any exception and the ledger file left on disk. 2,380 cases, 23,309 steps, from a fixed seed:

- 140 hostile ledgers (truncated JSON, a BOM, `NaN`, huge integers, `1.0` for `1`, invalid and
  leap days, a broken undo entry, lone surrogates, duplicate keys, 2,000,000 and 2,000,001 bytes,
  every missing key), each met by every action;
- 900 random lives of a ledger: integer and fractional clocks, steps around each deadline,
  suspensions of days, the clock set back, and odd numbers typed as arguments (`1_0`, `+5`,
  Arabic-Indic digits, `0x10`, `1e3`).

The Rust ledger must give the same answer at every step: refused or not, the same report, the same
bytes on disk. The Python exception type is not compared: `main()` prints one message for all of them.

To record again (deterministic: the same bytes every time):

```sh
python3 -X utf8 oracle/driver.py path/to/store.py ledger.jsonl
```

On Windows the driver stands in for the two POSIX-only calls (`fcntl.flock`, the directory
`fsync`) and for text mode's `\r\n`. The CI job `oracle` records again on Linux, where `store.py`
needs no help, and requires identical output.

## The motion

`motion/record.ps1` downloads `motion.cs`, compiles it with `PoseDump.cs` using the C# compiler
that ships with Windows, and writes 41 keyframes for each of 48 flights (three presets, eight
starting poses including interrupted ones, both directions) plus the spring curve.
