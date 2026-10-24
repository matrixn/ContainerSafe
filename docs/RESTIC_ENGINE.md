# ContainerSafe 0.12 — motorul Restic

În discuția inițială, etapa **v0.8** cerea extragerea comenzilor din `restic_lab.rs` într-un motor Rust reutilizabil, independent de interfață: configurare repository, secrete separate, rezultate și erori tipizate, backup, listare, verificare și restaurare. Aplicația a ajuns între timp la 0.11 prin dezvoltarea interfeței și SSH; această implementare continuă numerotarea cu **0.12**, fără revenire la o versiune mai mică.

## Ce funcționează

- `src/restic.rs` este parte din biblioteca comună folosită de CLI și desktop. Agentul viitor îl poate reutiliza fără dependență de Slint sau Docker.
- `RepositoryConfig` conține o cale locală absolută și un UUID de referință pentru secret. Apelantul furnizează `RepositorySecret` corespunzător; parola nu este serializată în configurație și nu are implementare Debug.
- `ResticEngine` primește executabilul configurat și expune `init`, `backup`, `snapshots`, `check` și `restore`. Rezultatele conțin ID-uri complete, metadate, statistici și verificări. JSON-ul invalid sau un backup incomplet reprezintă erori, nu succes.
- Parola este transmisă numai în mediul procesului copil. Comenzile sunt pornite direct, fără shell; parola nu apare în argumente sau erori. Variabilele moștenite pentru comenzi/fișiere alternative de parole sunt eliminate. Memoria parolei este ștearsă la eliberarea secretului; această măsură nu este o protecție împotriva administratorului sistemului.
- Timeout implicit: 300 secunde per comandă, configurabil prin API. Captura stdout este temporară, limitată la 16 MiB și eliminată la închidere; stderr nu este păstrat sau afișat. La timeout/depășire se oprește și se așteaptă procesul copil. Erorile au operație, categorie și cod de ieșire, fără mesajul brut al procesului.
- Backupul refuză surse relative, inexistente sau suprapuse cu repository-ul. Restaurarea cere un ID exact, destinație nouă și separată de repository; folosește `--overwrite never --verify`. După eșec, directorul parțial rămâne pentru inspecție și trebuie aleasă altă destinație la reîncercare.
- `check --read-data` verifică datele repository-ului; `restore --verify` verifică fișierele recuperate. Laboratorul adaugă comparații byte cu byte ale fișierelor fictive. Aceste verificări nu certifică consistența unei baze de date sau pornirea unei aplicații restaurate.

## Cum testezi pe Windows

Din PowerShell, în `C:\Workspace\containersafe`, cu Restic instalat:

```powershell
& C:\Users\radut\bin\restic.exe version
cargo run --locked -- lab-backup --restic C:\Users\radut\bin\restic.exe
```

Înlocuiește calea dacă ai Restic în alt director. Alternativ setează `$env:CONTAINERSAFE_RESTIC` sau folosește `restic` din PATH. Contractele JSON au fost validate cu **Restic 0.19.1**; alte versiuni nu au fost testate.

Rezultatul așteptat:

1. Apare `ContainerSafe 0.12` și un director temporar `containersafe-lab-*`.
2. Se inițializează un repository criptat și se salvează prima versiune a fișierului fictiv.
3. Se verifică integral repository-ul și se restaurează prima versiune.
4. Se modifică fișierul și se creează al doilea snapshot.
5. Se verifică din nou repository-ul și se restaurează separat versiunea nouă și cea veche.
6. Raportul JSON conține două snapshoturi, trei restaurări și `byte_comparisons_passed: true`, urmat de `[PASS] Two snapshots...`.

Poți deschide directorul afișat și compara `restored-v1`, `restored-v2` și `restored-old-again`. Parola aleatoare nu se păstrează; repository-ul acestui laborator servește numai rulării curente. Rularea următoare creează alt director. Nu se utilizează profilurile SSH, NAS-ul sau volumele Docker.

Teste automate, inclusiv un repository real temporar și respingerea parolei greșite:

```powershell
cargo test --locked
$env:CONTAINERSAFE_RESTIC = 'C:\Users\radut\bin\restic.exe'
cargo test --locked --test restic_integration -- --ignored
cargo test --locked --manifest-path native/Cargo.toml
cargo run --locked --manifest-path native/Cargo.toml
```

Ultima comandă pornește interfața existentă; verifică versiunea **0.12** în titlu/bară/About. Motorul nou este testabil din CLI; nu există încă un buton de backup de producție conectat la el.

## Pașii următori ai planului inițial

1. Agent Linux cu interfață clară, configurare și autorizare explicită a operațiilor, reutilizând motorul.
2. Stack Docker PostgreSQL cu date fictive și adapter pentru backup consistent; restaurare într-un stack separat și verificări la nivelul aplicației.
3. Integrarea fluxului în interfața Slint existentă: destinații, secrete persistente, progres, selecția snapshotului și raportul de restaurare. Scheduling și retention separat, cu reguli explicite.

Acestea sunt etape viitoare, fiecare cu increment de 0.01 și instrucțiuni proprii de testare. Versiunea 0.12 nu adaugă operații automate pe server, retenție/prune, repair/unlock, backend S3/SFTP sau o migrare la Tauri. Căile absolute locale reprezintă intenționat singura destinație acceptată acum. Linux/macOS nu au fost executate în această sesiune Windows.
