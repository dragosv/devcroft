"""embed-brush-runtime 4: the runtime shell against a reference `sh`.

    python3 qualify.py <runtime sh> <reference sh>

Prints each behaviour where they differ, and the qualification probes."""
import os, signal, subprocess, sys, tempfile, time
SH, REF = sys.argv[1], sys.argv[2]
D = tempfile.mkdtemp()
def run(sh, script, stdin=None, args=(), env=None, timeout=20):
    try:
        p = subprocess.run([sh, *args] if script is None else [sh, "-c", script, *args], input=stdin,
            capture_output=True, text=True, cwd=D, timeout=timeout, env=env)
        return p.returncode, p.stdout, bool(p.stderr)
    except subprocess.TimeoutExpired:
        return "timeout", "", False
open(D + "/s.sh", "w").write('#!/bin/sh\necho "$0" "$#" "$@"\nexit 4\n')
CORPUS = r'''
echo "a b" 'c d' e\ f $((1+2*3)) "${x:-def}" "${#x}"
set -- a "b c" d; echo $# "$2"; shift; echo "$@"; for i; do echo "[$i]"; done
x=foo.tar.gz; echo ${x%.gz} ${x%%.*} ${x#*.} ${x##*.}
f() { local v=1; echo "in $1"; return 3; }; f z; echo $?
if [ -d / ] && ! test -f /nope; then echo y; else echo n; fi
case abc in a*) echo m;; *) echo no;; esac
i=0; while [ $i -lt 3 ]; do i=$((i+1)); [ $i = 2 ] && continue; echo $i; done
cat <<EOF
v=$((2+2))
EOF
cat <<'EOF'
$x
EOF
echo out; echo err >&2
echo one | { read a; echo "got $a"; }
printf '%s-%s\n' a b; printf '%d %x\n' 10 255
(exit 7); echo $?
false || echo rescued; true && echo ok
set -e; false; echo no
set -u; echo $undefined_v; echo no
trap 'echo bye' EXIT; echo hi
x=$(echo sub; exit 3); echo $x $?
echo a > f1; echo b >> f1; cat < f1; rm f1
exec 3>f3; echo to3 >&3; exec 3>&-; cat f3
export E=1; sh -c 'echo $E' 2>/dev/null || echo nosh
A=1 env | grep ^A= || true
IFS=:; set -- $(echo a:b:c); echo $2
echo *.sh
cd /; pwd; cd - >/dev/null
eval 'echo ev$((1))'
command -v ls >/dev/null && echo found
type cd >/dev/null && echo t
umask 022; umask
unset x; echo ${x-unset} ${x+set}
readonly R=1; echo $R
alias q='echo aliased'; echo ok
echo $0 "$1"
while read -r l; do echo "<$l>"; done <<EOF
l1
l2 \n
EOF
getopts ab: o -a; echo $o
wait; echo waited $?
sleep 0.2 & wait; echo bg
kill -0 $$ && echo alive
echo ${PWD:+pwd}
[ "a" = a -a "b" = b ] && echo and
test 1 -lt 2 && echo lt
: ${d:=assigned}; echo $d
echo "tab	x" | tr '\t' ' '
exit 300
nosuchcommand_zz; echo $?
./s.sh p "q r"; echo $?
. ./s.sh; echo not-reached
(trap 'echo sub' EXIT; exit 2); echo $?
echo $((010 + 0x10)) $((5 % 3)) $((1<<4)) $((3>2?7:8))
export a=b c="d e"; echo $c
x="export Z=9"; $x; echo $Z
[ -t 0 ] || echo notty
echo "$(printf 'x\n\n\n')|"
sh -c 'kill -TERM $$' 2>/dev/null; echo $?
IFS=:; echo a:b
IFS=/; x=/usr/lib; set -- $x; echo $#
IFS=:; y="a:b"; echo "$y" $y
IFS=":"; echo $(printf a:b)
x=$(i=0; while [ $i -lt 20000 ]; do echo $i; i=$((i+1)); done); echo ${#x}
'''.strip("\n")
cases, cur, here = [], [], None
for line in CORPUS.split("\n"):
    cur.append(line)
    if here: 
        if line == here: here = None
    elif "EOF" in line:
        here = "EOF"
    if not here: cases.append("\n".join(cur)); cur = []
same = diff = 0
print("== corpus: differences from", REF)
for c in cases:
    a, b = run(SH, c, stdin=""), run(REF, c, stdin="")
    if a == b: same += 1
    else: diff += 1; print("  ", repr(c), "\n     runtime  ", a, "\n     reference", b)
print(f"{same} same, {diff} different of {len(cases)}")
print("== invocation")
for args, stdin in [(["./s.sh", "1", "2"], None), (["-s", "x", "y"], "echo $# $2\n"), ([], "echo stdin\nread l\nthe line\necho \"got $l\"\n"), ([], "if true\nthen echo multi\nfi\n"), ([], "cat <<X\nhere $((1+1))\nX\necho 'two\nlines'\n"), ([], "echo a \\\n b\nf() {\n echo in\n}\nf\n"), ([], "echo 'unfinished\n"), ([], "exit 6\n"), ([], "cat\nrest of input\n"), (["-e", "-c", "false; echo no"], None), (["-c", 'echo $0 $1', "nm", "ar"], None), (["nofile"], None), (["-c", "cat", "x"], "piped\n")]:
    a, b = run(SH, None, stdin or "", args), run(REF, None, stdin or "", args)
    print("  ", "same" if a == b else "DIFF", args, repr(stdin), "" if a == b else f"{a} vs {b}")
print("== qualification")
def probe(name, script, send=None, want=None):
    out = []
    for sh in (SH, REF):
        p = subprocess.Popen([sh, "-c", script], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, cwd=D)
        if send: time.sleep(0.6); p.send_signal(send)
        try: o, _ = p.communicate(timeout=6)
        except subprocess.TimeoutExpired: p.kill(); o = p.communicate()[0] + "<timeout>"
        out.append((p.returncode, o.strip()))
    print("  ", "PASS" if out[0] == out[1] else "FAIL", name, out[0], "" if out[0] == out[1] else f"reference {out[1]}")
probe("TERM trap, builtin loop", "trap 'echo T; exit 9' TERM; i=0; while [ $i -lt 2000000 ]; do i=$((i+1)); done; echo end", signal.SIGTERM)
probe("TERM trap, during wait", "trap 'echo T; exit 9' TERM; sleep 2 & wait; echo end", signal.SIGTERM)
for s in ("TERM", "HUP", "INT"): probe(f"trap '' {s}", f"trap '' {s}; sleep 1.5; echo survived", getattr(signal, "SIG" + s))
probe("$! set", "sleep 0.1 & [ -n \"$!\" ] && echo set; wait")
probe("wait $! status", "(exit 7) & wait $!; echo $?")
probe("wait non-child", "wait 999999; echo $?")
probe("kill default TERM", "sh -c 'trap \"echo got; exit 0\" TERM; sleep 1 & wait' & p=$!; sleep 0.3; kill $p; wait $p 2>/dev/null; echo done")
print("   posix mode:", [l for l in run(SH, "set -o")[1].splitlines() if l.startswith("posix")])
def med(sh, n=20):
    t = []
    for _ in range(n):
        s = time.perf_counter(); subprocess.run([sh, "-c", ":"]); t.append(time.perf_counter() - s)
    return sorted(t)[n // 2] * 1000
print(f"== startup, median ms: runtime {med(SH):.2f}, reference {med(REF):.2f}")
