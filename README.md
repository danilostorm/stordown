# StorDown

StorDown is a modern Windows download manager built around a **multi-link download engine**.

The goal is to combine ideas from classic download managers with explicit use of multiple local network interfaces/WAN paths, browser capture, cloud providers and a modern desktop UI.

## Current status

The repository now contains the first V0.1 bootstrap:

- Rust download engine
- HTTP/HTTPS downloads
- HTTP Range segmentation
- Per-worker bind to a local IPv4/IPv6 address
- Multiple enabled links with weights
- CLI proof of concept
- Tauri + React desktop shell
- Chrome/Edge Manifest V3 extension skeleton
- UDM Pro multi-WAN setup notes
- Windows CI for the Rust workspace

This is still an early proof of concept, not a production release.

## How StorDown uses two WANs

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

## CLI proof of concept

Install Rust, clone the repository and test with a large direct URL:

```powershell
cargo run -p stordown-cli -- download "https://example.com/large-file.iso" `
  --output "C:\Downloads\large-file.iso" `
  --connections 8 `
  --bind 192.168.30.101 `
  --bind 192.168.30.102
```

Replace the two IPs with the actual IPv4 addresses assigned to the two Ethernet adapters.

While it runs, check the UDM WAN graphs. With policy-based routes configured correctly, the HTTP workers should appear on WAN1 and WAN2 at the same time.

## Desktop development

```powershell
cd apps\desktop
npm install
npm run tauri dev
```

The first UI already accepts URL, destination, number of connections and the local bind IPs. Progress events, queue persistence and automatic NIC discovery are next.

## Browser extension

The `browser-extension` folder contains the first Chrome/Edge Manifest V3 integration. It adds **Baixar com StorDown** to link context menus and is prepared to communicate with a Windows native-messaging host.

Native-host installation and authenticated browser handoff are planned for the next browser milestone.

## Google Drive

Google Drive is planned as a first-class provider rather than being treated only as a captured browser URL. The design includes OAuth, shared links, byte-range downloads and later Shared Drive support.

Rclone remains useful as an optional compatibility adapter for additional cloud remotes.

## Project layout

```text
apps/
  cli/                 CLI proof of concept
  desktop/             Tauri + React desktop app
crates/
  stordown-core/       Native Rust download engine
browser-extension/     Chrome/Edge integration
docs/
  ARCHITECTURE.md
  ROADMAP.md
  UDM_MULTI_WAN.md
```

See [docs/ROADMAP.md](docs/ROADMAP.md) for planned milestones and [docs/UDM_MULTI_WAN.md](docs/UDM_MULTI_WAN.md) for the initial UDM setup.
