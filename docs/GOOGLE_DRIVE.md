# Google Drive in StorDown

StorDown treats Google Drive as a first-class provider for both downloads and uploads.

## Upload path

Large files use the Google Drive API resumable upload flow:

1. Create a resumable upload session.
2. Store the session URI.
3. Send file chunks with `Content-Range`.
4. Retry interrupted chunks.
5. Query the session state before resuming after ambiguous failures.
6. Persist the session URI in a later milestone so an application restart can resume it.

Google requires non-final resumable chunks to use sizes that are multiples of 256 KiB. StorDown currently defaults to 8 MiB.

## Authentication

The desktop application will use OAuth 2.0 with a Google **Desktop app** client.

The intended production UX is:

```text
StorDown
  -> Conectar Google Drive
  -> browser consent
  -> callback to StorDown
  -> refresh token stored with Windows-protected storage
```

During development, the backend upload command accepts an access token directly. This is temporary and will be removed from the normal UI once OAuth is wired.

StorDown should request the narrowest Drive scope that supports the selected workflow. Broader Drive-wide access should only be added if a feature actually requires it.

## Multi-WAN behavior

Google Drive resumable upload is sequential for a single file: a resumable session advances through one byte range after another.

That means:

- **Multiple files:** StorDown can upload different files simultaneously through different NICs/WANs and aggregate bandwidth.
- **One large file:** alternating chunks between WANs can provide failover/path control, but does not make the chunks simultaneous.
- **One large file using two WANs at the same time:** requires an optional relay/bonding mode. StorDown will send independent pieces over both WANs to a StorDown Relay, and the relay will reconstruct/stream the final object to Google Drive.

Planned architecture:

```text
Single 100 GB file
       |
       +-- WAN1 --\
       |           > StorDown Relay -> one resumable Drive upload
       +-- WAN2 --/
```

The relay is optional. Normal Drive uploads work directly from the desktop without it.

## Shared Drives

The resumable session endpoint is created with `supportsAllDrives=true`. Shared Drive destination selection and browsing will be added with the OAuth/provider UI.
