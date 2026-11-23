# Group 0 measurements (0.2, 0.4, 0.5), run by a shell started with the
# variables already set. Every tool here is the closure's (devbox), the
# runtime's, or the probe built from goprobe/.
T="${T:?set T to the itest directory}"
cd "$T"
export DC_RT="$T/rt"
export DYLD_INSERT_LIBRARIES="$T/libinterpose.dylib"
./rt/sh -c '
echo "== 0.2 Go"
./goprobe
./goprobe exec
echo "== 0.5 make from the closure, recipe through /bin/sh"
printf "all:\n\t@echo make-recipe:\$\${BASH_VERSION:-?}\n" > Makefile.t
make -f Makefile.t; echo "   exit=$?"
echo "== 0.5 python subprocess shell=True"
python3 -c "import subprocess; r = subprocess.run(\"echo py-shell:\$BASH_VERSION\", shell=True); print(\"   rc\", r.returncode)"
echo "== 0.5 npm run, script through /bin/sh"
mkdir -p npmt && printf "{\"name\":\"t\",\"version\":\"1.0.0\",\"scripts\":{\"hi\":\"echo npm-script:\$BASH_VERSION\"}}\n" > npmt/package.json
(cd npmt && HOME="$T/home" npm_config_cache="$T/npmcache" npm run --silent hi); echo "   exit=$?"
echo "== 0.5 node child_process.execSync (/bin/sh)"
node -e "console.log(String(require(\"child_process\").execSync(\"echo node-sh:\$BASH_VERSION\")).trim())"; echo "   exit=$?"
'
