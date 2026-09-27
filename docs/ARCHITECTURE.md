# StorDown Architecture

StorDown is designed as a download manager with a native multi-link engine instead of relying on generic router load balancing.

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
      +-- HTTP/HTTPS probe
      +-- HTTP Range segmentation
      +-- retry/resume engine
      +-- per-link socket binding
      +-- scheduler
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

## Multi-link model

Each HTTP segment is assigned to a configured link. A link is identified by a local source IP.

The Rust HTTP client is created with a local bind address. The UDM sees the two source IPs as separate clients and policy-based routing can send each one through a different WAN.

Example:

```text
Segment 0 -> 192.168.30.101 -> WAN1
Segment 1 -> 192.168.30.102 -> WAN2
Segment 2 -> 192.168.30.101 -> WAN1
Segment 3 -> 192.168.30.102 -> WAN2
```

This works for a single large file when the origin supports byte-range requests.

## Fallback behavior

If the origin does not support `Range: bytes=...`, StorDown falls back to a single connection for that file. Multiple independent files can still be distributed across links in a later scheduler milestone.

## Cloud providers

Google Drive will be implemented as a first-class provider using Drive APIs and OAuth. Rclone integration remains useful as an optional compatibility layer for additional remotes.

## Security boundary

Browser authentication material should only be transferred when necessary and should be scoped to the requested origin. StorDown should never persist session cookies unless the user explicitly enables it.
