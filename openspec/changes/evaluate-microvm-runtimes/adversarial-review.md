# Analiză adversarială — 2026-11-06

## Verdict

Direcția are sens ca schimbare de produs: un mediu Linux local adaptabil,
în VM, urmat de fleet dintr-o definiție reconstruibilă și verificată.
Nu rezultă încă alegerea hvi/libkrun, performanța acceptabilă sau o garanție
generală că devcroft este sigur pentru orice cod ostil.

Am analizat [explorarea microVM](../../../devcroft-1.0-microvm-exploration-2026-11-05.md)
și [workflow-ul ulterior](../../../local-to-fleet-workflow-2026-11-05.md), în această
ordine, raportat la codul și planurile locale. Exporturile rămân nemodificate.
Acesta este un review de arhitectură și specificație, fără boot sau benchmark.

## Ce rezistă și ce trebuie corectat

| Idee / presupunere | Contraexemplu adversarial | Decizie și destinație |
| --- | --- | --- |
| VM în jurul agentului rezolvă proiectele neîncrezute | Evaluarea providerului sau un build rulează înainte pe host | Păstrăm VM; mutăm și pregătirea. `vm-authority-boundary` |
| Lockfile înseamnă materializare de încredere | Lockfile-ul fixează exact un script malițios | Respingem justificarea de încredere; pinning și izolare sunt axe independente |
| Un materializator comun economisește tot costul | Un proiect modifică store-ul, daemonul sau cache-ul folosit de alt proiect | Inițial builder privat per job/proiect, publicare controlată de artefacte imutabile; cache-ul mutabil comun nu este acceptat |
| Kernel separat înseamnă întregul sistem protejat | Guest-ul trimite un path sau frame ostil controllerului, ori exploatează parserul de imagine | Hostul validează mesaje, lungimi și destinații; extragerea de straturi neîncrezute are loc izolat; digestul nu legitimează un kernel ales de proiect |
| Fără NIC înseamnă fără ieșire | TSI, forward SSH invers, socket map sau endpoint administrativ oferă rută alternativă | Inventariem toate canalele; fără TSI, passthrough generic, agent/X11/reverse forwarding implicit |
| Nono după `build()` este suficient | Prima instrucțiune guest poate rula în `build()` | Gate înainte de primul vCPU; mecanismul exact se măsoară în spike |
| Privilegii guest pentru instalări sunt incompatibile cu izolarea | Interzicerea oricărei instalări administrative face inutil modul adaptabil | Setup administrativ numai în VM privată; limita host trebuie să reziste și unui guest root |
| Snapshot/digest dovedește reproductibilitate | Imaginea include un pachet instalat manual sau loginul personal | Snapshot pentru reluare locală; fleet cere rețetă și reconstrucție curată, confirmat de utilizator |
| Un test verde dovedește completitudinea mediului | Testul este `true`, sare cazuri sau nu folosește serviciul lipsă | Dovezi pentru workload declarat; fără afirmație universală și fără „atestare de securitate” dintr-un guest neîncrezut |
| Verificare o dată, apoi orice branch | Branch-ul schimbă Dockerfile, lock, script, fixture sau surse compilate | Identități separate pentru mediu și sursă; cache reutilizabil, verificare exactă per context |
| Store partajat înseamnă cost marginal aproape zero | Fiecare kernel guest ține page cache; imaginile diferite dublează aceleași pachete | Nu transferăm cifrele process-tier; măsurăm RAM după build, storage fizic și toate helper-ele |
| „Suport Dev Containers” este o funcție mică | Metadata imaginii cere user, mounts, Features sau lifecycle neimplementat | Subset explicit imagine + lifecycle; câmpurile operative nesuportate sunt refuzate, inclusiv cele din metadata |
| Copiem `.git`, apoi exportăm un diff | Configul activează un helper; symlinkul sau alternate-ul indică în afara proiectului | Metadate guest reconstruite; date Git ostile interpretate în izolare; export staged, fără Git host pe date guest |
| Trimitem secretele după boot, deci nu ajung pe disk | Proiectul citește valoarea și o salvează singur | Transportul evită persistența făcută de devcroft, nu poate împiedica toate copiile făcute de workload; nu publicăm discul sesiunii ca bază fleet |
| Configurația repository-ului definește politica | Repository-ul își acordă `$HOME`, Docker socket sau chei | Manifestul cere; politica utilizatorului acordă. O definiție externă importată nu primește automat autoritate |
| RAM guest configurată limitează toate resursele | Loguri, disk sparse și helper-e consumă hostul | Limite și contabilizare host, refuz al capabilităților obligatorii neimplementate; nu confundăm RAM guest cu quota totală |

## Dovezi locale care schimbă designul

- [Provider::resolve](../../../src/provider/mod.rs) și apelul din
  [lifecycle/up.rs](../../../src/lifecycle/up.rs) sunt încă host-side.
  Același fișier are deja `run_provisioning` pentru hookuri: nu propunem să
  „reparăm din nou” ceva absent, ci să izolăm și evaluarea/materializarea
  pentru noul model VM. [sandbox-provisioning](../sandbox-provisioning/design.md)
  separă deja fazele, dar păstrează justificarea problematică „pinned = trusted”.
- [SessionBackend](../../../src/keeper/session.rs) întoarce `Child`, PGID și
  handle PTY locale. Keeper în guest păstrează aceste semantici; un adaptor
  superficial host-side nu rezolvă SFTP și forwarding.
- [Threat model](../../../docs/threat-model.md) exclude explicit codul ostil;
  [remove-gvisor-backend](../remove-gvisor-backend/design.md) fixează criterii
  pentru redeschidere. Propunerile noi nu rescriu dovezile runtime-ului actual.
- [Manifestless](../add-manifestless-mode/design.md) respinge baza implicită și
  promptingul; [Dev Container](../add-devcontainer-provider/design.md) vizează
  rootfs read-only în process-tier și include deja observații care invalidează
  presupuneri despre shell și metadata. Nu sunt implementări VM gata de reutilizat.
- [Fleet actual](../add-linux-agent-fleet/proposal.md) este Linux process-tier
  pentru cod de încredere. Nu justifică automat fleet VM/macOS.

## Triage și ordinea implementării

1. [evaluate-microvm-runtimes](proposal.md): două spike-uri directe, același
   harness, inclusiv mediu adaptabil. Poate concluziona „niciunul”.
2. [add-vm-project-runtime](../add-vm-project-runtime/proposal.md): bootstrap,
   pregătire izolată, authority, transport, keeper și lifecycle. Depinde de
   rezultat acceptat, nu doar de bifarea taskurilor spike.
3. [add-private-local-environments](../add-private-local-environments/proposal.md):
   selecție, workspace privat, import/export, definiție externă și subset Dev
   Containers. O bază necompletă este utilă local și rămâne neeligibilă fleet.
4. [gate-vm-fleet-on-verified-environments](../gate-vm-fleet-on-verified-environments/proposal.md):
   reconstrucție, identitate, invalidare și admitere atomică. Niciun agent nu
   începe sarcina doar fiindcă alt branch a trecut cândva o verificare.

Păstrăm un scope VM separat de native; nu sincronizăm propunerile în main specs
și nu modificăm codul runtime. `openspec/specs/` nu există încă în repository,
deci noile capabilități folosesc delte ADDED, cu limitele lor de aplicare explicite.

## Decizii primite și lucruri amânate

Utilizatorul a confirmat la 2026-11-06:

- Fleet cere definiție reconstruibilă, intrări fixate și verificare curată.
  Un snapshot sau un digest singur nu sunt suficiente.
- Prima etapă Dev Containers include imagine fixată și lifecycle în guest.
  Dockerfile, Features și Compose se proiectează separat.

Nu generăm un al cincilea proiect pentru hull/Lima: acestea nu sunt cele două
spike-uri cerute. Guest-ul comun numai pentru servicii macOS rămâne o problemă
distinctă, relevantă pentru aplicații Darwin. Nu adoptăm libkrun 1.x ca înlocuitor
nesolicitat al experimentului 2.0.

Brokerul de credențiale, snapshotul live, checkoutul host read-write, clonarea
direct din URL, orchestrarea remote și paritatea tuturor providerilor rămân
extensii separate. Ele nu sunt promise indirect de aceste specificații.

Catalogul inițial de baze, sintaxa CLI finală, pragurile de performanță și
versiunile OS de release trebuie alese înainte de release; nu blochează
specificarea proprietăților și experimentelor. Nu există date pentru alegerea
VMM-ului în acest review.

## Limitele verificării surselor

Commiturile upstream din export au fost păstrate ca puncte de pornire pentru
spike. Accesarea linkurilor GitHub fixate pentru hvi și libkrun prin browserul
de cercetare a eșuat (`Cache miss`) în această sesiune; nu le declar reverificate.
Referința Dev Containers publică nu a returnat conținut tehnic util. Prin urmare,
afirmațiile despre API-uri rămân ipoteze atribuite exporturilor, cu reverificare
obligatorie în experiment. Cerințele de produs sunt decizii propuse, nu afirmații
că acele API-uri le îndeplinesc deja.
