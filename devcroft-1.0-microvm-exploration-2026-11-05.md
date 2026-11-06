# devcroft 1.0: guest Linux minimal, hvi și libkrun 2.0

**Export consolidat al discuției din 5 noiembrie 2026.**

Documentul acoperă comparația cu Brig, evaluarea hull și Lima, direcția unui
guest minimal construit din closure-uri Nix, comparația hvi/libkrun și rolul
nono. Include planul celor două spike-uri cerute și corecțiile apărute pe
parcurs.

**Direcția exprimată de utilizator:** evaluarea a două implementări ale
aceluiași guest — una cu biblioteca hvi, cealaltă cu libkrun **2.0** — și
alegerea pe baza unei comparații practice. Guest-ul minimal este un candidat
pentru arhitectura devcroft 1.0.

**Starea dovezilor:** analiza a verificat documentație și cod sursă. Nu s-a
compilat sau pornit un guest devcroft prin niciuna dintre biblioteci în această
sesiune. Nu există benchmark comparativ efectuat aici. Exemplele de topologie,
împărțirea componentelor și etapele spike-urilor sunt propuneri.

Acest export consemnează explorarea. Adoptarea unui backend și modificarea
promisiunilor produsului necesită ulterior reconcilierea deciziilor și a
artefactelor OpenSpec existente.

## Cuprins

1. [Concluzia de lucru](#1-concluzia-de-lucru)
2. [Surse și versiuni verificate](#2-surse-și-versiuni-verificate)
3. [devcroft comparat cu Brig](#3-devcroft-comparat-cu-brig)
4. [Ce sunt Brig, hull și hvi](#4-ce-sunt-brig-hull-și-hvi)
5. [Ce ar însemna suportul hull în devcroft](#5-ce-ar-însemna-suportul-hull-în-devcroft)
6. [Hull comparat cu Lima](#6-hull-comparat-cu-lima)
7. [Arhitectura guest-ului minimal](#7-arhitectura-guest-ului-minimal)
8. [Provisioning și materializarea pentru Linux](#8-provisioning-și-materializarea-pentru-linux)
9. [Storage, closure-uri și workspace](#9-storage-closure-uri-și-workspace)
10. [Rețea, vsock și proxy](#10-rețea-vsock-și-proxy)
11. [Keeper, SSH și lifecycle](#11-keeper-ssh-și-lifecycle)
12. [hvi comparat cu libkrun](#12-hvi-comparat-cu-libkrun)
13. [Guest minimal cu libkrun 1.x și 2.0](#13-guest-minimal-cu-libkrun-1x-și-20)
14. [Combinarea nono cu VMM-ul și guest-ul](#14-combinarea-nono-cu-vmm-ul-și-guest-ul)
15. [Cele două spike-uri și comparația](#15-cele-două-spike-uri-și-comparația)
16. [Ce ar justifica devcroft 1.0](#16-ce-ar-justifica-devcroft-10)
17. [Decizii existente care trebuie revizitate](#17-decizii-existente-care-trebuie-revizitate)
18. [Corecții și precizări din discuție](#18-corecții-și-precizări-din-discuție)
19. [Întrebări încă deschise](#19-întrebări-încă-deschise)
20. [Referințe](#20-referințe)

## 1. Concluzia de lucru

devcroft are deja o sursă declarativă pentru toolchain, environment și servicii.
O direcție coerentă este să transforme rezultatul rezolvării într-un guest Linux
cu strictul necesar:

```text
manifest + lock + target Linux
             │
             ▼
     materializare și captură
             │
             ├── closure-uri read-only
             ├── environment
             ├── hookuri ca date
             └── declarații de servicii
             │
             ▼
kernel + init minimal + keeper + workspace privat
             │
             ▼
       VM per sandbox / agent
```

Nix ar furniza filesystemul de runtime al toolchain-ului, inclusiv linkerul,
bibliotecile și datele tranzitive. VMM-ul ar furniza kernelul separat și
dispozitivele necesare. devcroft ar continua să administreze politica,
provisioningul, sesiunile, serviciile, SSH și starea.

Forma propusă nu depinde de OCI, de o distribuție Linux completă sau de
instalarea Nix în fiecare guest runtime. Materializatorul poate avea Nix și
providerii instalați; rezultatul lui ajunge read-only în runtime.

**hvi și libkrun 2.0 sunt ambele candidate pentru această arhitectură.** Alegerea
nu este făcută. Avantajele posibile trebuie demonstrate pe același guest și
aceleași workload-uri, inclusiv cu confinementul propus pentru produs.

Execuția nativă rămâne justificată pentru Xcode, AppKit, code signing și teste
Darwin. Guest-ul acestei explorări este Linux, inclusiv pe macOS.

## 2. Surse și versiuni verificate

Referințele externe se află în secțiunea 20. Acolo unde este disponibil,
documentul folosește commitul citit, pentru ca o schimbare ulterioară pe `main`
să nu modifice retrospectiv baza comparației.

| Proiect | Versiune / commit inspectat | Observație |
|---|---|---|
| devcroft | `65b94bacefd154b91fe1876e7ad694bfd91d1bf6`, checkout local | README, capabilități, provider, keeper, fleet, SSH și documentele de design |
| Brig | `e26d9fb41de9f4309ba77deaccde02dea625cdaf` | Corespunde commitului țintă al release-ului `v0.5.1`, publicat la 2026-11-03 |
| hull | `a1462e92d8d94c4f77ae2c51fabd20656cf67dc6` | CLI, arhitectură, storage, networking, port Linux și documentație de performanță |
| hvi | `351d1a15728e998486232e4815ba075a22e44f56` | Biblioteca Rust evaluată separat; configurație, embedding, dispozitive și confinement |
| hvi fixat de hull | `0.1.0-rc.1`, commit scurt `65570c7` | Nu este același cod cu hvi inspectat separat |
| libkrun 2.0 | `6151710f4ea627cf0e5dbce78e4192fb4d64c2e9` | `2.0.0-dev`, API Rust și C; încă în dezvoltare |
| libkrun stabil | `865f9f4e486285dcc43aab96f085dedf11ea1454` | Ramura `stable-1.19.x`, manifestul declară `1.19.7` |
| nono | `0.77.0`, codul din registry-ul Cargo local | Versiunea fixată de devcroft |
| Lima | Documentație publică și fișiere `master`, citite la 2026-11-05 | Nu s-a fixat un commit Lima pentru această analiză; capabilitățile trebuie reverificate la versiunea aleasă pentru spike |

Nu s-a efectuat un audit de securitate al proiectelor. Citirea codului confirmă
existența unor mecanisme și ordinea lor, nu corectitudinea completă sub toate
workload-urile.

## 3. devcroft comparat cu Brig

### 3.1. Modelul produselor

Brig pune accentul pe pornirea unui agent într-o microVM, cu profiluri și
gestionarea credențialelor. devcroft pune accentul pe mediul declarat al
proiectului, serviciile sale, politica inspectabilă și accesul prin SSH.

| Aspect | devcroft actual | Brig inspectat |
|---|---|---|
| Unitate principală | Proiect + mediu din lockfile + politică | Agent + profil + imagine OCI + sesiune |
| Sistemul workload-ului | Nativ pe host: Linux sau macOS | Linux în guest, pe ambele platforme |
| Izolare | Landlock/Seatbelt și namespaces Linux; kernel comun cu hostul | MicroVM cu kernel propriu pe căile documentate |
| Scopul declarat al izolării | Protecție împotriva accidentelor în proiecte de încredere | Izolarea guest-ului de host prin VM; limitele sunt documentate separat |
| Mediu de dezvoltare | flox, nix, devbox, devenv; swift ca artifact-tier pe macOS | Imagine OCI furnizată sau construită separat |
| Reproductibilitate | Toolchain din definiție și lockfile; fără provider host/none | Imagine fixabilă prin digest; mediul specific proiectului trebuie pregătit în ea sau în guest |
| Servicii | Import, supraveghere, readiness și dependențe pentru providerii suportați | Procesele pot rula în guest; compose între sandboxuri este în lucru |
| Porturi | Namespace propriu pe Linux în modul deny; alocare unde loopback-ul este comun | Rețea proprie pe Linux și pe macOS/hvi; publicare explicită |
| Credențiale | Valorile reale ajung în environment; brokerul nu este implementat | Secret store, canale env/files și broker opțional |
| Editor | Server SSH integrat per sandbox | Profilul `herd` oferă SSH prin exec; nu este o funcție identică în toate profilurile |
| Instalare | Build din surse + provider | Release-uri, Homebrew și runtime bundle Linux |

### 3.2. Avantajele concrete ale Brig

**Kernel separat.** O compromitere a kernelului guest nu este același lucru cu
o compromitere a kernelului host. Este o limită mai puternică decât modelul
process-tier actual devcroft, fără ca asta să însemne absența riscului de VMM
escape.

**Broker de credențiale existent.** Rutele brokerului pot păstra secretul pe
host și livra guest-ului un token de sesiune. Brokerul atașează credențiala
cererii upstream și poate restrânge metode, căi și, pentru presetul GitHub,
repository-uri.

Brokerul este opțional. Livrarea directă prin environment sau fișiere rămâne
disponibilă. Ruta AWS are altă garanție: livrează chei temporare reale în
guest, deoarece acesta trebuie să semneze cererile AWS.

**Pornire accesibilă utilizatorului.** După instalarea infrastructurii,
`brig run claude <proiect>` cere mai puțină configurare inițială decât un proiect
devcroft nou cu mediu declarat și manifest.

**Distribuție și verificarea imaginilor.** Brig distribuie binare și verifică
imaginile și artefactele proprii. Modul implicit de verificare este `warn`,
cu refuzuri mai stricte pentru anumite eșecuri; `require` este o alegere
explicită. Nu trebuie descris ca verificare obligatorie a oricărei imagini.

### 3.3. Avantajele concrete ale devcroft

**Mediul proiectului este parte centrală din produs.** Toolchain-ul este
declarat în provider și lockfile; devcroft îi capturează environmentul și
granturile. Stratul personal de tooling poate adăuga agentul fără să aplice
hookurile sau celelalte variabile ale acelui strat peste proiect.

**Serviciile sunt integrate.** devcroft citește declarațiile providerului,
generează configurația supervisorului, așteaptă readiness și oprește procesele
cu sandboxul. Refuză câmpurile pe care nu le poate transporta fidel.

**Execuție macOS nativă.** Pentru proiectele Apple, aceasta este o diferență de
platformă, nu o optimizare. Guest-ul Linux din Brig nu poate executa AppKit sau
un build Darwin nativ.

**Politică explicabilă.** `policy --render` și `why` fac legătura dintre
manifest, regulile efective și originea lor. Un backend VM ar trebui să păstreze
această proprietate pentru toate canalele către host.

### 3.4. Limite comune și diferențe care trebuie nuanțate

- Un proiect montat read-write poate fi șters sau modificat în ambele produse.
  VM-ul nu oferă undo pentru scrierile permise.
- Fișiere precum hookurile Git sau `.git/config` pot determina execuție pe host
  la o comandă ulterioară. Brig documentează și avertizează asupra unor astfel
  de schimbări; avertizarea nu blochează scrierea.
- devcroft are deny-default pentru rețea. Brig are egress nerestricționat până
  la atașarea unei politici; postura `isolated` separă rețelele, nu filtrează
  implicit internetul.
- Brig filtrează hosturile prin DNS și adrese IP. Proxy-ul devcroft verifică
  ținta HTTP/CONNECT. Ambele au limite cu virtual hosting pe aceeași adresă.
- O allowlist de destinații nu este o garanție generală împotriva exfiltrării:
  o destinație permisă care acceptă uploaduri este un canal de ieșire.
- Brig partajează baza imaginii și are straturi private. Nu duplică obligatoriu
  întreaga imagine pentru fiecare agent.
- Măsurătoarea devcroft cu opt medii confirmă reutilizarea store-ului. Nu
  reprezintă un benchmark împotriva Brig și nu demonstrează partajarea
  buildurilor aplicației sau a memoriei guest-urilor.

Măsurătoarea locală înregistrată în roadmap, pe macOS ARM64 cu devenv, a fost
de 5,1 s și +7,9 MiB în store pentru primul sandbox, apoi 4,1–4,4 s și
+0,0 MiB în store pentru fiecare dintre celelalte șapte. Closure-ul a fost
mic, instanțele au fost verificate funcționale și nu au avut servicii. Aceste
numere nu se transferă guest-urilor VM.

Matricea de agenți devcroft înregistrează unsprezece CLI-uri care au pornit și
au ajuns la API cu chei dummy. Aceasta probează runtime-ul, transportul și TLS;
nu demonstrează conversații complete, tool use, editări sau login OAuth.

Brig are și un daemon opțional, `brigd`, pentru lifecycle și inventar prin
socket Unix. CLI-ul funcționează și fără el. Descrierea „Brig cere un daemon
central” ar fi inexactă.

## 4. Ce sunt Brig, hull și hvi

### 4.1. Straturile

O cale de execuție Brig pe macOS este:

```text
Brig                     profiluri, sesiuni, credențiale, politici
  └── hull               imagini OCI, rootfs, lifecycle
        └── hvi          RAM, vCPU, device model
              └── Hypervisor.framework
                    └── guest Linux
```

Hull poate folosi și `vz-runner` peste Virtualization.framework sau QEMU.
Brig pe Linux folosește nerdctl/containerd/urunc în implementarea inspectată.

### 4.2. Hull

Hull este runtime-ul care pregătește o imagine OCI și o pornește într-o VM.
Poate furniza kernel și init wrapper pentru imagini Linux obișnuite, fără
kernel propriu. Are cache, storage de instanță, exec, logs, stop/rm și un gateway
userspace separat.

Hull este o componentă utilizabilă independent de Brig. CLI-ul este Go;
backendurile sunt procese separate. Biblioteca hvi nu include toată
infrastructura hull.

Suportul principal hull este Apple Silicon. Portul Linux inspectat poate porni
și controla anumite instanțe prin containerd/urunc, dar networkingul și
gateway-ul nu sunt încă disponibile acolo. Nu este paritate funcțională
completă cu macOS.

### 4.3. hvi

`hvi` este un microVMM Apache-2.0 scris în Rust, dezvoltat de NOFire AI în
ecosistemul Brig. Poate fi executabil sau bibliotecă, fără Brig și fără hull.

| Host | Backend | Guest vizat |
|---|---|---|
| macOS Apple Silicon | Hypervisor.framework | aarch64 Linux |
| Linux ARM64 | KVM | aarch64 Linux |
| Linux x86-64 | KVM | x86-64 Linux |

Oferă boot direct de kernel, virtio-block, virtio-net, virtio-vsock și
filesystem sharing. Pe macOS servește virtiofs în proces; pe Linux folosește
un daemon vhost-user pornit de integrator.

API-ul de embedding verificat este:

```rust
pub fn boot(cfg: BootConfig) -> Result<Stop, BootError>;
```

Kernelul și initramfs-ul sunt furnizate ca bytes în `BootConfig`. Configurația
include discuri, share-uri, transport, RAM, vCPU și activarea confinementului.

API-ul este explicit instabil, nu există crate publicat și se fixează un
commit. Proiectul are teste și CI pe hardware real, dar utilizare mai restrânsă
decât libkrun, Firecracker sau QEMU și declară lipsa unui audit extern.

**Clarificarea suportului Linux:** hvi nu este limitat la macOS. În
`src/lib.rs`, selecția backendului include explicit Linux ARM64 și Linux
x86-64 prin KVM. Acesta este suport implementat în cod și declarat upstream,
nu un boot devcroft verificat local.

Acoperirea de test nu este uniformă: README-ul commitului inspectat spune că
CI bootează cele trei backenduri pe hardware real, dar nivelul atins diferă.
Aceleași joburi de boot nu atașează diskuri sau un agent vsock. Prezența
device-urilor în cod și testele lor unitare nu demonstrează întregul workload
devcroft pe Linux; acesta este unul dintre obiectivele spike-ului.

**Diferența de versiuni contează:** hull inspectat fixează un hvi mai vechi
decât biblioteca citită separat. De exemplu, limitele privind numărul de
discuri și unele funcții nu trebuie transferate între cele două versiuni.

## 5. Ce ar însemna suportul hull în devcroft

Au fost discutate două utilizări, cu costuri și obiective diferite.

### 5.1. Întregul sandbox Linux într-o VM

Providerul ar decide mediul, iar runtime-ul ar decide locul execuției:

```text
                   devcroft
                      │
          manifest + lock + politică
                      │
             mediu pentru target
                      │
           ┌──────────┴──────────┐
           ▼                     ▼
       proces nativ        VM pornită de hull
       keeper local        keeper Linux în guest
```

Hull ar furniza boot, rootfs, exec și mecanisme de transport. devcroft ar
continua să definească mediul proiectului, sesiunile, serviciile și politica.

Integrarea completă cere mai mult decât o implementare `HullSessionBackend`.
SFTP folosește filesystemul procesului serverului SSH, iar port forwardingul
deschide conexiuni din acel proces. Locul keeperului și serverului SSH decide
ce filesystem și ce rețea accesează aceste operații.

### 5.2. Numai serviciile Linux ale unui proiect macOS

Aceasta se potrivește cu designul existent `add-macos-service-vm`:

```text
macOS
├── aplicație și toolchain nativ
└── guest Linux comun
      ├── serviciile sandboxului A, izolate
      └── serviciile sandboxului B, izolate
```

Designul existent preferă un guest comun, cu Nix store comun și izolarea
serviciilor în interior. Sursa proiectului nu este montată implicit în guest:
un Postgres are nevoie de datele sale, nu de întregul checkout.

Hull ar putea fi evaluat în locul backendului Lima propus acolo. Această
integrare rezolvă compatibilitatea serviciilor pe macOS; nu pune agentul nativ
sau compilatorul Darwin în spatele unui kernel separat.

### 5.3. Un experiment inițial discutat

O variantă ieftină de verificare ar fi devcroft rulat integral într-un guest
hull, cu mai multe sandboxuri în interior. Ar verifica rapid compatibilitatea
providerilor, keeperului și editorului cu acel guest.

Topologia ar oferi un kernel separat față de host, comun tuturor sandboxurilor
din guest. Nu ar demonstra izolarea de kernel per agent. Direcția exprimată
ulterior de utilizator este evaluarea directă a bibliotecilor hvi și libkrun
2.0 cu același guest minimal.

## 6. Hull comparat cu Lima

### 6.1. Diferența de integrare

Lima oferă o mașină Linux administrată, cu provisioning, disc persistent, SSH
și integrare host–guest. Hull oferă o instanță pornită din imagine, cu un model
mai apropiat de un artefact executabil.

| Aspect | Hull | Lima |
|---|---|---|
| Intrare principală | Imagine OCI și parametri de boot | Imagine de sistem și configurație de mașină |
| Provisioning | Imagine și bootstrap pregătite de integrator | cloud-init, scripts și readiness probes |
| Exec | Guest agent peste transportul backendului | SSH, `limactl shell`, copiere și tuneluri |
| Persistență | Depinde de backend/rootfs și de politica integratorului | Discuri și lifecycle persistent administrate |
| Egress | Gateway cu allow/deny | Networking general; politica devcroft trebuie adăugată |
| Porturi | Publicare prin gateway | Forwarding automat sau explicit |
| Platforme | Principalmente Apple Silicon; Linux limitat | macOS Intel/ARM, Linux și alte platforme |
| Maturitate | Proiect tânăr | Proiect din 2021, cu adopție în Colima, Rancher Desktop și Finch |

Ambele pot susține un guest comun sau VM-uri distincte. Numele „microVM” nu
dovedește singur un avantaj de memorie sau pornire.

### 6.2. Unde hull este atractiv

- Un set explicit de intrări: imagine, kernel/initrd, RAM/vCPU, share-uri și
  gateway.
- Gateway per sandbox, cu politică în afara kernelului guest.
- Bază read-only reutilizată și stare privată per instanță.
- Mai puține comportamente automate de reconciliat cu manifestul devcroft.

Avantajul este mai mare dacă devcroft controlează imaginea guest-ului și vrea
instanțe scurte, reproductibile. Pentru un Linux administrativ de lungă durată,
infrastructura suplimentară Lima este adesea utilă.

### 6.3. Unde Lima este atractiv

- Administrarea utilizatorului, SSH, provisioningului și discurilor există
  deja.
- Un Nix store persistent comun are un loc natural într-un guest de lungă
  durată.
- SSH și forwardingul, inclusiv pentru socketuri Unix, simplifică primul
  transport spre un helper devcroft în guest.
- Proiectul are un istoric operațional mai lung.
- Modul `plain` poate dezactiva mounts, guest agent, containerd și forwardingul
  dinamic. SSH și provisioningul de bază continuă să funcționeze.

Pentru serviciile Linux ale aplicațiilor macOS native, Lima rămâne un candidat
puternic. Pentru un guest minimal per agent, cu filesystem produs direct din
closure-uri, evaluarea bibliotecilor VMM evită o parte din administrarea unei
mașini generale.

### 6.4. Configurații care trebuie controlate de devcroft

În template-ul implicit Lima citit:

- home-ul hostului este montat read-only;
- porturile guest-ului pot fi forwardate automat;
- loopback-ul hostului este accesibil prin `host.lima.internal`;
- variabilele de proxy se propagă implicit;
- configurațiile globale din `$LIMA_HOME/_config/` pot modifica instanța la
  restart.

Un backend devcroft ar trebui să folosească un `LIMA_HOME` propriu și o
configurație explicită. `networkOpts.hostLoopback: false` există în versiuni
recente pe VZ, cu limite documentate; nu echivalează cu un allowlist complet
de egress.

La hull, un store administrat de devcroft ar izola configurația față de
utilizarea manuală a hull, păstrând cache-ul comun instanțelor devcroft.

Valorile secrete nu ar trebui livrate prin configurația de boot: `hull run
--env` le poate persista în rootfs/initrd, iar environmentul de boot Lima
poate ajunge în fișiere guest. Livrarea după boot prin transport este o
problemă separată.

### 6.5. Performanță și securitate

Lima VZ și hull VZ folosesc Virtualization.framework. Diferențele provin din
guest, rootfs, servicii, provisioning, transport și configurație. Nu există în
această analiză dovezi că hull este în general mai rapid sau mai ieftin.

Datele de performanță publicate de hull nu măsoară toate backendurile și nici
gateway-ul. Timpul kernel-start–init nu este timpul complet până la proiect
utilizabil.

Lima tratează guest-ul, inclusiv root, ca potențial ostil. hvi își confinează
VMM-ul. Verdictul depinde de întreaga configurație: share-uri, device-uri,
socketuri, forwarding și autoritatea proceselor host-side. Eticheta de
microVM nu stabilește singură care integrare este mai sigură.

Checkpoint/restore hull este o capabilitate `vz`, cu block rootfs și limite
documentate. Nu este o funcție hvi. Nici nu garantează undo al efectelor
externe sau consistență la nivel de aplicație.

## 7. Arhitectura guest-ului minimal

### 7.1. Componentele

```text
HOST: macOS sau Linux
│
├── devcroft controller
│     ├── manifest, stare și lifecycle
│     ├── coordonarea materializării
│     ├── socketuri control / SSH
│     └── proxy egress separat
│
└── helper VMM dedicat
      └── hvi sau libkrun 2.0
            ├── kernel Linux
            ├── initramfs minimal
            ├── closure.img read-only
            ├── state.img privat, writable
            └── vsock
                  │
                  └── GUEST
                        ├── init PID 1
                        ├── keeper și server SSH
                        ├── agent / sesiuni / builduri
                        └── servicii
```

Un helper rulează o VM. Controllerul și proxy-ul rămân în alte procese.
Helperul nu are nevoie de credențialele API pe care un broker viitor le-ar
deține pe host.

Această separare este utilă cu ambele biblioteci:

- hvi aplică restricții permanente, folosește semnale și permite un singur run
  concurent per proces;
- libkrun are căi de oprire care termină procesul VMM;
- un crash VMM nu trebuie să termine controllerul sau celelalte instanțe.

### 7.2. Ce conține guest-ul

| Componentă | Conținut |
|---|---|
| Bootstrap | Kernel și initramfs cu init static mic |
| Runtime devcroft | Keeper, shell, transport, CA bundle și configurație minimă |
| Toolchain proiect | Closure-ul providerului pentru target Linux |
| Tooling personal | Closure-ul stratului de tools, când este configurat |
| Servicii | Binarele și datele tranzitive cerute de declarațiile providerului |
| Stare privată | Workspace, home, build outputs, caches și datele serviciilor |

Un filesystem posibil:

```text
/
├── init
├── nix/store/           closure-uri selectate, read-only
├── workspace/           proiect și build outputs
├── home/devcroft/       home privat
├── var/lib/services/    stare persistentă a serviciilor
├── run/                 tmpfs
├── tmp/                 tmpfs
├── proc/                procfs
├── sys/                 sysfs
├── dev/                 devtmpfs + devpts
└── etc/                 configurație minimă generată
```

Closure-ul furnizează libc, dynamic linker și alte dependențe ale programelor.
Tot rămân necesare anumite convenții de sistem: identitate utilizator, hostname,
NSS unde este folosit, certificate și eventual `/bin/sh` sau `/usr/bin/env`.

Shell-ul este în continuare o dependență devcroft, nu o cerință nouă pusă
proiectului. Poate fi rezolvat din closure sau furnizat în closure-ul runtime
devcroft, cu grant și proveniență vizibile.

### 7.3. Init-ul

Init-ul minimal ar trebui să:

1. Pregătească stdio și filesystemele virtuale.
2. Monteze diskurile și zonele tmpfs.
3. Configureze identitatea și convențiile minime de runtime.
4. Activeze loopback-ul și endpointul vsock.
5. Primească configurația de launch.
6. Pornească keeperul cu environment explicit.
7. Reape procesele orfane și coordoneze shutdownul.

Un binar Rust static poate face bootstrapul fără shell. Dacă init-ul este un
script, shell-ul și utilitarele folosite trebuie să fie disponibile înainte de
montarea closure-ului.

`fleet::init` conține logică relevantă pentru PID 1, reaping și semnale, dar este
scris pentru namespace-urile procesului actual. Reutilizarea este de evaluat;
nu este deja un init de VM gata de folosit.

### 7.4. Kernelul

Kernelul trebuie să conțină driverele pentru dispozitivele alese și funcțiile
pe care se bazează politica devcroft:

- initramfs și filesystemul imaginii;
- virtio și transportul potrivit VMM-ului;
- virtio-block și virtio-vsock;
- procfs, sysfs, devtmpfs, devpts și tmpfs;
- Landlock și configurația LSM necesară;
- namespace-uri și cgroups unde vor fi folosite;
- virtio-balloon și page reporting pentru evaluarea recuperării memoriei;
- ceas și entropie suficiente pentru TLS și SSH.

Kernelul și initramfs-ul se fixează și se construiesc per arhitectură. Pe
Apple Silicon targetul este `aarch64-linux`; pe Linux x86-64 este
`x86_64-linux`. Versiunea guest-ului și capabilitățile lui sunt distincte de
versiunea kernelului host.

## 8. Provisioning și materializarea pentru Linux

### 8.1. Darwin și Linux produc closure-uri diferite

Un mediu capturat pe Mac nativ conține binare Darwin. Montarea store-ului lui
într-un guest Linux nu transformă acele binare în executabile Linux.

```text
aceeași definiție și lock
         │
         ├── rezolvare aarch64-darwin → proces nativ macOS
         └── rezolvare aarch64-linux  → guest Linux
```

Pentru proiectele portabile, lockfile-ul poate fixa aceleași surse și versiuni,
iar pachetele realizate pentru fiecare sistem diferă. Nu toate proiectele sau
lockfile-urile providerilor pot produce targetul Linux cerut. Eligibilitatea
trebuie verificată per provider.

### 8.2. Materializator separat, runtime mic

Un mediu Linux de materializare, reutilizabil, ar putea conține:

- Nix;
- versiunile fixate ale providerilor folosiți;
- un helper devcroft pentru captură și export;
- accesul la cache-uri și registries necesar pachetelor.

Poate fi un guest administrativ separat sau un builder Linux disponibil.
Prima implementare nu trebuie să rezolve toate formele de provisioning.

```text
materializator Linux
  → rezolvă providerul fără hookurile proiectului
  → capturează env + unset + hookuri ca date + servicii
  → identifică closure-urile și rădăcinile lor
  → produce artefactul read-only
  → runtime-urile primesc artefactul
```

Minimalismul celor N runtime-uri este compatibil cu un materializator care are
instrumentele administrative necesare.

### 8.3. Separarea materializării de hookuri rămâne obligatorie

Un hook de activare nu trebuie să primească autoritatea daemonului Nix sau
drept de scriere în store-ul comun doar pentru că rulează într-o VM.

Ordinea de păstrat:

1. Materializarea pachetelor prin mecanismul providerului.
2. Captura hook-free a environmentului și a declarațiilor.
3. Execuția hookului proiectului sub politica de provisioning.
4. Captura exporturilor hookului.
5. Pornirea runtime-ului sub politica sa și adoptarea exporturilor.

Politicile de provisioning și runtime rămân distincte. Un registry necesar
hookului nu trebuie să rămână permis agentului după terminarea provisioningului.

O captură fără hook nu dovedește că orice evaluare sau build de pachete este
inofensiv. Limitele materializatorului și accesul lui la resurse comune trebuie
definite separat.

### 8.4. Căi și environment

Environmentele providerilor pot numi căi din workspace, de exemplu profile
generate în `.flox`, `.devbox` sau `.devenv`. Nu este suficient să copiezi
numai pachetele din store.

Captura și execuția trebuie să fie coerente asupra:

- căii proiectului în guest;
- symlinkurilor generate de provider;
- directoarelor de home și tmp;
- environmentului și variabilelor eliminate;
- fingerprintului, targetului și staleness-ului;
- separării artefactelor Darwin de cele Linux.

Ideal, materializatorul folosește aceeași cale guest a proiectului ca runtime-ul,
evitând rescrieri arbitrare de stringuri în environment.

## 9. Storage, closure-uri și workspace

### 9.1. Prima variantă discutată

```text
initramfs mic, comun
closure.img   read-only, reutilizabil pentru același mediu
state.img     writable, privat per sandbox
```

Imaginea cu pachete se montează astfel încât executabilele să-și regăsească
referințele absolute sub `/nix/store/<hash>-…`.

Avantaje:

- aceeași formă pe Linux și macOS;
- nu necesită virtiofsd pentru store pe Linux;
- filesystem și permisiuni Linux în guest;
- backendul deschide diskul read-only și refuză scrierile, inclusiv dacă guest-ul
  încearcă să-l remonteze writable;
- diskul privat poate conține workspace și date de servicii cu I/O Linux local.

### 9.2. Closure-ul nu ar trebui despachetat integral în initramfs

Un initramfs mare ar despacheta datele în memoria fiecărui guest. Pentru
toolchain-uri consistente, asta poate distruge avantajul de densitate.

Initramfs-ul ar trebui să asigure bootstrapul și montarea. Toolchain-ul poate
fi citit la nevoie din imaginea read-only.

### 9.3. Ce este și ce nu este partajat

Pentru opt instanțe ale aceluiași artefact, imaginea closure-ului se produce o
dată și se deschide read-only de mai multe VM-uri.

Pentru două medii diferite care folosesc multe pachete comune, două imagini raw
pot duplica acele date fizic. Store-ul Nix sursă poate deduplica pachetele, fără
ca formatul imaginilor rezultate să păstreze automat aceeași deduplicare.

Nici partajarea fișierului host nu garantează partajarea tuturor paginilor din
memoria guest. Fiecare kernel guest poate avea propriul page cache.

Prin urmare, trebuie măsurate separat:

- dimensiunea logică și cea alocată a imaginilor;
- costul construirii unui artefact nou;
- reutilizarea între instanțe identice;
- duplicarea între medii diferite;
- cache-ul host și page cache-ul guest;
- colectarea artefactelor când nu mai sunt folosite.

### 9.4. Workspace privat sau checkout partajat

| Variantă | Beneficiu | Cost |
|---|---|---|
| Checkout host read-write prin share | Editorul host vede imediat schimbările | I/O și coerență de filesystem sharing; modificările checkoutului sunt reale |
| Workspace privat pe disk guest | I/O local Linux și separare de checkoutul original | Acces prin Remote-SSH și un mecanism de import/export al rezultatului |

Pentru primul spike, workspace-ul privat reduce numărul de variabile.
Partajarea poate fi evaluată ulterior, ca probă separată, dacă workflow-ul o
cere.

Un mount read-only în guest nu garantează că alt proces host nu modifică sursa
artefactului. Imaginile trebuie publicate ca artefacte stabile și păstrate
nemodificate cât timp există consumatori.

### 9.5. Persistență și snapshot

`down` trebuie să păstreze datele pe care contractul devcroft le consideră
persistente; `rm` trebuie să le elimine. Un rootfs temporar sau recreat la
fiecare boot nu trebuie să piardă implicit datele serviciilor.

Copierea sau clonarea unui disk writable oprit este o operație diferită de un
snapshot de memorie. Nu se modifică imaginea ext4 din host în timp ce guest-ul
o are montată. Snapshoturile de disk activ necesită un protocol explicit de
consistență; nu fac parte din primul spike.

## 10. Rețea, vsock și proxy

### 10.1. Forma minimă propusă

```text
agent / build în guest
   → HTTP_PROXY pe loopback guest
   → relay devcroft
   → vsock
   → proxy devcroft pe host
   → destinația permisă
```

Guest-ul are loopback pentru servicii, fără virtio-net, TAP, DHCP sau NAT.
Filtrarea ieșirii rămâne în afara kernelului guest.

Proprietatea structurală este absența unei rute IP către exterior. Un workload
care ocolește variabilele de proxy nu obține prin asta o interfață de ieșire.
Mecanismul vsock expus de VMM trebuie totuși să limiteze ce operații și
endpointuri sunt accesibile.

### 10.2. Transportul hvi

Interfața `agent_sock` verificată creează un listener Unix pe host și leagă
fiecare conexiune de un stream vsock către guest CID 3, port 1024, cu host CID 2.
Transportul nu interpretează protocolul aplicației.

Controlul și SSH pot folosi acest mecanism. Pentru cererile egress guest →
host, o propunere este un canal persistent inițiat din host, cu multiplexarea
streamurilor cerute de guest.

Aceasta implică framing, identificatori, backpressure, închidere pe jumătate,
limite și tratarea pierderii conexiunii. Nu este deja furnizată de simpla
existență a `agent_sock`.

### 10.3. Transportul libkrun

libkrun oferă mapări configurabile între porturi vsock și socketuri Unix, pentru
conexiuni inițiate din ambele sensuri. Asta poate permite separarea directă a
controlului, SSH și relay-ului spre proxy.

**TSI trebuie dezactivat.** TSI execută operații socket în contextul rețelei
hostului. Guest fără NIC plus TSI nu este topologia fără ieșire directă dorită
de devcroft.

În 2.0 se creează `VsockDevice` cu `TsiFlags::empty()`. În 1.19.x există
dezactivarea vsock-ului implicit și adăugarea explicită a unui vsock cu flags 0.

### 10.4. Ce servicii funcționează și ce rămâne de definit

Procesele aceleiași VM pot comunica prin loopback și socketuri Unix guest.
Două VM-uri pot folosi simultan portul 5432. Accesul din host poate trece prin
SSH `-L`, fără publicare TCP automată.

Topologia HTTP/CONNECT nu furnizează implicit UDP arbitrar, QUIC sau acces IP
nerestricționat. Manifestele care cer astfel de capabilități necesită o
implementare sau un refuz explicit, în locul degradării silențioase.

### 10.5. Autentificarea proxy-ului nu este credential brokering

Tokenul per sesiune al proxy-ului actual devcroft autentifică folosirea
allowlistului. Nu înlocuiește cheia API a agentului.

Un broker viitor ar păstra cheia reală pe host și ar atașa-o cererilor
autorizate. VM-ul nu produce automat această funcție, iar simpla mutare a
agentului în guest nu ascunde cheia față de testele și buildurile care rulează
în același domeniu de acces.

## 11. Keeper, SSH și lifecycle

### 11.1. Keeper în guest

Propunerea preferată este keeperul în guest, cu `LocalSessionBackend` pentru
fork/exec local. Astfel se pot reutiliza semantica și codul existent pentru:

- exec și shell;
- PTY și resize;
- semnale și exit status;
- hookuri;
- supravegherea serviciilor;
- SSH și SFTP;
- port forwarding spre serviciile guest-ului.

`SessionBackend` rămâne util, dar nu este singura abstracție necesară. Tipurile
actuale de sesiune conțin `Child`, PGID și handle de resize, adică obiecte
locale. Un backend care ar executa fiecare comandă printr-un CLI VMM ar trebui
să reconcilieze aceste semantici și operațiile de filesystem/rețea SSH.

### 11.2. Transportul de control

Protocolul keeperului actual are framing și transportă spawn, stdin/stdout,
stderr, resize, semnale, exit și query. Este o bază reutilizabilă, dar tipurile
actuale de listener/client folosesc direct `UnixListener`/`UnixStream`.

Un transport guest ar necesita adaptarea acceptării și conexiunii sau un relay:

```text
devcroft exec pe host
   → endpoint local / adaptor
   → transport host–guest
   → keeper
   → proces guest
```

PID-urile și PGID-urile guest nu se semnalizează cu `kill()` din host. Semnalele
comenzilor trebuie să ajungă la keeperul guest. Hostul controlează separat
procesul VMM pentru oprire forțată.

### 11.3. SSH

```text
ssh proiect.devcroft
   → ProxyCommand devcroft proxy
   → socket host protejat
   → transport host–guest
   → russh în guest
```

Nu este necesar un listener TCP guest. Un director partajat care conține un
socket Unix nu transportă automat conexiunea peste două kerneluri.

Serverul russh din guest folosește filesystemul guest pentru SFTP și rețeaua
guest pentru `direct-tcpip`. Acesta este motivul pentru care locul serverului
contează.

Autentificarea, host key-ul și endpointurile administrative trebuie să rămână
separate de environmentul workload-ului, ca în corecțiile existente devcroft.

### 11.4. Stare, sănătate și oprire

`Meta` și verificarea sănătății trebuie să poată distinge:

- proces VMM viu;
- guest pornit;
- transport disponibil;
- keeper responsive;
- environment fresh/stale;
- servicii ready, failed sau unavailable.

Un PID host viu nu este suficient. Starea poate păstra și targetul, identitatea
artefactelor, versiunea VMM, protocolul guest și identitatea diskului privat.

Oprirea normală ar trebui să ceară keeperului oprirea serviciilor, apoi init-ului
shutdown și sincronizarea datelor. Un timeout poate escalada la oprirea VMM.
Forțarea VMM-ului este un crash pentru filesystem și aplicații; nu trebuie
raportată ca oprire grațioasă.

Un `request_stop` al VMM-ului nu echivalează singur cu shutdownul aplicațiilor.
Pentru ambele biblioteci, protocolul de oprire guest rămâne important.

## 12. hvi comparat cu libkrun

### 12.1. Matricea pentru guest-ul propus

| Criteriu | hvi inspectat | libkrun 2.0 inspectat |
|---|---|---|
| Integrare | Bibliotecă Rust, commit fixat | Bibliotecă Rust sau C, commit fixat |
| Stabilitate API | Explicit instabil | `2.0.0-dev`, încă instabil |
| Kernel și init propriu | Da | Da |
| Discuri closure/state | Da, raw, maximum patru | Da, raw și alte formate; suport block activat |
| Guest fără NIC, cu vsock | Da | Da, fără TSI |
| Confinement VMM | Inclus în proiect | De integrat de consumator |
| Transport vsock | Bridge host → guest, port fix în API-ul verificat | Mapări configurabile în ambele sensuri |
| Free-page reporting | Fără dispozitiv echivalent în setul verificat | Virtio-balloon, inclusiv mecanism `Replace` pe macOS |
| Virtiofs Linux | Daemon extern | Backend integrat |
| Lifecycle | `boot()` întoarce `Stop` sau eroare | Build/run cu căi de oprire care pot termina procesul |
| macOS | Apple Silicon, 15+ | Apple Silicon, 14+ conform documentației |
| Linux | ARM64 și x86-64 prin KVM | ARM64 și x86-64 relevante aici, prin KVM |
| Maturitate | Mai tânăr, consumatori mai puțini | Consumatori existenți în crun, krunkit și muvm; 2.0 are interfață nouă |
| Snapshot de memorie complet | Nu | Nu în API-ul verificat |

### 12.2. Avantajul hvi: confinement și suprafață de integrare mică

hvi deschide resursele principale și aplică restricțiile proprii:

- Seatbelt deny-default pe macOS, înainte de execuția guest-ului;
- allowlisturi seccomp pentru threadurile VMM/vCPU pe Linux.

Acesta este un avantaj față de construirea întregii confinări de către
devcroft. Interfața mică și implementarea raw block pot fi suficiente pentru
guest-ul cu două discuri și vsock.

Limitele nu dispar: API instabil, lipsa auditului extern, device-uri și
performanță care trebuie probate, restricții specifice platformei. Confinementul
nu reduce UID-ul și nu filtrează toate argumentele syscallurilor.

### 12.3. Avantajul libkrun: memorie și transport

Un workload de fleet nu rămâne la memoria consumată imediat după boot:

```text
boot → build mare → eliberare memorie → agent și servicii în idle
```

libkrun are free-page reporting. Pe macOS, modul `Replace` înlocuiește
blocurile de memorie raportate libere, pentru a permite recuperarea unor
pagini pe care `madvise()` nu le eliberează corespunzător.

Mecanismul are condiții: driver și configurație kernel potrivite, raportare de
pagini libere și o cale host de mapare compatibilă. Nu garantează automat că
page cache-ul Linux va fi eliberat sau că orice VM revine la footprint-ul de
boot. Trebuie măsurate ambele probe: alocare/eliberare controlată și build real.

Mapările vsock bidirecționale pot reduce codul nou pentru egress. Nu este
necesar să activezi TSI pentru a folosi vsock.

### 12.4. Storage și partajare

hvi are o implementare block restrânsă. libkrun are mai multe opțiuni, inclusiv
citiri paralele în API-ul 2.0 verificat. Mai multe opțiuni nu dovedesc o
performanță mai bună; comparația trebuie să folosească workload real și
semantici de durabilitate comparabile.

În particular, libkrun are moduri de sincronizare distincte, iar modul relaxed
pe macOS nu cere aceeași sincronizare cu hardware-ul ca modul full. Un benchmark
de bază de date trebuie să noteze și să alinieze garanția de flush, nu doar să
compare configurațiile implicite.

Pentru discuri raw private, asimetria virtiofs hvi Linux/macOS poate fi evitată.
Dacă un checkout host partajat devine obligatoriu, backendul virtiofs integrat
libkrun poate avea un avantaj de integrare. Și el cere o limită host explicită
asupra accesului VMM-ului la directoare.

### 12.5. Lifecycle și punctul de aplicare a restricțiilor

În libkrun 2.0 citit, `VmmBuilder::build()` ajunge la `start_vcpus()`.
Secvența „build, apoi aplică sandboxul, apoi run” nu garantează confinement
înainte de primul cod guest.

La hvi, bootstrapul și punctul propriu de confinement sunt parte din
implementare. API-ul `boot()` întoarce un rezultat al opririi, dar stopul VMM nu
înlocuiește protocolul de shutdown guest.

Un helper proces separat per VM este alegerea propusă pentru ambele. Pentru
libkrun este și adaptarea naturală la căile care cheamă `_exit()`.

### 12.6. Packaging și licențe

Ambele VMM-uri sunt Apache-2.0. Un kernel Linux distribuit separat continuă să
aibă propriile obligații de licență. Eliminarea libkrunfw din calea de boot nu
transformă guest-ul Linux într-un artefact integral Apache-2.0.

hvi nu are crate publicat. libkrun 2.0 poate fi consumat direct ca bibliotecă
Rust, cu device-urile necesare și fără interfața FFI când nu este folosită.
Prin urmare, o bibliotecă dinamică instalată separat nu este o condiție
inevitabilă pentru 2.0.

Ambele variante necesită kernel și payload guest versionate. Distribuția prin
`cargo publish` trebuie verificată: un pin Git utilizabil local nu garantează
o dependență publicabilă în forma cerută de crates.io.

Pe macOS, executabilul helper care folosește Hypervisor.framework are nevoie
de entitlementul și semnarea potrivite. Kernelul și payloadul trebuie fixate,
verificate și actualizate ca parte a distribuției.

## 13. Guest minimal cu libkrun 1.x și 2.0

### 13.1. Confirmarea pentru 2.0

API-ul oferă:

```rust
Payload::load_external(kernel_path, format, initrd_path, cmdline)
```

Această cale acceptă kernel extern, initramfs propriu și command line.
Documentația separă explicit acest boot de `krun-init` și `libkrunfw`.

Poți configura block devices, plain vsock, console și balloon fără să
folosești o imagine Ubuntu/Alpine/Fedora sau un runtime OCI.

### 13.2. Confirmarea pentru 1.19.x

Capabilitățile au fost verificate în codul `1.19.7`, nu presupuse pentru orice
release vechi din familia 1.x.

| Necesitate | API 1.19.x |
|---|---|
| Kernel + initramfs + command line | `krun_set_kernel(...)` |
| Dezactivare init injectat implicit | `krun_disable_implicit_init(...)` |
| Disk read-only sau writable | `krun_add_disk(...)` și variantele sale |
| Plain vsock fără hijacking TSI | `krun_disable_implicit_vsock()` + `krun_add_vsock(ctx, 0)` |
| Mapări vsock / Unix sockets | `krun_add_vsock_port2(...)` |
| Recuperarea paginilor guest libere | API de balloon free-page reporting |
| Pornire | `krun_start_enter(...)` |

Codul de start încarcă payloadul libkrunfw numai dacă nu a fost configurat un
kernel extern, un kernel bundle sau alt payload acceptat. Bootul cu kernel
extern și init propriu nu cere o distribuție completă.

Interfața publică stabilă 1.x este C, utilizabilă prin FFI din Rust.
`krun_start_enter()` preia procesul și îl termină la oprirea VM-ului.

**Alegerea exprimată în discuție este totuși libkrun 2.0 pentru spike.**
Confirmarea 1.19.x explică existența mecanismului; nu introduce un al treilea
spike obligatoriu.

## 14. Combinarea nono cu VMM-ul și guest-ul

### 14.1. Două locuri, două obiective

```text
HOST
└── helper VMM confinat
      └── kernel guest
            └── keeper restricționat prin nono
                  └── agent / builduri / servicii
```

Confinarea host limitează ce poate atinge procesul VMM. Politica nono din
guest limitează workload-ul în filesystemul și rețeaua guest.

Pe un Mac, mecanismul host poate fi Seatbelt, iar nono din guest este
Landlock. Sunt kerneluri și procese distincte, nu două profiluri Seatbelt
aplicate aceluiași proces.

### 14.2. Nono în guest

Această integrare are sens cu ambele biblioteci:

- păstrează politica proiectului și semantica keeperului;
- separă operațiile init-ului administrativ de cele ale workload-ului;
- limitează scrierile la workspace, home și datele permise;
- menține distincția provisioning/runtime;
- păstrează o cale comună pentru politica devcroft pe Linux nativ și Linux
  guest.

Kernelul guest trebuie să ofere capabilitățile Landlock cerute. Restricția se
aplică după montarea filesystemelor și crearea endpointurilor, înainte de
execuția codului proiectului. Identitatea și capabilitățile workload-ului
trebuie configurate explicit; init PID 1 nu presupune că agentul trebuie să
ruleze cu privilegiile sale.

O compromitere a kernelului guest poate anula politica din el. Diskul impus
read-only de host și proxy-ul de pe host rămân controale distincte.

### 14.3. Nono în jurul libkrun pe host

libkrun cere integratorului să limiteze autoritatea host a VMM-ului. Nono este
un candidat pentru această limitare.

Politica helperului diferă de cea a proiectului:

| Resursă host | Autoritate propusă |
|---|---|
| Kernel/initramfs selectate | Citire pentru bootstrap |
| Imagine closure | Read-only |
| Disk privat | Read-write |
| Socketuri necesare | Numai endpointurile explicite |
| Checkout original nepartajat | Fără acces |
| Credențiale host | Fără acces |
| Starea altor sandboxuri | Fără acces |

Două mecanisme sunt de evaluat:

1. **Nono înainte de `build()`.** Threadurile ulterioare moștenesc restricția,
   dar politica trebuie să permită operațiile necesare inițializării KVM/HVF,
   memoriei, diskurilor și transportului.
2. **Un punct explicit între pregătire și prima execuție guest.** Resursele
   sunt deschise înainte, apoi helperul se restricționează. Poate necesita o
   modificare upstream și hookuri pentru threadurile relevante.

Pe Linux, Landlock aplicat unui thread nu confinează automat toate threadurile
deja existente. Aplicarea înainte de crearea lor este o cale clară de
moștenire. Pe macOS, profilul este process-wide, dar ordinea față de prima
execuție guest rămâne importantă.

Compatibilitatea nu este demonstrată în această analiză. `Sandbox::apply_auto`
nu este automat un jail complet pentru un VMM. Rămân de verificat devices,
ioctluri, AF_UNIX, rețea și eventualele mecanisme suplimentare de namespaces
sau seccomp.

### 14.4. Nono în jurul hvi pe host

**macOS:** hvi are propriul Seatbelt aplicat la un punct potrivit. Devcroft a
măsurat deja că o a doua inițializare Seatbelt este refuzată în contextul său.
Pentru spike, se păstrează profilul hvi și se folosește nono în guest.

Înlocuirea profilului hvi cu unul generat de nono ar necesita o comparație a
autorității efective. Profilul general nono permite operații utile aplicațiilor
obișnuite, precum fork, exec și lookupuri Mach, pe care un VMM minimal poate să
nu le necesite. Simplul nume al bibliotecii nu garantează o politică mai
restrânsă.

**Linux:** nono/Landlock și seccomp din hvi pot fi complementare. Primul
restrânge resursele de filesystem, al doilea syscallurile. Ordinea de testat
este nono înainte de inițializarea hvi, astfel încât threadurile noi să
moștenească restricția. Instalarea ulterioară poate fi refuzată de filtrele
seccomp.

Dacă hvi nu mai poate deschide fișiere după bootstrap, beneficiul suplimentar
Landlock trebuie demonstrat. Dacă se adaugă virtiofs Linux, daemonul extern
are propriul proces și trebuie evaluat separat.

### 14.5. Descriptorii sunt parte din autoritate

Un helper confinat care deține deja un disk writable sau un socket poate
continua să folosească acea resursă în limitele mecanismelor aplicate.
Deschiderea înainte de restricție elimină nevoia unor granturi permanente de
cale, dar nu elimină autoritatea descriptorului.

Planul de launch ar trebui să poată inventaria:

- fișierele și device-urile deschise;
- accesul read-only/read-write;
- socketurile și sensul comunicației;
- logurile și stdout/stderr;
- mapările de memorie și orice descriptor transmis altui proces.

### 14.6. Matricea propusă pentru spike-uri

| Strat | libkrun 2.0 | hvi |
|---|---|---|
| Workload guest | Nono/Landlock | Nono/Landlock |
| VMM pe macOS | Confinement prin nono, de demonstrat | Seatbelt propriu hvi |
| VMM pe Linux | Nono și mecanisme suplimentare după nevoie | Seccomp hvi; Landlock suplimentar evaluat |
| Egress | Proxy separat pe host, prin vsock | Proxy separat pe host, prin vsock |

Comparația urmărește proprietățile obținute. Nu trebuie impusă aceeași
bibliotecă în fiecare strat dacă un backend oferă deja mecanismul potrivit.

## 15. Cele două spike-uri și comparația

### 15.1. Obiectivul cerut

Două implementări:

- **Spike H:** biblioteca hvi, commit fixat.
- **Spike K:** libkrun 2.0, commit fixat.

Ambele pornesc același guest minimal și rulează aceleași probe. Artefactele
comune sunt construite o dată per arhitectură.

### 15.2. Intrări comune

- Kernel identic și aceeași configurație.
- Initramfs identic și același init static.
- Imagine raw read-only identică pentru runtime și closure-uri.
- Copii inițial identice ale diskului writable.
- Același CPU/RAM configurat.
- Aceleași versiuni ale toolchain-ului, serviciilor și agentului.
- Fără NIC și fără TSI.
- Același contract logic de control, SSH și egress.
- Aceleași politici guest și aceleași obiective de confinement host.

Kernelul comun include virtio-balloon/page reporting pentru a permite
libkrun să demonstreze funcția. Această capabilitate nu trebuie dezactivată
doar pentru a face backendurile artificial identice.

Command line-ul consolei sau alte detalii de transport pot diferi. Se
înregistrează diferențele, fără a schimba workload-ul în ascuns.

### 15.3. Ce rămâne specific fiecărui adaptor

Fiecare spike are helperul său host care:

- configurează biblioteca;
- pornește VM-ul;
- expune transportul;
- livrează configurația și confirmă readiness;
- cere oprirea;
- raportează exit/eroare;
- eliberează resursele sau este oprit forțat de controller.

Nu este necesar un trait Rust general perfect înainte de primele rezultate.
Costul adaptorului și diferențele de lifecycle sunt rezultate ale
experimentului, nu detalii care trebuie ascunse de la început.

### 15.4. Etapa A — Substratul minimal

Probe comune:

1. Init-ul ajunge la un handshake de READY.
2. Filesystemele necesare sunt montate.
3. Closure-ul rulează read-only, fără Nix instalat în guest.
4. O comandă întoarce stdout, stderr și exit status corect.
5. Vsock funcționează bidirecțional, inclusiv EOF și half-close.
6. Diskul privat păstrează un fișier după cold restart.
7. Scrierea pe diskul closure este refuzată și de backend.
8. Un guest care nu ajunge la READY produce timeout și eroare atribuită.
9. Stopul și killul helperului nu lasă procese sau endpointuri active.

În această etapă, networkingul extern poate lipsi. Transportul se verifică
înainte de a introduce agent, servicii și întregul keeper.

### 15.5. Etapa B — Workload devcroft real

Se folosește un proiect fixat, cu:

- build Rust sau Go real;
- workload cu multe fișiere mici;
- un serviciu Postgres sau Redis cu readiness;
- un agent CLI care pornește și ajunge la API;
- o destinație egress permisă și una refuzată;
- două instanțe care folosesc simultan același port.

O cheie dummy poate proba pornirea și accesul TLS până la răspunsul API, dar nu
înlocuiește un test de conversație, tool use și editare. Cele două rezultate
trebuie etichetate separat.

Apoi se adaugă keeperul devcroft și se verifică:

- exec piped și interactiv;
- PTY, resize și semnale;
- exit normal și semnal;
- SSH exec și shell;
- SFTP/scp;
- `ssh -L` către serviciul guest;
- readiness și oprirea serviciilor;
- environmentul, HOME, shell și CA bundle;
- hookurile sub politica de provisioning.

### 15.6. Etapa C — Densitate și robustețe

Se rulează 1, 4 și 8 instanțe pe fiecare host disponibil. Se înregistrează
modelul mașinii, OS, arhitectura, versiunile, power mode și starea cache-ului.

Probe:

- pornire și oprire repetate;
- builduri concurente;
- idle cu agent și servicii;
- pierderea canalului vsock;
- terminarea keeperului și a init-ului;
- SIGKILL asupra helperului VMM;
- restart și reconciliere a stării;
- suspend/resume al hostului;
- refuzul pornirii când o capabilitate cerută lipsește.

Fiecare instanță trebuie să demonstreze că a executat workload-ul. Opt erori
rapide sau opt probe sărite nu sunt un rezultat de densitate.

### 15.7. Măsurători

| Măsurătoare | Ce decide |
|---|---|
| Launch → guest READY | Costul real până la comenzi |
| Launch → servicii ready | Costul până la proiect utilizabil |
| Memorie după boot | Costul instanței inactive |
| Memorie maximă în build | Densitate sub sarcină |
| Memorie după build și idle | Recuperarea memoriei și efectul cache-urilor |
| Durata buildului | Costul runtime-ului și storage-ului |
| I/O cu multe fișiere mici | Comportament relevant pentru toolchain-uri |
| CPU în idle | Costul instanțelor păstrate pornite |
| Storage comun și privat | Reutilizare și creștere marginală |
| Shutdown și cleanup | Timp și resurse rămase |
| Cod specific și patchuri | Costul integrării și întreținerii |

Pentru memorie se folosesc două probe:

1. **Controlată:** guest-ul alocă, atinge și eliberează pagini.
2. **Reală:** build, oprirea proceselor de build și idle cu serviciile active.

Pe macOS se urmărește footprint-ul procesului; pe Linux RSS/PSS și memoria
grupului de procese. Suma RSS poate număra de mai multe ori paginile partajate.
Memoria configurată a guest-ului nu este întreaga memorie host a VMM-ului.

vCPU configurat nu înseamnă cotă host de CPU. Limitele guest RAM nu includ
toate buffer-ele și procesele host auxiliare. Pe Linux pot fi evaluate
cgroupurile existente devcroft, cu control plane în afara bugetului workload-ului.

Se raportează repetări, mediane și intervale, cu cache rece și cald etichetate.
Pragurile de acceptare se aleg înainte de rezultate; nu sunt stabilite numeric
în această discuție.

### 15.8. Confinementul este parte din comparație

Pentru fiecare candidat se consemnează:

- punctul primei execuții guest;
- momentul aplicării restricțiilor host;
- threadurile acoperite;
- resursele disponibile înainte și după;
- inventarul descriptorilor;
- operațiile host permise și refuzate;
- codul sau modificările upstream necesare.

Probele negative trebuie să aibă controale pozitive: un fișier nepermis este
refuzat în timp ce diskul privat funcționează; egress-ul nepermis este refuzat
în timp ce destinația permisă răspunde.

Probe de verificat:

- checkoutul original nepartajat nu este accesibil;
- credențialele și starea altor instanțe nu sunt accesibile;
- helperul nu deschide conexiuni arbitrare;
- guest-ul nu ajunge la daemonul de pachete al materializatorului;
- closure-ul comun nu poate fi modificat;
- o instanță nu ajunge la diskul privat sau controlul alteia;
- lipsa confinementului produce refuz explicit.

Se repetă workload-urile cu confinementul activ. Un rezultat numai din rulări
neconfinate nu validează configurația pentru produs.

### 15.9. Ordinea criteriilor de alegere

1. Corectitudine: workload, transport, persistență și oprire.
2. Controlul accesului la host și canale explicite.
3. Densitate, în special memoria după build.
4. Cost de integrare, patchuri și întreținere.
5. Performanță: startup, build și I/O.

hvi ar câștiga dacă interfața mică și confinementul existent vin cu densitate,
transport și storage suficiente. libkrun 2.0 ar câștiga dacă funcțiile de
memorie și vsock reduc costul fleet-ului, iar confinarea se integrează corect.

Un candidat care funcționează numai pe una dintre platforme poate conduce la
restrângerea scope-ului sau la revizuirea strategiei. Nu se presupune un
fallback silențios între garanții diferite.

### 15.10. Livrabilele spike-urilor

- Commiturile și hashurile tuturor intrărilor.
- Configurația kernelului și procesul de construire a guest-ului.
- Scripturi și comenzi reproductibile pentru ambele adaptoare.
- Handshake și raport de capabilități guest.
- Loguri de probe funcționale și negative.
- Date brute și tabele de memorie, startup, I/O și cleanup.
- Profiluri host/guest și inventarul resurselor deschise.
- Diferențe Linux/macOS și ARM64/x86-64.
- Lista patchurilor necesare și a limitelor rămase.
- Recomandare finală: hvi, libkrun 2.0, continuarea evaluării sau respingerea
  formei VM-per-agent pe hardware-ul vizat.

## 16. Ce ar justifica devcroft 1.0

Utilizatorul a identificat această direcție ca potrivită pentru 1.0.
Promisiunea posibilă este:

> Mediul declarat al proiectului, pornit într-un guest Linux minimal cu kernel
> propriu, servicii și acces SSH, cu accesul la host controlat și inspectabil.

Aceasta ar combina componente deja existente devcroft cu o limită de kernel
nouă. Nix devine baza filesystemului runtime, nu numai sursa compilatorului.

Pentru o lansare 1.0 ar trebui demonstrate:

1. **Intrări restorable și identificate:** lockfile, target, closure, kernel și
   runtime guest.
2. **Contract funcțional complet:** up, exec, shell, SSH/SFTP, servicii,
   readiness, down, restart și persistență.
3. **Politică pentru toate traversările:** diskuri, share-uri, transport,
   egress și control.
4. **Utilizare paralelă practică:** workload-uri reale la 1/4/8 instanțe, cu
   costuri și limite publicate.
5. **Comportament stabil:** erori atribuite, recuperare după crash,
   suspend/resume și upgrade-uri previzibile.

Un boot reușit validează mecanismul. Aceste rezultate ar valida produsul.

Separarea de kernel nu rezolvă automat expunerea credențialelor către procesele
din guest, modificările proiectului, efectele asupra serviciilor externe sau
toate formele de consum de resurse. Garanția publicată trebuie derivată din
configurația completă și probele ei.

Execuția nativă și guest-ul Linux pot avea aceeași interfață de comenzi, cu
garanția efectivă afișată explicit în `up`, `status`, `doctor` și policy render.
Sintaxa publică pentru selectarea runtime-ului nu a fost decisă.

## 17. Decizii existente care trebuie revizitate

### 17.1. Regula backendului și a bibliotecii

`remove-gvisor-backend` păstrează `SessionBackend` și criterii pentru un viitor
backend: compoziție cu nucleul de sandboxing, funcții fleet, toolchain-uri
reale, model de izolare declarat și integrare ca bibliotecă din Rust.

hvi și libkrun se potrivesc formei de bibliotecă. Totuși, keeperul va rula ca
proces guest, iar ascendența proceselor guest nu este ascendență de procese
host. Invariantul despre keeper și un sandboxing binary trebuie reinterpretat
explicit pentru acest model; nu se presupune că textul vechi descrie deja VM-ul.

### 17.2. Izolarea actuală și threat model

Decizia curentă limitează promisiunea devcroft la process-tier accident
protection. O direcție VM per agent redeschide acel scope. Nu trebuie
prezentată ca și cum o nouă promisiune pentru cod ostil ar fi deja implementată.

### 17.3. „Environment resolves once, at up”

Rezolvarea o singură dată poate fi păstrată, dar „host-side” trebuie să
distinga hostul operatorului de un mediu Linux de materializare. Targetul și
locul rezolvării trebuie înregistrate.

### 17.4. Politica deterministă

Un singur profil de filesystem nu mai descrie toată autoritatea. Ar trebui să
fie inspectabile cel puțin:

- planul host al VMM-ului;
- dispozitivele și diskurile atașate;
- politica guest;
- politica de provisioning;
- egress-ul și transporturile;
- canalele bazate pe descriptori predeschiși.

### 17.5. Documentele existente

- [Deciziile proiectului](decisions.md): gVisor, libkrun și condițiile de
  redeschidere.
- [Threat model](threat-model.md): scopul protecției actuale.
- [Roadmap](roadmap.md): definițiile existente pentru versiunile următoare.
- [Firecracker vs libkrun](firecracker-vs-libkrun.md): comparația anterioară,
  cu microsandbox ca referință.
- [Strategia spike libkrun](libkrun-spike-strategy.md): planul anterior
  microsandbox-oracle → upstream libkrun.
- [Investigația microsandbox](microsandbox-investigation.md): prior art și
  limitele integrării bazate pe fork.
- [Direcția Firecracker](firecracker-direction.md): o arhitectură alternativă
  explorată anterior.
- [VM pentru servicii macOS](../openspec/changes/add-macos-service-vm/design.md):
  guest comun și backend Lima propus.
- [Provider devcontainer](../openspec/changes/add-devcontainer-provider/design.md):
  explorarea rootfs-ului OCI ca provider, pe altă axă decât alegerea VMM-ului.

Planul celor două spike-uri directe din acest export diferă de strategia
anterioară cu microsandbox ca oracle. Documentele trebuie reconciliate înainte
de implementarea unei schimbări formale.

## 18. Corecții și precizări din discuție

Această secțiune păstrează corecțiile, pentru a evita reutilizarea unor
afirmații intermediare mai puternice decât dovezile.

1. **Guest-ul minimal nu este exclusiv hvi.** libkrun 2.0 și seria verificată
   1.19.x acceptă kernel și initramfs propriu.
2. **Libkrunfw nu este obligatoriu pentru orice utilizare libkrun.** Kernelul
   extern poate elimina acea dependență din calea de boot; TSI cere însă
   suportul kernel specific.
3. **Libkrun 2.0 are API Rust nativ.** Nu este obligatoriu consumat numai prin
   C sau ca bibliotecă dinamică instalată separat.
4. **API stabil înseamnă ceva diferit pentru 1.x și 2.0.** Linia 1.x are
   contractul public C stabil; 2.0 este încă în dezvoltare. Utilizatorul a ales
   2.0 ca obiect al spike-ului.
5. **Hull și hvi au versiuni și responsabilități diferite.** Funcțiile hvi
   citite pe un commit nou nu se atribuie automat release-ului fixat de hull.
6. **Hull are acum și port Linux, dar limitat.** Descrierea simplă
   „macOS-only” ratează portul; descrierea „paritate Linux/macOS” îl
   supraestimează.
7. **Un SessionBackend nou nu rezolvă întreaga integrare.** SFTP, forwarding,
   semnalele, PID-urile și health-ul depind de locul keeperului și de transport.
8. **Kernelul macOS nu este virtualizat automat de nono.** Execuția nativă
   Seatbelt este un proces macOS care apelează kernelul macOS. Devcroft într-un
   guest Linux este o topologie separată.
9. **Nu există un benchmark hvi/libkrun în această sesiune.** Preferințele
   exprimate sunt ipoteze de integrare, nu rezultate de performanță.
10. **Partajarea store-ului nu dovedește partajarea RAM.** Page cache-ul
    fiecărui guest și recuperarea după build trebuie măsurate.
11. **„Fără NIC” în libkrun 1.x nu înseamnă automat „fără TSI”.** Se
    configurează plain vsock explicit.
12. **Proxy-ul devcroft verificat decide după ținta HTTP/CONNECT.** Nu s-a
    identificat inspecție TLS SNI în `src/proxy/server.rs`; o descriere de
    capabilitate care spune SNI nu schimbă implementarea citită.
13. **Confinementul hvi și nono nu se suprapun arbitrar pe macOS.** Se
    proiectează un profil host coerent; nono din Linux guest este independent.
14. **Aplicarea nono după `build()` libkrun 2.0 poate fi prea târzie.** Codul
    verificat pornește vCPU-uri în build.
15. **Kernel extern nu înseamnă distribuție fără obligații de licență.**
    Kernelul Linux rămâne un artefact cu licența și sursele corespunzătoare.
16. **Stop VMM nu este shutdown al aplicațiilor.** Protocolul guest de
    oprire și verificarea persistenței rămân necesare.
17. **Credential brokering este separat de autentificarea proxy-ului.**
    Tokenul proxy devcroft existent nu ascunde cheia API reală a agentului.
18. **hvi nu este macOS-only în codul inspectat.** Are backenduri KVM pentru
    Linux ARM64 și x86-64. Trebuie separate biblioteca hvi, integrarea hull și
    maturitatea fiecărei căi; suportul Linux declarat nu înlocuiește probele
    end-to-end ale guest-ului devcroft.

## 19. Întrebări încă deschise

### Produs și topologie

- VM per sandbox manual, per agent fleet sau ambele?
- Workspace privat ca mod inițial? Când este necesar checkoutul host partajat?
- Ce capabilități de rețea sunt suportate de prima variantă fără NIC?
- Ce promisiune exactă pentru workload-uri neîncrezute poate fi susținută după
  probele complete?

### Provisioning și closure-uri

- Care provider este primul și cum produce target Linux pe un Mac?
- Cum se administrează materializatorul și accesul lui la store/cache?
- Cum se exportă profilele și căile generate, nu numai pachetele?
- Ce unitate definește identitatea imaginii: closure + runtime + target +
  formatul filesystemului?
- Ce cost de deduplicare între medii diferite este acceptabil?

### Runtime și transport

- Un singur canal multiplexat sau canale separate control/SSH/egress?
- Cum se livrează configurația și secretele după boot, fără persistență
  accidentală?
- Ce handshake de versiune și capabilități leagă controllerul de runtime?
- Cum se păstrează erorile de strat și exit status de la workload la CLI?

### Confinement și resurse

- Poate libkrun 2.0 porni complet sub un profil nono aplicat înainte de build?
- Este necesar un hook upstream pentru confinement înainte de primul cod guest?
- Ce autoritate rămâne în helper prin descriptori și device-uri?
- Câtă memorie host este recuperată după build de fiecare candidat?
- Ce limite totale se pot aplica proceselor auxiliare pe fiecare platformă?

### Distribuție și 1.0

- Cum sunt construite, semnate și distribuite kernelul și payloadul guest?
- Cum se rezolvă dependențele Git nepublicate în distribuția devcroft?
- Ce versiuni macOS și Linux vor fi susținute de backendul VM?
- Ce probe devin regresii obligatorii înainte de upgrade VMM/kernel?
- Ce praguri de densitate, startup și storage justifică lansarea?

## 20. Referințe

### Brig, hull și hvi

- [Brig v0.5.1](https://github.com/brig-sh/brig/releases/tag/v0.5.1).
- [Brig README, commit inspectat](https://github.com/brig-sh/brig/blob/e26d9fb41de9f4309ba77deaccde02dea625cdaf/README.md).
- [Brig security](https://github.com/brig-sh/brig/blob/e26d9fb41de9f4309ba77deaccde02dea625cdaf/docs/security.md).
- [Brig policies](https://github.com/brig-sh/brig/blob/e26d9fb41de9f4309ba77deaccde02dea625cdaf/docs/policies.md).
- [Brig broker](https://github.com/brig-sh/brig/blob/e26d9fb41de9f4309ba77deaccde02dea625cdaf/docs/broker.md).
- [Brig profiles](https://github.com/brig-sh/brig/blob/e26d9fb41de9f4309ba77deaccde02dea625cdaf/docs/profiles.md).
- [Brig runtime-uri](https://github.com/brig-sh/brig/blob/e26d9fb41de9f4309ba77deaccde02dea625cdaf/docs/runtimes.md).
- [Brig daemon opțional](https://github.com/brig-sh/brig/blob/e26d9fb41de9f4309ba77deaccde02dea625cdaf/docs/brigd.md).
- [Brig herd/SSH](https://github.com/brig-sh/brig/blob/e26d9fb41de9f4309ba77deaccde02dea625cdaf/docs/herd.md).
- [Brig claims și probe](https://github.com/brig-sh/brig/blob/e26d9fb41de9f4309ba77deaccde02dea625cdaf/docs/claims.md).
- [Brig non-goals și direcții redeschise](https://github.com/brig-sh/brig/blob/e26d9fb41de9f4309ba77deaccde02dea625cdaf/docs/non-goals.md).
- [Hull architecture](https://github.com/brig-sh/hull/blob/a1462e92d8d94c4f77ae2c51fabd20656cf67dc6/docs/architecture.md).
- [Hull running](https://github.com/brig-sh/hull/blob/a1462e92d8d94c4f77ae2c51fabd20656cf67dc6/docs/running.md).
- [Hull storage](https://github.com/brig-sh/hull/blob/a1462e92d8d94c4f77ae2c51fabd20656cf67dc6/docs/storage.md).
- [Hull networking](https://github.com/brig-sh/hull/blob/a1462e92d8d94c4f77ae2c51fabd20656cf67dc6/docs/networking.md).
- [Hull egress](https://github.com/brig-sh/hull/blob/a1462e92d8d94c4f77ae2c51fabd20656cf67dc6/docs/network-egress.md).
- [Hull port Linux](https://github.com/brig-sh/hull/blob/a1462e92d8d94c4f77ae2c51fabd20656cf67dc6/docs/build.md).
- [Hull performanță și limitele datelor](https://github.com/brig-sh/hull/blob/a1462e92d8d94c4f77ae2c51fabd20656cf67dc6/docs/performance.md).
- [Hull checkpoint/restore](https://github.com/brig-sh/hull/blob/a1462e92d8d94c4f77ae2c51fabd20656cf67dc6/docs/checkpoint-restore.md).
- [Versiunea hvi fixată de hull](https://github.com/brig-sh/hull/blob/a1462e92d8d94c4f77ae2c51fabd20656cf67dc6/hvi.pin).
- [hvi README](https://github.com/brig-sh/hvi-vmm/blob/351d1a15728e998486232e4815ba075a22e44f56/README.md).
- [hvi embedding](https://github.com/brig-sh/hvi-vmm/blob/351d1a15728e998486232e4815ba075a22e44f56/docs/embedding.md).
- [hvi architecture](https://github.com/brig-sh/hvi-vmm/blob/351d1a15728e998486232e4815ba075a22e44f56/docs/architecture.md).
- [hvi kernel și rootfs](https://github.com/brig-sh/hvi-vmm/blob/351d1a15728e998486232e4815ba075a22e44f56/docs/guest-images.md).
- [hvi storage și shares](https://github.com/brig-sh/hvi-vmm/blob/351d1a15728e998486232e4815ba075a22e44f56/docs/storage-and-sharing.md).
- [hvi networking](https://github.com/brig-sh/hvi-vmm/blob/351d1a15728e998486232e4815ba075a22e44f56/docs/networking.md).
- [hvi security](https://github.com/brig-sh/hvi-vmm/blob/351d1a15728e998486232e4815ba075a22e44f56/docs/security.md).
- [hvi limite cunoscute](https://github.com/brig-sh/hvi-vmm/blob/351d1a15728e998486232e4815ba075a22e44f56/docs/limitations.md).
- [hvi BootConfig](https://github.com/brig-sh/hvi-vmm/blob/351d1a15728e998486232e4815ba075a22e44f56/src/config.rs).
- [hvi selecția backendurilor macOS/Linux](https://github.com/brig-sh/hvi-vmm/blob/351d1a15728e998486232e4815ba075a22e44f56/src/lib.rs).
- [hvi confinement Seatbelt](https://github.com/brig-sh/hvi-vmm/blob/351d1a15728e998486232e4815ba075a22e44f56/src/sandbox/seatbelt.rs).

### libkrun

- [libkrun 2.0 README](https://github.com/containers/libkrun/blob/6151710f4ea627cf0e5dbce78e4192fb4d64c2e9/README.md).
- [libkrun 2.0 lifecycle](https://github.com/containers/libkrun/blob/6151710f4ea627cf0e5dbce78e4192fb4d64c2e9/docs/vm-lifecycle.md).
- [libkrun 2.0 init](https://github.com/containers/libkrun/blob/6151710f4ea627cf0e5dbce78e4192fb4d64c2e9/docs/init.md).
- [libkrun 2.0 networking](https://github.com/containers/libkrun/blob/6151710f4ea627cf0e5dbce78e4192fb4d64c2e9/docs/networking.md).
- [libkrun 2.0 security model](https://github.com/containers/libkrun/blob/6151710f4ea627cf0e5dbce78e4192fb4d64c2e9/docs/security-model.md).
- [libkrun 2.0 manifest și features](https://github.com/containers/libkrun/blob/6151710f4ea627cf0e5dbce78e4192fb4d64c2e9/src/libkrun/Cargo.toml).
- [libkrun 2.0 kernel extern](https://github.com/containers/libkrun/blob/6151710f4ea627cf0e5dbce78e4192fb4d64c2e9/src/libkrun/src/api/payload.rs).
- [libkrun 2.0 build/run API](https://github.com/containers/libkrun/blob/6151710f4ea627cf0e5dbce78e4192fb4d64c2e9/src/libkrun/src/api/vmm_builder.rs).
- [libkrun 2.0 device builders: vsock, block, balloon](https://github.com/containers/libkrun/blob/6151710f4ea627cf0e5dbce78e4192fb4d64c2e9/src/libkrun/src/api/device_builders.rs).
- [libkrun 2.0 start vCPU în build](https://github.com/containers/libkrun/blob/6151710f4ea627cf0e5dbce78e4192fb4d64c2e9/src/libkrun/src/vmm/builder.rs).
- [libkrun 2.0 stop și `_exit`](https://github.com/containers/libkrun/blob/6151710f4ea627cf0e5dbce78e4192fb4d64c2e9/src/libkrun/src/vmm/mod.rs).
- [libkrun 1.19.x API verificat](https://github.com/containers/libkrun/blob/865f9f4e486285dcc43aab96f085dedf11ea1454/include/libkrun.h).
- [libkrun 1.19.x implementarea kernel/vsock/start](https://github.com/containers/libkrun/blob/865f9f4e486285dcc43aab96f085dedf11ea1454/src/libkrun/src/lib.rs).

### Lima

Aceste surse sunt live sau pe `master`, nu fixate la un commit în analiză:

- [Lima overview](https://lima-vm.io/docs/).
- [VM types](https://lima-vm.io/docs/config/vmtype/).
- [VZ](https://lima-vm.io/docs/config/vmtype/vz/).
- [Filesystem mounts](https://lima-vm.io/docs/config/mount/).
- [Networking user-mode](https://lima-vm.io/docs/config/network/user/).
- [Port forwarding](https://lima-vm.io/docs/config/port/).
- [SSH](https://lima-vm.io/docs/usage/ssh/).
- [Plain mode](https://lima-vm.io/docs/config/plain/).
- [Discuri](https://lima-vm.io/docs/config/disk/).
- [Template implicit](https://github.com/lima-vm/lima/blob/master/templates/default.yaml).
- [Threat model](https://github.com/lima-vm/lima/blob/master/.oss-scanner/threat_model.md).
- [Drivere extensibile](https://lima-vm.io/docs/dev/drivers/).

### Dovezi și componente locale devcroft

- [README](../README.md).
- [Comparația existentă](comparison.md).
- [Capabilități declarate](../src/backend_capabilities.rs).
- [Agenți verificați și limitele măsurătorilor](agents.md).
- [Probleme cunoscute](known-gaps.md).
- [Provider și Resolution](../src/provider/mod.rs).
- [Captura environmentului](../src/provider/capture.rs).
- [Tooling separat](../src/tooling.rs).
- [Shell din closure](../src/shell.rs).
- [Closure roots și retenție](../src/fleet/closure.rs).
- [Init pentru namespace-uri](../src/fleet/init.rs).
- [Mount view](../src/fleet/mount.rs).
- [Lifecycle up](../src/lifecycle/up.rs).
- [Stare și Meta](../src/lifecycle/state.rs).
- [Protocol keeper](../src/keeper/protocol.rs).
- [SessionBackend](../src/keeper/session.rs).
- [Server SSH](../src/ssh/server.rs).
- [SFTP](../src/ssh/sftp.rs).
- [ProxyCommand](../src/ssh/proxy.rs).
- [Proxy egress](../src/proxy/server.rs).
- [Proiecția politicii în nono](../src/policy/capability_set.rs).
- [Criteriile backendului după eliminarea gVisor](../openspec/changes/remove-gvisor-backend/design.md).
- [Separarea provisioning/runtime](../openspec/changes/sandbox-provisioning/design.md).

Codul nono inspectat este versiunea `0.77.0`, în modulele
`src/sandbox/mod.rs`, `src/sandbox/linux.rs` și `src/sandbox/macos.rs` din
pachetul Cargo. Referință publică a API-ului:
[nono 0.77.0](https://docs.rs/nono/0.77.0/nono/).
