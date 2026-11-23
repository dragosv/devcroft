T="${T:?set T to the itest directory}"
cd "$T"
export DC_RT="$T/rt"
export DYLD_INSERT_LIBRARIES="$T/libinterpose.dylib"
echo "== 2) session shell started WITH the interposer in its environment"
./rt/sh -c '
echo "-- shebang #!/bin/sh run by brush:"; ./s.sh
echo "-- /bin/echo (applet in runtime):"; /bin/echo userland-ok
echo "-- /bin/pwd (applet in runtime):"; /bin/pwd
echo "-- /bin/ls (not in runtime, must be refused, not host):"; /bin/ls / >/dev/null; echo "   exit=$?"
echo "-- /usr/bin/make (Apple, host):"; /usr/bin/make --version >/dev/null; echo "   exit=$?"
'
