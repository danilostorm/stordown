# StorDown Roadmap

## V0.1 - Multi-link proof of concept

- [x] Rust workspace
- [x] HTTP/HTTPS engine
- [x] HTTP Range segmentation
- [x] Bind workers to multiple local IPs
- [x] Weighted link model
- [x] CLI proof of concept
- [x] Initial Tauri/React desktop UI
- [x] Browser extension skeleton
- [x] Google Drive resumable upload engine
- [x] Batch Google Drive uploads distributed across multiple NICs
- [x] Upload workspace in the desktop UI
- [ ] Live transfer progress events
- [ ] Pause/resume metadata persisted to disk
- [ ] Retry individual failed download segments
- [ ] Automatic NIC discovery on Windows
- [ ] Per-link throughput measurement

## V0.2 - Desktop transfer manager

- [ ] Unified download/upload queue
- [ ] Categories and destination rules
- [ ] Persistent history
- [ ] Scheduler
- [ ] Speed limits
- [ ] Per-transfer connection controls
- [ ] Smart weighted balancing
- [ ] Failover when one link disappears
- [ ] File integrity verification
- [ ] Windows file picker integration

## V0.3 - Browser integration

- [ ] Windows native-messaging host
- [ ] One-click Chrome/Edge installer
- [ ] Automatic download interception
- [ ] Controlled cookie/header handoff
- [ ] Download all links
- [ ] Site-specific capture rules

## V0.4 - Google Drive

- [ ] OAuth desktop login
- [ ] Secure refresh-token storage
- [ ] Drive browser / destination picker
- [ ] Shared Drive browsing
- [ ] Shared links
- [ ] Range-aware Drive downloads
- [ ] Persist and resume Drive upload sessions after restart
- [ ] Rclone remote adapter

## V0.5 - Multi-WAN upload acceleration

- [ ] Smart per-file assignment across WANs
- [ ] Per-WAN upload telemetry
- [ ] Upload failover and reassignment
- [ ] StorDown Relay protocol
- [ ] One large upload striped across two or more WANs through Relay
- [ ] Optional self-hosted Relay on VPS/Unraid

## Later

- [ ] OneDrive / Dropbox adapters
- [ ] Remote StorDown agent
- [ ] Send downloads to another machine
- [ ] Linux build
- [ ] Firefox extension
- [ ] 3+ simultaneous links
- [ ] Optional QUIC/MPTCP relay transport
