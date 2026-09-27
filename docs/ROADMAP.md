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
- [ ] Live progress events
- [ ] Pause/resume metadata
- [ ] Retry individual failed segments
- [ ] Automatic NIC discovery on Windows
- [ ] Per-link throughput measurement

## V0.2 - Desktop download manager

- [ ] Download queue
- [ ] Categories and destination rules
- [ ] Persistent history
- [ ] Scheduler
- [ ] Speed limits
- [ ] Per-download connection controls
- [ ] Smart weighted balancing
- [ ] Failover when one link disappears
- [ ] File integrity verification

## V0.3 - Browser integration

- [ ] Windows native-messaging host
- [ ] One-click Chrome/Edge installer
- [ ] Automatic download interception
- [ ] Controlled cookie/header handoff
- [ ] Download all links
- [ ] Site-specific capture rules

## V0.4 - Cloud

- [ ] Google Drive OAuth
- [ ] Shared Drive / shared links
- [ ] Range-aware Drive downloads
- [ ] Rclone remote adapter
- [ ] OneDrive / Dropbox adapters

## Later

- [ ] Remote StorDown agent
- [ ] Send downloads to another machine
- [ ] Multi-WAN upload engine
- [ ] Linux build
- [ ] Firefox extension
- [ ] 3+ simultaneous links
- [ ] Optional QUIC/MPTCP relay mode for non-range traffic
