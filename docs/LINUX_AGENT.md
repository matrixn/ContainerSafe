# Actualizare 0.29

Integrarea desktop pentru backupuri manuale de fișiere este disponibilă. Vezi [configurarea și limitele actuale](MANUAL_BACKUPS.md). Secțiunile de mai jos descriu fundația agentului 0.13.

# ContainerSafe 0.13 — agent Linux

Agentul rulează **o cerere JSON per proces**, pe stdin/stdout. Poate fi invocat local sau ca o comandă SSH după autentificarea și verificarea identității serverului. Nu ascultă pe un port, nu instalează servicii și nu execută comenzi shell din cerere. Operațiile sunt sincrone; după răspuns procesul se închide.

## Compilare și testare izolată

Pe Linux, cu toolchain-ul indicat în `rust-toolchain.toml` și un linker C:

```bash
cargo build --locked --no-default-features --bin containersafe-agent
target/debug/containersafe-agent --version
target/debug/containersafe-agent --capabilities
cargo test --locked --no-default-features
python3 scripts/agent_lab.py \
  --agent target/debug/containersafe-agent \
  --restic /cale/absoluta/catre/restic
```

Laboratorul necesită Python 3 și [Restic 0.19.1](https://github.com/restic/restic/releases/tag/v0.19.1), versiunea validată. Creează automat un director temporar cu fișiere fictive, configurație și parolă aleatoare, apoi:

1. Inițializează un repository criptat și creează două snapshoturi diferite.
2. Verifică toate datele din repository.
3. Șterge sursa fictivă, restaurează ambele versiuni separat și compară byte cu byte, demonstrând recuperarea după pierderea sursei.
4. Verifică refuzul suprascrierii, traversării `../`, joburilor necunoscute, câmpurilor injectate, parolelor cu permisiuni prea largi, surselor ce conțin secrete și operațiilor neaprobate.
5. Afișează `{"status": "PASS", "version": "<versiunea curentă>", "snapshots": 2, "verified_restores": 2, ...}` și șterge fișierele temporare.

Nu se conectează la NAS sau Docker. Pentru release se poate folosi `cargo build --locked --release --no-default-features --bin containersafe-agent`. Executabilul nu este încă inclus în pachetele desktop. Pe Windows poate fi compilat pentru verificarea codului și a protocolului; citirea configurației/secretelor pentru execuție răspunde explicit `unsupported_platform`, până există validări ACL echivalente.

În checkout-ul Windows curent au fost pregătite și verificate copii locale de test în `target/linux-agent` (ignorate de Git). Din PowerShell poți repeta laboratorul direct în Ubuntu WSL:

```powershell
wsl -d Ubuntu-22.04 -- bash -lc 'cd /mnt/c/Workspace/containersafe && python3 scripts/agent_lab.py --agent target/linux-agent/containersafe-agent --restic target/linux-agent/restic'
```

Pe un checkout nou trebuie mai întâi compilat agentul și instalat Restic, folosind comenzile Linux de mai sus; copiile din `target/` nu sunt livrate prin Git.

`--no-default-features` exclude componenta `server-management`: agentul nu are nevoie să fie client SSH sau să acceseze keyring-ul desktop și nu cere bibliotecile D-Bus. Din 0.14, build-urile obișnuite CLI/desktop folosesc `russh` cu `ring`, fără OpenSSL/libssh2. Nu dezactiva funcționalitățile implicite când compilezi desktopul.

## Configurație locală

Rulează agentul sub un utilizator dedicat, cu acces numai la directoarele necesare. Nu este un sandbox: permisiunile sistemului de operare și configurația administrată local rămân limita de acces. Nu îl rula cu sudo/root pentru a ocoli permisiunile fișierelor.

Exemplu pentru date fictive, de adaptat explicit înainte de orice utilizare:

```json
{
  "restic_executable": "/usr/local/bin/restic",
  "timeout_seconds": 300,
  "jobs": [{
    "id": "8a0f141f-35f5-40e4-b421-232f6982df35",
    "repository": {
      "path": "/srv/containersafe-lab/repository",
      "secret_reference": "bcdfdb60-7879-43f8-a083-6a8394ef215e"
    },
    "secret_file": "/home/backup/.config/containersafe/lab.secret",
    "sources": ["/srv/containersafe-lab/source"],
    "restore_root": "/srv/containersafe-lab/restores",
    "allowed_operations": ["init", "backup", "snapshots", "check", "restore"]
  }]
}
```

Toate căile trebuie să fie absolute. Directoarele sursă și rădăcina restaurărilor trebuie pregătite de operator. `restic_executable` este un executabil de încredere, ales pe server, fără căutare implicită în PATH. Timeout acceptat: 1–86400 secunde per comandă. Configurația maximă: 1 MiB; fișierul parolei: 16 KiB, UTF-8, o linie, cu newline final opțional. Secretele nu se includ în cereri sau configurație.

Fișierele trebuie să fie obișnuite, fără symlink la ultima componentă, deținute de utilizatorul agentului sau de root. Configurația nu trebuie să fie modificabilă de grup/alții; parola trebuie să fie accesibilă numai proprietarului. Recomandat:

```bash
chmod 700 /home/backup/.config/containersafe
chmod 600 /home/backup/.config/containersafe/agent.json
chmod 600 /home/backup/.config/containersafe/lab.secret
```

Păstrează executabilul, configurația, secretele și directoarele părinte sub controlul unui utilizator de încredere. Parola se citește numai după autorizarea operației și se transmite motorului Restic prin mediul procesului copil. Nici stdout, nici erorile protocolului nu conțin parola sau stderr brut al Restic. Metadatele snapshoturilor și căile fișierelor apar în rezultatele autorizate.

`allowed_operations` lipsește sau este gol = **nici o operație permisă**. Acordă numai operațiile necesare; un client autorizat să facă backup poate repeta operația și consuma spațiu. Configurația nu oferă încă rate limiting sau quota. Sursele nu trebuie să conțină secrete; verificarea căilor cunoscute nu detectează toate copiile/hardlinkurile create de operator.

## Protocol și invocare SSH

Cerere `request.json` pentru listare:

```json
{
  "protocol_version": 1,
  "request_id": "a83de0c7-7319-4b24-af97-23845d3f4d58",
  "job_id": "8a0f141f-35f5-40e4-b421-232f6982df35",
  "action": {"operation": "snapshots"}
}
```

```bash
/usr/local/bin/containersafe-agent \
  --config /home/backup/.config/containersafe/agent.json < request.json

# Exemplu doar: configurația SSH trebuie să verifice identitatea serverului.
ssh -T -o StrictHostKeyChecking=yes backup@HOST \
  '/usr/local/bin/containersafe-agent --config /home/backup/.config/containersafe/agent.json' \
  < request.json
```

Jobul trebuie să existe pe server. `request_id` identifică răspunsul; nu implementează deduplicare și nu reprezintă autentificare. Accesul este autorizat de SSH/utilizatorul sistemului și apoi de configurația jobului. O cheie SSH dedicată poate fi restricționată de administrator la comanda fixă a agentului, fără shell, forwarding sau PTY; instalarea acestei restricții nu este automată în 0.13. Nu folosi `StrictHostKeyChecking=no`.

Acțiuni disponibile:

| `action` | Efect |
| --- | --- |
| `{"operation":"init"}` | Inițializează numai repository nou/gol |
| `{"operation":"backup"}` | Folosește exclusiv sursele configurate local |
| `{"operation":"snapshots"}` | Listează ID-urile și metadatele snapshoturilor |
| `{"operation":"check","read_data":true}` | Verifică integral datele; fără `read_data` verifică structura |
| `{"operation":"restore","snapshot_id":"ID_HEX_COMPLET_64","directory":"drill-1"}` | Restaurează întregul snapshot într-un director nou sub `restore_root` |

Numele directorului de restaurare acceptă 1–64 caractere ASCII: litere, cifre, `-`, `_`. Nu acceptă căi absolute, separatori sau `..`. Structura absolută a surselor din snapshot se păstrează sub destinație. Restaurarea cere ID complet, niciodată `latest`, nu suprascrie și rulează verificarea conținutului. Un eșec poate lăsa un director parțial: pentru reîncercare alege un nume nou. Țintele suprapuse cu repository-ul ori sursele configurate sunt refuzate.

O cerere are maximum 64 KiB și trebuie urmată de EOF. Protocolul respinge câmpuri și operații necunoscute. Răspunsul este un singur obiect JSON, cu `protocol_version`, `application_version`, `request_id`, `status` și `result` sau `error`. Coduri de proces: 0 succes, 1 eroare de protocol/configurare/politică/Restic, 2 invocare greșită ori imposibilitatea scrierii răspunsului. La JSON invalid sau versiune necunoscută, `request_id` poate fi null. Erorile Restic expun numai categoria și codul numeric de ieșire.

## Limitele acestei etape

Agentul nu descoperă încă Docker, nu face dump PostgreSQL, nu programează backupuri și nu este conectat la butoanele interfeței desktop. Nu instalează nimic pe serverele existente. Următoarea etapă este un stack PostgreSQL fictiv, backup consistent și restaurare verificată la nivelul aplicației; abia apoi conectăm orchestrarea la interfață.
