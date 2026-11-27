#!/usr/bin/env python3
"""Turn exec logs into an option corpus (`add-own-userland` 5.1).

An exec log is what the macOS redirect library writes when `up` runs with
`DEVCROFT_LOG_EXECS` set: `.devcroft/<name>/run/exec.log`, one line per exec,
the path as the caller named it and then each argument, tab-separated, with
`\\`, tab and newline escaped.

    python3 scripts/exec-corpus.py LOG [LOG ...] [--only grep,sed,find]

prints, per command name (the base name of argv[0]), how often it ran and
every option token it was given, with counts: an argument starting with `-`,
up to a `--`. `find`'s primaries (`-name`, `-type`) count as options, which
is what qualifying `findutils` needs. Operands are not printed: they are the
agent's file names and patterns, not the utility's interface.

A command the shell did not find is not in the log (bash searches `PATH`
itself and never execs), so the corpus says what ran, not what was wanted.
"""

import argparse
import collections
import os
import sys


def unescape(field):
    out, i = [], 0
    while i < len(field):
        c = field[i]
        if c == "\\" and i + 1 < len(field):
            out.append({"\\": "\\", "t": "\t", "n": "\n"}.get(field[i + 1], field[i + 1]))
            i += 2
        else:
            out.append(c)
            i += 1
    return "".join(out)


def records(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            fields = line.rstrip("\n").split("\t")
            truncated = fields[-1] == "[truncated]"
            if truncated:
                fields.pop()
            fields = [unescape(x) for x in fields]
            if len(fields) < 2:
                continue
            yield fields[0], fields[1:], truncated


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("logs", nargs="+")
    p.add_argument("--only", help="comma-separated command names to report")
    a = p.parse_args()
    only = set(a.only.split(",")) if a.only else None

    runs = collections.Counter()
    options = collections.defaultdict(collections.Counter)
    sources = collections.defaultdict(set)
    truncated = 0
    for log in a.logs:
        for path, argv, cut in records(log):
            truncated += cut
            name = os.path.basename(argv[0] or path)
            if only and name not in only:
                continue
            runs[name] += 1
            sources[name].add(log)
            for arg in argv[1:]:
                if arg == "--":
                    break
                if arg.startswith("-") and arg != "-":
                    options[name][arg] += 1

    print(f"# Exec corpus: {len(a.logs)} log(s)\n")
    print("| command | runs | logs | options (count) |")
    print("|---|---:|---:|---|")
    for name, n in sorted(runs.items(), key=lambda kv: (-kv[1], kv[0])):
        opts = ", ".join(
            f"`{o}` ({c})" for o, c in sorted(options[name].items(), key=lambda kv: (-kv[1], kv[0]))
        )
        print(f"| `{name}` | {n} | {len(sources[name])} | {opts} |")
    if truncated:
        print(f"\n{truncated} line(s) were truncated by the library; their last arguments are missing.")


if __name__ == "__main__":
    sys.exit(main())
