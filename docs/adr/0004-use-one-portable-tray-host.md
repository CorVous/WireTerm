---
status: accepted
---

# Use one portable tray host

WireTerm remains one portable Windows process, but its lifetime is no longer tied to the editor viewport. Minimize retains normal Windows taskbar behavior; closing the editor hides it while Playlist playback and the Host bridge continue, **Open WireTerm** restores it, and only explicit **Quit WireTerm** from the notification-area menu terminates the process. If the notification-area icon cannot be created, closing retains normal exit behavior so WireTerm can never become inaccessible; this supersedes ADR-0001 without adding a service, second daemon, startup task, installer, or updater.
