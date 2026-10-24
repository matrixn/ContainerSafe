# ContainerSafe 0.15 — interfață reorganizată

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

1. Verifică versiunea **0.15**, logo-ul și tema deschisă. Conectează serverul existent prin fluxul normal; portul și verificarea SSH nu se schimbă.
2. Extinde un proiect Compose și selectează un container din el. Verifică numele, descrierea domeniului comun și sursele din dreapta. Restrânge separat Compose și Standalone.
3. Caută numele unui container; proiectul părinte trebuie să rămână vizibil. Golește căutarea pentru a reveni la arbore.
4. Folosește **Storage** și **Databases**. Schimbă o alegere din dropdown: un PostgreSQL permite `Logical dump`, în timp ce directorul său brut nu oferă `Include`. Redis fără adapter executabil permite numai Pending/Exclude.
5. Deschide **Backup plans**: numerele trebuie să reflecte imediat modificările. **Review** revine la proiectul corect. Revizuiește alegerile, apoi salvează și validează prin acțiunile existente.
6. Verifică hover-ul, cursorul și starea butoanelor activate/dezactivate. Detaliile pot fi selectate și copiate cu Ctrl+C; etichetele scurtate își păstrează valoarea completă la focalizare. Arborele copiază numele rândului focalizat cu Ctrl+C.

Schimbarea unei selecții marchează planul nesalvat. Validarea nu autorizează executarea unui backup de producție. Ferestrele serverelor păstrează validarea câmpurilor, tooltipurile și confirmarea explicită a fingerprintului SSH.

## Verificare vizuală fără NAS

Build-ul de dezvoltare oferă un fixture explicit, cu 25 de containere și căi fictive:

```powershell
cargo run --locked --manifest-path native/Cargo.toml -- --preview-report native/tests/fixtures/ui-report.json
```

Titlul afișează `Local preview · no server connection`. Modul nu citește profilurile reale, nu face autentificare, nu se conectează la servere și folosește configurație temporară. Selecțiile făcute în preview sunt numai în memorie. Flag-ul este disponibil doar în build-urile debug; folosirea zilnică rămâne comanda normală de mai sus.

```powershell
cargo test --locked --manifest-path native/Cargo.toml
cargo clippy --locked --manifest-path native/Cargo.toml -- -D warnings
```

Testele verifică ierarhia, expandarea/căutarea, păstrarea indicilor originali după sortare, numărarea selecțiilor după identitate și opțiunile permise de politica surselor. Inspecția vizuală se face separat în aplicația compilată.
