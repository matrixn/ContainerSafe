# ContainerSafe 0.29 — backupuri manuale de fișiere

Această etapă conectează interfața Slint la agentul Linux și motorul Restic existent.
Poți salva sursele aprobate ale unui plan, vedea snapshoturile, verifica toate datele
și restaura un snapshot exact într-un director nou. Nu este încă recuperare completă
a unui stack: exporturile PostgreSQL/MySQL/Redis, SQLite consistent, fișierele Compose,
dependențele externe, programarea și retenția nu sunt implementate în acest flux.
Un plan cu `Logical dump` selectat este refuzat, nu degradat la copierea volumului.

## 1. Destinația implicită

În **File → Settings → Default backup destination** alege:

| Tip | Exemplu | Cerințe |
| --- | --- | --- |
| Folder on Docker server | `/volume1/ContainerSafe/backups` | Director pe server, accesibil utilizatorului SSH; preferabil alt disc/share |
| This computer (SFTP) | `sftp:backup-pc:/C:/ContainerSafe` | Server SSH/SFTP activ pe PC, accesibil din NAS, cheie dedicată și host key verificat |
| Remote SFTP / S3 | `sftp:backup-remote:/srv/backups` sau `s3:https://endpoint/bucket/prefix` | SFTP configurat sau bucket S3 HTTPS și credențiale AWS pe serverul Docker |

Setarea se salvează automat, fără parole. Când creezi prima configurare pentru un
plan se adaugă un sufix cu UUID-ul serverului și numele planului, ca depozitele să
rămână separate. Modificarea destinației implicite nu mută backupurile existente.
În **Backup plans → Backup…** poți modifica tipul și destinația acelui plan și
apăsa **Save plan**. Selecțiile sunt salvate odată cu planul.

**This computer folosește SFTP, nu un transfer direct într-un dosar Windows.**
Un `C:\Backups` simplu nu este acceptat: trebuie configurată adresa SFTP accesibilă
din serverul Docker. Pentru porturi și chei folosește un alias în `~/.ssh/config`
al utilizatorului agentului:

```sshconfig
Host backup-pc
    HostName ADRESA_PC
    User utilizator_backup
    Port 22
    IdentityFile /home/backup/.ssh/backup_pc_ed25519
    IdentitiesOnly yes
```

Verifică amprenta PC-ului independent și înregistreaz-o în `known_hosts` înainte
de primul backup. ContainerSafe impune `StrictHostKeyChecking=yes` și `BatchMode=yes`
pentru transportul SFTP Restic. Nu introduce parola în URL și nu dezactiva verificarea.
S3 acceptă numai HTTPS. Folosește `~/.aws/credentials`, protejat cu `chmod 600`,
sau mediul securizat al procesului agentului; aplicația nu salvează cheile S3 în JSON.
Vezi și [documentația oficială Restic pentru destinații](https://restic.readthedocs.io/en/stable/030_preparing_a_new_repo.html).

## 2. Prima configurare a agentului

1. Conectează serverul și selectează **Include** numai pentru sursele dorite în Containers.
2. Deschide **Backup plans → Backup…** pentru planul respectiv.
3. Verifică lista de surse, destinația și căile Linux ale agentului, Restic, parolei
   și directorului de restaurare. Valorile `/home/backup/...` sunt exemple: adaptează-le
   utilizatorului SSH real, de exemplu `/var/services/homes/matrixn/...` pe DSM.
4. **Export setup** creează într-un subdirector nou `agent.json` și `SETUP.md`.
   Nu suprascrie o configurare existentă și nu include parole.
5. Urmează `SETUP.md`: instalează binarul Linux al agentului și Restic 0.19.1,
   copiază configurarea pe server, creează fișierul parolei și directorul de restaurare.
   Configurarea/parola trebuie protejate; agentul trebuie să aibă acces real la surse.
6. Păstrează separat o copie securizată a parolei Restic. Fără ea nu poți recupera datele.

Agentul se compilează pe Linux cu:

```sh
cargo build --locked --no-default-features --bin containersafe-agent
```

Pentru un NAS Linux x64 cu biblioteci de sistem mai vechi, folosește varianta statică
musl, nu presupune că binarul GNU compilat pe Ubuntu este compatibil cu DSM:

```sh
rustup target add x86_64-unknown-linux-musl
RUSTFLAGS="-C linker=$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld" \
  cargo build --locked --target x86_64-unknown-linux-musl --no-default-features --bin containersafe-agent
```

Comanda este pentru un host de compilare Linux x64 cu toolchain rustup. Binarul rezultat
este în `target/x86_64-unknown-linux-musl/debug/containersafe-agent` dacă nu ai configurat
alt `CARGO_TARGET_DIR`. În checkoutul local, copia verificată este în
`target/linux-agent/containersafe-agent`; fișierele din `target/` nu se publică prin Git.
Compatibilitatea efectivă pe DSM trebuie verificată separat după instalare.

Nu există instalare automată a agentului pe NAS, modificare automată a permisiunilor
sau escaladare prin sudo. `sudo -n docker` pentru discovery nu acordă implicit
agentului dreptul de a citi directoarele gazdei. Eroarea de acces trebuie rezolvată
prin permisiuni limitate acordate utilizatorului agentului, nu prin dezactivarea verificărilor.

## 3. Primul backup și testul de restaurare

Începe cu un container de test și câteva fișiere fără bază de date activă.

1. Bifează acceptarea limitării backupului la nivel de fișiere.
2. **Initialize repository**, o singură dată pentru o destinație nouă.
3. **Back up now**: se recitește inventarul Docker, se verifică sursele/politicile,
   se confirmă jobul agentului și apoi se creează snapshotul criptat.
4. **List snapshots**, apoi **Check all data**.
5. Selectează snapshotul dorit și **Test restore**. Fișierele se scriu pe serverul
   Docker într-un subdirector nou `test-<UUID>` al directorului configurat.
   Restaurarea verifică conținutul prin Restic și nu înlocuiește datele originale.
6. Modifică fișierul de test și repetă backupul. Verifică existența ambelor snapshoturi
   și recuperarea fiecărei versiuni în directoare separate.

**Activity** păstrează ultimele 50 de rezultate pentru serverul selectat.
Un timeout SSH nu dovedește că procesul de pe server s-a oprit: verifică snapshoturile
și procesele serverului înainte să reîncerci. Nu există anulare garantată a unui job remote.
Nu există procent real de progres Restic: popupul arată operațiunea în curs.

Dacă modifici selecțiile sau destinația, exportă și instalează configurarea agentului
actualizată. Agentul compară din nou sursele și depozitul în aceeași cerere care pornește
backupul. Un job diferit este refuzat cu `PlanChanged`; nu se face fallback la alte surse.
Un backup parțial sau eșuat nu este afișat ca reușit.

## Verificare efectuată în dezvoltare

- Build/check/test/clippy Windows și teste SSH loopback cu identitate verificată.
- Agent Linux + Restic real: două snapshoturi, restaurări separate și comparații byte cu byte.
- Teste pentru selecții schimbate, volume DB blocate, corelarea răspunsului cu cererea,
  permisiuni, limite de ieșire și destinații cu verificare strictă SSH/HTTPS.
- Nu s-au rulat backupuri pe NAS sau pe destinații externe reale S3/SFTP/PC.

Următoarea etapă pentru backup complet de aplicație rămâne adaptorul PostgreSQL,
cu restaurare într-un stack Docker izolat și verificare la nivel de aplicație.
