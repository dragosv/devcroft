# Tasks — add-not-applicable-status

## 1. The vocabulary

- [ ] 1.1 `Status::NotApplicable`, with a doc comment that says what it is
      *not*: not an unenforced capability, not an unmeasured one, not one
      devcroft declined. Those have words.
- [ ] 1.2 `Display` renders it `not-applicable`. `doctor` renders whatever
      `Display` produces, so nothing there should need touching — verify
      that rather than assume it.
- [ ] 1.3 Check the exhaustive `match` sites. The compiler finds them; the
      point of this task is to look at each and decide, rather than let
      `_ =>` swallow the new value somewhere it matters.

## 2. The entry

- [ ] 2.1 `abstract-unix-sockets` on macOS moves from `unverified` to
      `not-applicable`, citing both probes: Python `bind("\0…")` and the C
      `sockaddr_un` with explicit `addrlen`. Two, because the first alone
      could have been Python passing `sun_path` as a C string rather than
      Darwin lacking the namespace.
- [ ] 2.2 Linux's half is untouched. If this change alters the Linux
      column at all, something has gone wrong.

## 3. Do not let the new word spread

- [ ] 3.1 Re-read every `unsupported` entry and confirm each is a real
      loss rather than an absent threat — but **change nothing** on the
      strength of a re-read. Anything that looks mislabelled gets
      measured first, in its own change; the proposal's second open
      question is exactly this and is deliberately out of scope here.
- [ ] 3.2 `per-agent-network-namespace` on macOS stays `unsupported`. It
      is the test case: sandboxes share the host's port table as a direct
      result, which is a loss a reader must weigh. If the new word can be
      argued onto that row, the word is too loose and this change is
      wrong.

## 4. Documentation

- [ ] 4.1 `docs/known-gaps.md`: record the measurement and why it needed a
      new word, including that the old status was not merely stale but
      *false* once measured.
- [ ] 4.2 `docs/implementation-log.md`.

## 5. Verification

- [ ] 5.1 build, clippy, fmt, rustdoc clean.
- [ ] 5.2 Full suite, skips reviewed.
- [ ] 5.3 `openspec validate --all`.
- [ ] 5.4 `devcroft doctor` on macOS shows `not-applicable` on that row and
      no remaining `unverified` — which, if it holds, is the first time
      every capability in the matrix has been measured on this platform.
- [ ] 5.5 Re-run on Linux, and confirm the Linux column is byte-identical.
