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
- multiple-file Drive upload distribution across NICs/WANs;
- CLI proof of concept for downloads and Drive uploads;
- Tauri + React desktop interface with Download and Upload workspaces;
- Chrome/Edge Manifest V3 extension skeleton;
- UDM Pro multi-WAN setup notes;
- automatic discovery of active physical Windows NICs;
- per-interface public-IP route tests to verify UDM WAN policies;
- live transfer progress and real-time speed per local interface/WAN;
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

OAuth login inside StorDown is the next Drive milestone, so users will not need to handle access tokens manually.

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
