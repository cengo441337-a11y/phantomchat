# phantomchat

A new Flutter project.

## Getting Started

This project is a starting point for a Flutter application.

A few resources to get you started if this is your first Flutter project:

- [Learn Flutter](https://docs.flutter.dev/get-started/learn-flutter)
- [Write your first Flutter app](https://docs.flutter.dev/get-started/codelab)
- [Flutter learning resources](https://docs.flutter.dev/reference/learning-resources)

For help getting started with Flutter development, view the
[online documentation](https://docs.flutter.dev/), which offers tutorials,
samples, guidance on mobile development, and a full API reference.

## Background-Empfang auf Android

PhantomChat führt das Relay-Listening über einen **Foreground Service**
(`RelayForegroundService.kt`) im Hintergrund weiter, damit Nachrichten
auch dann ankommen, wenn die App geschlossen ist. Eine persistente
Notification ("PhantomChat — Hintergrund-Empfang aktiv") signalisiert dem
Nutzer, dass der Dienst läuft — Android verlangt diese Anzeige für jeden
Foreground Service und sie kann nicht weggewischt werden.

**Aktivierung:** Standardmäßig **AUS**. Der Nutzer muss den Dienst in den
Einstellungen ("Hintergrund-Empfang aktivieren") explizit anschalten.
Eine zweite, ebenfalls opt-in Toggle ("Bei Geräte-Start automatisch
starten") sorgt dafür, dass `BootReceiver` den Dienst nach einem Reboot
wieder hochfährt — auch das standardmäßig **AUS** (Privacy by default;
ein Messenger, der sich beim Boot automatisch ans Netz hängt, leakt
Metadaten).

### OEM-Einschränkungen ("Don't kill my app!")

Einige Android-Hersteller — namentlich **Xiaomi (MIUI)**, **Huawei
(EMUI)**, **OnePlus (OxygenOS)**, **Samsung (One UI)** und
**Vivo/Oppo** — beenden Hintergrund-Dienste teils nach wenigen Minuten,
unabhängig vom `WAKE_LOCK` und der Foreground-Notification. Eine
ausführliche, OEM-spezifische Anleitung pflegt das Projekt
[Don't Kill My App!](https://dontkillmyapp.com/) — dort sind für jedes
Gerät die nötigen Schritte (Auto-Start zulassen, App im Akku-Manager
sperren, Hintergrund-Aktivität erlauben) dokumentiert.

Sobald PR #8A (Wave 8A — Battery-Optimization-Exclusion-Request-Flow)
gemerged ist, fragt PhantomChat den Nutzer beim ersten Aktivieren des
Hintergrund-Empfangs aktiv nach der Akku-Optimierungs-Ausnahme
(`REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`); das fängt einen Großteil der
OEM-Kills ab, ersetzt aber die manuellen Schritte für Xiaomi/Huawei/usw.
nicht vollständig.
