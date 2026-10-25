# ContainerSafe 0.34 — backupuri manuale de fișiere

Din 0.34 poți salva direct pe PC prin **This PC via SSH tunnel**, fără server SSH
pe PC, sau poți descărca un export criptat prin **SFTP pull** ori **SSH binary streaming**.
Vezi [pașii de configurare din README](../README.md#backups-and-transfers-to-this-pc-031034)
și [arhitectura și limitele transferurilor](PC_TRANSFERS.md). Opțiunea existentă
**This computer (SFTP)** își păstrează sensul: NAS-ul trimite date către serverul SFTP de pe PC.

## Mod implicit: Restic direct prin SSH, fără agent

În **Backup plans → Backup… → Backup execution**, alege **Restic directly over SSH**.
Acesta este modul implicit inclusiv pentru planurile salvate înainte de 0.30. Câmpurile
agentului dispar; căile și destinațiile deja salvate sunt păstrate pentru verificare.
Pentru modul **ContainerSafe Linux agent (advanced)** apar din nou numai câmpurile
specifice agentului. Destinația afișează eticheta și explicațiile pentru dosar pe NAS,
PC prin SFTP sau storage extern SFTP/S3, în funcție de selecție.

Pentru DSM 7.4:

1. Instalează **Restic**, nu agentul ContainerSafe, într-o cale Linux executabilă
   pe NAS. Poți folosi un director al utilizatorului, de exemplu
   `/var/services/homes/matrixn/bin/restic`, fără instalare în directoarele sistemului.
   Completează aceeași cale la **Restic executable**. Verifică arhitectura NAS-ului;
   binarul Linux x64 din `target/linux-agent/restic` este copia locală folosită în teste.
2. Creează un fișier cu parola repository-ului pe NAS, deținut de utilizatorul SSH,
   cu permisiuni `600` sau `400`, fără symlink. Directorul părinte ar trebui să fie `700`.
   Nu introduce parola în URL, JSON sau argumentele comenzilor. Păstrează o copie securizată.
3. Alege un director existent și accesibil pentru restaurările de test, separat de
   surse și repository. Pentru o destinație locală, directorul părinte al repository-ului
   trebuie să existe și să fie accesibil la scriere.
4. Verifică sursele selectate și acceptă limitarea la backupuri de fișiere.
   Rulează **Save plan → Initialize repository** (o singură dată), apoi
   **Back up now → List snapshots → Check all data → Test restore**.
5. **Export setup** produce instrucțiuni pentru modul selectat. În modul direct exportă
   `SETUP.md` și `backup-plan.json`, fără `agent.json` și fără parole.

Utilizatorul din grupul DSM administrators poate intra prin SSH, dar acest lucru și
dreptul `sudo -n docker` nu acordă automat acces la fișierele containerelor sau repository.
Execuția directă folosește contul SSH, fără sudo automat, modificări ACL, chmod/chown
recursive sau schimbări de sudoers. Configurează acces limitat la directoarele necesare.
Serviciul User Home trebuie activat pentru a folosi directoarele homes; altfel alege
o partajare dedicată, accesibilă contului. Vezi documentația Synology pentru
[SSH](https://kb.synology.com/en-global/DSM/help/DSM/AdminCenter/system_terminal?version=7)
și [User Home](https://kb.synology.com/en-uk/DSM/help/DSM/AdminCenter/file_user_advanced).

Scriptul de preflight este trimis prin canalul SSH verificat, rulează cu `sh` POSIX
și nu instalează servicii sau scripturi persistente. Verifică fișierul parolei,
sursele și suprapunerile înainte de Restic; statusul 3 (backup incomplet) este tratat
ca eșec. Restaurarea cere un snapshot exact și un director nou și folosește `--verify`.
Un timeout SSH nu garantează oprirea procesului de pe NAS: verifică înainte de reîncercare.

Pe NAS-ul de test s-au verificat doar în citire DSM **7.4**, x64 și utilitarele `sh`,
`stat`, `readlink`, `id`. Restic nu a fost găsit în PATH sau în `/usr/local/bin/restic`,
`/opt/bin/restic`, `/usr/bin/restic`. Nu s-au instalat fișiere și nu s-au creat backupuri
pe NAS. Fluxul complet cu Restic real, pierderea sursei și două restaurări verificate
a fost testat în laborator Linux cu `scripts/direct_restic_lab.py`.

## Mod avansat: agentul Linux (opțional)

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
