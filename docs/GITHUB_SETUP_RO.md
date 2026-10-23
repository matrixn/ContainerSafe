# Publicarea ContainerSafe pe GitHub — checklist

## 1. README si documentatie

- Pune `README.md`, `CONTRIBUTING.md`, `SECURITY.md` si `CHANGELOG.md` in radacina repository-ului.
- Pastreaza imaginea `native/assets/splash.png` in Git; README-ul o afiseaza automat.
- Adauga `.github/ISSUE_TEMPLATE/*.yml` pentru bug reports si feature requests.
- Verifica daca repo-ul este public inainte sa promovezi linkul.
- Inainte de `git add .`, verifica `.gitignore` si evita sa publici `backup-plan.json`, `backup-selection.json`, `.env`, chei SSH sau loguri private.

## 2. Descriere pentru GitHub About

**Description (de copiat):**

> Rust + Slint desktop toolkit for Docker discovery, backup readiness audits, safe planning, and future verified recovery.

**Website (optional):** lasa gol daca nu ai un site oficial; nu inventa un domeniu.

**Topics (copiaza in About → Topics, fara #):**

```text
rust
slint
docker
docker-compose
backup
restic
self-hosted
homelab
nas
disaster-recovery
desktop-app
postgresql
backup-management
```

GitHub Topics se configureaza din pagina repo-ului, in zona About, prin iconita de editare. Hashtagurile puse in README nu seteaza Topics automat.

## 3. Social Preview

GitHub → Repository Settings → General → Social preview (daca aceasta optiune este disponibila). Incarca un banner cu branding ContainerSafe. Imaginea de splash din `native/assets/splash.png` poate servi ca preview initial, dar textul este in romana; pentru promovare internationala merita o versiune cu text in engleza.

## 4. Setari utile

- Activeaza **Issues** pentru feedback.
- Activeaza **Discussions** doar daca vei modera comunitatea; nu il promovam in README pana atunci.
- Configureaza **private vulnerability reporting** din sectiunea Security, daca optiunea este disponibila.
- Adauga un **LICENSE** doar dupa ce decizi drepturile reale de utilizare. Publicarea pe GitHub nu face automat proiectul open-source.
- Pentru distributia Slint desktop comerciala, verifica separat licenta Slint aplicabila.

## 5. Exemplu de mesaj de lansare in comunitate

> I'm building ContainerSafe, a Rust + Slint desktop tool for self-hosters who want to understand what their Docker apps need before they can be backed up and restored. The current preview discovers containers and Compose stacks, inventories persistent data, flags database risks, and creates reviewable backup plans. I also have a local Restic backup/restore lab working. Production backups are not ready yet — I'm looking for feedback on the workflows you actually struggle with. What would you want a tool like this to get right?

## 6. Commit sigur

```powershell
cd C:\Workspace\containersafe

git status --short
git diff --stat
# Verifica sa nu fie date private/secrete in staging.
git add README.md CONTRIBUTING.md SECURITY.md CHANGELOG.md .github docs native/assets/splash.png native/ui/splash.slint native/ui/main.slint native/src/main.rs
git diff --cached --stat
git commit -m "feat: Slint startup splash and project documentation"
git push origin main
```

Daca ai deja modificari importante in `native/src/main.rs`, compara-le inainte de a suprascrie cu versiunea din arhiva.
