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
- [x] Live transfer progress events
- [x] Segmented HTTP resume metadata persisted to disk
- [x] Retry and resume individual failed HTTP segments
- [x] Automatic physical NIC discovery on Windows
- [x] Per-link WAN/public-IP route verification
- [x] Live per-link throughput measurement during transfers

## V0.2 - Desktop transfer manager

- [x] Unified download/upload queue
- [x] Categories and destination rules for browser-captured downloads
- [x] Persistent SQLite history
- [x] Persistent HTTP download scheduler
- [ ] Speed limits
- [x] Pause / resume / cancel controls for active transfers
- [x] Smart adaptive balancing for segmented HTTP downloads
- [x] Automatic HTTP segment failover when one link disappears
- [ ] File integrity verification
- [x] Windows file picker integration

## V0.3 - Browser integration

- [x] Windows native-messaging host
- [ ] One-click Chrome/Edge installer
- [x] Automatic interception for direct HTTP/HTTPS downloads
- [ ] Controlled cookie/header handoff
- [ ] Download all links
- [ ] Site-specific capture rules

## V0.4 - Google Drive

- [x] OAuth desktop login with PKCE + loopback callback
- [x] Secure refresh-token storage using the Windows credential store
- [ ] Drive browser / destination picker
- [ ] Shared Drive browsing
- [ ] Shared links
- [ ] Range-aware Drive downloads
- [ ] Persist and resume Drive upload sessions after restart
- [ ] Rclone remote adapter

## V0.5 - Multi-WAN upload acceleration

- [ ] Smart per-file assignment across WANs
- [x] Per-WAN upload telemetry for direct Drive batches
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
