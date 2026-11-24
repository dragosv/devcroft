#!/bin/bash
# add-sandbox-identity 0.4 and the Linux command-line half of 0.5. Run as an
# ordinary user with a subordinate range, newuidmap, setfacl, git and python3.
# Leaves its directory (printed last) for inspection.
set -u
umask 022
here=$(cd "$(dirname "$0")" && pwd)
W=$(mktemp -d "$HOME/idmap.XXXXXX") || exit 1
cc -O1 -o "$W/idmap" "$here/idmap.c" || exit 1
me=$(id -un)
sub=$(awk -F: -v u="$me" '$1==u{print $2;exit}' /etc/subuid)
subg=$(awk -F: -v u="$me" '$1==u{print $2;exit}' /etc/subgid)
mkdir "$W/ws" "$W/home" "$W/home2"
chmod 0755 "$W" "$W/home" "$W/home2"
echo host > "$W/ws/user-file"
git -C "$W/ws" init -q
git -C "$W/ws" config user.email t@t
git -C "$W/ws" config user.name t
git -C "$W/ws" add -A
git -C "$W/ws" commit -qm base
inside() { "$W/idmap" "$sub" "$subg" "$W/ws" "$W/home" "$@"; }
quiet() { grep -v '^ok  \|^note'; }

echo "== identity"
inside sh -c 'id; cat /proc/self/uid_map; stat -c "%u %A %n" "$HOME" .
  echo pid=$$; grep -E "^(Uid|CapEff|CapBnd|NoNewPrivs)" /proc/self/status
  ls /proc/self/fd > /dev/null && echo fd-ok'

echo "== workspace without an ACL"
inside sh -c 'echo x > new; echo write=$?; cat user-file; echo x > "$HOME/f"; echo home=$?' 2>&1 | quiet

echo "== workspace with ACLs, inside"
setfacl -R -m "u:$sub:rwX" -m "d:u:$sub:rwX" -m "d:u:$(id -u):rwX" "$W/ws" || exit 1
inside sh -c 'echo x > new; mkdir d; echo y > d/f
  echo edit >> user-file; echo append=$?
  git status --short 2>&1 | head -1
  git config --global --add safe.directory "$PWD"
  git status --short > /dev/null && echo git-status-ok
  git add -A && git -c user.email=t@t -c user.name=t commit -qm inside && echo git-commit-ok
  python3 -c "
import os
for m in (0o644, 0o600): os.close(os.open(\"m%o\" % m, os.O_CREAT | os.O_WRONLY, m))
os.mkdir(\"d755\", 0o755); os.mkdir(\"d700\", 0o700); open(\"plain\", \"w\").close()"' 2>&1 | quiet

echo "== from the host, as the user"
cd "$W/ws" || exit 1
stat -c '%u %A %n' new d d/f m644 m600 d755 d700 plain user-file
for f in new d/f m644 m600 plain user-file; do
  if echo e >> "$f"; then r=edit; else r=REFUSED; fi 2> /dev/null
  echo "$f: $r, $(getfacl -cp "$f" 2> /dev/null | grep mask)"
done
for d in d d755 d700; do touch "$d/z" 2> /dev/null && echo "create in $d: ok" || echo "create in $d: REFUSED"; done
cp m644 m644.new && mv -f m644.new m644 && echo "m644 by replace: ok, owner $(stat -c %u m644)"
git status --short 2>&1 | head -2
echo q > hostfile
git add -A 2>&1 | head -2
git commit -qm host 2>&1 | head -2
git log --oneline 2>&1 | head -3

echo "== a second id; an id outside the range"
"$W/idmap" $((sub + 1)) $((subg + 1)) "$W/ws" "$W/home2" sh -c 'id -u; echo x > "$HOME/g" && echo home-ok' 2>&1 | quiet
"$W/idmap" 5 5 "$W/ws" "$W/home2" true
stat -c '%u %n' "$W/home" "$W/home2"

echo "== removing what the sandbox made, as the user"
rm -rf d d755 2>&1 | head -2
[ -e d ] && echo "d: left" || echo "d: removed"
[ -e d755 ] && echo "d755: left" || echo "d755: removed"
rm -f "$W/home/f" 2>&1 | head -1
echo "left in place: $W"
