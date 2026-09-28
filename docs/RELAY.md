# StorDown Relay v1

StorDown Relay is the cooperating remote endpoint used when the destination protocol itself cannot accept parallel pieces of one file.

A normal Google Drive resumable upload is sequential. Relay changes the first leg:

```text
Windows PC
  +-- chunk 0 -> NIC1 -> UDM policy -> WAN1 --+
  +-- chunk 1 -> NIC2 -> UDM policy -> WAN2 --+--> StorDown Relay
  +-- chunk 2 -> fastest healthy link --------+       |
                                                       +-> assembled file
                                                       +-> destination adapter (next milestone)
```

The client uses independent HTTP PUT requests for different file chunks, so the UDM can route each local source IP through a different WAN. This is real concurrent striping to the Relay rather than ordinary per-flow load balancing.

## Protocol v1

The HTTP API is intentionally small:

```text
GET  /health
POST /v1/sessions
GET  /v1/sessions/<id>
PUT  /v1/sessions/<id>/chunks/<index>
POST /v1/sessions/<id>/complete
```

A session records the filename, total size, chunk size and optional final SHA256. Chunks are independently idempotent and may arrive out of order.

Each chunk may include:

```text
X-StorDown-Chunk-SHA256: <hex digest>
```

The Relay verifies the digest before atomically committing the chunk.

When `complete` is called, the server verifies that every expected chunk exists with the expected size, assembles them in order, calculates the final SHA256 and atomically publishes the assembled file.

Session metadata and chunks live on disk, so a Relay restart does not lose upload state.

## Authentication

Set a long random token:

```text
STORDOWN_RELAY_TOKEN=<secret>
```

Clients send it as a Bearer token. Do not expose an unauthenticated Relay to the public Internet.

For Internet deployment, put the Relay behind HTTPS (for example Caddy, Traefik, Nginx or Cloudflare Tunnel). The v1 service itself listens on plain HTTP so TLS termination can remain with the user's reverse proxy.

## Docker / Unraid

Build from the repository root:

```bash
docker build -f apps/relay/Dockerfile -t stordown-relay .
```

Run:

```bash
docker run -d \
  --name stordown-relay \
  -p 17834:17834 \
  -e STORDOWN_RELAY_TOKEN='change-me' \
  -v /mnt/user/appdata/stordown-relay:/data \
  --restart unless-stopped \
  stordown-relay
```

Useful environment variables:

```text
STORDOWN_RELAY_BIND=0.0.0.0:17834
STORDOWN_RELAY_DATA=/data
STORDOWN_RELAY_TOKEN=<secret>
```

## CLI proof of concept

With two Windows NIC addresses that the UDM policy-routes to different WANs:

```powershell
$env:STORDOWN_RELAY_TOKEN="secret"
cargo run -p stordown-cli -- upload-relay --file "D:\\Uploads\\movie.mkv" --relay-url "https://relay.example.com" --bind 192.168.30.101 --bind 192.168.30.102 --chunk-mib 16
```

StorDown creates several concurrent chunk requests. The adaptive link pool samples throughput and failures, assigns new work to the healthier/faster path, and retries a failed chunk through another link.

The CLI prints the Relay session ID. Supplying that ID again with `--session-id` asks the Relay which chunks already exist and uploads only the missing pieces.

## Current boundary

Relay v1 proves and implements the multi-WAN transport and restart-safe remote staging layer. It does not yet push the assembled file from the Relay into Google Drive or another final cloud destination.

That next layer will be a destination-adapter pipeline, starting with Google Drive. This distinction matters because it avoids claiming that a staged Relay upload has already reached the user's final cloud destination.
