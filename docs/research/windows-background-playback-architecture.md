# Windows background playback architecture

## Decision

Ship one per-user, tray-resident `wireterm.exe`, not a Windows service and not a
separate playback daemon.

The process owns the complete host lifetime:

1. The main thread runs eframe/winit, the editor window, and the notification
   area icon. Closing the editor hides its root viewport; it does not terminate
   the process.
2. A playback supervisor owns the playlist state machine and committed
   configuration snapshots.
3. One serial actor is the only component allowed to open or hold the selected
   serial device.
4. One dedicated STA renderer thread owns WebView2 and its Win32 message pump,
   as already chosen by
   [the extension-rendering decision](https://github.com/CorVous/WireTerm/issues/8).
5. Optional trusted transforms remain bounded child processes in Windows Job
   Objects. They are render jobs, not persistent extension hosts.

All long-lived components communicate with typed commands and status events.
The editor reads status snapshots and submits commands; it does not directly
touch serial, scheduling, WebView2, or mutable persisted state.

Package the same executable as a full-trust MSIX desktop application and
distribute it with an `.appinstaller` file. Declare a packaged desktop
`windows.startupTask` that launches `wireterm.exe --background`, and expose an
opt-in **Start WireTerm when I sign in** setting. App Installer supports direct
installation of MSIX packages and `.appinstaller`-configured update checks,
including background update settings.
([Microsoft: packaged desktop startup tasks](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/desktop-to-uwp-extensions#start-an-executable-file-when-users-log-into-windows),
[Microsoft: App Installer overview](https://learn.microsoft.com/en-us/windows/msix/app-installer/app-installer-file-overview),
[Microsoft: App Installer update settings](https://learn.microsoft.com/en-us/windows/msix/app-installer/how-to-create-appinstaller-file#step-6-add-update-setting))

This is a background-capable desktop application, not a machine daemon.
Playback starts after the user signs in, runs with that user's credentials and
files, and ends when the user explicitly chooses **Quit WireTerm** or signs
out.

## Why one tray-resident process

### Editor close is a view transition, not an application shutdown

eframe exposes `ViewportCommand::CancelClose` and
`ViewportCommand::Visible(bool)`. On a root viewport close request, WireTerm
must send `CancelClose` and then hide the viewport. A tray **Open WireTerm**
command makes it visible and focused again. Only tray **Quit WireTerm**, an
explicit editor **Quit**, Windows session shutdown, or a fatal process failure
ends the event loop.
([egui `ViewportCommand`](https://docs.rs/egui/latest/egui/viewport/enum.ViewportCommand.html))

Create the tray icon on the eframe/winit event-loop thread. The `tray-icon`
crate requires a Win32 event loop on Windows and recommends forwarding tray
events so that the loop wakes. eframe also permits
`egui::Context::request_repaint` from another thread and calls application
logic after such a request even while the UI is hidden. Therefore the tray
event handler and background status channel can wake eframe through a cloned
`egui::Context`; the hidden editor need not poll.
([`tray-icon` platform and event-loop contract](https://docs.rs/tray-icon/latest/tray_icon/),
[`eframe::App` hidden-UI and repaint contract](https://docs.rs/eframe/latest/eframe/trait.App.html))

The MVP tray menu should contain:

- **Open WireTerm**
- **Pause** or **Resume**
- **Next item**
- **Refresh current item**
- **Start WireTerm when I sign in**
- **Quit WireTerm**

Its icon/tooltip and editor status should project the same runtime snapshot:
running, paused, disconnected, refreshing/rendering/sending, or item error.
The tray is a controller and status view, never a second playback engine.

### A service is the wrong user boundary

Windows services run in session 0 and cannot directly interact with the signed-in
user. Microsoft recommends a separate per-session GUI plus IPC when a service
needs user interaction. That split would be necessary merely to regain the tray
and editor, while also complicating access to the user's extension folders,
Credential Locker entries, and WebView2 profile.
([Microsoft: interactive services](https://learn.microsoft.com/en-us/windows/win32/services/interactive-services),
[Microsoft: session 0 isolation](https://learn.microsoft.com/en-us/windows/win32/services/service-changes-for-windows-vista#session-0-isolation))

WireTerm has no MVP requirement to display before sign-in or serve several
users simultaneously. A service would therefore add privilege, installation,
IPC, update, and ownership boundaries with no product benefit.

### A separate tray UI and playback daemon are also unnecessary

A two-process desktop design would keep playback alive if the editor process
exited, but the selected behavior is that closing the editor does not exit its
process. Two persistent processes would require a versioned IPC API, coordinated
updates, recovery when either half is stale, and arbitration over serial and
configuration. The single process already isolates failure-prone work at the
right boundaries: WebView2 on an STA thread and transforms in disposable child
processes.

## Single instance and exactly-one serial ownership

Acquire a per-logon-session instance guard before starting eframe. On Windows,
use a `Local\WireTerm.<stable-id>` named mutex with a security descriptor
restricted to the current logon SID. `CreateMutexW` reports
`ERROR_ALREADY_EXISTS` when the named object already exists; the `Local\`
namespace keeps independent interactive sessions separate.
([Microsoft: `CreateMutexW`](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw))

The primary instance also hosts a local-only named pipe protected by the same
logon SID. A later launch sends an activation such as `OpenEditor`, waits for an
acknowledgement, and exits before constructing eframe or opening serial.
Microsoft documents duplex named pipes for same-machine IPC and recommends a
logon-SID DACL to exclude other sessions; do not rely on the permissive default
pipe descriptor.
([Microsoft: named pipes](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes),
[Microsoft: named-pipe security](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights))

The mutex prevents two authorities; the serial actor enforces ownership inside
the authority:

- only the serial actor calls open/close/read/write on the port;
- the GUI asks it to select or reconnect a device instead of opening a probe
  handle;
- the playback supervisor submits a frame and awaits a typed acknowledgement;
- at most one send is active, and a bounded pending slot retains the current
  item while disconnected;
- explicit device changes cancel or finish the current transfer before the
  actor closes and reopens;
- actor exit closes the handle before process shutdown continues.

Do not expose the serial-port object through shared application state. That
would turn "one process" into accidental multi-owner access.

## Thread and actor layout

| Owner | Lifetime and responsibility |
| --- | --- |
| eframe main thread | Win32/winit event loop, root editor viewport, tray icon, command submission, status projection |
| playback supervisor | Ordered playlist state machine, refresh policy, dwell timing, failure/skip policy, reconnect resume, revision boundaries |
| serial actor | The sole serial handle, protocol transfer, panel acknowledgement, reconnect/backoff, device status |
| WebView2 STA renderer | One reusable isolated controller, Liquid-produced HTML/CSS capture, serialized preview/playback render jobs |
| fetch workers | Bounded network requests and cancellation; return JSON or typed failures |
| transform child | One bounded JSON stdin/stdout invocation in a Job Object per requested transform |
| state writer | Serialized configuration/checkpoint writes and publication of immutable revisions |

The WebView2 worker must remain separate from the eframe thread. Microsoft
requires WebView2 to be created on an STA thread with a message pump, requires
all WebView2 calls and callbacks on that thread, and warns that blocking its
message pump prevents asynchronous completions.
([Microsoft: WebView2 threading model](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/threading-model))

Give scheduled playback renders priority over interactive previews, but never
interrupt a capture midway through its transaction. This preserves one
WebView2 owner and prevents preview traffic from indefinitely delaying a
playlist.

## Playback lifetime and recovery

Model playback explicitly rather than as a timer attached to the editor:

```text
Paused
  -> Refreshing -> Rendering -> Sending -> Dwelling
                         |          |
                         |          +-> DisconnectedPending
                         +-> Failed -> advance to next item
```

- Refresh the selected item when its turn begins.
- Render and send one immutable item revision.
- Begin dwell only after the serial actor receives the panel's display
  acknowledgement.
- On item refresh/render failure, retain the current panel frame, record the
  error, and advance. No MVP error frame is sent.
- On disconnect, stop advancement and retain that item as
  `DisconnectedPending`. Reconnect refreshes and sends it before dwell begins.
- **Next item** cancels work at a defined cancellation boundary and advances;
  it never concurrently writes another frame.

Persist a small checkpoint after every meaningful transition:

- desired mode (`running` or `paused`);
- stable current playlist-item ID;
- last panel-confirmed item ID and timestamp;
- committed playlist revision;
- last error summary, excluding secrets.

Use replace-on-success persistence so an interrupted write leaves the prior
checkpoint valid. On process restart, reload the last committed configuration.
If desired mode was running, treat the checkpointed current item as pending,
refresh it, and send it again; a duplicate display is safer than silently
skipping an item whose prior completion is uncertain. If it was paused, restore
paused state. If the device is absent, enter `DisconnectedPending`.

Register the running process with Windows
`RegisterApplicationRestart("--background", ...)` as best-effort integration
for installer/OS restart and eligible crash or hang recovery. It is not a
watchdog: Windows may ask the user before restarting a crashed or hung app, and
it will not restart a process that failed within its first 60 seconds.
([Microsoft: `RegisterApplicationRestart`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-registerapplicationrestart))

At `WM_QUERYENDSESSION` or an update-requested graceful shutdown, stop accepting
new commands, atomically save the latest checkpoint, cancel bounded fetch/render
work, terminate any transform Job Objects, close serial, and exit within a
short deadline. StartupTask restores the process at the next enabled user
logon; Application Restart handles supported update/restart flows.

## Concurrent editor and author-file changes

The editor works against a mutable draft. **Apply** is one transaction:

1. validate the entire playlist and referenced item settings;
2. write the new configuration through a temporary file and atomic
   replace-on-success operation;
3. assign a monotonically increasing revision;
4. publish an immutable `Arc<PlaylistRevision>` to playback and the UI.

Playback holds the revision with which a turn began. A newer applied revision
takes effect at the next item boundary, never between refresh, render, send,
and acknowledgement. If the current item is deleted or reordered, it completes
or fails under its old snapshot, then the supervisor resolves the next stable
item ID in the newest revision. This prevents partial edits from changing a
frame in flight.

External extension-file notifications remain invalidation signals as resolved
in the rendering-stack decision. After debounce, load and validate a complete
new extension revision; keep the prior valid revision when reload fails.
A preview and a playlist turn may reference different immutable revisions, but
the single WebView2 queue serializes their captures. Secrets remain indirect
Credential Locker references in every persisted snapshot, never copied into
playlist files, checkpoints, status, or logs.

## Startup, installation, and updates

Use a single signed MSIX package containing `wireterm.exe`, bundled application
assets/fonts, and the WebView2 Evergreen bootstrapper path selected by the
rendering-stack decision. Keep mutable playlists, checkpoints, logs, previews,
and generated extension scaffolds in per-user writable data locations, never
inside the installed package.

The startup setting should use the package manifest/API rather than an
installer-authored `HKCU\...\Run` value:

- Windows presents registered startup tasks in Startup Apps/Task Manager.
- Users retain control and can disable the task.
- the package-relative executable identity survives versioned MSIX updates;
- uninstall removes package registration without bespoke registry cleanup.

Microsoft documents that a packaged desktop startup task runs at user logon,
requires the application to have been launched once to register, and cannot be
silently re-enabled after the user disables it.
([Microsoft: desktop application startup integration](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/desktop-to-uwp-extensions#start-an-executable-file-when-users-log-into-windows),
[Microsoft: `StartupTask`](https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.startuptask))

Publish an `.appinstaller` beside each signed MSIX release with on-launch and
automatic-background update settings. App Installer associates the installed
package with that update source and can check for newer packages without
embedding a custom updater in WireTerm.
([Microsoft: create an App Installer file](https://learn.microsoft.com/en-us/windows/msix/app-installer/how-to-create-appinstaller-file))

An update that needs WireTerm to stop must use graceful shutdown/restart, never
launch a new-version process beside an old serial owner. The instance guard
and pipe handshake remain the last defense: the new activation redirects to
the old instance until Windows has actually released it.

## Rejected alternatives

### Windows service plus per-user GUI

Rejected for the MVP because services are isolated from the interactive
session. It would require a second executable, authenticated IPC, cross-session
state, elevated installation, and explicit rules for which user's playlist and
credentials control the device.

### Separate playback daemon and eframe editor

Rejected because it solves editor-close behavior that hiding the root viewport
already solves. It also creates two versioned binaries and an IPC protocol and
makes serial/configuration ownership a distributed-systems problem.

### Exit on editor close and rely on Task Scheduler

Rejected because a scheduled relaunch does not provide continuous dwell
timing, immediate tray controls, or deterministic ownership after a normal
window close. Task Scheduler is also the wrong abstraction for a process that
must remain available throughout the signed-in session.

### Registry `Run` startup and a custom updater

Viable for an unpackaged fallback, but not the primary recommendation. It
requires WireTerm to maintain install paths, startup registration, update
replacement, restart coordination, and uninstall cleanup that MSIX and App
Installer already model.

## Implementation slices and acceptance checks

1. **Tray-resident shell**
   - Close hides the editor while a synthetic playlist continues through at
     least two dwell transitions.
   - Tray open, pause/resume, next, refresh, and quit work while hidden.
   - Background launch creates no visible editor or taskbar window.
2. **Single activation**
   - Fifty simultaneous launches produce one primary process.
   - Every secondary activation either opens the primary editor and exits or
     reports a bounded startup failure; none opens serial.
   - Separate signed-in Windows sessions may each run an instance, but only the
     selected session can acquire the physical port; failure is surfaced, not
     retried aggressively.
3. **Serial actor**
   - Instrumented tests prove only one open handle and one write transaction.
   - Disconnect during render, send, and dwell reaches the documented pending
     state and reconnect displays the pending item before dwell.
4. **Revision handoff**
   - Apply reorder/delete/settings changes during every playback phase.
   - The in-flight turn uses one revision; the next boundary uses the newest
     valid revision.
   - Invalid extension reload preserves the last valid revision and surfaces
     its error.
5. **Restart**
   - Clean quit stays stopped; close-to-tray stays running.
   - Sign-out/reboot with startup enabled restores running or paused intent.
   - Forced termination at each checkpoint write leaves readable state and
     resumes the checkpointed item without skipping.
6. **Render/process integration**
   - Hidden/tray-only operation completes WebView2 captures on its STA worker.
   - Preview load cannot starve a scheduled render.
   - Quit/update leaves no WebView2 worker or transform descendant.
7. **Packaging**
   - Clean install, update, rollback failure, and uninstall leave user content
     intact and package binaries consistent.
   - Startup registration is visible and user-controllable in Windows.
   - An update never permits old and new processes to own serial concurrently.

## Remaining decisions and fog

- **Prototype required:** issue 8's open question remains: confirm hidden or
  off-screen WebView2 capture while the editor viewport is hidden. This
  architecture supplies an independent STA/message-pump thread but does not
  prove the controller paints in tray-only operation.
- **Distribution decision:** choose the MSIX signing identity, certificate
  trust/distribution path, update host, and supported Windows minimum version.
  These affect release operations, not the process topology.
- **Library decision:** validate a Rust `tray-icon` version against the exact
  eframe/winit version and prove hidden-window tray wakeups before freezing
  dependencies.
- **Shutdown policy:** choose concrete fetch, render, transform, and serial
  cancellation deadlines for update and Windows shutdown.
- **Multi-session policy:** the recommendation permits one process per logon
  session while the physical serial device remains exclusive. Decide whether a
  later release should identify the owning session more explicitly when fast
  user switching occurs.
- **Crash policy:** Application Restart is best effort and user-mediated for
  crashes/hangs. Add a watchdog only if measured reliability shows that
  unattended crash recovery is a real requirement.
