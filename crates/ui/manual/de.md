# AIS.ChatCraft

AIS.ChatCraft ist ein Programm für den Rechner, mit dem du mit AIS.chat und anderen KI-Diensten chattest. Deine Chats, Dokumente und Whiteboards liegen auf diesem Rechner und sind durchsuchbar.

## Erste Schritte

1. **Anmelden.** Unten links auf das Konto klicken oder im Startbildschirm „Bei AIS.chat anmelden“ wählen.
2. **Modell wählen.** Im Eingabefeld auf „Backend · Modell“ klicken.
3. **Schreiben.** Enter sendet, Umschalt+Enter macht eine neue Zeile.

Weitere Dienste (Claude, OpenAI, Ollama, LM Studio …) fügst du unter **Einstellungen › Backends** hinzu.

## Anmelden

**AIS.chat.** „Im Browser anmelden“ öffnet ein eigenes Browserfenster mit der AIS.chat-Anmeldung (VIDIS, Schulkonto oder Microsoft 365). Nach der Anmeldung übernimmt AIS.ChatCraft die Sitzung und schließt das Fenster. Dafür braucht es Chrome, Chromium, Edge, Brave oder Vivaldi. Die Sitzung gilt einige Stunden; ist sie abgelaufen, meldet AIS.ChatCraft dich beim nächsten Start ohne Fenster neu an, solange die VIDIS-Sitzung noch gilt.

Ohne passenden Browser: den Sitzungs-Cookie `__Secure-ais-chat-app.authjs.session-token` aus den Entwicklerwerkzeugen (F12) des Browsers kopieren und im Anmeldedialog einfügen.

**Claude (Abo).** Nutzt dein claude.ai-Abo über die lokale Claude-CLI. Vorher im Terminal einmal `claude auth login` ausführen.

**Claude (API), OpenAI und andere.** API-Schlüssel eintragen. Lokale Server wie Ollama oder LM Studio brauchen keinen Schlüssel.

Zugangsdaten liegen im Schlüsselbund des Systems, nie in der Datenbank.

## Chatten

- **Modell wechseln:** jederzeit über „Backend · Modell“; die nächste Antwort kommt vom neuen Modell.
- **Antwort anhalten:** Esc oder das Quadrat im Eingabefeld.
- **Neu erzeugen:** „Neu erzeugen“ unter der letzten Antwort oder Strg+R.
- **Bearbeiten:** über einer eigenen Nachricht auf das Stift-Symbol; alles danach wird neu erzeugt.
- **Anhänge:** Büroklammer oder Dateien ins Fenster ziehen. Bilder und PDFs gehen an Modelle, die sie lesen können; Textdateien werden mitgeschickt.
- **Diktieren:** Mikrofon im Eingabefeld. Der Text landet im Eingabefeld, du schickst ihn selbst ab. Braucht ein OpenAI-kompatibles Backend mit Transkription.
- **Vorlesen:** „Vorlesen“ unter einer Antwort. Nutzt AIS.chat, wenn du dort angemeldet bist.
- **Kontextfenster:** oben rechts im Chat, wie viel vom Kontext des Modells belegt ist. „≈“ heißt geschätzt.
- **AIS.chat-Kontingent:** unten links, wie viele Token-Punkte noch übrig sind.

**Assistenten, Figuren und Lernszenarien.** „Assistenten & Figuren entdecken“ in der Seitenleiste (oder Strg+K) öffnet eine Seite mit allem, was AIS.chat dir anbietet: eigene, an der Schule geteilte und offizielle Einträge, mit Bild und Beschreibung.

- **Suchen:** Strg+F, dann Name oder Beschreibung. Groß- und Kleinschreibung und Umlaute spielen keine Rolle („uebung“ findet „Übung“). Bleibt genau ein Treffer, öffnet Enter ihn.
- **Eingrenzen:** Alle, Assistenten, Figuren, Lernszenarien; dazu „★ Favoriten“ und „Eigene“. Favoriten markierst du in AIS.chat.
- **Details:** Ein Klick auf eine Karte zeigt Beschreibung, Begrüßung, Aufgabe, ob die Websuche an ist, und Gesprächsanfänge. „Chat starten“ (oder Enter) beginnt einen neuen Chat; ein Gesprächsanfang startet ihn und schickt den Satz gleich ab.
- **Zurück:** Esc, erst zur Übersicht, dann zum Chat.

Bis zu fünf Favoriten stehen als Abkürzung in der Seitenleiste. Bilder speichert AIS.ChatCraft im Datenordner (`personas`) und lädt sie nach einem halben Tag neu.

Chats von AIS.chat tragen in der Seitenleiste das AIS.chat-Zeichen. Über „AIS.CHAT – VERLAUF“ holst du ältere AIS.chat-Chats herüber und schreibst darin weiter.

## Quellen pro Chat

Über **Quellen** im Eingabefeld legst du fest, was dieser Chat nutzen darf:

- **Gedächtnis:** deine Notizen.
- **Projektanweisungen:** die Anweisungen des Projekts, in dem der Chat liegt.
- **Werkzeuge:** Dokumente, Whiteboards, Zeitpläne, Aufgaben.
- **Andere Chats durchsuchen.**
- **Einzelne Konnektoren.**

Für einen neuen Chat stellst du die Quellen vor der ersten Nachricht ein. Das Menü bleibt offen, bis du „Schließen“ klickst.

Bei AIS.chat sind Gedächtnis und Projektanweisungen zuerst aus. Schaltest du sie ein, gehen sie am Anfang deiner nächsten Nachricht mit und bleiben im AIS.chat-Verlauf; erneut nur, wenn sie sich ändern. Werkzeuge und Konnektoren nimmt AIS.chat nicht an. Seine eigenen Werkzeuge (Websuche, Bilder, Diagramme) legt das Land fest, bei Assistenten und Figuren auch deren Einstellung in AIS.chat; das Menü zeigt sie.

## Projekte

Projekte bündeln Chats mit gemeinsamen Anweisungen, etwa „Antworte für Siebtklässler, kurz und mit Alltagsbeispielen“. Ein Klick auf ein Projekt in der Seitenleiste zeigt seine Chats und startet einen neuen Chat darin. Rechtsklick auf einen Chat › „In Projekt“ verschiebt ihn.

## Dokumente und Whiteboards

**Aus einer Antwort:** „Als Dokument“ oder „Als Whiteboard“ unter einer Antwort öffnet sie rechts neben dem Chat. Aus einer Gliederung wird auf dem Whiteboard eine Mindmap.

**Dokumente** sind Markdown. Oben wechselst du zwischen Bearbeiten und Vorschau. „Mit KI überarbeiten“ schickt das Dokument mit deiner Anweisung an das gewählte Modell; die vorige Fassung bleibt unter **Versionen** (Pfeil-Symbol) erhalten. Exportieren als Markdown oder HTML (HTML lässt sich im Browser als PDF drucken).

**Whiteboards:** Doppelklick legt eine Notiz an. Eine ausgewählte Notiz zeigt „+“ an ihren Seiten für verbundene Notizen. Tab legt eine Unternotiz an. F1 oder „?“ auf dem Whiteboard zeigt alle Kürzel.

**Im Chat:** Der Schalter „Im Chat“ schickt das geöffnete Dokument oder Whiteboard mit der nächsten Nachricht als Kontext mit. Modelle mit Werkzeugen ändern es dann direkt. AIS.chat schlägt Änderungen in einem Block vor; unter der Antwort übernimmst du sie mit „Ins Dokument übernehmen“ oder „Aufs Whiteboard“.

## Gemeinsam bearbeiten

Dokumente und Whiteboards lassen sich live mit anderen bearbeiten. Dafür braucht es einen Server für die Zusammenarbeit (`aiscraft-collab-server`, z. B. von der Schule oder dem Träger betrieben). Ohne Server bleibt alles auf deinem Rechner.

**Teilen:** „Teilen“ oben im Dokument oder Whiteboard. Beim ersten Mal die Adresse des Servers eintragen (und den Schlüssel, falls er einen verlangt). „Teilen starten“ zeigt einen Code wie `K7M2Q-X9TDA` und einen Link; beides weitergeben.

**Beitreten:** „Teilen“ › Code oder Link einfügen › „Beitreten“, oder in der Seitenleiste „+“ bei Dokumente & Boards. Mit dem Link geht es auch ohne eingetragenen Server.

Änderungen erscheinen bei allen sofort. Ohne Verbindung arbeitest du weiter; beim nächsten Verbinden gleicht AIS.ChatCraft alles ab. „Nicht mehr teilen“ hört auf abzugleichen, deine Kopie bleibt. „Raum für alle beenden“ (nur wer geteilt hat) löscht den Raum auf dem Server. Wer den Code hat, kann mitbearbeiten.

## Gedächtnis

Schreib im Chat „Merke dir: …“, und AIS.ChatCraft legt eine Notiz an. Notizen entstehen nur aus deinen eigenen Worten, nie aus Antworten, Webseiten oder Dateien. Jede Notiz zeigt, woher sie stammt. Unter **Einstellungen › Gedächtnis** bearbeitest, ordnest und vergisst du Notizen. Vergessene Notizen lassen sich wiederherstellen.

## Konnektoren und Freigaben

Konnektoren sind MCP-Server, deren Werkzeuge Modelle aufrufen können: Obsidian, Dateien, Webseiten abrufen und viele mehr. Unter **Einstellungen › Konnektoren** gibt es Vorlagen, etwa für deinen Obsidian-Tresor.

Werkzeuge, die nur lesen, laufen direkt. Alles andere fragt vorher, mit genau den Angaben, die ausgeführt würden. „Einmal erlauben“ gilt nur für diese Angaben. Die **Vertrauensprüfung** zeigt dazu, ob deine eigenen Worte die Aktion decken; sie muss dich wörtlich zitieren.

Werkzeuge gibt es bei Claude (API und Abo) und OpenAI-kompatiblen Backends.

## Zeitpläne und Aufgaben

**Zeitpläne** (Einstellungen › Zeitpläne) schicken einen Auftrag wiederkehrend oder einmal ab, etwa werktags um 7 Uhr: `0 7 * * 1-5`. Die Antwort landet in einem eigenen Chat. **Beobachten** meldet nur, wenn sich etwas geändert hat. Modelle können selbst eine Nachfrage planen („Ich schaue Freitag nach“). Zeitpläne laufen, solange AIS.ChatCraft läuft; Werkzeuge, die eine Freigabe brauchen, sind dabei abgeschaltet.

**Aufgaben** laufen im Hintergrund in einem eigenen Chat (Titel mit ▶), während du weiterchattest. Das Ergebnis kommt in den Chat zurück, aus dem die Aufgabe kam.

## Fähigkeiten und Hinweise

**Fähigkeiten:** Nach Aufgaben mit mehreren Werkzeugschritten merkt sich AIS.ChatCraft das Vorgehen, ohne persönliche Daten. Passende Vorgehen gehen bei ähnlichen Anfragen als Hinweis mit, nie als Erlaubnis.

**Erinnern:** Passende Ausschnitte aus früheren Chats gehen als Kontext mit.

**Hinweise** (standardmäßig aus): Stündlich, außerhalb der Ruhezeit, achtet AIS.ChatCraft auf offene Zusagen, Fristen und Erinnerungswünsche in deinen Nachrichten und meldet sich höchstens einmal pro Punkt.

## Tresor

Im Tresor (Einstellungen › Tresor) liegen Zugangsdaten und Einmalcodes, die AIS.ChatCraft selbst in Konnektor-Aufrufe einsetzt. Das Modell schreibt nur Platzhalter wie `{{vault:schulportal.password}}` und sieht die Werte nie; jeder Aufruf mit Tresor-Daten fragt vorher. Einträge lassen sich aus einem Bitwarden-CSV-Export übernehmen. Die CSV-Datei danach löschen.

## Suche und Verlauf

Strg+Umschalt+F oder das Suchfeld oben in der Seitenleiste durchsucht alle Chats. Rechtsklick auf einen Chat: umbenennen, anheften, in ein Projekt verschieben, exportieren, löschen.

## Nutzung

Unter **Einstellungen › Nutzung** stehen die Tokens pro Monat, Backend und Modell, bei Claude über API auch die Kosten. Mit einer **Monatsgrenze** fragt AIS.ChatCraft vor dem Senden nach, sobald sie erreicht ist.

## Tastenkürzel

| Kürzel | Was |
|---|---|
| Strg+K | Befehle, Chats, Projekte und Modelle suchen |
| Strg+N | Neuer Chat |
| Strg+B | Seitenleiste ein und aus |
| Strg+Umschalt+F | Alle Chats durchsuchen |
| Strg+F | Auf der Seite „Entdecken“: suchen |
| Strg+R | Antwort neu erzeugen |
| Esc | Antwort anhalten |
| Strg+, | Einstellungen |
| Strg+Umschalt+E | Chat exportieren |
| F1 | Dieses Handbuch |
| Enter / Umschalt+Enter | Senden / neue Zeile |

## Daten und Datenschutz

- Chats, Dokumente, Whiteboards, Notizen und Einstellungen liegen in einer Datenbank auf diesem Rechner (unter Linux in `~/.local/share/aiscraft`).
- Zugangsdaten, Konnektoren und der Tresor liegen im Schlüsselbund des Systems.
- Was du an ein Backend schickst, verarbeitet dieses Backend. Bei AIS.chat gelten dessen Regeln für Schulen.
- Angehängte Dateien kopiert AIS.ChatCraft in seinen Datenordner, damit sie auch später noch mitgehen.

## Für Agenten und Skripte

- `aiscraft-cli chat "Text"` schickt eine Nachricht und gibt die Antwort aus; `aiscraft-cli status`, `search`, `export`, `call <befehl> '<json>'`.
- `aiscraft-cli mcp` ist ein MCP-Server mit allen Befehlen als Werkzeuge.
- `aiscraft --control` startet einen JSON-Steuerkanal auf 127.0.0.1 (Token in der Datei `control-token` im Datenordner).
- `aiscraft-cli manual` gibt dieses Handbuch aus.
