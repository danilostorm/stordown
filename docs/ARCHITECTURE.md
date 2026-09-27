# StorDown Architecture

StorDown is a Windows transfer manager with a native multi-link engine. The router is responsible for deterministic WAN policy; StorDown is responsible for opening the right connections on the right local source IPs.

## Main components

```text
Browser Extension
      |
      | Native Messaging
      v
StorDown Desktop (Tauri + React)
      |
      v
stordown-core (Rust)
      |
      +-- HTTP/HTTPS downloads
      +-- HTTP Range segmentation
      +-- Google Drive resumable upload
      +-- per-link socket binding
      +-- queue / scheduler
      +-- cloud provider adapters
      |
      +----------+----------+
      |                     |
      v                     v
Local IP / NIC 1       Local IP / NIC 2
      |                     |
      v                     v
UDM PBR -> WAN1        UDM PBR -> WAN2
```

## Download multi-link model

Each HTTP segment is assigned to a configured link. A link is identified by a local source IP.

The Rust HTTP client binds to that IP. The UDM sees the source addresses separately and policy-based routing can send each one through a different WAN.

```text
Segment 0 -> 192.168.30.101 -> WAN1
Segment 1 -> 192.168.30.102 -> WAN2
Segment 2 -> 192.168.30.101 -> WAN1
Segment 3 -> 192.168.30.102 -> WAN2
```

This can aggregate two WANs for one large download when the origin supports byte ranges.

## Upload multi-link model

Cloud upload protocols are not always symmetrical with downloads.

For Google Drive, one resumable upload session advances sequentially through byte ranges. StorDown therefore uses two levels:

```text
Multiple files
  file A -> NIC1 -> WAN1 -> Drive session A
  file B -> NIC2 -> WAN2 -> Drive session B
  file C -> NIC1 -> WAN1 -> Drive session C
```

This aggregates WAN bandwidth across a batch immediately.

For one large file to consume two WANs simultaneously, the planned **StorDown Relay** mode uses independent paths from the PC to a relay and a single reconstructed stream from the relay to the cloud provider.

```text
One large file
      |
   +--+--+
   |     |
 WAN1   WAN2
   \     /
 StorDown Relay
      |
 Google Drive
```

Direct mode remains the default and does not require a relay.

## Fallback behavior

If an HTTP origin does not support `Range: bytes=...`, StorDown falls back to a single connection for that file.

Google Drive uploads use resumable chunks and retry/query the resumable session after ambiguous network failures.

## Cloud providers

Google Drive is the first first-class provider:

- resumable uploads;
- multiple-file WAN distribution;
- Shared Drive-compatible session creation;
- OAuth desktop flow planned next;
- Drive downloads and shared links planned next.

Rclone remains useful as an optional compatibility layer for additional remotes.

## Security boundary

Browser authentication material should only be transferred when necessary and scoped to the requested origin.

Google refresh tokens must be stored with Windows-protected credential storage. The current development-only access-token input will be removed from the normal interface after OAuth is wired.
