# AIS.ChatCraft: Designentwürfe 2 und 3

Zwei eigenständige Entwürfe als Mockups, noch ohne Code in `crates/`. Entwurf 1 (Material 3, behutsam)
steht in `docs/design-review-2026-11.md`. Er war zu zaghaft („sieht noch spröde aus“, „fast wie vorher“).
Beide neuen Entwürfe zeigen dieselben sechs Bildschirme mit demselben Inhalt. So lassen sie sich Bild
für Bild vergleichen.

| | Entwurf 2 „Mutig“ | Entwurf 3 „Glas“ |
|---|---|---|
| Startseite | `screens/d2-01-start.png` | `screens/d3-01-start.png` |
| Chat hell | `screens/d2-02-chat-hell.png` | `screens/d3-02-chat-hell.png` |
| Chat dunkel | `screens/d2-03-chat-dunkel.png` | `screens/d3-03-chat-dunkel.png` |
| Chat mit Dokument | `screens/d2-04-dokument.png` | `screens/d3-04-dokument.png` |
| Leeres Whiteboard | `screens/d2-05-whiteboard-leer.png` | `screens/d3-05-whiteboard-leer.png` |
| Einstellungen | `screens/d2-06-einstellungen.png` | `screens/d3-06-einstellungen.png` |
| Glas ohne Unschärfe (so zeichnet egui allein) | | `screens/d3-07-ohne-unschaerfe.png` |

Alle Bildschirme haben 1280 × 800 px. Neben jeder Datei liegt eine `@2x`-Fassung (2560 × 1600).
Der dunkle Chat zeigt dasselbe Gespräch, nur weiter nach unten gescrollt, mit dem Codeblock.

## Dateien

- `aiscraft-entwuerfe.figcraft`: die FigCraft-Datei mit allen 13 Rahmen. Oben steht Entwurf 2, darunter Entwurf 3.
- `build/`: Die Bildschirme werden per Skript gebaut, nicht von Hand gezeichnet.
  - `fc.py`: Client für den Steuerport von FigCraft
  - `kit.py`: Layout-Bausteine, Linien-Icons auf 24er-Raster, AIS.chat-Logo, WCAG-Kontrastrechnung
  - `themes.py`: alle Farben, Radien und Schatten beider Entwürfe, hell und dunkel
  - `app.py`: Komponenten und Bildschirme (für beide Entwürfe dieselbe Anatomie)
  - `run.py`: baut die Bildschirme, rendert die PNGs und speichert die Datei
  - `check.py`: prüft alle Textfarben auf WCAG AA, bei Glas über dem ungünstigsten Hintergrund

Neu bauen:

```sh
~/Applications/figcraft-0.18.1-linux-x86_64.AppImage --control 7979 \
  --automation-read-root design --automation-write-root design &
cd design/build && python3 run.py          # alles
python3 run.py d3-04                       # nur einen Bildschirm
python3 check.py                           # Kontraste
```

Schrift: Barlow in Regular, Medium, SemiBold, Bold und ExtraBold (OFL, die Schrift von AIS.chat). Die
App enthält heute Regular, Medium und SemiBold. Bold muss für beide Entwürfe dazukommen (Datei aus
`google/fonts`, ofl/barlow). Code steht in JetBrains Mono. Formeln stehen hier in Inter, weil Barlow
weder `₂` noch `→` hat. In der App setzt `craft-math` die Formeln.

---

## Entwurf 2 „Mutig“

**Idee.** Der Unterschied soll auf den ersten Blick sichtbar sein. Eine nachtblaue Seitenleiste mit
heller Schrift trennt Navigation und Arbeit klar. Rechts davon liegt eine helle, ruhige Fläche. Die
Hierarchie entsteht durch die Schrift: Titel in Barlow Bold, Tabellenköpfe in Bold auf Nachtblau,
nummerierte Schritte mit Akzentkreisen. Nutzernachrichten stehen als getönte Blasen rechts, Antworten
frei und mit mehr Luft. Die Eingabe ist eine erhobene Karte mit deutlichem Senden-Knopf. Die
Startseite begrüßt mit Namen und bietet Vorschlags-, Favoriten- und „Zuletzt“-Karten. Koralle ist ein
sparsamer Zweitakzent: aktiver Eintrag in der Seitenleiste, Favoritenstern, „Neu“-Hinweis.

### Palette

| Rolle | Hell | Dunkel |
|---|---|---|
| Seitenleiste (Verlauf oben → unten) | `#0C2E4E` → `#081F37` | `#071B30` → `#041222` |
| Seitenleiste Text / Nebentext | `#C9D8E6` / `#8FAFCC` | `#C2D2E2` / `#87A3BF` |
| Seitenleiste aktiv (Fläche / Text) | `#17507D` / `#FFFFFF` | `#123F63` / `#FFFFFF` |
| Hauptfläche | `#F3F6F9` | `#0D1723` |
| Karte / Panel | `#FFFFFF`, Rand `#DFE6EE` | `#142234`, Rand `#22364D` |
| Vertieft (Formel, Segmente) | `#EAF0F5` | `#1A2A3E` |
| Text / Text 2 / Nebentext | `#0D1B2A` / `#33445A` / `#566578` | `#E8EEF5` / `#BDCAD8` / `#8E9FB2` |
| Akzent (AIS-Blau) / Text darauf | `#107FAD` / `#FFFFFF` | `#38A8DB` / `#04131F` |
| Akzent hell / Text darauf | `#DFF0F8` / `#0A5C80` | `#113A53` / `#9AD7F2` |
| Link | `#0B6E98` | `#7CC8EC` |
| Zweitakzent Koralle (Fläche / Text) | `#F0663F`, `#FDE7DF` / `#A93A18` | `#FF8A66`, `#3D2219` / `#FFB59C` |
| Nutzerblase | `#DCEEF8` | `#16405C` |
| Tabellenkopf | `#0C2E4E`, Text `#FFFFFF` | `#1B3A57`, Text `#FFFFFF` |
| Codeblock | `#0C2238`, Kopf `#14304C` | `#081422`, Kopf `#0F2033` |

### Schriftskala (Barlow)

| Stufe | Größe / Schnitt | Einsatz |
|---|---|---|
| Display | 44 Bold, −0,8 Laufweite | Begrüßung auf der Startseite |
| Titel L | 26–28 Bold | Dokumenttitel, Einstellungen |
| Titel M | 22–23 Bold | Überschrift in Antworten, Bereich „Einstellungen“ |
| Titel S | 18–20 Bold | Chat-Titel, Panel-Titel, Abschnitte der Startseite |
| Untertitel | 22 Medium | „Woran arbeiten wir heute?“ |
| Fließtext | 15,5 Regular, Zeilenhöhe 24–25 | Nachrichten |
| UI | 14,5 Regular / SemiBold | Seitenleiste, Knöpfe |
| Klein | 12,5–13,5 Regular | Meta, Hinweise |
| Label | 11,5 SemiBold, Versalien, +1,1 Laufweite | Abschnitte der Seitenleiste, „FORMEL“ |

### Komponenten

- Seitenleiste 268 px: Logo-Kachel (weiß, AIS.chat-Logo), Pillenknopf „Neuer Chat“, Suchfeld mit
  „Strg K“. Darunter Abschnitte in Versalien mit Plus, Einträge mit 17-px-Linienicon und ein
  korallenfarbener Balken am aktiven Eintrag. Unten Konto mit Avatar und Status.
- Chat-Kopf: Titel 20 Bold, Projekt-Chip, Meta, Kontext-Pille mit Balken, Icon-Knöpfe.
- Nutzerblase: Radius 20, Ecke oben rechts 6, höchstens 470 px breit.
- Antwort: Absenderzeile mit Logo, Fließtext, nummerierte Liste mit Akzentkreisen, Formelblock mit
  Akzentbalken, Tabelle mit Radius 14 und nachtblauem Kopf, Codeblock mit Kopfzeile und Kopieren.
  Darunter Aktionen als umrandete Pillen: Kopieren, Vorlesen, Als Dokument, Als Whiteboard, neu erzeugen.
- Eingabe: Karte mit Radius 22, weichem Schatten in zwei Lagen, Werkzeug-Icons, Modellwahl mit
  Logo-Punkt, „Quellen“ und „Erstellen“ als Chips, Mikrofon, runder Senden-Knopf 40 px in AIS-Blau.
- Startseite: Datum als Akzent-Label, Begrüßung, große Eingabe, vier Vorschlagskarten mit
  Icon-Kachel, Favoriten aus AIS.chat als Karten mit farbigem Rundbild, „Zuletzt bearbeitet“.
- Artefakt-Panel 468 px: Symbolkachel, Titel, „Teilen“ als Primärknopf, Schließen. Darunter
  Segment „Bearbeiten | Vorschau“, „Im Chat“, Versionen-Chip „V2“, Export. Das Dokument liegt als
  Blatt auf vertiefter Fläche. Unten das Feld „Mit KI überarbeiten“ mit Primärknopf.
- Whiteboard: schwebende Werkzeugleiste als Pille, Punktraster, Leerzustand mit zwei Startknöpfen,
  schwebende Farbleiste mit Zoom.
- Einstellungen: Dialog mit Radius 24, linke Leiste mit Icons, „Darstellung“ mit Schema-Karten
  (Hell, Dunkel, System), Seitenleiste nachtblau oder hell, Akzentfarbe, Schriftgröße, Dichte,
  Bewegung reduzieren.

### Umsetzung in egui

**Aufwand: mittel, etwa 5–7 Arbeitstage.** Fast alles geht mit Bordmitteln:

- Farben, Radien, Schatten: `Frame` mit `corner_radius` (pro Ecke) und `epaint::Shadow` (eine Lage
  pro Frame; die zweite Lage durch einen zweiten, äußeren Frame). Die Tokens kommen in `theme.rs`.
- Verlauf in der Seitenleiste: egui füllt Formen nicht mit Verläufen. Ein `Mesh` mit Farben pro Ecke
  reicht für ein Rechteck. Einfacher wäre eine flache Farbe `#0A2743`, die man kaum unterscheidet.
- Barlow Bold als weitere Familie einbinden (egui kennt keine Schriftgewichte, jede Stärke ist eine
  `FontFamily::Name`). Laufweite und Zeilenhöhe gehen über `TextFormat`.
- Linienicons: Die App zeichnet heute Glyphen. Vorschlag: Lucide als Icon-Schrift (ISC-Lizenz) als
  eigene Familie einbinden. Die Icons bleiben dann scharf und lassen sich färben. Jedes Icon muss
  gegen die Glyphen-Prüfung im kittest-Harness (Regel 8 in AGENTS.md).
- Blasen, Karten, Pillen: eigene kleine Widgets in `widgets.rs`.
- Tabellenkopf und Formelblock: hängt davon ab, wie `markup.rs`/egui_commonmark Tabellen zeichnet.
  Ein farbiger, fetter Kopf braucht dort einen eigenen Renderer für Tabellen.

**Risiken.** Die Kartenraster der Startseite (4 Spalten, umbrechend bei schmalen Fenstern) brauchen
in egui eigene Breitenrechnung, es gibt kein Flex-Layout. Alle kittest-Snapshots ändern sich auf
einmal. Die dunkle Seitenleiste muss im hellen Schema überall gesondert gefärbt werden: Menüs,
Tooltips und Kontextmenüs der Leiste erben sonst helle Farben.

---

## Entwurf 3 „Glas“

**Idee.** Leicht und räumlich. Hinter allem liegt ein weicher Verlauf mit fünf unscharfen Lichtern:
AIS-Blau, Lavendel, Mint, Pfirsich. Darauf schweben drei Glasflächen mit 12 px Abstand: Seitenleiste,
Chat, Panel. Sie sind lichtdurchlässig, haben einen hellen Innenrand oben (1 px weiß) und weiche
Schatten. Die Seitenleiste ist am durchsichtigsten, die Arbeitsflächen sind dichter, damit Text ruhig
steht. Die Eingabe schwebt als dichteste Glaskarte über dem Verlauf. Primärflächen tragen einen
Verlauf in AIS-Blau. Lavendel ist der Zweitakzent für Favoriten und Hinweise.

### Palette

| Rolle | Hell | Dunkel |
|---|---|---|
| Hintergrund (Verlauf) | `#E4EEF7` → `#EEE9F7` | `#07111F` → `#11132E` |
| Lichter (weich, Unschärfe 140–170) | `#6FC3EC` `#B9A8F4` `#8EE0D2` `#FFCBAE` `#9FD6F2` | `#0F6A97` `#4A3AA0` `#126E66` `#6B3352` `#0E4F7A` |
| Seitenleiste | `#FFFFFF` 48 % | `#0B1626` 65 % |
| Chatfläche | `#FFFFFF` 65 % | `#0D1A2B` 72 % |
| Panel / Dialog | `#FFFFFF` 77 % / 82 % | `#0D1A2B` 80 % |
| Karte auf Glas | `#FFFFFF` 72 %, Rand `#FFFFFF` 95 % | `#FFFFFF` 6 %, Rand `#FFFFFF` 10 % |
| Eingabe | `#FFFFFF` 90 % | `#13243A` 86 % |
| Innenkante (oben, 1 px) | `#FFFFFF` 70–100 % | `#FFFFFF` 12–16 % |
| Text / Text 2 / Nebentext | `#0E1E2E` / `#2F4256` / `#4A5D71` | `#EEF4FA` / `#C7D3E0` / `#9AABBD` |
| Akzent-Verlauf / Text darauf | `#107FAD` → `#0A5E86` / `#FFFFFF` | `#5CC3EE` → `#2A93C6` / `#04131F` |
| Akzent hell / Text darauf | `#D3ECF8` 80 % / `#0A5577` | `#38A8DB` 18 % / `#A6DDF5` |
| Zweitakzent Lavendel (Fläche / Text) | `#7B5CE5`, `#E9E2FC` / `#5434C4` | `#A48BFF` / `#CDBEFF` |
| Nutzerblase | `#CFE8F6` 90 % | `#2A7FB0` 30 % |
| Codeblock | `#0F2236` 90 % | `#050C16` 70 % |

### Schriftskala

Wie bei „Mutig“. Glas braucht keine eigene Skala, die Hierarchie trägt dieselbe Barlow-Staffel. Auf
Glas steht nichts kleiner als 12,5 px. Nebentext auf Glas ist nie heller als `#4A5D71` (hell) bzw.
`#9AABBD` (dunkel).

### Komponenten

Dieselbe Anatomie wie in Entwurf 2: gleiche Seitenleiste, Nachrichten, Eingabe, Panels und
Einstellungen. Anders sind:

- Alle Hauptflächen sind abgerundete Glasflächen (Radius 24) mit 12 px Abstand zum Fensterrand und
  untereinander, statt bündig anzuliegen.
- Der aktive Eintrag in der Seitenleiste ist eine weiße, leicht erhobene Glaspille statt eines
  farbigen Balkens.
- Tabellenkopf hellblau (`#E2EEF7`) mit dunkelblauem Bold-Text statt Nachtblau.
- Eingabe, Werkzeugleiste und Farbleiste des Whiteboards schweben mit mehrlagigem Schatten.
- Unter dem Einstellungsdialog wird das Fenster weichgezeichnet. In der Darstellung gibt es die Wahl
  „Transparenz: Aus | Verlauf | Fenster“.

### Was egui kann und was nicht

egui hat **keine Hintergrund-Unschärfe für Inhalte im Fenster**. Der Renderer (glow, wgpu) malt
Dreiecke, ein `backdrop-filter` wie im Browser fehlt. Echte Unschärfe hinter dem Fenster gibt es nur
über ein durchsichtiges Fenster plus Compositor. Der Entwurf ist deshalb so gebaut, dass er ohne
Unschärfe genauso aussieht:

1. **Der Hintergrund ist schon weich.** Verlauf und Lichter sind ein statisches Bild. egui malt es als
   Textur (einmal erzeugt und mitgeliefert oder beim Start gerechnet) oder als Mesh mit Farben pro
   Ecke. Lichtdurchlässige Flächen darüber wirken mattiert, ganz ohne Blur.
2. **Unter Glas liegt nie scharfer Inhalt.** Kein Text scrollt sichtbar unter eine dünne Glasfläche.
   Die Eingabe ist zu 90 % deckend, darüber blendet ein Verlauf den Verlauf aus.
3. **Beweis:** `screens/d3-07-ohne-unschaerfe.png` ist der helle Chat mit allen
   `backgroundBlur`-Effekten entfernt. Er ist von `d3-02` kaum zu unterscheiden.

| Effekt im Entwurf | egui allein | braucht Compositor |
|---|---|---|
| Weicher Verlauf mit Lichtern | ja (Textur oder Mesh) | nein |
| Lichtdurchlässige Flächen (Alpha) | ja (`Color32` mit Alpha) | nein |
| Innenkante oben (1 px hell) | ja (eigener Pfad im Painter) | nein |
| Weiche Schatten, mehrlagig | ja (`epaint::Shadow`, eine Lage pro Frame) | nein |
| Akzent-Verläufe auf Knöpfen | ja (Mesh), sonst flaches AIS-Blau | nein |
| Unschärfe hinter Panels über dem eigenen Hintergrund | nein, nicht nötig (Hintergrund ist weich) | nein |
| Weichzeichnen der App hinter dem Einstellungsdialog | nein: Ersatz ist eine stärkere Abdunkelung (`#C9DCEB` 40 % → 70 %) | nein |
| **Desktop scheint durch das Fenster („Fenster“-Modus)** | nein | **ja**: durchsichtiges Fenster (`ViewportBuilder::with_transparent`) plus Hyprland-Blur (`decoration:blur` für durchsichtige Fenster), macOS Vibrancy (`NSVisualEffectView`) oder Windows 11 Mica/Acrylic, z. B. über das Crate `window-vibrancy` |

Auf GNOME und vielen X11-Setups gibt es keine Unschärfe hinter Fenstern. Dort muss „Fenster“
ausgegraut sein oder auf „Verlauf“ zurückfallen.

### Umsetzung in egui

**Aufwand: Entwurf 2 plus 3–5 Tage im Fenster, plus 2–4 Tage für den „Fenster“-Modus auf drei
Systemen.** Alle Komponenten aus Entwurf 2 werden wiederverwendet, nur mit Glas-Tokens.

- Hintergrund-Textur: PNG je Schema (hell, dunkel) mitliefern und beim Skalieren strecken. Weil sie
  weich ist, sieht man das Strecken nicht. Alternativ zur Laufzeit als Mesh aus radialen Verläufen.
- Schwebendes Layout: Außenabstand 12 px, Radius 24, die drei Flächen als `Frame`s mit Alpha-Füllung.
- „Fenster“-Modus: eframe mit durchsichtigem Viewport, dann pro System: Hyprland (Regel in der
  Hyprland-Konfiguration, kein Code), macOS/Windows über `window-vibrancy` mit dem rohen
  Fenster-Handle aus winit.

**Risiken.**

- Kontrast hängt vom Hintergrund ab. Über dem eigenen Verlauf ist alles geprüft (siehe unten). Im
  „Fenster“-Modus liegt ein unbekanntes Hintergrundbild darunter. Die Flächen müssen dann mindestens
  80 % deckend sein, sonst ist AA nicht sicher.
- Durchsichtige Fenster verhalten sich unter Wayland, X11, macOS und Windows unterschiedlich (Schatten,
  Ränder, Verhalten beim Vergrößern). Das muss auf jedem System getestet werden.
- Leistung: Ein ganzes Fenster mit Alpha-Mischung ist für die GPU wenig Arbeit. Auf schwachen
  Schul-Laptops mit Software-Rendering (llvmpipe) kann es merklich sein. Dann ist „Transparenz: Aus“
  nötig, und alle Flächen werden deckend.
- Lesbarkeit für Menschen mit Sehschwäche: Glas kostet Ruhe im Bild. „Transparenz: Aus“ muss es
  geben, ebenso Kopplung an „Kontrast erhöhen“ des Systems, wo es das gibt.

---

## Kontrast (WCAG AA)

`build/check.py` prüft jede Textfarbe gegen ihren Untergrund. Bei Glas rechnet es alle Glaslagen
über jeden Hintergrundton zusammen und meldet den schlechtesten Wert. Als Hintergrundtöne nimmt es
beide Verlaufsenden und jedes Licht in voller Stärke, also ungünstiger als der weichgezeichnete
Hintergrund je ist. Alle Werte erreichen 4,5 : 1 oder mehr. Die knappsten:

| Paar | Mutig hell | Mutig dunkel | Glas hell | Glas dunkel |
|---|---|---|---|---|
| Text auf Akzent (Knöpfe) | 4,51 | 6,97 | 4,51 (Verlauf: 4,51) | 6,97 (Verlauf: 5,44) |
| Nebentext auf Hauptfläche | 5,49 | 6,66 | 5,35 | 5,81 |
| Nebentext in der Seitenleiste | 6,05 | 6,64 | 4,71 | 5,60 |
| Link | 5,24 | 9,73 | 4,74 | 7,93 |

Das AIS-Blau `#107FAD` mit weißer Schrift liegt mit 4,51 : 1 genau an der Grenze. Hellere Blautöne
dürfen deshalb nicht hinter weiße Schrift. Deshalb beginnt der Akzent-Verlauf bei „Glas hell“ bei
`#107FAD` und wird nur dunkler.

## Offene Entscheidungen

1. **Welcher Entwurf?** Oder eine Mischung: zum Beispiel die Struktur und Typografie von „Mutig“ mit
   „Glas“ als wählbarem Schema in den Einstellungen.
2. **Seitenleiste nachtblau auch im dunklen Schema?** Heute ist sie dort nur etwas dunkler als der
   Inhalt. Die Alternative wäre ein hellerer Ton als Abhebung.
3. **Zweitakzent:** Koralle (Mutig) oder Lavendel (Glas). Und wie sparsam: nur Favoriten und Hinweise
   oder auch der aktive Eintrag?
4. **Wählbare Akzentfarbe** (gezeigt in den Einstellungen): ja, oder fest AIS-Blau, damit die Marke
   eindeutig bleibt?
5. **Icons:** Lucide als Icon-Schrift einbinden (scharf, einheitlich) oder bei den heutigen Glyphen
   bleiben?
6. **„Fenster“-Modus bei Glas:** überhaupt anbieten (2–4 Tage, drei Systeme testen) oder nur Glas
   über dem eigenen Verlauf?
7. **Startseite:** Begrüßung mit Vornamen (aus dem AIS.chat-Konto) und „Zuletzt bearbeitet“ als
   eigener Block, oder schlichter wie heute?
