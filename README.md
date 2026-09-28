# StorDown

StorDown is a modern Windows **download + upload manager** built around a multi-link transfer engine.

The project combines classic download-manager features with explicit use of multiple local network interfaces/WAN paths, browser capture and first-class cloud providers.

## Current V0.1 bootstrap

The repository currently contains:

- Rust HTTP/HTTPS download engine;
- HTTP Range segmentation;
- per-worker bind to local IPv4/IPv6 addresses;
- multiple enabled links with weights;
- Google Drive resumable upload engine;
- native Google Drive browser with My Drive + Shared Drives browsing;
- Google Drive/Workspace shared-link import with resource-key support;
- Range-aware Google Drive blob downloads through the Smart Multi-WAN engine;
- Google Workspace export from the Cloud browser (Docs, Sheets, Slides, Drawings and Apps Script);
- multiple-file Drive upload distribution across NICs/WANs;
- CLI proof of concept for downloads and Drive uploads;
- Tauri + React desktop interface with Download and Upload workspaces;
- Chrome/Edge Manifest V3 extension skeleton;
- UDM Pro multi-WAN setup notes;
- automatic discovery of active physical Windows NICs;
- per-interface public-IP route tests to verify UDM WAN policies;
- live transfer progress and real-time speed per local interface/WAN;
- pause, resume and cancel controls for active downloads/uploads;
- persistent segmented HTTP download parts with automatic retry/resume;
- unified download/upload queue with up to two simultaneous jobs;
- persistent SQLite transfer history in the StorDown app-data folder;
- native Windows save/open dialogs for download destinations and upload file selection;
- Chrome/Edge native-messaging bridge with context-menu and automatic capture;
- explicit per-site cookie handoff for authenticated browser downloads;
- browser per-site capture/ignore policies and active-page batch link capture;
- desktop one-click Native Messaging registration for Chrome and Edge;
- automatic download categories and extension-based destination rules;
- persistent HTTP download scheduler with restart recovery;
- adaptive Multi-WAN chunk scheduling with automatic link failover;
- adaptive Google Drive batch assignment with resumable chunk failover;
- persistent Google Drive resumable-upload checkpoints across app/Windows restarts;
- per-transfer aggregate speed limits for HTTP downloads and Drive upload batches;
- optional SHA256 verification before HTTP downloads are marked complete;
- GitHub Actions CI.

This is still an early proof of concept, not a production release.

## Two-WAN downloads

```text
Large file
   |
   +-- Segment 0 --> NIC/IP 1 --> UDM policy --> WAN1
   +-- Segment 1 --> NIC/IP 2 --> UDM policy --> WAN2
   +-- Segment 2 --> NIC/IP 1 --> UDM policy --> WAN1
   +-- Segment 3 --> NIC/IP 2 --> UDM policy --> WAN2
```

The origin must support byte-range requests for one file to be split across both links.

A normal Ethernet switch between the PC and the UDM is fine. The UDM must see both NIC source IPs separately and route each one to a different WAN.

## Google Drive uploads

StorDown now has the first native Drive upload engine using resumable sessions.

For multiple files:

```text
arquivo1.mkv -> NIC1 -> WAN1 -> Google Drive
arquivo2.mkv -> NIC2 -> WAN2 -> Google Drive
arquivo3.mkv -> NIC1 -> WAN1 -> Google Drive
arquivo4.mkv -> NIC2 -> WAN2 -> Google Drive
```

This allows a batch to use both Internet links at the same time.

A single Google Drive resumable upload is sequential, so one large file does not directly use both WANs simultaneously. The planned **StorDown Relay** mode will solve that case by striping pieces over multiple WANs to a relay that reconstructs the stream before sending it to Drive.

See [docs/GOOGLE_DRIVE.md](docs/GOOGLE_DRIVE.md).

## CLI download proof of concept

```powershell
cargo run -p stordown-cli -- download "https://example.com/large-file.iso" `
  --output "C:\Downloads\large-file.iso" `
  --connections 8 `
  --bind 192.168.30.101 `
  --bind 192.168.30.102
```

Replace the sample IPs with the real addresses of the two Ethernet adapters.

## CLI Google Drive upload proof of concept

The temporary development flow accepts an OAuth access token through an environment variable:

```powershell
$env:STORDOWN_GOOGLE_ACCESS_TOKEN="ACCESS_TOKEN"

cargo run -p stordown-cli -- upload-drive `
  --file "C:\Uploads\arquivo1.mkv" `
  --file "C:\Uploads\arquivo2.mkv" `
  --bind 192.168.30.101 `
  --bind 192.168.30.102 `
  --chunk-mib 8
```

The desktop app uses its built-in OAuth flow; the environment-token example above is only for CLI development.

## Desktop development

```powershell
cd apps\desktop
npm install
npm run tauri dev
```

The current UI has separate Download and Google Drive Upload workspaces.

## Browser extension

The `browser-extension` folder contains the first Chrome/Edge Manifest V3 integration. It adds **Baixar com StorDown** to link context menus and is prepared to communicate with a Windows native-messaging host.

## Project layout

```text
apps/
  cli/                 CLI proof of concept
  desktop/             Tauri + React desktop app
crates/
  stordown-core/       Native Rust transfer engine
browser-extension/     Chrome/Edge integration
docs/
  ARCHITECTURE.md
  GOOGLE_DRIVE.md
  ROADMAP.md
  UDM_MULTI_WAN.md
```

See [docs/ROADMAP.md](docs/ROADMAP.md) for planned milestones.


## Network auto-detect

The desktop app can now ask Windows for active **physical** network adapters and fill the StorDown bind-IP list automatically.

The **Testar WANs** action opens one outbound request bound to each selected local IP. StorDown shows the public IP observed on each path, which makes it easy to confirm whether UDM policy-based routing is actually sending NIC1 and NIC2 through different WANs.

```text
Ethernet 1  192.168.x.x -> public IP A
Ethernet 2  192.168.x.y -> public IP B
```

Different public IPs confirm that the two source interfaces are reaching the Internet through distinct egress paths.


## Live multi-WAN telemetry

The desktop backend now streams progress events from the Rust transfer engine into the Tauri UI.

During a download or Google Drive upload, StorDown shows:

- overall transferred bytes and percentage;
- per-file upload progress;
- the local interface used by each worker/file;
- live bytes per second per local IP/WAN;
- aggregate speed across all active links.

For segmented HTTP downloads the progress counter is shared across workers, so one file can show aggregate progress while each WAN still reports its own throughput. Google Drive batch uploads report each file independently while the sidebar shows the throughput of each WAN.


## Pause, retry and HTTP resume

StorDown now keeps segmented HTTP download state beside the destination as a temporary `.stordown.parts` directory.

Each Range segment is written independently. If a segment connection drops, StorDown reopens that segment from the last byte already written instead of restarting the whole file. If the app is stopped or the transfer is cancelled, the partial segment files remain available. Starting the same URL again with the same destination, file size and segment count reuses those bytes.

The desktop UI also exposes **Pausar**, **Retomar** and **Cancelar** for an active transfer. Google Drive uploads honor pause/cancel between resumable chunks. Drive session persistence across a full application restart remains a separate roadmap item.


## Unified queue and persistent history

Downloads and Google Drive uploads now enter the same StorDown queue instead of blocking the desktop UI until one transfer finishes.

The first queue scheduler allows up to **two active transfer jobs at the same time**. Each job can still use multiple HTTP workers and multiple bound NIC/WAN paths internally.

Transfer metadata is stored in a local SQLite database under the StorDown application-data directory. The database records:

- transfer ID and direction;
- source and destination;
- provider;
- status;
- transferred and total bytes;
- configured connection count;
- bound local IPs;
- timestamps and last error.

On application startup, transfers that were left as running or paused are marked as **interrupted** so the UI never pretends that a dead process is still active. Completed, failed and cancelled entries remain visible in the history until the user removes them.


## Windows file selection

The desktop UI now uses native Windows dialogs instead of requiring users to type every path manually.

- **Downloads:** `Procurar…` opens a Save dialog and suggests a filename derived from the URL when possible.
- **Google Drive uploads:** `Selecionar arquivos…` opens the Windows multi-file picker and fills the upload batch automatically.

Manual path editing remains available for advanced workflows and scripting-style use.


## Browser capture

StorDown now includes a Rust native-messaging host for Chrome and Edge.

When the extension captures a direct HTTP/HTTPS download, the path is:

```text
Browser extension
  -> Chrome/Edge Native Messaging
  -> stordown-native-host.exe
  -> 127.0.0.1:17832
  -> StorDown Desktop
  -> persistent transfer queue
  -> Multi-WAN download engine
```

The browser copy is cancelled only after the desktop returns an acknowledgement that the transfer entered the StorDown queue. If StorDown is not running or the native host is unavailable, the browser download is left alone.

See [browser-extension/README.md](browser-extension/README.md) for development installation and native-host registration.


## Download categories and destination rules

StorDown can now organize downloads captured by the browser before they enter the transfer queue.

Rules are stored in the local SQLite database and contain:

- a category name;
- one or more file extensions;
- a destination folder;
- enabled/disabled state;
- priority.

Example:

```text
Vídeos     mkv, mp4, avi  -> C:\Downloads\Vídeos
Imagens    jpg, png, webp -> C:\Downloads\Imagens
Arquivos   zip, rar, 7z   -> D:\Downloads\Compactados
```

When Chrome/Edge captures a direct download, StorDown derives the filename, checks enabled rules in priority order, creates the destination folder if necessary, generates a collision-safe filename and then adds the transfer to the normal persistent queue.

If no rule matches, the normal Windows Downloads folder remains the fallback.


## Persistent download scheduler

HTTP/HTTPS downloads can now be scheduled directly from the desktop Download screen.

When a future date/time is selected, StorDown stores the transfer with status `scheduled` and its start timestamp in SQLite. The scheduler waits until that time before consuming a normal queue slot.

Scheduled HTTP downloads survive a StorDown restart:

```text
schedule -> SQLite -> close StorDown
                    -> open StorDown
                    -> restore scheduled jobs
                    -> wait until target time
                    -> queued
                    -> running
```

The Agendador screen lists pending scheduled downloads and allows cancellation before they start. Browser-captured downloads still start immediately unless a later browser scheduling workflow is added.


## Smart Multi-WAN balancing and failover

Segmented HTTP downloads no longer assign every Range worker to a fixed WAN for the entire file.

StorDown now breaks a large file into multiple scheduling waves while keeping the configured connection limit. Each completed range contributes a throughput sample for the interface that carried it. The adaptive scheduler uses an exponentially weighted throughput estimate, configured link weight, active-worker count and recent failures when deciding which WAN receives the next block.

```text
first wave
  WAN1 -> sample ~48 MB/s
  WAN2 -> sample ~27 MB/s

next waves
  WAN1 -> receives more pending blocks
  WAN2 -> remains active, but receives fewer blocks
```

If a segment request or stream fails on one interface, that link enters a short exponential cooldown and the unfinished Range is retried from the bytes already written using another healthy interface. A permanently bad path is probed less often while the remaining WAN continues carrying work.

The adaptive scheduler supports more than two enabled local interfaces; the practical number of usable Internet paths still depends on the router and policy-based routing configuration.


## Google Drive Smart Multi-WAN

Google Drive batch uploads now share the same adaptive link pool used by the download engine.

For a batch, StorDown limits the number of simultaneously active files, samples confirmed resumable chunks and continuously decides which local interface should carry the next chunk. Faster links receive more work while active-worker pressure prevents one WAN from monopolizing every file.

If a Drive chunk fails because one WAN disappears, StorDown:

```text
failed chunk
  -> marks that WAN unhealthy
  -> puts it in exponential cooldown
  -> queries the resumable session offset through another healthy WAN
  -> keeps already confirmed bytes
  -> retries the remaining chunk through another interface
```

A single Google Drive resumable file is still sequential at the provider protocol level, so switching WANs improves failover and adaptation but does not make one file upload through two WANs in parallel. Parallel striping of one large upload remains the job of the planned StorDown Relay.


## Authenticated browser downloads

The Chrome/Edge extension can now capture downloads that depend on an authenticated browser session without asking for blanket cookie access.

The user authorizes an individual website from the extension popup. Only then does the extension read cookies for that origin and attach a controlled `Cookie` header plus the download referrer to the Native Messaging request.

The desktop sanitizes incoming browser headers against an allowlist and keeps them in memory for the HTTP transfer rather than storing cookie values in the normal SQLite history.


## Speed limits and integrity verification

StorDown now supports an optional aggregate speed cap per transfer. The limit is entered in Mbps and is shared by all workers/WANs belonging to that transfer, so an 80 Mbps cap means approximately 80 Mbps total rather than 80 Mbps per interface.

HTTP downloads also accept an optional expected SHA256 value. After the final file is assembled, StorDown streams the file through SHA256 verification before marking the transfer complete. A mismatch fails the transfer and reports both expected and calculated hashes.

Scheduled downloads persist both the speed limit and expected SHA256 in SQLite, so those options survive a StorDown restart.


## Browser site policies and batch capture

The Chrome/Edge extension can override global automatic capture per origin with **inherit**, **always capture** or **never capture** policies.

It can also scan the active page and submit up to 250 unique HTTP/HTTPS links to StorDown, using a small bounded concurrency so the Native Messaging bridge is not flooded. Each accepted URL still goes through the normal Windows NIC detection, category rules, persistent queue and Multi-WAN download engine.


## Chrome / Edge integration installer

The desktop Settings screen can install the StorDown Native Messaging bridge for the current Windows user. The user pastes the extension ID once, and StorDown copies the native-host executable to LocalAppData, creates the host manifest and registers both Chrome and Edge HKCU NativeMessagingHosts entries.

The repository PowerShell installer supports the same flow plus browser selection and uninstall. A future published/signed extension package will remove the remaining development step of manually loading the extension and copying its ID.


## Google Drive destination browser

The Upload workspace no longer requires users to manually copy a Google Drive folder ID. Once the account is connected, **Escolher pasta…** opens a native StorDown folder browser that supports:

- My Drive;
- Shared Drives;
- nested folder navigation;
- selecting the current folder as the upload destination.

Older OAuth sessions need to reconnect once because native Drive downloads add the restricted `drive.readonly` content scope. Public distribution will require Google's corresponding OAuth verification.


## Google Drive Multi-WAN downloads

The **Cloud** workspace now lists files as well as folders from My Drive and Shared Drives. For normal binary/blob files, **Baixar** opens the Windows destination picker and places the transfer into the StorDown queue as a Google Drive job.

Drive blob content is requested with `files.get?alt=media`. Google supports HTTP byte ranges on these downloads, so StorDown can use the same segmented Smart Multi-WAN engine as a normal HTTP source:

```text
Google Drive file
      |
      +-- Range 0 -> WAN1
      +-- Range 1 -> WAN2
      +-- Range 2 -> adaptive scheduler
      +-- failed Range -> healthy WAN
      |
      -> assembled Windows file
```

Google Workspace-native documents are shown in the browser but direct Range download is disabled until the separate export workflow is implemented.


## Google Workspace exports

Native Google Workspace documents appear in the Cloud browser with **Exportar** instead of the normal Range-download button.

StorDown presents compatible target formats, for example DOCX/PDF for Docs, XLSX/PDF/CSV for Sheets, and PPTX/PDF for Slides. The selected export is queued with pause/cancel telemetry and the normal Windows destination picker.

Workspace export responses do not support HTTP Range, so these transfers use one direct connection rather than pretending they can aggregate WAN1 + WAN2. The standard Drive `files.export` endpoint is also limited to 10 MB of exported content; Google Vids requires a separate long-running download API.


## Google Drive upload restart recovery

Drive upload batches now persist restart-safe checkpoints. StorDown stores per-file offsets and metadata in SQLite, while the sensitive Google resumable-session URI is kept in the Windows credential store rather than the database.

After a crash, app restart or Windows reboot, an upload that was active becomes **Interrompido**. Once the Google Drive account is connected/restored, **Retomar upload** reopens the saved session, verifies the server-side offset and continues only the unfinished files/bytes.

If Google has expired the saved resumable session, StorDown transparently creates a new session for that file. Completed files in the same batch remain skipped.


## Google Drive shared links

Paste a Drive/Docs/Sheets/Slides share URL into the Cloud workspace and StorDown resolves the item directly through the Drive API.

Blob files can be sent to the Smart Multi-WAN Range downloader, Workspace-native documents can be exported, and shared folders can be opened in the Cloud browser. Link `resourcekey` values are preserved and sent through the Google Drive resource-key header when required.
