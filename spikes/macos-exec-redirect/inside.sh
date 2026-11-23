T="${T:?set T to the itest directory}"
cd "$T"
echo "== 0) controls, no interposer"
echo "-- system()/popen() via /bin/sh:"; ./prog; echo "   exit=$?"
echo "-- shebang #!/bin/sh via posix_spawn:"; ./prog2 ./s.sh; echo "   exit=$?"
export DC_RT="$T/rt"
export DYLD_INSERT_LIBRARIES="$T/libinterpose.dylib"
echo "== 1) with interposer"
echo "-- system()/popen():"; ./prog
echo "-- shebang #!/bin/sh via posix_spawn:"; ./prog2 ./s.sh
echo "-- shebang #!/bin/sh from brush:"; ./s.sh
echo "-- /bin/echo (applet in runtime):"; /bin/echo userland-ok
echo "-- /bin/ls (not in runtime -> must be refused, not host):"; /bin/ls / >/dev/null; echo "   exit=$?"
echo "-- /usr/bin/make (SIP binary, host):"; /usr/bin/make --version >/dev/null; echo "   exit=$?"
CLOSURE_SH="$(command -v bash || true)"
echo "-- closure bash ($CLOSURE_SH) calling absolute /bin/sh:"
if [ -n "$CLOSURE_SH" ]; then "$CLOSURE_SH" -c '/bin/sh -c "echo closure-child:\$BASH_VERSION"'; fi
