# devcroft: mediu local adaptabil, fleet dintr-o definiție completă

**Export consolidat al direcției de produs discutate la 5 noiembrie 2026.**

> **Local: poți începe cu un mediu de bază controlat și îl poți adapta în
> sandbox. Fleet: pornește numai dintr-o definiție completă și fixată a
> mediului.**

Această regulă este centrul documentului. Ea leagă adopția în afara ecosistemului
Nix de suportul pentru proiecte neîncrezute, fără dependențe de proiect instalate
pe host, și de pregătirea ulterioară a aceluiași proiect pentru fleet.

Documentul exportă cerințele exprimate și propunerile discutate. **Nu descrie
funcții deja implementate.** Devcroft actual are alt contract: cere un provider
suportat și nu oferă limita VM discutată aici. Adoptarea direcției necesită o
schimbare formală a specificațiilor și deciziilor existente.

## Cuprins

1. [Direcția de produs](#1-direcția-de-produs)
2. [Cerințe exprimate și propuneri](#2-cerințe-exprimate-și-propuneri)
3. [Izolare, completitudine și reproductibilitate](#3-izolare-completitudine-și-reproductibilitate)
4. [Experiența principală de intrare](#4-experiența-principală-de-intrare)
5. [Trei surse ale mediului local](#5-trei-surse-ale-mediului-local)
6. [Adaptarea mediului în sandbox](#6-adaptarea-mediului-în-sandbox)
7. [Configurația pe care o produce lucrul local](#7-configurația-pe-care-o-produce-lucrul-local)
8. [Proiect extern și configurație proprie](#8-proiect-extern-și-configurație-proprie)
9. [Reconstrucția curată și eligibilitatea fleet](#9-reconstrucția-curată-și-eligibilitatea-fleet)
10. [Execuția în fleet](#10-execuția-în-fleet)
11. [Host, checkout și workspace privat](#11-host-checkout-și-workspace-privat)
12. [Clonarea Git și începutul limitei de izolare](#12-clonarea-git-și-începutul-limitei-de-izolare)
13. [Politică, rețea și credențiale](#13-politică-rețea-și-credențiale)
14. [Runtime și responsabilitățile componentelor](#14-runtime-și-responsabilitățile-componentelor)
15. [Stare, persistență și experiența utilizatorului](#15-stare-persistență-și-experiența-utilizatorului)
16. [Scenarii complete](#16-scenarii-complete)
17. [Ce trebuie să demonstreze spike-urile](#17-ce-trebuie-să-demonstreze-spike-urile)
18. [Relația cu planurile existente](#18-relația-cu-planurile-existente)
19. [Întrebări încă deschise](#19-întrebări-încă-deschise)
20. [Rezumat și referințe](#20-rezumat-și-referințe)

## 1. Direcția de produs

### 1.1. Problema de adopție

Un instrument pentru lucru cu repository-uri externe nu se poate baza exclusiv
pe proiecte care au deja un mediu Nix, devenv, flox sau devbox configurat.
Multe proiecte au numai manifestele limbajelor, scripturi de instalare și
instrucțiuni în README. Unele au Dev Containers. Altele presupun pur și simplu
un laptop deja pregătit.

În discuție, utilizatorul a caracterizat această situație prin „99% din
proiecte”. Este o apreciere a obstacolului de adopție, nu o statistică măsurată
în această analiză. Cerința rezultată este concretă: **un proiect fără mediu
declarat trebuie să poată primi valoare din devcroft înainte de a fi migrat.**

Utilizatorul trebuie să poată:

- deschide un proiect extern fără să îi instaleze toolchain-ul pe host;
- lucra local într-un mediu izolat;
- descoperi și instala în sandbox ce lipsește;
- descrie mediul corect, dacă dorește;
- verifica acea definiție într-un mediu curat;
- folosi rezultatul în fleet.

### 1.2. Promisiunea de intrare

O formulare de produs propusă este:

> Deschide local un proiect într-un mediu izolat. Toolchain-ul, dependențele și
> serviciile sunt pregătite în mediul administrat de devcroft, fără instalări
> de proiect pe host. Când definiția mediului este completă și verificată, o
> poți folosi pentru fleet.

Reproductibilitatea rămâne importantă. Rolul ei se schimbă în experiența de
intrare: pentru local poate fi construită progresiv; pentru fleet este o
condiție înainte de execuție.

Nix poate rămâne o fundație tehnică pentru baze și medii, fără să devină o
condiție pe care fiecare utilizator trebuie s-o înțeleagă sau s-o configureze
înainte de primul proiect.

### 1.3. Parcursul complet

```text
proiect extern
      │
      ▼
git clone pe host → cd proiect → devcroft up
      │
      ▼
VM + workspace de lucru + bază identificată
      │
      ▼
lucru local și adaptarea mediului
      │
      ├── utilizatorul poate rămâne aici
      │
      ▼
definiție completă + intrări fixate + verificare declarată
      │
      ▼
reconstrucție curată în izolare
      │
      ▼
mediu verificat și identificat
      │
      ▼
fleet: N agenți, aceeași bază pregătită,
workspace și stare privată pentru fiecare
```

`devcroft up` este comanda existentă folosită pentru a ilustra experiența
țintă. Detectarea fără manifest, importul în VM și pregătirea pentru fleet din
această schemă sunt comportamente propuse. Comenzile pentru salvarea,
verificarea și promovarea mediului nu au fost alese.

## 2. Cerințe exprimate și propuneri

### 2.1. Cerințele exprimate de utilizator

1. **Proiectele neîncrezute sunt centrale pentru succesul devcroft.** Izolarea
   trebuie să fie adecvată acestui caz de utilizare.
2. **Modul local trebuie să folosească o VM.** Candidații discutați pentru
   implementare sunt biblioteca hvi și libkrun 2.0.
3. **Adopția trebuie să depășească nișa Nix.** Proiectele fără mediu declarat și
   cele cu Dev Containers trebuie luate în calcul.
4. **Local poți începe cu o bază controlată și o poți adapta.** Lipsa unei
   definiții complete nu trebuie să blocheze orice utilizare locală.
5. **Fleet cere un mediu complet reproductibil.** O sesiune configurată doar
   interactiv nu este suficientă.
6. **Utilizatorul poate configura el un proiect extern.** Nu trebuie să aștepte
   ca maintainerul proiectului să adopte devcroft sau Nix.
7. **Dependențele proiectului nu se instalează pe host.** Pregătirea lor aparține
   mediului administrat de devcroft.
8. **Fluxul principal este familiar:** utilizatorul clonează proiectul, intră
   în director și pornește devcroft acolo.
9. **Se evaluează două spike-uri:** hvi și libkrun 2.0, pe workload-uri comune.

### 2.2. Propuneri de arhitectură și experiență

Următoarele mecanisme susțin cerințele, dar detaliile lor nu au fost adoptate
ca specificație finală:

- workspace privat în guest pentru proiectele neîncrezute;
- acces prin editor Remote-SSH;
- baze de dezvoltare versionate și profiluri de limbaj;
- configurație externă asociată repository-ului, pe lângă cea comisă;
- reconstrucție curată drept pas obligatoriu pentru eligibilitatea fleet;
- materializator izolat și artefact read-only reutilizabil;
- proxy host și transport vsock;
- nono pentru politica workload-ului în guest și confinement host adaptat VMM-ului;
- import și export controlat între checkoutul host și workspace-ul guest.

Separarea aceasta evită prezentarea unei recomandări de implementare ca și cum
ar fi deja o alegere de produs sau o funcție disponibilă.

## 3. Izolare, completitudine și reproductibilitate

### 3.1. Trei proprietăți independente

| Proprietate | Întrebarea |
|---|---|
| Izolare | Ce poate atinge proiectul pe host, în alte instanțe și în exterior? |
| Completitudinea mediului | Există toolchain-ul, bibliotecile, serviciile și pregătirea necesare? |
| Reproductibilitate | Poate fi refăcut mediul declarat din intrări fixate? |

Un mediu poate fi bine izolat, deși proiectul nu compilează încă. Un mediu
poate conține toate instrumentele necesare, deși unele au fost instalate
interactiv și nu sunt descrise. O definiție fixată poate reproduce un mediu
care nu este suficient pentru proiect.

De aceea, disponibilitatea sandboxului și disponibilitatea proiectului trebuie
raportate separat.

### 3.2. Regula local/fleet

| Aspect | Mod local propus | Fleet propus |
|---|---|---|
| VM pentru proiecte neîncrezute | Obligatorie | Obligatorie pentru același caz de utilizare |
| Configurație de proiect la început | Poate lipsi | Definiția mediului trebuie să existe înainte de pornire |
| Bază versionată | Da | Da, ca parte a mediului identificat |
| Mediu complet la prima pornire | Nu este obligatoriu | Obligatoriu înainte de lansarea agenților |
| Instalări interactive | Permise în instanța privată, sub politica aplicată | Nu pot fi o dependență nedeclarată a pornirii |
| Reconstrucție curată verificată | Disponibilă pentru formalizare | Cerută pentru mediul acceptat |
| Dependențe de proiect pe host | Nu | Nu |
| Modificarea bazei comune | Nu | Nu |

**Flexibilitatea locală privește maturitatea definiției mediului. Nu justifică
acces implicit mai larg la host.**

### 3.3. Garanțiile trebuie numite precis

O bază fixată identifică ceea ce a pornit devcroft. Nu dovedește că a fost
identificat întregul mediu al proiectului.

O imagine fixată prin digest poate furniza același filesystem inițial fiecărui
agent. Nu dovedește singură că rețeta ei este restorable, că setupul proiectului
este complet sau că rezultatul buildului aplicației este bit-reproductibil.

Un build și o comandă de verificare reușite oferă dovezi pentru workload-ul
exercitat. Nu demonstrează toate căile posibile ale proiectului. Definiția
eligibilității fleet trebuie să precizeze ce rezultat verificat acceptă.

## 4. Experiența principală de intrare

### 4.1. Comenzile utilizatorului

```sh
git clone https://github.com/autor/proiect
cd proiect
devcroft up
```

Acesta este fluxul principal discutat pentru 1.0. Nu presupune că proiectul are
deja `devcroft.toml`, un provider Nix sau un Docker instalat pe laptop.

### 4.2. Ce face devcroft

1. Identifică directorul și configurația de proiect disponibilă.
2. Citește declarațiile relevante ca date, fără a activa mediul pe host.
3. Alege explicit sursa mediului sau cere utilizatorului o alegere locală când
   informația nu este suficientă; regula exactă de selecție rămâne de stabilit.
4. Pornește infrastructura VM înainte de orice execuție controlată de proiect.
5. Pregătește workspace-ul guest și mediul ales.
6. Rulează setupul în izolare, cu politica sa.
7. Expune sesiunile și endpointul editorului.
8. Arată ce funcționează, ce lipsește și dacă mediul este pregătit pentru fleet.

Detectarea poate însemna citirea unui `devcontainer.json`, `flake.nix` sau
manifest de limbaj. Evaluarea unui fișier executabil, buildurile și hookurile
aparțin fazei izolate.

Un manifest de limbaj poate sugera un profil de bază, dar nu este dovada
completitudinii mediului. De exemplu, `package.json` nu descrie toate
bibliotecile de sistem sau serviciile necesare aplicației.

### 4.3. Prima utilizare fără configurație devcroft

Un rezultat util poate fi:

```text
VM: disponibilă
workspace: privat, în guest
mediu: bază Python identificată
setup proiect: nedefinit
fleet: nepregătit — definiția mediului nu este completă
```

Acesta este un exemplu conceptual de informație afișată, nu wording sau format
de CLI decis. Utilizatorul poate intra în mediu și continua pregătirea.

## 5. Trei surse ale mediului local

### 5.1. Proiect cu mediu declarat

Un proiect cu Nix, devenv, flox sau devbox oferă deja o parte importantă din
definiție. Devcroft o folosește pentru a materializa toolchain-ul și a captura
environmentul și serviciile.

Pentru modul VM, targetul este Linux. O rezolvare `aarch64-darwin` de pe Mac nu
se poate executa într-un guest `aarch64-linux`. Locul materializării și
targetul trebuie înregistrate.

Chiar un mediu cu lockfile poate avea hookuri sau builduri controlate de
proiect. **Un lockfile fixează intrările; nu le conferă încredere.** Evaluarea,
materializarea și activarea care execută astfel de cod trebuie să aibă o
limită potrivită.

Un proiect cu provider existent poate fi mai aproape de fleet, dar existența
fișierului nu înlocuiește verificarea setupului și serviciilor.

### 5.2. Proiect cu Dev Container

Dev Containers reprezintă o cale de adopție importantă: utilizatorul trebuie
să poată valorifica definiția pe care proiectul o are deja.

Suprafața de compatibilitate discutată are mai multe niveluri:

1. Imagine preconstruită, identificată prin digest.
2. Dockerfile și build context.
3. Features și dependențele lor.
4. Lifecycle commands.
5. Compose, servicii, volumes și identitate de rețea.

Un release poate acoperi o parte din acestea, dar mesajul „suport Dev
Containers” trebuie legat de câmpurile efectiv transportate. Un câmp care cere
execuție, acces sau semantică neimplementată nu trebuie să dispară în tăcere.

Cerințe de integrare:

- buildurile rulează într-un mediu izolat, nu prin Docker socketul hostului;
- Features sunt cod executat în pregătire și primesc aceeași disciplină;
- comenzile de lifecycle rulează în faza izolată corespunzătoare;
- `initializeCommand`, definit de format ca execuție host, este refuzat sau
  primește o semantică alternativă explicită, fără execuție pe laptop;
- referințele la căi, variabile și mounturi host sunt mediate de politica
  utilizatorului;
- configurația efectivă include și metadata relevantă din imagine, nu numai
  fișierul proiectului;
- utilizatorul, porturile, datele serviciilor și ordinea pornirii sunt
  reconciliate cu modelul devcroft.

Un builder sau un engine de containere poate exista în guest dacă implementarea
aleasă are nevoie de el. Cerința este absența dependenței de un engine
administrat de utilizator pe host și a autorității asupra hostului. Nu rezultă
automat că trebuie folosită virtualizare imbricată: buildurile și containerele
din guest pot folosi kernelul acelui guest, dacă suportul necesar există.

**Această cale folosește rootfs-ul imaginii.** Guest-ul minimal cu closure Nix
este potrivit mediilor closure-tier; nu trebuie impus ca format unic pentru
toate proiectele.

### 5.3. Proiect fără mediu declarat

Devcroft furnizează sau permite alegerea unei baze cunoscute:

- bază generală de dezvoltare;
- profil Python, Node, Rust, Go sau JVM;
- bază personală selectată de utilizator.

Lista exactă nu a fost decisă. Important este contractul:

- baza are versiune și identitate verificabilă;
- alegerea este vizibilă;
- devcroft nu pretinde că a dedus întregul mediu al proiectului;
- instanța are stare writable privată;
- utilizatorul poate instala ce lipsește fără să modifice baza altor instanțe;
- lipsa definiției complete nu împiedică lucrul local.

Baza poate fi construită intern cu Nix sau distribuită ca imagine. Această
alegere nu trebuie să oblige utilizatorul să învețe providerul intern.

## 6. Adaptarea mediului în sandbox

### 6.1. Lucrul local este și o etapă de descoperire

Utilizatorul sau agentul poate:

- instala o bibliotecă nativă necesară;
- alege versiunea runtime-ului;
- crea un virtualenv sau instala dependențele aplicației;
- configura o bază de date locală guest;
- încerca o comandă de setup;
- descoperi variabilele și destinațiile de rețea necesare;
- rula builduri și teste.

Toate acestea se petrec în VM. Instanța poate rămâne utilă chiar dacă nu este
încă eligibilă pentru fleet.

### 6.2. Stare privată și autoritate administrativă

Un strat writable privat trebuie să permită modificările locale. Baza comună
rămâne nemodificată.

Nu este decis dacă instalările administrative se fac printr-un helper de
setup, o sesiune administrativă izolată sau alt mecanism. Cerința este ca
privilegiile necesare instalării să rămână în instanța guest și să nu ofere
acces la host, la store-ul comun sau la alte instanțe.

Politica nono din guest nu trebuie să facă imposibilă chiar adaptarea locală
promisă. Instalarea de pachete și execuția obișnuită a workload-ului pot avea
faze și autorități distincte.

### 6.3. Instalarea interactivă nu devine automat definiție

Un exemplu de stare locală acumulată:

```text
bază Python fixată
  + bibliotecă de sistem instalată manual
  + variabilă exportată într-un shell
  + director creat de un script
  + Postgres pornit manual
```

Sesiunea poate funcționa. Pentru fleet, fiecare dependență necesară trebuie
reprezentată în definiție sau într-un artefact acceptat explicit, împreună cu
setupul și identitatea lui.

Devcroft poate ajuta la redactarea configurației, inclusiv printr-un agent.
Nu trebuie să presupună că poate reconstrui intenția completă din istoricul
shellului sau din diferențele unui disk.

### 6.4. Nicio obligație de promovare

Utilizatorul poate folosi numai modul local:

- pentru a inspecta un repository;
- pentru o modificare punctuală;
- pentru a rula un proiect extern;
- pentru a explora un setup dificil.

Formalizarea mediului devine necesară când dorește fleet sau o reconstrucție
predictibilă, nu ca taxă obligatorie înainte de primul rezultat util.

## 7. Configurația pe care o produce lucrul local

### 7.1. Pachetul de pregătire

Definiția suficientă pentru proiect ar trebui să cuprindă:

| Componentă | Conținut |
|---|---|
| Bază și target | Artefactul inițial, arhitectura și sistemul guest |
| Toolchain | Runtime-uri, compilatoare, linkere, utilitare și biblioteci native |
| Dependențe aplicație | Lockfile-uri și pașii de instalare necesari |
| Setup | Comenzi declarate, ordine, directoare și fișiere generate |
| Environment | Valori nesensibile și referințe explicite la secrete |
| Servicii | Configurație, readiness, dependențe și oprire |
| Date inițiale | Schema, fixtures sau seedurile necesare fiecărei instanțe |
| Rețea | Cerințe distincte pentru pregătire și runtime |
| Verificare | Comanda sau comenzile care probează pregătirea proiectului |
| Identitate | Intrările fixate și artefactele materializate |

Nu este vorba numai despre o listă de pachete. Un proiect care găsește
compilatorul, dar nu are schema bazei de date sau un fișier generat, nu este
pregătit pentru multiplicare în fleet.

### 7.2. Ce trebuie fixat

În funcție de sursa mediului:

- reviziile surselor providerului;
- pachetele și dependențele lor;
- imaginea și platforma efectivă;
- bazele unui Dockerfile și versiunile Features;
- lockfile-urile limbajelor;
- versiunile helperilor și formatul artefactului;
- intrările setupului care influențează mediul.

Un setup care descarcă „latest” fără a păstra identitatea intrării poate
reconstitui alt mediu mai târziu. Simpla memorare a comenzii nu rezolvă această
problemă.

### 7.3. Trei rezultate diferite

| Rezultat | Ce oferă | Ce nu demonstrează singur |
|---|---|---|
| Snapshot al sesiunii | Reluarea unei stări locale | Reconstrucția stării din definiție |
| Definiție declarată | Descrierea pașilor și intrărilor | Că definiția este completă și funcționează |
| Reconstrucție curată verificată | Dovezi că definiția acoperă workload-ul declarat | Toate comportamentele posibile ale proiectului |

Snapshotul poate fi util pentru continuitatea locală. **Nu este suficient
singur pentru promovarea în fleet.**

## 8. Proiect extern și configurație proprie

### 8.1. Configurația nu cere adoptarea upstream

Utilizatorul poate păstra definiția:

- în repository, pentru review și distribuire cu proiectul;
- într-o configurație proprie sau a organizației, asociată repository-ului.

Locul și formatul configurației externe nu au fost decise. Fleet trebuie să
poată consuma o definiție explicită și fixată chiar dacă autorul repository-ului
nu a comis un manifest devcroft.

Aceasta permite contribuții la proiecte externe fără modificări administrative
nedorite în patchul trimis upstream.

### 8.2. Identitatea asocierii

Asocierea nu ar trebui să depindă numai de numele directorului. Trebuie
stabilită o identitate suficientă pentru:

- repository și origine;
- subdirectorul proiectului într-un monorepo;
- target și sursa mediului;
- revizia definiției externe;
- revizia sursei folosite la verificare.

Schimbarea remote-ului, a contextului de build sau a subproiectului nu trebuie
să reutilizeze accidental o pregătire destinată altui proiect.

### 8.3. Definiția mediului și granturile utilizatorului

Configurația mediului se poate distribui. Autoritatea asupra laptopului și a
conturilor personale aparține utilizatorului.

Un repository poate cere un secret sau un mount, dar nu trebuie să poată
obține automat home-ul, cheia SSH sau credențialele cloud prin simpla
declarare a lor în manifest.

Aceeași regulă se aplică definițiilor externe importate de la altcineva:
utilitatea lor pentru setup nu le conferă autoritate implicită asupra hostului.

## 9. Reconstrucția curată și eligibilitatea fleet

### 9.1. Verificarea care face diferența

După salvarea configurației, devcroft pregătește un mediu nou folosind numai
intrările declarate:

```text
definiție salvată + intrări fixate
                  │
                  ▼
            instanță curată
                  │
                  ▼
       materializare și setup izolat
                  │
                  ▼
       servicii ready + verificare proiect
                  │
                  ▼
       rezultat înregistrat pentru acele intrări
```

Instanța curată nu preia modificările interactive ale sesiunii exploratorii,
home-ul ei sau fișierele ei generate nedeclarat.

Cache-urile de artefacte pot fi folosite dacă identitatea intrărilor și
rezultatelor este respectată. „Curat” înseamnă independență de starea
accidentală a sesiunii, nu obligația de a descărca și compila totul de fiecare
dată.

### 9.2. Verificarea proiectului

Comanda potrivită depinde de proiect. Exemple:

- compilarea aplicației;
- pregătirea testelor;
- o verificare de tipuri și lint;
- pornirea serviciilor și un smoke test;
- o combinație declarată de astfel de operații.

Un eșec trebuie atribuit: definiție invalidă, materializare eșuată, resursă
refuzată, serviciu indisponibil sau verificare a proiectului eșuată.

Nu este suficient „VM-ul a pornit”. Nici un succes obținut prin probe sărite
nu trebuie confundat cu demonstrarea pregătirii.

### 9.3. Condițiile propuse pentru fleet

Înainte de lansare:

1. Există o definiție explicită a mediului.
2. Intrările necesare sunt fixate și pot fi recuperate sau sunt disponibile
   ca artefacte cu o garanție acceptată explicit.
3. Toolchain-ul și serviciile sunt complete pentru workload-ul declarat.
4. Setupul este descris și poate pregăti o instanță nouă.
5. Reconstrucția curată și verificarea declarată au reușit.
6. Nu există o dependență cunoscută de modificări interactive nesalvate.
7. Rezultatul verificării este legat de identitatea mediului și de contextul
   sursei.

Aceasta este formalizarea propusă a cerinței utilizatorului. Nu există încă un
format sau o comandă implementată pentru această eligibilitate.

### 9.4. Când verificarea devine veche

O schimbare a definiției, targetului, lockfile-urilor, bazei, setupului,
serviciilor sau verificării poate invalida rezultatul anterior.

Și sursa poate influența mediul: un Dockerfile din repository, un fișier Nix
sau o comandă care compilează codul din branch trebuie incluse în contextul
relevant. Regula exactă de staleness trebuie definită, fără a presupune că
orice modificare de cod cere refacerea întregii baze.

## 10. Execuția în fleet

### 10.1. Ce se multiplică

```text
mediu pregătit și identificat
        ├── agent A + workspace A + servicii A + date A
        ├── agent B + workspace B + servicii B + date B
        └── agent C + workspace C + servicii C + date C
```

Fleet consumă baza pregătită și definiția verificată. Fiecare agent are propria
stare writable, propriile servicii și propriul workspace.

Nu trebuie presupus că fiecare agent reutilizează o instanță complet pregătită
cu sursa altui branch. Unele operații de setup sau verificare sunt specifice
workspace-ului; ele se declară și se execută pentru contextul fiecărui agent.

### 10.2. Reutilizarea costului

Pachetele, imaginile și alte artefacte comune pot fi materializate o dată și
reutilizate. Acest lucru nu garantează că:

- fiecare agent are cost zero de pornire;
- buildurile aplicației sunt partajate;
- memoria page cache este comună guest-urilor;
- datele serviciilor pot fi partajate writable.

Formatul de storage și mecanismele de cache trebuie măsurate separat.

### 10.3. Instalațiile agentului în timpul unei sarcini

Un agent poate crea fișiere și build outputs în workspace-ul său. Politica
poate permite sau restrânge și alte operații. Ceea ce nu trebuie permis ca
model de pregătire este dependența de „fiecare agent va descoperi și instala
singur mediul lipsă”.

O schimbare necesară a mediului de bază revine în definiție și în ciclul de
verificare. Astfel, agenții următori pornesc dintr-un mediu cunoscut.

## 11. Host, checkout și workspace privat

### 11.1. Ce înseamnă „nimic instalat local”

În sensul discutat: **nu se cer instalări de dependențe ale proiectului pe
host**.

| Pe host | În mediul administrat |
|---|---|
| Devcroft și helperii săi distribuiți | Providerii necesari materializării |
| Git, pentru fluxul manual ales | Compilatoare, runtime-uri și biblioteci |
| Editorul utilizatorului | Serverul editorului remote, unde este necesar |
| API-urile de virtualizare ale platformei | Builder și eventual engine guest |
| Artefacte cache, kernel și diskuri ca date | Baze de date și alte servicii |

Kernelul guest, diskurile și cache-urile ocupă spațiu pe laptop. „Fără
instalarea toolchain-ului pe host” nu înseamnă lipsa datelor locale sau a unui
runtime devcroft distribuit pentru platformă.

Utilizatorul nu ar trebui să configureze separat Nix, flox, devbox, Docker sau
Postgres pe host ca să încerce un proiect prin această cale.

### 11.2. Importul propus

Pentru un proiect neîncrezut, recomandarea discutată este workspace privat:

```text
checkout host
     │
     ▼
import de sursă și istoric în guest
     │
     ▼
lucru în workspace privat
     │
     ▼
inspectare și export al rezultatului
```

Importul nu trebuie să piardă în tăcere modificările necomise sau fișierele
neurmărite necesare proiectului. Scope-ul importului și tratamentul lor sunt
de stabilit și trebuie făcute vizibile.

Nu se copiază orbește `.git` ca autoritate administrativă. Acesta poate
conține hookuri, comenzi în configurație, referințe de worktree, alternates
sau căi către alte directoare. Sursa și istoricul necesar se importă printr-un
mecanism care produce metadate Git controlate în guest.

Symlinkurile și căile din datele importate nu trebuie urmate de host ca acces
implicit la alte directoare.

### 11.3. Editorul și rezultatul

Cu workspace privat, schimbările apar în guest. Devcroft trebuie să spună
unde lucrează utilizatorul și cum deschide endpointul Remote-SSH.

La final, rezultatul poate fi exportat ca patchuri, commituri sau fișiere
selectate; forma exactă nu este decisă. Exportul trebuie să păstreze controlul
asupra destinației host și să nu importe automat configurația executabilă Git
sau starea administrativă a guest-ului.

Inspecția rezultatului nu conferă încredere automată tuturor fișierelor
exportate: scripturile și configurațiile rămân cod care poate fi executat
ulterior de unelte host.

### 11.4. Partajarea read-write este altă semantică

Un checkout host montat read-write oferă integrare imediată cu editorul local,
dar permite schimbări reale inclusiv în `.git` și în fișiere pe care hostul le
poate executa ulterior.

Această variantă ar trebui evaluată separat și numită explicit. Nu are aceeași
proprietate de separare față de checkout ca workspace-ul privat propus pentru
cazul neîncrezut.

## 12. Clonarea Git și începutul limitei de izolare

### 12.1. Răspunsul despre hookuri

Git poate executa `post-checkout` după checkoutul inițial al `git clone`.
`--no-checkout` omite acel checkout și acel apel, dar nu reprezintă o garanție
generală asupra tuturor operațiilor unei clonări.

În clonarea remote obișnuită:

| Sursă | Comportament |
|---|---|
| `.git/hooks` al remote-ului | Nu este transferat ca director de hookuri activ |
| `.git/config` al remote-ului | Nu devine configurația locală a clonei |
| Director versionat `.githooks/` | Nu se activează numai prin prezență |
| Template Git instalat pe host | Poate furniza hookuri locale active |
| `core.hooksPath` configurat local/global | Poate selecta hookurile executate |
| `.gitattributes` din proiect | Poate selecta un filtru deja definit în configurația locală |

Un filtru `smudge` sau `process`, precum Git LFS, poate rula în timpul
checkoutului dacă este configurat pe mașină. Comanda filtrului nu este
definită doar de `.gitattributes`.

Prin urmare, repository-ul remote nu își instalează în mod normal propriul
hook doar prin clonare. Nici nu este corect să se promită că `git clone` nu
execută niciodată programe sau că nu are suprafață de vulnerabilități.

### 12.2. Contractul pentru fluxul principal

Pentru `git clone` pe host urmat de devcroft în folder, limita este:

> Devcroft izolează pregătirea și execuția proiectului începând cu deschiderea
> lui prin devcroft.

Operațiile anterioare, inclusiv clonarea manuală, un setup pornit de utilizator
sau un automatism al editorului host, nu pot fi protejate retrospectiv.

La intrarea în devcroft, evaluarea providerilor, instalările și comenzile
proiectului se mută în VM. Folosirea Git pe host asupra unor metadate modificate
de guest nu trebuie introdusă ulterior ca scurtătură de import/export.

### 12.3. Pornirea din URL este o cale suplimentară

O viitoare intrare directă prin URL poate clona și face checkout în guest cu
o configurație Git controlată. Ar include clonarea în limita administrată și
ar elimina necesitatea Git host pentru acea cale.

Este o opțiune discutată pentru produs, fără sintaxă de comandă decisă. **Nu
înlocuiește fluxul principal cerut: clonare manuală, intrare în director,
pornire devcroft.**

Un hook local care ar porni devcroft după clonare intervine prea târziu pentru
a izola checkoutul deja efectuat și nu este necesar pentru workflow-ul principal.

## 13. Politică, rețea și credențiale

### 13.1. Manifestul proiectului exprimă nevoi

Pentru proiectele neîncrezute, configurația repository-ului nu poate fi
autoritatea finală asupra resurselor utilizatorului.

```text
cerințe proiect
       +
limite și alegeri ale utilizatorului
       ↓
granturi efective inspectabile
```

Schema de compoziție exactă rămâne de proiectat. Proprietatea cerută este că
o declarație a proiectului nu își poate acorda singură home-ul, Docker socketul
host sau toate cheile utilizatorului.

### 13.2. Setup și runtime au nevoi diferite

Pregătirea poate necesita registries, package caches sau downloaduri care nu
sunt necesare sesiunilor agentului după setup.

Politicile rămân distincte:

- materializarea și setupul folosesc accesul declarat pentru acea fază;
- runtime-ul primește numai accesul său;
- procesele de setup nu lasă în urmă canale cu autoritatea mai largă a
  pregătirii.

Explorarea locală poate descoperi noi nevoi de rețea. Alegerea lor trebuie să
fie vizibilă și să poată intra în definiția salvată. Nu este decis mecanismul
interactiv sau schema exactă a acestor alegeri.

### 13.3. Credențialele nu sunt rezolvate de VM

O cheie reală în environment poate fi citită de procesele din același domeniu
de acces: agent, build scripts, teste și dependențe. VM-ul izolează acel domeniu
de host; nu separă automat agentul de proiect în interior.

Direcția de broker discutată păstrează, unde protocolul permite, credențiala
pe host și oferă guest-ului un token restrâns pentru un endpoint. Livrarea
directă, când este necesară, rămâne o alegere explicită asupra unei
credențiale cu autoritate limitată.

Configurația reproductibilă conține referințe și nevoi, nu valorile secrete.
Snapshoturile și imaginile de mediu nu trebuie să devină mecanismul de
distribuire a loginurilor personale către agenți.

### 13.4. Ce nu trebuie confundat

- Un allowlist de domenii limitează destinațiile; nu limitează toate acțiunile
  autorizate la acele destinații.
- Tokenul proxy-ului egress autentifică utilizarea proxy-ului; nu este automat
  un broker al cheilor API.
- Un lockfile fixează codul; nu îl face de încredere.
- Un guest pornit nu dovedește că provisioningul a fost confinat.
- Un VMM confinat nu ascunde autoritatea descriptorilor pe care îi deține deja.

## 14. Runtime și responsabilitățile componentelor

### 14.1. Rolurile propuse

```text
HOST
├── controller devcroft
│     ├── configurație, stare și lifecycle
│     ├── import / export
│     └── coordonarea pregătirii și verificării
├── proxy / broker separat
└── helper VMM dedicat
      └── hvi sau libkrun 2.0
            └── GUEST LINUX
                  ├── init minimal
                  ├── pregătire izolată
                  └── keeper + SSH + workload + servicii
```

Materializatorul sau builderul poate avea o VM separată, reutilizabilă, ori
poate folosi instanța locală după reguli explicite. Topologia exactă nu este
decisă. Cerința este ca primul cod controlat de proiect să ruleze deja într-o
limită adecvată și să nu primească autoritate asupra altor medii.

### 14.2. Rolul nono

Nono din guest poate păstra politica filesystem/process/network a keeperului
și workload-ului. Kernelul guest trebuie configurat pentru capabilitățile
Landlock cerute.

Confinarea VMM-ului pe host este o responsabilitate distinctă:

- hvi are Seatbelt propriu pe macOS și seccomp pe Linux;
- libkrun cere integrarea unei confinări potrivite;
- nono este un candidat pentru limitarea resurselor host ale helperului;
- pe macOS nu se suprapun arbitrar profilul Seatbelt hvi și încă un profil
  nono în același proces.

Mai ales la libkrun 2.0, momentul confinării trebuie verificat: codul inspectat
poate porni vCPU-uri în `build()`, înainte de apelul `run()`.

### 14.3. Două forme de guest

| Sursa mediului | Formă posibilă |
|---|---|
| Provider closure-tier | Kernel + init mic + closure read-only + disk privat |
| Dev Container | Kernel + bootstrap + rootfs-ul imaginii + strat privat |
| Bază devcroft adaptabilă | Bază versionată + stare writable privată + unelte de pregătire |

Mediul local adaptabil cere verificarea instalărilor de sistem și a
persistenței. Nu este suficient un spike care rulează un singur executabil
static dintr-un guest read-only.

### 14.4. Guest Linux și proiecte macOS

Pe Mac, modul VM discutat rulează Linux. Xcode, AppKit și buildurile Darwin
native nu se mută în acel guest. O eventuală cale nativă are altă garanție și
trebuie numită separat.

Workload-urile locale Linux sunt ținta directă a acestui workflow. Extinderea
la alt sistem guest nu a fost decisă.

## 15. Stare, persistență și experiența utilizatorului

### 15.1. Stări conceptuale

```text
proiect identificat
       ↓
bază selectată
       ↓
sandbox local disponibil
       ↓
mediu adaptat, cu posibile schimbări nesalvate
       ↓
definiție salvată și intrări fixate
       ↓
reconstrucție și verificare în curs
       ├── eșec → revizuirea definiției
       └── succes → mediu eligibil pentru fleet
                           ↓
                 intrări relevante schimbate
                           ↓
                   verificare devenită veche
```

Acestea sunt concepte de produs, nu nume de status sau o schemă de protocol
deja adoptată.

### 15.2. Ce trebuie să fie vizibil

Utilizatorul ar trebui să poată afla:

- în ce workspace lucrează;
- ce mediu și target au fost alese și din ce sursă;
- dacă VM-ul, keeperul și serviciile răspund;
- ce setup a fost executat și cu ce rezultat;
- ce parte este declarată și ce schimbări locale rămân nesalvate, în măsura
  în care devcroft le poate identifica;
- dacă definiția a fost verificată și pentru ce intrări;
- de ce fleet este permis sau refuzat;
- ce resurse sunt acordate efectiv.

Devcroft nu trebuie să afirme că toate modificările semnificative au fost
înțelese automat. Reconstrucția curată este proba care completează descrierea.

### 15.3. Oprire, reluare și ștergere

Oprirea locală trebuie să păstreze workspace-ul și adaptările pe care produsul
le declară persistente. Reluarea unei sesiuni locale nu este aceeași operație
cu reconstruirea definiției.

Reconstrucția de verificare ar trebui să folosească o instanță separată, fără
să distrugă sesiunea exploratorie. Ștergerea unui sandbox nu trebuie să
șteargă checkoutul host sau definiția externă reutilizabilă printr-o asociere
implicită.

Politica exactă de retenție pentru diskuri, snapshoturi și artefacte comune
rămâne de stabilit. `down`, `rm` și recrearea trebuie să aibă efecte precise
și consecvente cu lifecycle-ul devcroft.

## 16. Scenarii complete

### 16.1. Repository Python fără configurație de mediu

1. Utilizatorul clonează și intră în director.
2. Devcroft pornește o bază Python identificată într-o VM.
3. Proiectul este importat în workspace privat.
4. Instalarea dependențelor arată că lipsește o bibliotecă nativă.
5. Utilizatorul o instalează în instanță și obține un build funcțional.
6. Salvează biblioteca, versiunea Python, dependențele și setupul într-o
   definiție proprie.
7. O instanță curată reproduce setupul și trece verificarea declarată.
8. Fleet folosește definiția și artefactele pregătite, cu stare privată per agent.

Nici Python, nici biblioteca nativă, nici serviciile nu sunt instalate pe host.

### 16.2. Repository cu Dockerfile și Dev Container Features

1. Devcroft citește configurația și explică suprafața suportată.
2. Buildul și Features rulează în builderul izolat.
3. Imaginea rezultată este identificată, iar runtime-ul are strat privat.
4. Lifecycle commands pregătesc workspace-ul guest.
5. Utilizatorul lucrează local și completează definiția, dacă este necesar.
6. Pentru fleet, fixează toate intrările cerute și verifică un start curat.

Un Dockerfile funcțional local, dar dependent de downloaduri nefixate, nu este
automat suficient pentru garanția strictă fleet.

### 16.3. Repository cu devenv și servicii

1. Devcroft folosește definiția și lockfile-ul proiectului pentru target Linux.
2. Materializarea și hookurile sunt separate și confinate corespunzător.
3. Serviciile pornesc în guest și raportează readiness.
4. Utilizatorul verifică proiectul și salvează eventualele completări.
5. Fleet reutilizează mediul pregătit, fiecare agent cu propriile servicii și
   date.

Prezența providerului scurtează pregătirea; nu elimină verificarea că proiectul
chiar funcționează din definiția sa.

### 16.4. Contribuție la un proiect care nu acceptă fișiere devcroft

Utilizatorul păstrează mediul în configurația sa externă. Modificările trimise
upstream conțin numai contribuția la aplicație. Definiția proprie poate fi
versionată separat și poate alimenta fleet.

### 16.5. Utilizare locală scurtă

Utilizatorul pornește sandboxul, inspectează proiectul și face o modificare
mică. Exportă rezultatul și șterge instanța. Nu este obligat să definească un
mediu fleet pentru a beneficia de izolare și de absența instalărilor pe host.

## 17. Ce trebuie să demonstreze spike-urile

### 17.1. Ambele forme de utilizare

Spike-urile hvi și libkrun 2.0 trebuie să compare:

1. **Mediu declarat:** guest mic, closure read-only, disk privat, keeper și servicii.
2. **Mediu local adaptabil:** bază versionată, instalări în guest, stare privată
   persistentă și reconstrucție ulterioară din definiție.

Primul servește eficiența mediilor bine definite și a fleet-ului. Al doilea
servește adopția proiectelor fără configurație completă.

### 17.2. Probe funcționale relevante

- Pornire fără provider sau toolchain instalat pe host.
- Importul unui proiect existent, inclusiv modificări locale conform regulii alese.
- Instalare de pachete și dependențe numai în instanța privată.
- Build și test real, cu biblioteci native.
- Servicii cu readiness și persistență după restart.
- Editor remote, SSH exec, PTY, SFTP și port forwarding.
- Export de rezultat fără importul metadatelor administrative guest.
- Salvarea unei definiții și reconstrucția într-o instanță separată curată.
- Refuz fleet înainte de definiție completă și verificare.
- Invalidarea rezultatului când se schimbă intrările relevante.

### 17.3. Probe pentru proiecte neîncrezute

- Primul cod al proiectului rulează după existența limitei VM.
- Setupul nu primește autoritate asupra hostului sau a altor instanțe.
- Manifestul nu își poate acorda singur directoare și credențiale host.
- Checkoutul original rămâne inaccesibil workload-ului în varianta privată.
- Baza comună rămâne read-only din perspectiva fiecărui guest.
- Egress-ul respectă politica fazei curente, inclusiv când clientul ocolește
  variabilele proxy.
- Confinementul VMM-ului este activ înainte de prima execuție guest.
- Crashurile și oprirea forțată nu lasă helperi sau canale active pentru alte
  instanțe.

Probele negative trebuie însoțite de controale pozitive: proiectul funcționează
cu resursele permise în timp ce accesul nepermis este refuzat.

### 17.4. Măsurători

Pe aceeași configurație guest și workload, pentru 1, 4 și 8 instanțe:

- timp până la VM și keeper responsive;
- timp până la proiect și servicii utilizabile;
- cost de prima pregătire și reutilizare;
- memorie la boot, în vârful buildului și după idle;
- recuperarea memoriei eliberate de guest;
- build I/O și workload cu multe fișiere mici;
- storage comun și privat;
- CPU idle;
- shutdown, restart și cleanup;
- cod specific, patchuri upstream și cost de întreținere.

Maturitatea familiei libkrun îi oferă un avantaj de experiență, dar API-ul 2.0
este încă nou. hvi are confinare proprie și o interfață mai restrânsă. Alegerea
finală trebuie să includă corectitudinea și izolarea, înaintea vitezei de boot.

## 18. Relația cu planurile existente

### 18.1. Schimbarea față de contractul actual

Regula actuală „nu există mod nereproductibil” și refuzul providerilor host/none
au fost scrise pentru un produs care cere un mediu declarat la intrare.

Noua direcție locală oferă o bază controlată și o instanță adaptabilă în VM.
Aceasta nu este passthrough către toolchain-ul host, dar este o schimbare reală
a condiției de reproductibilitate la intrare și trebuie descrisă ca atare.

Scopul actual de accident protection trebuie revizitat separat pentru
promisiunea proiectelor neîncrezute. Backendul VM și confinarea lui sunt parte
din dovada necesară.

### 18.2. Manifestless mode

`add-manifestless-mode` identifică deja costul de adopție și propune detectare
de provider și medii ad-hoc. Designul său actual:

- refuză o bază implicită când nu găsește configurație;
- discută salvarea a ceea ce a funcționat;
- lasă relația cu fleet deschisă.

Direcția nouă clarifică scopul: local permite baza identificată și explorarea;
fleet cere definiția completă, intrările fixate și verificarea.

### 18.3. Dev Container provider

`add-devcontainer-provider` propune în prezent materializarea unei imagini
read-only pe Linux, în process-tier, și refuză `build`, Features și Compose în
prima etapă.

Acest plan nu acoperă încă experiența discutată pentru proiectele locale
obișnuite. Ar trebui revizitat pentru:

- runtime VM și pe macOS;
- builder izolat;
- instalări și lifecycle care necesită stare writable privată;
- metadata efectivă a imaginilor;
- limitele explicite ale compatibilității Dev Containers;
- eligibilitate fleet distinctă de simpla utilizare locală a imaginii.

### 18.4. Mediul reproductibil nu este sinonim cu Nix

Formatul acceptat pentru garanția fleet trebuie decis pe proprietăți:
completitudine, intrări fixate, restabilire, setup și verificare.

Un closure Nix și o imagine fixată au garanții diferite. Aceste diferențe
trebuie afișate și explicate. Cerința strictă fleet nu trebuie diluată doar
pentru că un filesystem poate fi copiat identic; nici nu trebuie legată
artificial de prezența unui fișier Nix în repository dacă o definiție externă
poate îndeplini criteriile.

### 18.5. Relația cu exportul tehnic anterior

[Explorarea microVM pentru 1.0](devcroft-1.0-microvm-exploration-2026-11-05.md)
descrie hvi, libkrun, hull, Lima și guest-ul minimal. Prezentul document
completează acea explorare cu regula de adopție și parcursul local → fleet.

În exportul anterior, unele alegeri erau încă deschise, inclusiv rolul
proiectelor neîncrezute și topologia locală. Cerințele ulterioare ale
utilizatorului sunt consemnate aici; cele două documente trebuie citite în
această ordine temporală.

## 19. Întrebări încă deschise

### Selectarea mediului

- Ce bază generală și ce profiluri de limbaj sunt livrate inițial?
- Cum se aleg și se raportează bazele când proiectul nu declară nimic?
- Cum se rezolvă ambiguitatea între provider, Dev Container și configurația proprie?
- Ce interacțiuni sunt disponibile local și care este comportamentul non-TTY?

### Configurare și verificare

- Care este formatul definiției externe și identitatea asocierii cu repository-ul?
- Ce poate genera automat devcroft și ce necesită o decizie a utilizatorului?
- Care sunt comenzile pentru salvare, verificare și pregătirea pentru fleet?
- Ce criterii exacte diferențiază garanțiile closure, image și mediu local adaptat?
- Ce schimbări invalidează rezultatul verificării?

### Workspace și Git

- Ce modificări necomise, fișiere ignorate și fișiere neurmărite intră în import?
- Cum se importă istoricul fără configurație și hookuri host nedorite?
- Cum se tratează worktrees, submodule și monorepo-uri?
- Ce formate de export sunt suportate și cum se aplică în checkoutul host?
- Există un mod share read-write separat și cum este explicată garanția sa?

### Runtime și setup

- hvi sau libkrun 2.0 după cele două spike-uri?
- Builder/materializator separat sau în instanța locală?
- Cum se fac instalările administrative fără a lărgi autoritatea workload-ului obișnuit?
- Ce suprafață Dev Containers este necesară pentru primul release util?
- Cum sunt păstrate snapshoturile locale și datele serviciilor?

### Politică și distribuție

- Cum se compun cererile proiectului cu limitele utilizatorului?
- Cum se livrează credențialele după boot și ce rute pot fi brokered?
- Ce profiluri de rețea sunt potrivite pentru explorare, setup și runtime?
- Cum sunt distribuite și actualizate bazele, kernelul și helperul VMM?
- Ce praguri de performanță și densitate justifică lansarea?

## 20. Rezumat și referințe

### 20.1. Regula care leagă produsul

**Local pregătești și lucrezi într-o VM, fără dependențe de proiect pe host.
Fleet multiplică un mediu complet, fixat și verificat.**

Un proiect extern poate intra prin calea familiară `git clone`, `cd`,
`devcroft up`. Poate folosi o definiție existentă, un Dev Container sau o bază
devcroft. Utilizatorul poate rămâne local sau poate formaliza mediul pentru
fleet, inclusiv printr-o configurație proprie, fără adoptare upstream.

Acesta este workflow-ul propus pentru adopție dincolo de nișa Nix. Izolarea
este condiția de execuție; reproductibilitatea completă este condiția de
promovare în fleet.

### 20.2. Documente locale relevante

- [Explorarea microVM pentru devcroft 1.0](devcroft-1.0-microvm-exploration-2026-11-05.md).
- [Decizii actuale](decisions.md).
- [Threat model actual](threat-model.md).
- [Roadmap](roadmap.md).
- [Probleme și limite cunoscute](known-gaps.md).
- [Manifestless mode — proposal](../openspec/changes/add-manifestless-mode/proposal.md).
- [Manifestless mode — design](../openspec/changes/add-manifestless-mode/design.md).
- [Dev Container provider — proposal](../openspec/changes/add-devcontainer-provider/proposal.md).
- [Dev Container provider — design](../openspec/changes/add-devcontainer-provider/design.md).
- [Separarea provisioning/runtime](../openspec/changes/sandbox-provisioning/design.md).
- [Criteriile backendurilor](../openspec/changes/remove-gvisor-backend/design.md).
- [SSH și matricea editorilor](ssh-validation.md).

### 20.3. Surse externe pentru comportamentul Git

- [Git hooks — `post-checkout`](https://git-scm.com/docs/githooks#_post_checkout).
- [Git clone](https://git-scm.com/docs/git-clone).
- [Git init — template directory](https://git-scm.com/docs/git-init#_template_directory).
- [Git attributes și filtre](https://git-scm.com/docs/gitattributes).
- [Documentația hooks, versiunea 2.54.0 inspectată](https://github.com/git/git/blob/v2.54.0/Documentation/githooks.adoc).
- [Documentația attributes, versiunea 2.54.0 inspectată](https://github.com/git/git/blob/v2.54.0/Documentation/gitattributes.adoc).

Referințele și commiturile pentru hvi, libkrun, hull, Lima și nono se află în
[exportul tehnic anterior](devcroft-1.0-microvm-exploration-2026-11-05.md#20-referințe).
