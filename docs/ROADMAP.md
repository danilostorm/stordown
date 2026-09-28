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
- [x] Per-transfer aggregate speed limits
- [x] Pause / resume / cancel controls for active transfers
- [x] Smart adaptive balancing for segmented HTTP downloads
- [x] Automatic HTTP segment failover when one link disappears
- [x] Optional SHA256 download integrity verification
- [x] Windows file picker integration

## V0.3 - Browser integration

- [x] Windows native-messaging host
- [x] One-click Chrome/Edge Native Messaging registration from desktop
- [ ] Published/signed browser extension package for store-style installation
- [x] Automatic interception for direct HTTP/HTTPS downloads
- [x] Controlled per-site cookie/header handoff
- [x] Download all links from the active page
- [x] Site-specific capture/ignore policies

## V0.4 - Google Drive

- [x] OAuth desktop login with PKCE + loopback callback
- [x] Secure refresh-token storage using the Windows credential store
- [x] Drive browser / destination picker
- [x] Shared Drive browsing
- [x] Shared links / public Drive link import
- [x] Range-aware blob-file Drive downloads with Smart Multi-WAN
- [x] Persist and resume Drive upload sessions after restart
- [x] Google Workspace document export (Docs/Sheets/Slides/Drawings/Apps Script)
- [ ] Rclone remote adapter

## V0.5 - Multi-WAN upload acceleration

- [x] Smart per-file/chunk assignment across WANs for Drive batches
- [x] Per-WAN upload telemetry for direct Drive batches
- [x] Drive upload failover and chunk reassignment between WANs
- [x] StorDown Relay v1 protocol + restart-safe chunk staging
- [x] One large file striped concurrently across two or more WANs to Relay staging
- [x] Optional self-hosted Relay service for VPS/Unraid
- [ ] Relay destination adapter: Google Drive
- [ ] Desktop Relay workspace + persistent Relay session resume

## Later

- [ ] OneDrive / Dropbox adapters
- [ ] Remote StorDown agent
- [ ] Send downloads to another machine
- [ ] Linux build
- [ ] Firefox extension
- [ ] 3+ simultaneous links
- [ ] Optional QUIC/MPTCP relay transport
