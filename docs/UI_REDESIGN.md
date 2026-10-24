# ContainerSafe 0.15 — interfață reorganizată

Actualizare **0.16**: Backup plans afișează numai proiectele/containerele pentru care există cel puțin o opțiune `Include` sau `Logical dump`. `No backup` și `Exclude` nu sunt metode de backup. Inventarul complet rămâne în Containers. Pentru Compose, eligibilitatea este la nivelul proiectului, conform domeniului comun al surselor din raport.

`No backup` înlocuiește eticheta Pending și înseamnă că metoda nu a fost aleasă încă; decizia necesită în continuare revizuire pentru validare. `Exclude` este alegerea explicită de a omite elementul. Valoarea salvată rămâne `pending`, deci selecțiile existente nu necesită migrare.

Fiecare selector are un **i încercuit**: ține mouse-ul deasupra pentru explicația opțiunilor efectiv disponibile sau apasă pentru explicații detaliate. Bazele de date fără metodă au suplimentar o iconiță informativă de avertizare lângă nume. Aceasta explică adapterul lipsă și motivul interdicției de a copia fișierele bazei active. PostgreSQL oferă în continuare doar planificarea unui dump; executarea adapterului nu este implementată încă.

Interfața folosește alb și griuri deschise, text albastru închis și accente teal pentru acțiuni. Logo-ul real apare în stânga numelui aplicației. Butoanele, listele de selecție, checkboxurile și arborele au cursor de apăsare și feedback vizual; controalele dezactivate rămân distincte.

## Containers și Backup plans

În prototip, Applications și Backup plans afișau aceeași componentă. Din 0.15 au roluri separate:

- **Containers:** inventar și revizuire. Arborele este `Compose → proiect → containere`, respectiv `Standalone → containere`. Săgețile extind/restrâng, căutarea găsește proiecte sau containere, iar selectarea arată sursele și bazele de date în dreapta.
- **Backup plans:** sumar centralizat al selecțiilor, cu numărul operațiilor selectate, elementelor excluse și deciziilor în așteptare pentru fiecare proiect/container. **Review** deschide proiectul corect în Containers. Nu este o listă de backupuri executate sau programate.

Raportul existent descrie stocarea la nivel de proiect Compose. Selectarea unui container membru păstrează acest domeniu comun, indicat prin `project-wide storage`; nu inventează o asociere exclusivă între fiecare mount și container. Identificatorii interni `compose/...` și `standalone/...`, UUID-urile serverelor și formatul fișierelor salvate sunt păstrate.

## Cum testezi cu serverul tău

```powershell
cargo run --locked --manifest-path native/Cargo.toml
```

1. Verifică versiunea **0.22**, logo-ul și tema deschisă. Conectează serverul existent prin fluxul normal; portul și verificarea SSH nu se schimbă.
2. Extinde un proiect Compose și selectează un container din el. Verifică numele, descrierea domeniului comun și sursele din dreapta. Restrânge separat Compose și Standalone.
3. Caută numele unui container; proiectul părinte trebuie să rămână vizibil. Golește căutarea pentru a reveni la arbore.
4. Folosește **Storage** și **Databases**. Verifică `No backup` și explicațiile iconițelor de lângă selectoare. Un PostgreSQL permite `Logical dump`, în timp ce directorul său brut nu oferă `Include`. Redis fără adapter selectabil permite numai No backup/Exclude și are o iconiță care explică limitarea.
5. Deschide **Backup plans**: apar numai proiectele/containerele cu Include sau Logical dump activat; dezactivarea ultimei metode elimină imediat planul. **Review** revine la proiectul corect. Revizuiește alegerile, apoi salvează și validează prin acțiunile existente.
6. Verifică hover-ul, cursorul și starea butoanelor activate/dezactivate. Textele obișnuite nu se selectează. Erorile rămân selectabile și copiabile. În lista serverelor, Copiază adresa copiază hostul împreună cu portul.

Schimbarea unei selecții marchează planul nesalvat. Validarea nu autorizează executarea unui backup de producție. Ferestrele serverelor păstrează validarea câmpurilor, tooltipurile și confirmarea explicită a fingerprintului SSH.

## Verificare vizuală fără NAS

Build-ul de dezvoltare oferă un fixture explicit, cu 25 de containere și căi fictive:

```powershell
cargo run --locked --manifest-path native/Cargo.toml -- --preview-report native/tests/fixtures/ui-report.json
```

Titlul afișează `Local preview · no server connection`. Modul nu citește profilurile reale, nu face autentificare, nu se conectează la servere și folosește configurație temporară. Selecțiile făcute în preview sunt numai în memorie. Flag-ul este disponibil doar în build-urile debug; folosirea zilnică rămâne comanda normală de mai sus.

Fixture-ul de 25 de containere conține 12 proiecte/standalone, însă numai **immich** și **mail-api** au metode selectabile: Backup plans trebuie să arate 2 planuri. În Containers → immich → Databases, iconița de avertizare Redis explică de ce nu oferă o metodă, iar iconița de lângă selectorul PostgreSQL explică No backup, Logical dump și Exclude.

```powershell
cargo test --locked --manifest-path native/Cargo.toml
cargo clippy --locked --manifest-path native/Cargo.toml -- -D warnings
```

Testele verifică ierarhia, expandarea/căutarea, păstrarea indicilor originali după sortare, numărarea selecțiilor după identitate și opțiunile permise de politica surselor. Inspecția vizuală se face separat în aplicația compilată.

În Setări, verifică cele trei carduri de grup și salvarea automată sub setarea modificată. La pornire, splashscreen-ul are 550 × 353, fără titlu/butoane, și se centrează pe monitor; păstrează întârzierea de 2,5 secunde după încărcare.
