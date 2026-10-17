# App Remover

Ein kleiner Programm-Entferner für Windows im Stil von Revo Uninstaller, geschrieben in Rust mit [egui](https://github.com/emilk/egui).
Programm auswählen, **Remove** klicken, fertig. Kein Assistent, keine verschachtelten Menüs.

## Was „Remove“ macht

1. Beendet die laufenden Prozesse des Programms (bei Apps wie Adobe Creative Cloud auch die bekannten Hintergrunddienste).
2. Startet den Deinstaller des Programms **still**, wo das geht (MSI `/qn`, Inno Setup `/VERYSILENT` oder der hinterlegte
   „Quiet“-Befehl).
3. **Fehlt der Deinstaller oder schlägt er fehl** (die typische Meldung *„Deinstallationsprogramm nicht verfügbar“*),
   bricht es nicht ab, sondern entfernt das Programm von Hand.
4. Sucht und löscht Reste: Installationsordner, passende Ordner in Programme / ProgramData / AppData, Startmenü- und
   Desktop-Verknüpfungen, Registry-Schlüssel, Autostart-Einträge, Dienste, geplante Aufgaben und zum Schluss den Eintrag
   unter „Apps & Features“.
5. Registry-Schlüssel werden vorher als `.reg`-Dateien nach `%LOCALAPPDATA%\AppRemover\backups\...` gesichert.
   Dateien werden endgültig gelöscht; gesperrte Dateien werden beim nächsten Neustart entfernt.

Die Erkennung ist absichtlich vorsichtig. Was genau den Namen des Programms trägt, wird automatisch gelöscht; was nur
ähnlich aussieht, wird nur angezeigt, wenn **Review leftovers** angehakt ist (oder mit `--include-medium`).
Herstellerordner wie `Adobe` werden nie gelöscht, nur durchsucht. Wer Creative Cloud entfernt, behält also Photoshop.
Gesperrt sind außerdem:

- alles in `C:\Windows` (auf jedem Laufwerk) sowie andere Windows-eigene Bereiche (z. B. `ProgramData\Microsoft`,
  `WindowsApps`, Windows Defender, `AppData\Local\Packages`),
- Laufwerkswurzeln, Programme-/Benutzer-/Temp-Ordner, persönliche Ordner (Dokumente, Bilder, OneDrive …) und
  Registry-Hauptschlüssel,
- App Remover selbst.

Zeigt ein Eintrag auf einen System- oder gemeinsamen Ordner (z. B. `C:\Windows\System32` oder `D:\Games`, in dem andere
Programme liegen), wird dieser Ordner weder gelöscht noch zum Erkennen von Diensten/Aufgaben/Autostart benutzt, und dort
laufende Prozesse werden nicht beendet. Tragen mehrere Programme denselben Namen (z. B. Python 3.11 und 3.12), wird nichts
Gemeinsames automatisch gelöscht. Verknüpfungen/Junctions werden beim Löschen nie verfolgt.

## Bedienung

- Einfach lostippen: Die Suche filtert sofort, auch wenn das Suchfeld gerade nicht fokussiert ist.
- Pfeiltasten wählen aus, **Enter** oder **Entf** (oder Doppelklick) entfernt, Enter bestätigt.
- **Sortieren:** Standardmäßig nach Größe, größte zuerst. Über „Sort by“ (oder Klick auf einen Spaltenkopf) geht es
  auch nach Name, Laufwerk, Hersteller oder Version; der Knopf daneben dreht die Reihenfolge um.
- **Laufwerke:** Die Spalte „Drive“ zeigt, auf welcher Festplatte ein Programm liegt. Über „Drive“ lässt sich die Liste auf
  ein Laufwerk beschränken; das Menü zeigt pro Laufwerk Anzahl und Gesamtgröße der Programme.
- Programme, die ihre Größe nicht angeben, werden im Hintergrund vermessen (Größe des Installationsordners).

## Windows-Sicherheit (SmartScreen / Defender)

App Remover ist nicht digital signiert. Neue, unsignierte Programme, die Administratorrechte verlangen und Dateien und
Registry-Einträge löschen, stuft Windows oft als verdächtig ein, auch wenn sie harmlos sind. Seit Version 0.4.0
vermeidet App Remover typische Auslöser (keine versteckte PowerShell, kein `taskkill`, keine Besitzübernahme von
Ordnern) und enthält Versionsangaben und ein Icon.

**Vertrauenswürdig im Sinne von Windows wird eine exe aber nur durch eine Code-Signatur** mit einem echten Zertifikat,
z. B. kostenlos für Open-Source-Projekte über die [SignPath Foundation](https://signpath.org) oder kostenpflichtig über
Microsofts „Azure Artifact Signing“. Ein selbst erstelltes Zertifikat hilft nicht.

Wenn Defender die Datei meldet oder in Quarantäne verschiebt:

1. In **Windows-Sicherheit → Viren- & Bedrohungsschutz → Schutzverlauf** nachsehen, was gemeldet wurde.
2. Die Datei bei Microsoft als Fehlalarm melden: <https://www.microsoft.com/en-us/wdsi/filesubmission>
   („Software developer“ oder „Home customer“ → „Incorrectly detected as malware“). Microsoft prüft das meist innerhalb
   weniger Tage und nimmt die Erkennung dann für alle zurück.

Defender-Ausnahmen oder das Abschalten des Virenschutzes sind dafür nicht nötig und nicht empfohlen.

## Herunterladen / bauen

`AppRemover.exe` gibt es unter **Releases** bzw. **Actions → build → Artifacts**. Beim Start fragt Windows einmal nach
Administratorrechten. Da die exe nicht signiert ist, warnt SmartScreen eventuell: „Weitere Informationen → Trotzdem
ausführen“.

Selbst bauen (Windows, [Rust](https://rustup.rs)): `cargo build --release` → `target\release\AppRemover.exe`

Kommandozeile (in einem Administrator-Terminal):

```
AppRemover list adobe
AppRemover remove "Adobe Creative Cloud" --dry-run   # nur anzeigen, was passieren würde
AppRemover remove "Adobe Creative Cloud"             # ohne Rückfragen entfernen
AppRemover remove "Irgendwas" --force                # Deinstaller überspringen
AppRemover --demo                                    # Oberfläche mit erfundenen Programmen ausprobieren
```

## Entwicklung

Die gesamte Logik (Programmliste, Deinstaller-Aufruf, Reste-Erkennung, Schutzregeln, Aufräumen) liegt in der Bibliothek
unter `src/` und wird mit `cargo test --lib` getestet (läuft auf Linux und Windows). Die Windows-Aufrufe (`winreg`,
`reg`, `sc`, `schtasks`, `taskkill`) sind dünne Hüllen in `registry.rs` und `system.rs`.
`AppRemover --demo` startet die Oberfläche mit einer erfundenen Registry und ändert garantiert nichts.

**Stand:** Auf einem echten Windows-PC mit echten Programmen ist das Entfernen noch nicht ausprobiert worden; bei wichtigen
Programmen zuerst `--dry-run` oder **Review leftovers** verwenden.
