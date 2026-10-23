# ContainerSafe — Splash Screen nativ (Slint + Rust)

Aceasta modificare se aplica peste proiectul actual `C:\Workspace\containersafe`.

## Fisiere

```text
native/
  assets/
    splash.png           # imaginea de splash generata
  ui/
    main.slint           # re-exporta SplashWindow si include corectiile Slint 1.18
    splash.slint         # imagine, mesaj incarcare, procent si progress bar
  src/
    main.rs              # startup pe thread separat, apoi deschide fereastra principala
README.md                # pagina de prezentare GitHub
```

## Instalare

1. Fa un commit sau un backup al fisierelor actuale `native/src/main.rs` si `native/ui/main.slint`.
2. Dezarhiveaza pachetul in `C:\Workspace\containersafe`, pastrand directoarele din arhiva.
3. Nu sterge `native/Cargo.toml` sau `native/build.rs` deja existente.
4. Executa:

```powershell
cd C:\Workspace\containersafe
cargo fmt --manifest-path .\native\Cargo.toml
cargo test --manifest-path .\native\Cargo.toml
cargo run --manifest-path .\native\Cargo.toml
```

## Cum functioneaza

- Slint afiseaza imediat `SplashWindow` care contine imaginea `native/assets/splash.png`.
- Un worker Rust citeste `backup-plan.json` si `backup-selection.json` din radacina proiectului. **NU executa SSH la pornire.**
- Worker-ul trimite mesaje de progres; un `slint::Timer` pe thread-ul GUI preia aceste mesaje si actualizeaza `loading-progress` si `loading-message`.
- La terminare, interfata principala este creata pe thread-ul Slint si este afisata, apoi splashul dispare.
- Exista un minim de ~950 ms pentru a preveni clipirea ferestrei. Nu este un progres bazat pe un timer artificial; valorile 22/50/76/88/100 reprezinta etape, nu estimari ale timpului ramas.

## Etape vizibile

| Progres | Mesaj | Actiune |
|---:|---|---|
| 5% | Pornesc ContainerSafe... | Splash afisat |
| 22% | Citesc inventarul Docker salvat... | Citire JSON local, **fara Docker remote** |
| 50% | Incarc selectiile pentru backup... | Citire/interpretare selectie locala |
| 76% | Reconciliez configuratia locala... | Asociere plan si selectie |
| 88% | Pregatesc fereastra principala... | Creeaza window si instaleaza callbackuri |
| 100% | Gata | Fereastra principala este afisata |

**Atentie:** In prototip, `load_state()` trateaza erorile de citire generand un plan gol, cu un mesaj in fereastra principala. Splashul poate ajunge la 100% si cand nu exista `backup-plan.json`; asta inseamna ca startup-ul s-a terminat, nu ca exista un inventar Docker valid.

## Personalizare

- Imagine: `native/assets/splash.png`.
- Dimensiunea ferestrei: `native/ui/splash.slint`, proprietatile `width` / `height`.
- Mesajele si etapele: `native/src/main.rs`, `load_state()` si `main()`.
- Culorile progresului: `native/ui/splash.slint`.

**Important Slint:** Nu pune simultan `width` cu `min-width` sau `height` cu `min-height` pe acelasi `Window`. Foloseste `preferred-width` / `preferred-height` cand ai nevoie si de limite minime. Pentru modulo foloseste `Math.mod(index, 2)`, nu `%`.

## Teste

1. Ruleaza aplicatia cu fisierele JSON existente — mesajele trebuie sa treaca prin etapele de incarcare.
2. Porneste fara `backup-plan.json` intr-un mediu de test separat — aplicatia ar trebui sa ajunga la interfata principala si sa afiseze lipsa planului, fara sa contacteze NAS-ul.
3. Verifica daca fereastra principala se poate inchide normal.
4. In mod development, testeaza modificarea imaginii; imaginea este incorporata la compilare (`@image-url`).

## Limite

- Pachetul modifica numai splashul si pornirea Slint. Nu activeaza backup Docker real.
- Codul a fost verificat static; **compilarea Slint 1.18.1 pe Windows nu a fost executata aici**. Daca apare o eroare de compilare, trimite exact eroarea.
- Mesajele pot fi localizate ulterior in engleza, separat de textul deja incorporat in imagine.
