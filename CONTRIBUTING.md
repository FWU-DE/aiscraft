# Mitmachen bei AIS.ChatCraft

AIS.ChatCraft ist ein Chat-Programm für Lehrkräfte: ein nativer Client für
[AIS.chat](https://app.ais-chat.schule) und andere KI-Dienste, geschrieben in Rust mit egui. Jede Hilfe
ist willkommen: Fehler melden, Ideen beschreiben, Texte verbessern, Code beitragen.

## Fehler melden und Ideen einbringen

- **Fehler:** Lege ein [Issue](https://github.com/FWU-DE/aiscraft/issues/new) an. Schreib dazu, was du
  getan hast, was passiert ist und was du erwartet hast, dazu Betriebssystem, Version (Einstellungen →
  Über) und das Backend (AIS.chat, Claude, …). Ein Bildschirmfoto hilft.
- **Ideen:** Beschreib im Issue, was du im Unterricht oder in der Vorbereitung erreichen willst, nicht
  nur die Lösung. Funktionen, die AIS.chat im Browser hat und die hier fehlen, bekommen das Label
  `ais-chat-parity`.
- **Keine Zugangsdaten, Schülerdaten oder Chatverläufe mit personenbezogenen Daten** in Issues,
  Bildschirmfotos oder Logs. Das Repository ist öffentlich.
- Sicherheitslücken bitte nicht öffentlich melden, sondern über
  [GitHub Security Advisories](https://github.com/FWU-DE/aiscraft/security/advisories/new).

## Code beitragen

### Einrichten

```sh
git clone https://github.com/FWU-DE/aiscraft && cd aiscraft
cargo run -p aiscraft                     # die App
cargo test --workspace                    # alle Tests
cargo clippy --workspace --all-targets    # muss ohne Warnungen durchlaufen
```

Unter Linux brauchst du die Entwicklungspakete für X11/Wayland, GTK 3, ALSA und D-Bus (Debian/Ubuntu:
`libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libxcursor-dev libgl1-mesa-dev
libgtk-3-dev libasound2-dev libdbus-1-dev pkg-config`). macOS und Windows brauchen nur Rust.

### Wie das Projekt aufgebaut ist

[AGENTS.md](AGENTS.md) beschreibt den Aufbau (Crates und ihre Schichten), die Regeln und wie wir die
Anbindung an AIS.chat aktuell halten. Bitte vor dem ersten Beitrag lesen. Die wichtigsten Regeln:

1. **Nie abstürzen.** Kein `unwrap`, `expect`, `panic!` oder Indexzugriff, der abstürzen kann, kein
   `unsafe`. Eingaben von Servern, Dateien, Modellen und Menschen sind feindlich.
2. **Alles ist ein Befehl.** Neues Verhalten ist ein Befehl in `crates/engine/src/commands.rs`; Oberfläche,
   Kommandozeile, Steuerkanal und MCP-Server rufen ihn über seine Kennung auf.
3. **Jeder Befehl hat einen Funktionstest.** Ein Wächtertest schlägt fehl, wenn einer fehlt.
4. **Tests öffnen keine Fenster** und nutzen weder Browser noch Mikrofon oder Lautsprecher.
5. **Zugangsdaten** liegen im Schlüsselbund des Systems, nie in der Datenbank oder in Logs.
6. **Oberflächentexte sind Deutsch**, kurz und in der Du-Form. Neue Symbole gegen die eingebauten
   Schriften prüfen.
7. Wo eine Bibliothek aus [storytold/craft-libs](https://github.com/storytold/craft-libs) die Aufgabe
   erledigt, nimm sie (siehe [docs/craft-libs.md](docs/craft-libs.md)).

Oberflächenänderungen prüfst du mit dem Schnappschuss-Beispiel, ohne die App zu öffnen:

```sh
cargo run -p aiscraft-ui --example snapshot -- --demo --out shot.png \
  --script '[["chat.send", {"text": "Hallo"}], ["ui.show", {"what": "settings"}]]'
```

### Pull Requests

- Ein Thema pro Pull Request, mit Bezug auf das Issue (`Closes #123`).
- Die Commit-Nachricht sagt, was sich für die Nutzerinnen und Nutzer ändert.
- Die CI baut und testet auf Linux, macOS und Windows; sie muss grün sein.
- Das Handbuch in der App (`crates/ui/manual/de.md`) wird mit angepasst, wenn sich Bedienung ändert.
- Kein Code aus nicht-öffentlichen Repositories und keine Geheimnisse im Code.

### KI-Agenten

Beiträge mit KI-Agenten sind willkommen. Die Agenten lesen [AGENTS.md](AGENTS.md); Commits nennen den
Agenten als Co-Autor. Die Verantwortung für einen Pull Request liegt bei dem Menschen, der ihn öffnet.

## Lizenz

Mit deinem Beitrag stimmst du zu, dass er unter denselben Lizenzen wie das Projekt steht:
[MIT](LICENSE-MIT) oder [Apache 2.0](LICENSE-APACHE), nach Wahl der Nutzenden.

## Danke

Alle, die mitgewirkt haben, stehen unter
[Contributors](https://github.com/FWU-DE/aiscraft/graphs/contributors).
