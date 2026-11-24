"""embed-brush-runtime 2.2a: Go workloads in a macOS mise sandbox, with the
host shell and without. Needs a devcroft built with the spike patch in
design.md ("Measured: 2.2a") and examples/test-row-sh, in target/spike."""
import os,shutil,subprocess,tempfile
R=os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))); T=R+'/target/spike/debug'
def run(spike):
    d=os.path.realpath(tempfile.mkdtemp(prefix='m22a',dir='/private/tmp')); n=os.path.basename(d).replace('_','').lower()
    for f in ('mise.toml','mise.lock','go.mod'): shutil.copy(f'{R}/samples/mise-sample/macos-arm64/{f}',d)
    open(d+'/main.go','w').write('''package main
import ("fmt";"os/exec")
//go:generate sh -c "echo gen-by-name > gen1"
//go:generate ./gen.sh
//go:generate /bin/sh -c "echo gen-abs > gen3"
//go:generate env FOO=1 sh -c "echo gen-env > gen4"
//go:generate go run ./tool
func main(){
 for _,a:=range [][]string{{"/bin/sh","-c","echo abs"},{"sh","-c","echo name"},{"./gen.sh"},{"/usr/bin/env","sh","-c","echo env"},{"/bin/ls","main.go"},{"ls","main.go"}} {
  o,e:=exec.Command(a[0],a[1:]...).CombinedOutput(); fmt.Printf("  %v -> %q %v\\n",a,o,e)}
}
''')
    open(d+'/main_test.go','w').write('''package main
import ("os/exec";"testing")
func TestAbs(t *testing.T){ o,e:=exec.Command("/bin/sh","-c","echo hi").CombinedOutput(); if e!=nil {t.Fatalf("%s %v",o,e)} }
func TestName(t *testing.T){ o,e:=exec.Command("sh","-c","echo hi").CombinedOutput(); if e!=nil {t.Fatalf("%s %v",o,e)} }
func TestShebang(t *testing.T){ o,e:=exec.Command("./gen.sh").CombinedOutput(); if e!=nil {t.Fatalf("%s %v",o,e)} }
func TestPure(t *testing.T){}
''')
    open(d+'/gen.sh','w').write('#!/bin/sh\necho gen-shebang > gen2\n'); os.chmod(d+'/gen.sh',0o755)
    os.mkdir(d+'/tool'); open(d+'/tool/t.go','w').write('package main\nimport "os"\nfunc main(){os.WriteFile("gen5",[]byte("x"),0644)}\n')
    open(d+'/devcroft.toml','w').write(f'[sandbox]\nname="{n}"\n[env]\nprovider="mise"\n[filesystem]\nallow=["."]\n[network]\ndefault="deny"\n')
    env=dict(os.environ,DEVCROFT_KEEPER_EXE=T+'/devcroft')
    if spike: env['DEVCROFT_SPIKE_SHELL']=T+'/examples/test-row-sh'
    dc=lambda *a: subprocess.run([T+'/devcroft',*a],cwd=d,env=env,capture_output=True,text=True,timeout=200)
    u=dc('up'); print('=====','no host shell' if spike else 'today (host /bin granted)','up',u.returncode,u.stderr[-300:] if u.returncode else '')
    if u.returncode: return
    r=dc('policy','--render').stdout
    print([l.strip() for l in r.splitlines() if l.strip().startswith(('/bin ','/usr/bin/env','/bin/sh','sh (by'))])
    W='go test -v . 2>&1; go test -v -exec "env DYLD_INSERT_LIBRARIES=$DYLD_INSERT_LIBRARIES DEVCROFT_NS=$DEVCROFT_NS" . 2>&1'
    for c in ('go version','go build -o prog . && ./prog','go vet .; echo rc=$?','go run . ',W,'go generate ./... ; echo rc=$?; ls gen1 gen2 gen3 gen4 gen5 2>&1'):
        o=dc('exec','--','sh','-c',c); print('$',c); print((o.stdout+o.stderr).rstrip()[-1500:])
    print('rm',dc('rm','--yes').returncode,d)
run(True); run(False)
