# AIS.ChatCraft

AIS.ChatCraft ist ein Programm für den Rechner, mit dem du mit AIS.chat und anderen KI-Diensten chattest. Deine Chats, Dokumente und Whiteboards liegen auf diesem Rechner und sind durchsuchbar.

AIS.ChatCraft ist kein offizielles Werkzeug des AIS.chat-Teams, sondern ein Community-Experiment. Es nutzt AIS.chat über die Web-App; Funktionen können sich ändern oder ausfallen. Rückmeldungen und Ideen gehen über [GitHub](https://github.com/FWU-DE/aiscraft/issues/new).

## Erste Schritte

1. **Anmelden.** Unten links auf das Konto klicken oder im Startbildschirm „Bei AIS.chat anmelden“ wählen.
2. **Modell wählen.** Im Eingabefeld auf das Modell klicken.
3. **Schreiben.** Enter sendet, Umschalt+Enter macht eine neue Zeile.

Weitere Dienste (Claude, OpenAI, Ollama, LM Studio …) fügst du unter **Einstellungen › Dienste** hinzu.

### Startseite

Ist kein Chat offen, zeigt AIS.ChatCraft die Startseite: oben das Eingabefeld (Schreiben beginnt einen neuen Chat), darunter Karten. In „Modern“, „Glas“ und „Omarchy“ begrüßt dich die Startseite mit Datum und Namen (aus der AIS.chat-Anmeldung, sonst aus dem Benutzerkonto des Rechners).

- **Vorschläge** (Modern, Glas, Omarchy): Stundenentwurf, Arbeitsblatt, Elternbrief und Quiz; ein Klick schreibt einen Anfang ins Eingabefeld, den du anpasst und abschickst.
- **Zuletzt bearbeitet** (Modern, Glas, Omarchy): die drei Chats, Dokumente oder Whiteboards, an denen du zuletzt gearbeitet hast.
- **Favoriten:** deine mit Stern markierten Assistenten, Dialogpartner und Lernszenarien aus AIS.chat; ein Klick startet einen Chat. „Alle entdecken“ öffnet die Übersicht.
- **Zeitpläne:** die nächsten Läufe; „+ Zeitplan anlegen“ legt einen neuen an.
- **Kontingent & Verbrauch:** die AIS.chat-Token-Punkte (sind sie aufgebraucht, nimmt AIS.chat bis zum nächsten Zeitraum keine Nachrichten an), die Tokens dieses Monats pro Dienst, die Monatsgrenze und wie lange Anmeldungen (etwa das Claude-Abo) noch gelten. Es steht nur da, was die Dienste melden.
- **Benachrichtigungen:** was Zeitpläne, Beobachtungen, Nachfragen, Hinweise und Aufgaben gemeldet haben, neue zuerst. Ein Klick öffnet den Chat dazu; „Alle gelesen“ markiert alles. Dieselbe Liste öffnet die Glocke oben rechts; die Zahl daran zählt die neuen. Die letzten 50 bleiben gespeichert; die Meldung ans System kommt weiterhin.

**Chat schließen:** das × oben rechts im Chat (in Modern, Glas und Omarchy im Menü ⋯ oben rechts), Esc bei leerem Eingabefeld oder „Chat schließen“ in den Befehlen (Strg+K). Der Chat bleibt in der Seitenleiste.

## Anmelden

**AIS.chat.** „Im Browser anmelden“ öffnet ein eigenes Browserfenster mit der AIS.chat-Anmeldung (VIDIS, Schulkonto oder Microsoft 365). Nach der Anmeldung übernimmt AIS.ChatCraft die Sitzung und schließt das Fenster. Dafür braucht es Chrome, Chromium, Edge, Brave oder Vivaldi. Die Sitzung gilt einige Stunden; ist sie abgelaufen, meldet AIS.ChatCraft dich beim nächsten Start ohne Fenster neu an, solange die VIDIS-Sitzung noch gilt.

Ohne passenden Browser: den Sitzungs-Cookie `__Secure-ais-chat-app.authjs.session-token` aus den Entwicklerwerkzeugen (F12) des Browsers kopieren und im Anmeldedialog einfügen.

**Claude (Abo).** Nutzt dein claude.ai-Abo über die lokale Claude-CLI. Vorher im Terminal einmal `claude auth login` ausführen.

**Claude (API), OpenAI und andere.** API-Schlüssel eintragen. Lokale Server wie Ollama oder LM Studio brauchen keinen Schlüssel.

Zugangsdaten liegen im Schlüsselbund des Systems, nie in der Datenbank.

**Ersatzmodelle.** Unter **Einstellungen › Dienste** legst du eine Reihenfolge von Ersatzmodellen fest. Scheitert eine Anfrage am Modell (überlastet, Serverfehler, Zeitüberschreitung, Kontingent aufgebraucht), antwortet das erste passende Ersatzmodell, und unter der Antwort steht „Mit Ersatzmodell … beantwortet, weil …“. Bei Fehlern am Inhalt, etwa wenn AIS.chat eine Anfrage wegen unangemessener Inhalte blockiert, wird nicht gewechselt. Ersatzmodelle bei einem anderen Dienst bekämen den Chat; das passiert nur, wenn du „Auch Ersatzmodelle bei anderen Diensten nutzen“ einschaltest, und nie für AIS.chat-Chats oder Assistenten.

## Chatten

- **Modell wechseln:** jederzeit über „Dienst · Modell“; die nächste Antwort kommt vom neuen Modell.
- **Antwort anhalten:** Esc oder das Quadrat im Eingabefeld.
- **Neu erzeugen:** „Neu erzeugen“ unter der letzten Antwort oder Strg+R.
- **Bearbeiten:** über einer eigenen Nachricht auf das Stift-Symbol; alles danach wird neu erzeugt.
- **Fassungen:** Nach Bearbeiten oder Neu erzeugen bleibt die vorige Fassung erhalten. Mit **‹ 1/2 ›** an der Nachricht (bei Antworten über dem Text) wechselst du zwischen ihnen; was danach kam, wechselt mit. Das Modell, die Suche und der Export sehen nur die gezeigte Fassung. Bei AIS.chat gibt es die Fassungen nur in AIS.ChatCraft: Der Server führt einen eigenen Verlauf, in dem Neu erzeugen die letzte Antwort ersetzt und eine bearbeitete Nachricht hinzukommt; ‹ › ändert daran nichts.
- **Anhänge:** Büroklammer oder Dateien ins Fenster ziehen. Bilder und PDFs gehen an Modelle, die sie lesen können; Textdateien werden mitgeschickt. Für AIS.chat siehe „Dateien für AIS.chat“.
- **Diktieren:** Mikrofon im Eingabefeld. Der Text landet im Eingabefeld, du schickst ihn selbst ab. Braucht einen OpenAI-kompatiblen Dienst mit Transkription.
- **Vorlesen:** „Vorlesen“ unter einer Antwort. Nutzt AIS.chat, wenn du dort angemeldet bist.
- **Speichern als:** „Speichern als“ unter einer Antwort schreibt sie als Word, PDF oder Markdown; Excel nimmt die Tabellen der Antwort, eine pro Blatt. Stehen Termine in der Antwort (ein ics-Block oder eine Tabelle mit Datum), speichert „Kalenderdatei (.ics)“ sie für Outlook, Google, Apple oder Thunderbird.
- **Erstellt mit KI:** Was du aus einer AIS.chat-Antwort speicherst (Word, PDF, Markdown, Excel, Arbeitsblätter, Lern-Apps, H5P, Kalenderdateien, der Chat als Markdown), bekommt am Ende; gespeicherte Bilder tragen ihn als Kommentar in der Datei „Erstellt mit KI (AIS.chat, Modell …), Datum“; das verlangen die Nutzungsbedingungen von AIS.chat. Einstellungen › Fähigkeiten & Hinweise: für alle Dienste, nie, und ob der Prompt mitgeschrieben wird. Kopieren bleibt unverändert; ein Rechtsklick auf „Kopieren“ bietet „Mit Kennzeichnung kopieren“.
- **Lesbarkeit:** „Lesbarkeit“ unter einer Antwort zeigt Wortzahl, Flesch-Wert, LIX, etwa welche Klassenstufe und wie viele Sätze länger als 20 Wörter sind. Fährst du über die Zeile, siehst du die langen Sätze. Berechnet wird auf diesem Rechner; nichts geht nach außen.
- **Links prüfen:** „Links prüfen“ unter einer Antwort fragt jede Adresse darin einmal an (höchstens 20). Tote Links stehen durchgestrichen unter der Antwort. Adressen im lokalen Netz fragt AIS.ChatCraft nie an. Automatisch für jede Antwort: Einstellungen › Fähigkeiten & Hinweise (standardmäßig aus, weil dabei fremde Seiten kontaktiert werden).
- **Nachdenken:** Modelle, die vor der Antwort nachdenken (Claude, Denkmodelle bei Ollama, LM Studio oder DeepSeek), zeigen eine Zusammenfassung davon zugeklappt als „Nachgedacht“ über der Antwort; ein Klick klappt sie auf. Bei AIS.chat stehen dort die Analyseschritte. Wie gründlich Claude nachdenkt, stellst du pro Chat unter **Quellen** › „Nachdenken vor der Antwort“ ein: Standard, Wenig, Mittel, Gründlich oder Sehr gründlich. Gründlicher dauert länger und kostet mehr.
- **Antwortstil:** „Stil“ im Eingabefeld legt fest, wie dieser Chat antwortet: Normal, Knapp, Erklärend, Formell oder „Eigener Stil …“ nach einem Beispieltext, den du einfügst (etwa einem eigenen Elternbrief; übernommen wird nur der Stil, nicht der Inhalt). Der Stil gilt für diesen Chat; ein neuer Chat beginnt wieder normal. Bei AIS.chat geht er als Hinweis in deiner nächsten Nachricht mit, danach erst wieder, wenn du ihn änderst.
- **Temporärer Chat:** „Temporärer Chat“ oben auf der Startseite (anderswo auch „Inkognito“ genannt) beginnt vor der ersten Nachricht einen Chat, der nur im Arbeitsspeicher liegt: Er steht nicht in der Seitenleiste, wird nicht durchsucht oder erinnert, legt keine Notizen an (auch nicht mit „Merk dir“) und lernt keine Fähigkeiten. Notizen aus dem Gedächtnis gehen zuerst nicht mit (unter „Quellen“ zuschaltbar). Sobald du ihn schließt oder einen anderen Chat öffnest, ist er samt Kopien seiner Anhänge weg. Was du an den Dienst schickst, verarbeitet dieser trotzdem nach seinen Regeln; bei AIS.chat steht der Chat dort im Verlauf. Dokumente, die du daraus anlegst, bleiben.
- **Kontextfenster:** oben rechts im Chat, wie viel vom Kontext des Modells belegt ist. „≈“ heißt geschätzt. Die Tokens einer einzelnen Antwort stehen im Tooltip am Modellnamen über ihr.
- **Bilder erzeugen:** „🖼 Bild erzeugen“ in der Seitenleiste (oder ein Bildmodell im Modellmenü) startet einen Bild-Chat bei AIS.chat: beschreibe, was das Bild zeigen soll, und wähle das Format. Fehlt der Eintrag, hat dein Land die Bilderzeugung in AIS.chat ausgeschaltet.
- **AIS.chat-Kontingent:** unten links, wie viele Token-Punkte noch übrig sind.
- **Formeln und Diagramme:** Formeln in LaTeX (`$…$` und `\(…\)` im Text, `$$…$$` und `\[…\]` als eigene Zeile) und Mermaid-Flussdiagramme und -Mindmaps (```` ```mermaid ````) erscheinen gezeichnet, in Antworten wie in der Dokument-Vorschau. Fährst du über eine Formel, siehst du ihren Quelltext; ein Rechtsklick kopiert ihn. Eine Mindmap öffnest du mit „Als Whiteboard“ zum Weiterbearbeiten. Andere Mermaid-Arten zeigt AIS.ChatCraft als Quelltext.

**Assistenten, Dialogpartner und Lernszenarien.** „Entdecken“ in der Seitenleiste (oder Strg+K) öffnet eine Seite mit allem, was AIS.chat dir anbietet: eigene, an der Schule geteilte und offizielle Einträge, mit Bild und Beschreibung.

- **Suchen:** Strg+F, dann Name oder Beschreibung. Groß- und Kleinschreibung und Umlaute spielen keine Rolle („uebung“ findet „Übung“). Bleibt genau ein Treffer, öffnet Enter ihn.
- **Eingrenzen:** Alle, Assistenten, Dialogpartner, Lernszenarien; dazu „★ Favoriten“ und „Meine“.
- **Favoriten:** Zeige auf eine Karte und klicke auf den Stern oben rechts, oder auf „☆ Als Favorit markieren“ in den Details und im neuen Chat. Favoriten gelten auch in AIS.chat im Browser.
- **Details:** Ein Klick auf eine Karte zeigt Beschreibung, Begrüßung, Aufgabe, ob die Websuche an ist, und Gesprächsanfänge. „Chat starten“ (oder Enter) beginnt einen neuen Chat; ein Gesprächsanfang startet ihn und schickt den Satz gleich ab.
- **Zurück:** Esc, erst zur Übersicht, dann zum Chat.

Bis zu fünf Favoriten stehen als Abkürzung in der Seitenleiste. Bilder speichert AIS.ChatCraft im Datenordner (`personas`) und lädt sie nach einem halben Tag neu.

Chats von AIS.chat tragen in der Seitenleiste das AIS.chat-Zeichen. Über „AIS.CHAT – VERLAUF“ holst du ältere AIS.chat-Chats herüber und schreibst darin weiter.

## Schnelleingabe

Eine Frage von überall stellen, ohne erst das Fenster zu suchen: Lege im Desktop ein Tastenkürzel an, das `aiscraft --quick` startet. Das Fenster kommt mit einem neuen Chat nach vorn, der Cursor steht im Eingabefeld. Läuft AIS.ChatCraft noch nicht, startet es so. Eine schon laufende App nimmt die Frage an, wenn sie über die Schnelleingabe oder mit `aiscraft --control` gestartet wurde (sie hört dann auf 127.0.0.1, geschützt durch die Datei `control-token`); sonst trägst du `--control` im Starter ein.

- **Mit Bildschirmfoto:** `aiscraft --quick --screenshot`. Das Foto liegt dann als Anhang im Eingabefeld; du schreibst deine Frage dazu.
- **Frage gleich mitgeben:** `aiscraft --quick Was heißt Photosynthese auf Englisch?` schickt sie sofort ab.

AIS.ChatCraft liest dafür keine Tasten mit; das Kürzel gehört deinem Desktop. So legst du es an:

- **Hyprland:** in `~/.config/hypr/hyprland.conf` (bei Omarchy `~/.config/hypr/bindings.conf`): `bind = SUPER, A, exec, aiscraft --quick` und für das Foto `bind = SUPER SHIFT, A, exec, aiscraft --quick --screenshot`. Fotos macht `grim`.
- **GNOME:** Einstellungen › Tastatur › Tastenkürzel anpassen › Eigene Tastenkürzel › „+“, Befehl `aiscraft --quick`. Fotos macht `gnome-screenshot` (nachinstallieren, falls es fehlt).
- **KDE Plasma:** Systemeinstellungen › Tastatur › Tastenkürzel › Neu › Befehl oder Skript, Befehl `aiscraft --quick`. Fotos macht Spectacle.
- **Windows:** Rechtsklick auf die Verknüpfung von AIS.ChatCraft › Eigenschaften › Ziel um ` --quick` ergänzen, darunter „Tastenkombination“ setzen (Strg+Alt+…). Fotos macht Windows selbst.
- **macOS:** In der App „Kurzbefehle“ einen Kurzbefehl „Shell-Skript ausführen“ mit `/Applications/AIS.ChatCraft.app/Contents/MacOS/aiscraft --quick` anlegen und ihm in den Details ein Tastenkürzel geben. Das erste Bildschirmfoto fragt nach der Erlaubnis „Bildschirmaufnahme“.

Unter Wayland erlauben manche Desktops einem Programm nicht, sein Fenster selbst nach vorn zu holen; dann blinkt es in der Leiste. Bei Hyprland hilft `bind = SUPER, A, exec, aiscraft --quick && hyprctl dispatch focuswindow class:ai.uds.aiscraft`.

## Quellen pro Chat

Über **Quellen** im Eingabefeld legst du fest, was dieser Chat nutzen darf:

- **Gedächtnis:** deine Notizen.
- **Projektanweisungen:** die Anweisungen des Projekts, in dem der Chat liegt.
- **Werkzeuge:** Dokumente, Whiteboards, Zeitpläne, Aufgaben.
- **Andere Chats durchsuchen.**
- **Auch Chats mit anderen Diensten an … geben:** Ohne diesen Haken gehen nur Ausschnitte aus Chats mit demselben Dienst mit; Chats mit Claude erreichen AIS.chat also nicht ungefragt. Was deshalb wegblieb, steht als „Nicht mitgeschickt (anderer Dienst): …“ unter deiner Nachricht.
- **Websuche:** Claude (API) sucht und liest selbst im Netz. Andere Dienste mit Werkzeugen lesen Webseiten unter einer Adresse, nur lesend und nicht im lokalen Netz. Die genutzten Seiten stehen als **Quellen** unter der Antwort.
- **Einzelne Konnektoren.**

Für einen neuen Chat stellst du die Quellen vor der ersten Nachricht ein. Das Menü bleibt offen, bis du „Schließen“ klickst.

Bei AIS.chat sind Gedächtnis und Projektanweisungen zuerst aus. Schaltest du sie ein, gehen sie am Anfang deiner nächsten Nachricht mit und bleiben im AIS.chat-Verlauf; erneut nur, wenn sie sich ändern. Werkzeuge und Konnektoren nimmt AIS.chat nicht an. Seine eigenen Werkzeuge (Websuche, Bilder, Diagramme) legt das Land fest, bei Assistenten und Dialogpartnern auch deren Einstellung in AIS.chat; das Menü zeigt sie.

## Belege in Antworten

Mit Claude (API) gehen angehängte Dateien als Dokumente mit. Die Antwort nennt dann, welche Stelle einer Datei eine Aussage belegt; unter ihr steht etwa „Belege: [1] igel.txt“ oder „[2] plan.pdf, S. 3“. Fährst du über einen Beleg, siehst du die Aussage und das Zitat.

Ein Klick zeigt die Stelle: im Text der Datei markiert, mit dem Text davor und danach. Bei PDFs steht die Seite dabei, bei Webseiten aus der Websuche das Zitat und der Link. AIS.chat, OpenAI-kompatible Server und das Claude-Abo liefern keine Belege.

## Projekte

Projekte bündeln Chats mit gemeinsamen Anweisungen, etwa „Antworte für Siebtklässler, kurz und mit Alltagsbeispielen“. Ein Klick auf ein Projekt in der Seitenleiste zeigt seine Chats und startet einen neuen Chat darin. Rechtsklick auf einen Chat › „In Projekt“ verschiebt ihn.

## Dokumente und Whiteboards

**Aus einer Antwort:** „Als Dokument“ oder „Als Whiteboard“ unter einer Antwort öffnet sie rechts neben dem Chat. Aus einer Gliederung wird auf dem Whiteboard eine Mindmap.

**Dokumente** sind Markdown. Oben wechselst du zwischen Bearbeiten und Vorschau. „Mit KI überarbeiten“ schickt das Dokument mit deiner Anweisung an das gewählte Modell; die vorige Fassung bleibt unter **Versionen** (Pfeil-Symbol) erhalten. Exportieren (Pfeil nach unten) als Markdown, HTML, Word (DOCX) oder PDF.

**Whiteboards:** Doppelklick legt eine Notiz an. Eine ausgewählte Notiz zeigt „+“ an ihren Seiten für verbundene Notizen. Tab legt eine Unternotiz an. F1 oder „?“ auf dem Whiteboard zeigt alle Kürzel.

**Dateien vom Modell:** Modelle mit Werkzeugen erstellen auf Wunsch Excel-Tabellen (mit Kopfzeile und Formeln), PowerPoint-Folien (Titel, Stichpunkte, Tabellen, Sprechernotizen) sowie Word-Dateien und PDFs, z. B. „Mach mir eine Notenliste als Excel-Datei“. Die Datei erscheint als Karte unter der Antwort; „Speichern …“ fragt, wohin sie soll. AIS.ChatCraft führt dafür keinen Code des Modells aus. PDFs nutzen die Standardschriften; Emojis und seltene Zeichen erscheinen dort als ?.

**Im Chat:** Der Schalter „Im Chat“ schickt das geöffnete Dokument oder Whiteboard mit der nächsten Nachricht als Kontext mit. Modelle mit Werkzeugen ändern es dann direkt. AIS.chat schlägt Änderungen in einem Block vor; unter der Antwort übernimmst du sie mit „Ins Dokument übernehmen“ oder „Aufs Whiteboard“.

## Lern-Apps und Arbeitsblätter

**Lern-Apps** sind kleine interaktive Programme in einer einzigen HTML-Datei, etwa ein Bruch-Trainer oder ein Quiz. **Arbeitsblätter** sind Aufgabenblätter, die Schülerinnen und Schüler am Rechner lösen und die du als Arbeitsblatt und Lösungsblatt druckst.

**Erstellen mit AIS.chat:** Schreib das Thema ins Eingabefeld, z. B. „Brüche vergleichen, Klasse 6“, und wähle unten **Erstellen** › **Lern-App** oder **Arbeitsblatt**. AIS.ChatCraft schickt eine ausführliche Anfrage mit allen Vorgaben (eine Datei, kein Internet, barrierefrei, Deutsch) und öffnet das Ergebnis rechts. Im Chat steht davon nur dein Thema; die Vorgaben liegen eingeklappt darunter unter „Formatvorgabe“ und klappen per Klick auf. Kommt in einer Antwort ein ```` ```html ````-Block mit einer ganzen HTML-Datei oder ein ```` ```aiscraft-worksheet ````-Block vor, zeigt AIS.ChatCraft darunter eine Karte mit „Als Lern-App öffnen“ bzw. „Als Arbeitsblatt öffnen“. Kleine HTML-Schnipsel werden nie von selbst zu Lern-Apps.

**Whiteboard und Termine:** **Erstellen** › **Whiteboard** bittet um eine Mindmap, einen Cluster, einen Ablauf oder eine Zeitleiste; die Antwort kommt als ```` ```aiscraft-board ````-Block mit der Karte „Whiteboard öffnen“, die Anordnung übernimmt AIS.ChatCraft. Ist der Block fehlerhaft, bittet AIS.ChatCraft einmal um Korrektur, danach bleibt die Antwort als Text. **Erstellen** › **Termine** bittet um Termine als Kalenderdatei; gespeichert wird sie mit „Speichern als“ › „Kalenderdatei (.ics)“.

**Erstellen mit Claude oder OpenAI:** Modelle mit Werkzeugen legen Lern-Apps und Arbeitsblätter direkt an, wenn du darum bittest („Mach mir ein Arbeitsblatt zur Fotosynthese“). „Erstellen“ funktioniert genauso. Ein Dokument wird mit **Als Arbeitsblatt** oben im Dokument zu einem Arbeitsblatt.

**Ändern:** Mit „Mit KI ändern“ oben in der Lern-App oder im Arbeitsblatt, oder im Chat mit eingeschaltetem „Im Chat“ („Ändere die App: mehr Aufgaben“). Jede Änderung ist eine neue Fassung unter **Versionen**. Bei AIS.chat übernimmst du eine geänderte Fassung mit „In die Lern-App übernehmen“ bzw. „Ins Arbeitsblatt übernehmen“.

**Lern-Apps ansehen:** Rechts siehst du den Quelltext (bearbeitbar). **Öffnen** zeigt die App in einem eigenen Vorschaufenster, oder, wenn die Vorschau nicht installiert ist, im Browser. **Speichern …** legt die HTML-Datei ab, z. B. zum Weitergeben an deine Klasse; sie läuft offline in jedem Browser.

**Arbeitsblätter:** Unter **Bearbeiten** änderst du Titel, Aufgaben, Lösungen, Punkte, Tipps und Erklärungen, sortierst Aufgaben (⏶ ⏷) und fügst neue hinzu: Auswahl (eine oder mehrere richtig), richtig oder falsch, Lückentext (Lücken als `[[Lösung|Alternative]]`), Zuordnen, Reihenfolge, kurze Antwort, Zahl (mit Toleranz), freier Text (mit Musterlösung) und Info-Text. Unter **Lösen** füllst du es aus; **Prüfen** zeigt je Aufgabe richtig, teilweise richtig oder falsch und zählt die Punkte. Freie Texte bewertet die Lehrkraft. **Zurücksetzen** löscht alle Antworten.

**Ohne Punkte:** Den Schalter **Punkte vergeben** findest du unter **Bearbeiten** neben Fach und Klasse. Ist er aus, verschwinden die Punktefelder, **Prüfen** zeigt „4 von 6 richtig“ (teilweise richtige Aufgaben als „teilweise“), und Arbeits- und Lösungsblatt nennen keine Punkte. Die Punkte der Aufgaben bleiben gespeichert und sind wieder da, wenn du den Schalter einschaltest. Für neue Arbeitsblätter, auch die von der KI, gilt Einstellungen › Fähigkeiten und Hinweise › **Arbeitsblätter mit Punkten**; ist das aus, vergibt das Modell keine Punkte.

**Druckvorschau:** Der Reiter **Druckvorschau** zeigt die Seiten genau so, wie sie aus dem Drucker kommen; oben wechselst du zwischen Arbeitsblatt und Lösungsblatt.

**Drucken:** **Drucken** › „Arbeitsblatt drucken“ oder „Lösungsblatt drucken“ öffnet ein PDF im PDF-Programm, dort druckst du. Das Arbeitsblatt hat Linien und Kästchen zum Ausfüllen, das Lösungsblatt die Lösungen. Als PDF oder Word speichern geht im selben Menü. PDFs nutzen die Standardschriften; Emojis und seltene Zeichen erscheinen als ?.

**Datenschutz:** Eine Lern-App kann nichts aus dem Internet laden und nichts senden: AIS.ChatCraft gibt jeder geöffneten oder gespeicherten App eine strenge Sicherheitsregel (Content-Security-Policy) mit, die alle Verbindungen sperrt; das Vorschaufenster erlaubt außerdem keine Links nach außen und keine neuen Fenster. Lern-Apps und Arbeitsblätter liegen nur auf deinem Rechner, bis du sie speicherst oder teilst. Schreib keine Namen von Schülerinnen und Schülern in die Anfrage.

## H5P

**H5P** sind interaktive Übungen, die Lernplattformen wie Moodle, Mebis oder ByCS direkt abspielen. AIS.ChatCraft legt H5P-Inhalte als eigene Art neben Dokument, Whiteboard, Arbeitsblatt und Lern-App an und speichert sie als `.h5p`-Datei, die du in der Lernplattform hochlädst.

**Erstellen:** Thema ins Eingabefeld schreiben und **Erstellen** › **H5P-Quiz** wählen. Mit AIS.chat kommt die Antwort als ```` ```aiscraft-h5p ````-Block mit einer Karte „Als H5P öffnen“; Modelle mit Werkzeugen legen den Inhalt direkt an.

**Welche Aufgaben:** Multiple Choice (H5P.MultiChoice), Richtig oder falsch (H5P.TrueFalse), Lückentext zum Eintippen (H5P.Blanks) und Wörter in Lücken ziehen (H5P.DragText). Mehrere Aufgaben oder eine Einleitung werden ein Quiz (H5P.QuestionSet). Kurze Antworten und Zahlen aus Arbeitsblättern werden Lückentexte mit einer Lücke; Zahlen prüft H5P ohne Toleranz. Zuordnen, Reihenfolge, In Gruppen sortieren, freie Texte und Info-Texte hat H5P so nicht: AIS.ChatCraft nennt sie und lässt sie weg. Lösungen mit `/`, `:` oder `*` kann H5P nicht prüfen. Was beim Speichern verloren geht (Tipps, Toleranzen), steht unter den Aufgaben.

**Bearbeiten und Ausprobieren:** Unter **Bearbeiten** änderst du Titel, Einleitung und Aufgaben wie beim Arbeitsblatt, ohne Punkte. **Ausprobieren** löst die Aufgaben in AIS.ChatCraft. Das sieht anders aus als in der Lernplattform: einen H5P-Player hat AIS.ChatCraft nicht.

**H5P-Dateien öffnen:** Zieh eine `.h5p`-Datei ins Fenster oder wähle sie mit 📎. Sie wird geprüft (Größe bis 25 MB, Pfade, nur erlaubte Dateitypen) und im Daten-Ordner abgelegt. Quizze aus den vier Typen oben kannst du bearbeiten; Bilder und Bibliotheken im Paket bleiben beim Speichern erhalten, Bilder zeigt AIS.ChatCraft aber nicht. Andere Inhaltstypen (z. B. Interactive Video, Course Presentation) zeigt AIS.ChatCraft mit Namen und Bibliotheken an und speichert sie unverändert.

**In der Lernplattform:** Die Datei bringt die H5P-Bibliotheken nicht mit. Die Lernplattform braucht die Inhaltstypen in diesen Versionen: H5P.QuestionSet 1.20, H5P.MultiChoice 1.16, H5P.TrueFalse 1.8, H5P.Blanks 1.14, H5P.DragText 1.10. Fehlt einer, lehnt sie die Datei ab; dann bitte die Administration, die Inhaltstypen über den H5P-Hub zu aktualisieren.

## Gemeinsam bearbeiten

Dokumente und Whiteboards lassen sich live mit anderen bearbeiten. Dafür braucht es einen Server für die Zusammenarbeit (`aiscraft-collab-server`, z. B. von der Schule oder dem Träger betrieben). Ohne Server bleibt alles auf deinem Rechner.

**Teilen:** „Teilen“ oben im Dokument oder Whiteboard. Beim ersten Mal die Adresse des Servers eintragen (und den Schlüssel, falls er einen verlangt). „Teilen starten“ zeigt einen Code wie `K7M2Q-X9TDA` und einen Link; beides weitergeben.

**Beitreten:** „Teilen“ › Code oder Link einfügen › „Beitreten“, oder in der Seitenleiste „+“ bei Dokumente & Boards. Mit dem Link geht es auch ohne eingetragenen Server.

Änderungen erscheinen bei allen sofort. Ohne Verbindung arbeitest du weiter; beim nächsten Verbinden gleicht AIS.ChatCraft alles ab. „Nicht mehr teilen“ hört auf abzugleichen, deine Kopie bleibt. „Raum für alle beenden“ (nur wer geteilt hat) löscht den Raum auf dem Server. Wer den Code hat, kann mitbearbeiten.

## Gedächtnis

Schreib im Chat „Merke dir: …“, und AIS.ChatCraft legt eine Notiz an. Notizen entstehen nur aus deinen eigenen Worten, nie aus Antworten, Webseiten oder Dateien. Jede Notiz zeigt, woher sie stammt. Unter **Einstellungen › Gedächtnis** bearbeitest, ordnest und vergisst du Notizen. Vergessene Notizen lassen sich wiederherstellen.

**Sag es einmal.** Sagst du etwas Neues zu einer Sache, die schon notiert ist („Merke dir: Nicht mehr Anna, sondern Bob macht die Rechnungen“), ersetzt die neue Notiz die alte. Die alte landet bei den vergessenen Notizen mit dem Hinweis „ersetzt durch …“ und lässt sich wiederherstellen. Ob eine Notiz eine ältere ersetzt, prüft das gewählte Modell nur, wenn es deine Notizen ohnehin bekommt; sonst fragt AIS.ChatCraft dich.

**Schreibstil.** „Schreibstil aus meinen Nachrichten lernen“ (Einstellungen › Gedächtnis) liest deine eigenen Nachrichten in Chats mit dem gewählten Dienst und legt eine Stilnotiz an: Anrede und Gruß, du oder Sie, Satzlänge, Ton. Antworten, Dateien, Werkzeug-Ergebnisse und Aufträge von Zeitplänen und Aufgaben zählen nicht. Namen aus Klassenlisten, Namen nach einer Anrede („Liebe Frau …“), E-Mail-Adressen, Telefonnummern und alle Ziffern nimmt AIS.ChatCraft vorher heraus, auch aus der fertigen Notiz. Der Stil geht nur mit, wenn du etwas schreiben lässt (eine Mail, einen Brief, eine Antwort), und lässt sich pro Chat unter „Quellen“ abschalten.

## Konnektoren und Freigaben

Konnektoren sind MCP-Server, deren Werkzeuge Modelle aufrufen können: Obsidian, Dateien, Webseiten abrufen und viele mehr. Unter **Einstellungen › Konnektoren** gibt es Vorlagen, etwa für deinen Obsidian-Tresor.

Werkzeuge, die nur lesen, laufen direkt. Alles andere läuft nur, wenn du darum gebeten hast, sonst fragt AIS.ChatCraft vorher, mit genau den Angaben, die ausgeführt würden. Die Karte nennt die wenigen Werte, auf die es ankommt, fett: Empfänger, Betreff oder Titel, Datum, Datei, Betrag. Alle Angaben stehen eingeklappt darunter. „Einmal erlauben“ gilt nur für genau diese Angaben.

**Eine Bitte deckt alle Schritte.** Bittest du um etwas, das mehrere Aufrufe braucht („Verschieb alle Termine der Projektwoche um einen Tag“), fragt AIS.ChatCraft nicht bei jedem Schritt neu. Die **Vertrauensprüfung** liest dazu jede deiner Nachrichten zusammen mit der Antwort, auf die sie reagiert; ein „ja, mach das“ auf einen konkreten Vorschlag zählt so als Bitte. Sie nennt die Nachricht, aus der die Erlaubnis stammt, und vergleicht jeden ausdrücklichen Wert (Empfänger, Datum, Datei, Betrag) mit der Aktion; AIS.ChatCraft prüft das nach. Ein abweichender Wert, ein Wert, der nur im Vorschlag stand, angehaltene Anfragen und alles, was aus Werkzeug-Ergebnissen stammt, erlauben nichts. Aktionen, um die du nicht gebeten hast, löschende Werkzeuge und Tresor-Einträge fragen immer. Wartet eine Frage, laufen reine Lese-Aufrufe desselben Schritts weiter; andere Aufrufe enden mit dem Hinweis, dass für sie nicht gefragt wurde. Aufgaben und Zeitpläne führen nie etwas aus, das eine Freigabe braucht.

**Eigene Regeln** (Einstellungen › Konnektoren) sind die zweite Quelle der Vertrauensprüfung neben deinen Worten. „Erlauben“ lässt Passendes ohne Nachfrage laufen, etwa „Termine im Kalender immer eintragen“ (nie Löschendes oder Tresor-Einträge). „Verbieten“ lehnt Passendes ab, ohne zu fragen, etwa „nie Mails an Eltern senden“, auch wenn du im Chat darum bittest. Verbote gewinnen immer. Eine Regel gilt für alle Konnektoren oder nur für einen. Regeln gelten für Aktionen, die etwas ändern oder verschicken, und auch bei ausgeschalteter Vertrauensprüfung; Aufgaben und Zeitpläne öffnen sie nicht.

**Eine Freigabe, ein Ergebnis.** Eine freigegebene Aktion oder ein Tresor-Eintrag läuft nie zweimal: nicht bei einem Doppelklick, nicht aus einem zweiten Fenster und nicht nach einem Neustart. Bricht die Verbindung mitten im Aufruf ab, versucht AIS.ChatCraft es nicht erneut, sondern meldet „unsicher – bitte prüfen“; prüfe dann im Zielprogramm, ob es geklappt hat. Dieselbe Aktion fragt danach wieder, mit diesem Hinweis auf der Karte. Was beim Beenden noch lief, meldet AIS.ChatCraft beim nächsten Start ebenso.

Werkzeuge gibt es bei Claude (API und Abo) und OpenAI-kompatiblen Diensten.

### Pakete mit einem Klick

MCP-Pakete (`.mcpb`, wie die Erweiterungen von Claude Desktop) installierst du unter **Einstellungen › Konnektoren › Paket installieren …**. Vorher zeigt AIS.ChatCraft, was das Paket darf: welches Programm es startet (mit deinen Rechten, ohne Abschottung), welche Ordner, Dateien und Zugangsdaten es bekommt, und seine Einstellungen. Erst „Installieren und starten“ legt es unter `plugins/` im Datenordner ab und startet es. Zugangsdaten liegen im Schlüsselbund. Versteckte Ordner wie `.ssh` gibt AIS.ChatCraft keinem Paket.

- Seine Werkzeuge folgen denselben Freigaben wie jeder Konnektor. Verbote, die ein Paket mitbringt, gelten als eigene Regeln; Erlaubnisse übernimmt AIS.ChatCraft nie.
- Abgewiesen werden Pakete mit Pfaden, die aus dem Paket hinausführen, mit Verknüpfungen, mit einem Programm außerhalb des Pakets oder für ein anderes Betriebssystem.
- Aus- und einschalten oder entfernen geht in derselben Liste; Entfernen löscht Dateien, Einstellungen und Zugangsdaten.

### Plug-ins

Plug-ins erweitern AIS.ChatCraft, ohne es neu zu bauen. Sie folgen dem offenen Format [Agent Plugins](https://agent-plugins.org/specification), das auch Nod nutzt: `plugin.json` beschreibt das Plug-in, `mcp.json` seine Werkzeuge (MCP-Server), `skills/<name>/SKILL.md` seine Fähigkeiten in Markdown. Ein Plug-in-Ordner oder eine ZIP-Datei lässt sich wie ein Paket über **Plug-in-Ordner …** oder **Paket installieren …** prüfen und installieren. Wer Plug-ins von Hand in den Ordner `plugins` im Datenordner legt, findet sie nach **Ordner neu einlesen** in der Liste; sie starten erst, wenn du sie einschaltest.

- Die Werkzeuge folgen denselben Freigaben wie jeder Konnektor; Verbote im Plug-in (`extensions` › `ai.uds.aischatcraft` › `rules`) gelten als deine Regeln, Erlaubnisse nie.
- Fähigkeiten sind Hinweise für das Modell, wenn eine Bitte dazu passt, nie eine Erlaubnis. Skripte darin führt AIS.ChatCraft nicht aus.
- Ein fehlerhafter Server oder eine fehlerhafte Fähigkeit wird übergangen und genannt; der Rest lädt. Programme müssen im Plug-in liegen oder ein reiner Programmname sein (etwa `node`); Verknüpfungen nach außen werden nicht übernommen. Das alte HTTP+SSE wird nicht unterstützt.

### Ansichten von Konnektoren

Manche Konnektoren liefern zu einem Ergebnis eine kleine Oberfläche, etwa einen Kalender zum Auswählen oder eine Tabelle zum Filtern (MCP Apps, wie in Claude Desktop). Unter dem Ergebnis erscheint dann die Karte **Ansicht öffnen**. Die Ansicht öffnet im eigenen Vorschaufenster, nie im Hauptfenster, in den Farben deines Looks.

- Netz gibt es dort nur zu den Adressen, die der Konnektor ausdrücklich nennt, sonst keins. Ansichten, die beliebiges Netz (`*`), unverschlüsseltes `http`, fremde eingebettete Seiten, den falschen Typ oder mehr als 2 MB verlangen, öffnet AIS.ChatCraft nicht. Kamera, Mikrofon und Standort gibt es nie.
- Aktionen aus der Ansicht laufen wie Aufrufe des Modells: Lesendes direkt, alles andere fragt mit genau den Angaben. Eine Ansicht erreicht nur Werkzeuge ihres eigenen Konnektors. Links aus der Ansicht öffnen erst, wenn du auf „Im Browser öffnen“ klickst.
- Ohne Vorschaufenster (eine Fassung ohne `aiscraft-preview`) bleibt der Text des Ergebnisses im Chat; die Ansicht öffnet dann nicht, auch nicht im Browser.
- Noch nicht: Nachrichten der Ansicht an den Chat (`ui/message`).

### Lokale Dateien

„Lies die PDF in Downloads und fasse sie zusammen“ oder „Speichere den Bericht in Dokumente“ geht ohne eigenen Konnektor. Lokale Dateien sind zuerst aus. Fragt ein Modell danach, erscheint im Chat eine Frage; „Erlauben“ schaltet sie ein und führt die Aktion aus, „Ablehnen“ lässt sie aus. Aufgaben und Zeitpläne können sie nicht einschalten und greifen nie auf Dateien zu. Abschalten unter **Einstellungen › Konnektoren**.

- Erlaubt sind dein Persönlicher Ordner und externe Laufwerke. Verknüpfungen werden aufgelöst; zählt, wohin sie führen.
- Immer gesperrt: versteckte Ordner und Dateien (`.ssh`, `.config`, Browserprofile, Schlüsselbund), Programmordner (`Library`, `AppData`), Dateien mit Geheimnissen (`.env`, Schlüssel wie `id_rsa` oder `.pem`, Passwort-Datenbanken wie `.kdbx`), die Daten von AIS.ChatCraft und Ordner, die du unter **Ordner sperren** einträgst.
- Auflisten läuft ohne Frage. Lesen (Text bis 256 KB, PDF als Text) und Speichern (Text bis 1 MB) folgen den Freigaben oben; Ersetzen einer Datei fragt immer. Programme und Skripte legt AIS.ChatCraft nicht an.

## Zeitpläne und Aufgaben

**Zeitpläne** (Einstellungen › Zeitpläne) schicken einen Auftrag wiederkehrend oder einmal ab, etwa werktags um 7 Uhr: `0 7 * * 1-5`. Die Antwort landet in einem eigenen Chat. **Beobachten** meldet nur, wenn sich etwas geändert hat. Modelle können selbst eine Nachfrage planen („Ich schaue Freitag nach“). Zeitpläne laufen, solange AIS.ChatCraft läuft; Werkzeuge, die eine Freigabe brauchen, sind dabei abgeschaltet.

**Vorlagen** unter den Zeitplänen legen einen Zeitplan mit einem Klick an: Morgenüberblick (Termine heute, offene Mails), Wochenvorschau, Mails des Tages und Vertretungsplan beobachten. Fehlt dafür etwas, etwa ein Konnektor für Kalender oder E-Mail oder ein Modell, das Webseiten lesen kann, steht das an der Vorlage, und sie lässt sich erst anlegen, wenn es da ist.

**Ideen aus deinen Bitten.** Bittest du an mindestens drei Tagen um dasselbe, etwa jeden Montag „Fass mir die Mails der Schulleitung zusammen“, schlägt AIS.ChatCraft unter den Zeitplänen einen Zeitplan vor: mit deinem Auftrag, dem Rhythmus und dem Modell, das du dafür genommen hast. „Als Zeitplan anlegen“ legt ihn an, „Anpassen …“ öffnet ihn vorher zum Ändern, „Nein danke“ lässt ihn nie wieder auftauchen. Gesucht wird nur auf diesem Gerät, in deinen eigenen Nachrichten, ohne Modell. Sind die Hinweise an, meldet sich eine neue Idee einmal in den Benachrichtigungen. Das Modell jedes Zeitplans wählst du in seiner Karte.

**Aufgaben** laufen im Hintergrund in einem eigenen Chat (Titel mit ▶), während du weiterchattest. Solange Aufgaben, Zeitpläne oder Beobachtungen laufen, steht über dem Eingabefeld eine Statuszeile, etwa „2 Aufgaben laufen · Elternbrief prüfen …“; ein Klick öffnet den Chat. Das Ergebnis kommt als kleine Karte in den Chat zurück, aus dem die Aufgabe kam: Titel, eine Zeile Ergebnis und „Details“, das den Chat der Aufgabe öffnet.

## Fähigkeiten und Hinweise

**Fähigkeiten:** Nach Aufgaben mit mehreren Werkzeugschritten merkt sich AIS.ChatCraft das Vorgehen, ohne persönliche Daten. Passende Vorgehen gehen bei ähnlichen Anfragen als Hinweis mit, nie als Erlaubnis.

**Erinnern:** Passende Ausschnitte aus früheren Chats gehen als Kontext mit. Bei AIS.chat stehen sie und passende Vorgehen oben in deiner Nachricht, je Chat nur einmal, weil AIS.chat sie im Verlauf behält; „Mitgeschickt: …“ unter der Nachricht zeigt, welche früheren Chats und Vorgehen mitgingen. Ausschnitte früherer Chats nimmt AIS.ChatCraft nur, wenn „Andere Chats durchsuchen“ in den Quellen an ist.

**Hinweise** (standardmäßig aus): Stündlich, außerhalb der Ruhezeit, achtet AIS.ChatCraft auf offene Zusagen, Fristen und Erinnerungswünsche in deinen Nachrichten und meldet sich höchstens einmal pro Punkt.

## Tresor

Im Tresor (Einstellungen › Tresor) liegen Zugangsdaten und Einmalcodes, die AIS.ChatCraft selbst in Konnektor-Aufrufe einsetzt. Das Modell schreibt nur Platzhalter wie `{{vault:schulportal.password}}` und sieht die Werte nie; jeder Aufruf mit Tresor-Daten fragt vorher. Einträge lassen sich aus einem Bitwarden-CSV-Export übernehmen. Die CSV-Datei danach löschen.

## Namen schützen

Für Zeugnisbemerkungen, Elternbriefe und Förderpläne: Vor dem Senden ersetzt AIS.ChatCraft Namen von Schülerinnen und Schülern und andere persönliche Angaben durch Platzhalter. Aus „Lea Müller“ wird `[S1] [N1]`, aus einer E-Mail-Adresse `[E-Mail 1]`. Der Dienst (AIS.chat, Claude, ein anderer Server) sieht nur die Platzhalter; in der Antwort stehen bei dir wieder die Namen, auch beim Kopieren und beim Speichern als Markdown, Word oder PDF.

**Klassenliste:** Unter **Einstellungen › Namen schützen** eine Liste einlesen (CSV, Excel, OpenDocument oder Kontakte als vcf) oder Namen einfügen, eine Person pro Zeile. Eine Liste gilt für alle Chats oder nur für ein Projekt. Sobald es eine Liste gibt, ist der Schutz in passenden Chats an; der Schalter **Namen schützen** neben „Quellen“ schaltet ihn pro Chat aus oder ein.

**Was erkannt wird:**

- Vornamen allein, Nachnamen allein und ganze Namen aus der Liste, auch „Leas“, „Jonas'“, in Großbuchstaben oder „Müller, Lea“. Dieselbe Person behält im ganzen Chat denselben Platzhalter.
- E-Mail-Adressen, Telefonnummern (deutsche Schreibweisen), IBAN (mit Prüfsumme), Geburtsdaten (mit „geb.“ oder im Schulalter) und Adressen mit Straße und Hausnummer.

**Vorschau:** Über dem Eingabefeld steht, was genau rausgeht; Ersetzungen sind markiert. Ein Klick auf einen Treffer lässt ihn stehen oder ersetzt ihn doch. **Unsichere Treffer** (Namen, die auch Wörter sind, wie „Mark“ oder „Koch“, Namen in Kleinbuchstaben, Daten ohne „geb.“) werden ebenfalls ersetzt, damit im Zweifel nichts rausgeht; antippen lässt einen stehen. Auch Zeitpläne und Hintergrund-Aufgaben ersetzen sie. Mit **+ Begriff ersetzen** ergänzt du einen Namen, der in keiner Liste steht; er wird in diesem Chat immer ersetzt.

**Grenzen:** Spitznamen, Tippfehler, Namen außerhalb der Listen und Namen in Bildern oder PDF-Dateien, die als Datei hochgeladen werden, erkennt AIS.ChatCraft nicht. Text aus Dokumenten (Word, Text, Tabellen als CSV) wird mitgeprüft. Die Suche findet geschützte Namen nicht, denn gespeichert ist, was rausging. Von AIS.chat erzeugte Word- oder PDF-Dateien enthalten die Platzhalter. Die Vorschau ist eine Hilfe, keine Garantie: Lies den Text vor dem Senden.

**Wo die Zuordnung liegt:** nur in der Datenbank auf diesem Gerät, pro Chat. Sie steht in keinem Protokoll, geht an keinen Dienst und ist nicht im JSON-Export. **Platzhalter des Chats anzeigen** (Befehlspalette) zeigt sie; mit dem Chat wird sie gelöscht.

## Suche und Verlauf

**Im offenen Chat:** Strg+F öffnet ein Suchfeld über den Nachrichten. Es zeigt „2 von 5“, Enter (oder ›) springt zum nächsten Treffer, Umschalt+Enter (oder ‹) zum vorigen; die Nachricht mit dem Treffer ist umrandet. Groß- und Kleinschreibung spielt keine Rolle. Esc schließt die Suche.

**Alle Chats:** Strg+Umschalt+F oder das Suchfeld oben in der Seitenleiste durchsucht alle Chats: zuerst die Titel (alle Wörter müssen darin vorkommen), dann die Nachrichten. Rechtsklick auf einen Chat: umbenennen, anheften, in ein Projekt verschieben, exportieren, löschen.

## Nutzung

Unter **Einstellungen › Nutzung** stehen die Tokens pro Monat, Dienst und Modell, bei Claude über API auch die Kosten. Mit einer **Monatsgrenze** fragt AIS.ChatCraft vor dem Senden nach, sobald sie erreicht ist.

Darunter steht das **Aktionsprotokoll**: jeder Modellaufruf (Modell, Tokens, bei Claude über API die Kosten) und jede Werkzeugaktion mit Ziel, den wichtigsten Angaben und warum sie erlaubt war: „Nur lesen“, „Deine Bitte“ mit deinen Worten, „Deine Regel“, „Deine Freigabe“ oder warum sie nicht lief. Zugangsdaten aus dem Tresor stehen nie darin. Es behält die letzten 5000 Einträge; **Als CSV exportieren** speichert alle als Tabelle.

## Updates

Einmal am Tag schaut AIS.ChatCraft nach, ob es eine neue Version gibt. Gibt es eine, steht über deinem Konto ein Hinweis: **Herunterladen** öffnet den Download im Browser, **Was ist neu** die Seite mit den Änderungen. **Diese Version überspringen** blendet den Hinweis für diese Version dauerhaft aus, **Später** bis zum nächsten Start. Installiert wird nichts von selbst.

Dafür geht eine anonyme Anfrage an `api.github.com` (die Releases von FWU-DE/aiscraft), ohne Konto und ohne Daten aus deinen Chats. Unter **Einstellungen › Über** schaltest du das ab, prüfst mit **Jetzt prüfen** sofort und siehst deine Version.

## Erscheinungsbild

Unter **Einstellungen › Darstellung** wählst du, wie AIS.ChatCraft aussieht. Jede Wahl gilt sofort. Beim ersten Start fragt der Begrüßungsdialog danach. Wer AIS.ChatCraft schon eingerichtet hatte, sieht die Frage nach dem Update einmal.

- **Modern** (Standard): nachtblaue Seitenleiste, kräftige Schrift, Karten auf heller Fläche. Koralle markiert den aktiven Eintrag, Favoriten und Hinweise.
- **Glas**: leichte, lichtdurchlässige Flächen auf einem weichen Farbverlauf. Lavendel ist der Zweitakzent.
- **Omarchy**: sieht aus wie „Modern“, nimmt aber die Farben des Desktop-Themes und folgt ihm, wenn du es wechselst. Wo ein Theme zu wenig Kontrast hätte, hellt AIS.ChatCraft die Schrift auf oder dunkelt sie ab, damit alles lesbar bleibt.

**Farbschema:** Hell, Dunkel oder wie das System. Bei Omarchy bestimmt das Desktop-Theme hell oder dunkel.

**Transparenz** (nur Glas): „Verlauf“ zeigt die Glasflächen lichtdurchlässig über dem Verlauf. „Aus“ macht sie deckend. Das ist ruhiger und auf schwachen Rechnern schneller. Echte Unschärfe hinter dem Fenster (der Desktop scheint durch) gibt es nicht.

**Zoom** vergrößert Schrift und Bedienelemente. **Enter sendet** schaltet zwischen Enter und Strg+Enter zum Senden um.

Die Akzentfarbe ist überall das AIS-Blau.

## Tastenkürzel

| Kürzel | Was |
|---|---|
| Strg+K | Befehle, Chats, Projekte und Modelle suchen |
| Strg+N | Neuer Chat |
| Strg+B | Seitenleiste ein und aus (in einem schmalen Fenster unter 1000 Pixeln ist sie zuerst eingeklappt) |
| Strg+Umschalt+F | Alle Chats durchsuchen |
| Strg+F | Im offenen Chat suchen (Enter: nächster Treffer, Umschalt+Enter: voriger, Esc: schließen); auf der Seite „Entdecken“: suchen |
| Strg+R | Antwort neu erzeugen |
| Esc | Antwort anhalten; bei leerem Eingabefeld: Chat schließen |
| Strg+, | Einstellungen |
| Strg+Umschalt+E | Chat exportieren |
| F1 | Dieses Handbuch |
| Enter / Umschalt+Enter | Senden / neue Zeile |

## Dateien für AIS.chat

AIS.chat nimmt nur csv, docx, htm, html, md, odp, ods, odt, pdf, pptx, tex, txt, xlsx und Bilder (jpg, png, svg, webp) an, höchstens 20 MB pro Datei und fünf Bilder pro Chat. AIS.ChatCraft bereitet deine Dateien deshalb auf deinem Gerät vor, bevor sie hochgehen. Was passiert, steht auf dem Anhang, bevor du sendest.

- **Umwandeln:** Alte Word-, Excel- und PowerPoint-Dateien (.doc, .xls, .ppt), RTF, E-Books (.epub), E-Mails (.eml, .mbox), Kalender (.ics) und Kontakte (.vcf) gehen als Markdown-Datei hoch, eine einzelne Tabelle als CSV, iPhone-Fotos (HEIC) und AVIF als JPEG. Code und andere Textdateien gehen als .txt. Die Datei wird hochgeladen, nicht in die Nachricht eingefügt: So kostet sie nicht bei jeder Antwort Token-Punkte.
- **Fotos:** Standort, Kameradaten und andere Metadaten bleiben auf dem Gerät; Fotos werden auf 1000 Pixel verkleinert (das macht AIS.chat ohnehin). Auf dem Anhang steht dann „Standortdaten entfernt“.
- **Nur einen Teil senden:** Bei PDFs, Tabellen und Präsentationen trägst du auf dem Anhang ein, was mitgehen soll: „Nur Seiten 3-7“ (auch „1, 4, 6-8“), „Nur Blatt Noten“ (Name oder Nummer, geht als CSV) oder „Nur Folien 2-5“ (geht als Markdown). Weniger Daten verlassen das Gerät.
- **Geht nicht:** Dateien mit Passwort, kopiergeschützte E-Books und Typen, die AIS.chat nicht annimmt (z. B. .zip), meldet AIS.ChatCraft vor dem Senden mit einem Hinweis. Entferne den Passwortschutz zuerst.

Andere Backends bekommen Dateien wie bisher; was sie gar nicht lesen könnten (etwa .doc), lesen sie als umgewandelten Text.

## Daten und Datenschutz

- Chats, Dokumente, Whiteboards, Notizen und Einstellungen liegen in einer Datenbank auf diesem Rechner (unter Linux in `~/.local/share/aiscraft`).
- Zugangsdaten, Konnektoren und der Tresor liegen im Schlüsselbund des Systems.
- Was du an einen Dienst schickst, verarbeitet dieser Dienst. Bei AIS.chat gelten dessen Regeln für Schulen.
- Angehängte Dateien kopiert AIS.ChatCraft in seinen Datenordner, damit sie auch später noch mitgehen.
- Die Update-Prüfung fragt einmal am Tag anonym bei GitHub nach der neuesten Version (abschaltbar, siehe „Updates“).

## Für Agenten und Skripte

- `aiscraft-cli chat "Text"` schickt eine Nachricht und gibt die Antwort aus; `aiscraft-cli status`, `search`, `export`, `call <befehl> '<json>'`.
- `aiscraft-cli mcp` ist ein MCP-Server mit allen Befehlen als Werkzeuge.
- `aiscraft --control` startet einen JSON-Steuerkanal auf 127.0.0.1 (Token in der Datei `control-token` im Datenordner).
- `aiscraft --quick [--screenshot] [Frage]` gibt eine Frage über diesen Kanal an die laufende App (Befehl `chat.quick_ask`), siehe „Schnelleingabe“.
- `aiscraft-cli manual` gibt dieses Handbuch aus.

## Wenn etwas nicht klappt

- **Kein Speicherplatz mehr frei:** Ist die Festplatte voll, sagt AIS.ChatCraft das beim Speichern, Exportieren oder Anhängen, statt einen unerwarteten Fehler zu melden. Platz schaffen und noch einmal versuchen; eine vorhandene Datei bleibt dabei unverändert. Das ist kein Fehler der App, melden musst du ihn nicht.
- **Ein lokaler Dienst antwortet nicht:** Läuft ein Modell auf diesem Rechner (Ollama, LM Studio) und das Programm ist beendet, nennt die Meldung Adresse und Port. Starte das Programm und sende noch einmal.
- **Die Claude-CLI wurde beendet:** Sende die Nachricht noch einmal. Passiert es wieder, starte im Terminal `claude` und prüfe, ob sie läuft, dann AIS.ChatCraft neu.
- **Ein Konnektor antwortet nicht mehr:** Unter **Einstellungen › Konnektoren** mit **Neu verbinden** wieder starten.
- **Die Vorschau einer Lern-App schließt sich gleich wieder:** Die Lern-App noch einmal öffnen; wenn das wieder passiert, AIS.ChatCraft neu starten.

## Mitmachen

AIS.ChatCraft ist offen: Fehler melden, Ideen einbringen oder Code beitragen geht über GitHub.

- [So kannst du mitmachen](https://github.com/FWU-DE/aiscraft/blob/main/CONTRIBUTING.md)
- [Fehler melden oder Idee vorschlagen](https://github.com/FWU-DE/aiscraft/issues/new)
- Keine Zugangsdaten oder Schülerdaten in Fehlermeldungen: das Repository ist öffentlich.
