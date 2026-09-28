# Google Drive in StorDown

StorDown treats Google Drive as a first-class provider for both downloads and uploads.

## Upload path

Large files use the Google Drive API resumable upload flow:

1. Create a resumable upload session.
2. Store the session URI.
3. Send file chunks with `Content-Range`.
4. Retry interrupted chunks.
5. Query the session state before resuming after ambiguous failures.
6. Persist the session checkpoint so an application restart can resume it.

Google requires non-final resumable chunks to use sizes that are multiples of 256 KiB. StorDown currently defaults to 8 MiB.

## Authentication

The Windows desktop application now implements the installed-app OAuth flow with **PKCE** and a local loopback callback.

```text
StorDown
  -> Conectar Google Drive
  -> default system browser
  -> Google consent
  -> http://127.0.0.1:<random-port>
  -> authorization code + state validation
  -> token exchange with PKCE verifier
  -> refresh token stored in Windows secure credential storage
```

The normal desktop upload flow no longer requires users to paste an access token. StorDown refreshes an expired access token from the saved refresh token.

For development, the Google Desktop OAuth client ID can be supplied in the UI or through `STORDOWN_GOOGLE_CLIENT_ID`. The distributed application will ship with the project's public client ID configured. Installed applications cannot treat a client secret as confidential, so the flow does not depend on embedding one.

StorDown requests `https://www.googleapis.com/auth/drive.file` for files it creates/manages and `https://www.googleapis.com/auth/drive.readonly` for the native Cloud browser and Drive downloads. The read-only scope grants content access across the user's Drive and is a **restricted** Google scope, so a public StorDown distribution will need the appropriate Google OAuth verification before broad release.

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


## Folder browser and Shared Drives

StorDown now has a native destination browser inside the Windows desktop UI. After the Google account is connected, the upload screen can list folders from **My Drive** and the user's **Shared Drives**, navigate nested folders and place the selected folder ID directly into the resumable upload request.

The browser uses Drive metadata only; StorDown does not download file contents just to render the picker. The OAuth flow requests both:

```text
https://www.googleapis.com/auth/drive.file
https://www.googleapis.com/auth/drive.readonly
```

Existing sessions created before Drive content download was added must be disconnected and authorized again so Google can grant the new read-only content scope.

Shared-drive folder listing uses `supportsAllDrives=true`, `includeItemsFromAllDrives=true`, and the selected shared-drive ID. Upload creation already uses `supportsAllDrives=true`, so choosing a writable folder in a Shared Drive feeds directly into the existing upload engine.


## Range-aware Drive downloads

The desktop **Cloud** workspace can browse files in My Drive and Shared Drives and enqueue regular Drive blob files into the same segmented downloader used for HTTP/HTTPS.

The content endpoint is:

```text
GET https://www.googleapis.com/drive/v3/files/<fileId>?alt=media
Authorization: Bearer <access token>
Range: bytes=<start>-<end>
```

Google Drive supports byte-range partial downloads for blob files. StorDown therefore reuses its existing adaptive Range engine:

```text
Drive blob file
   |
   +-- Range A -> NIC1 -> WAN1
   +-- Range B -> NIC2 -> WAN2
   +-- Range C -> fastest healthy link
   +-- failed range -> retry from saved bytes on another link
```

This includes Smart Multi-WAN scheduling, per-link telemetry, pause/resume, partial-part persistence and automatic segment failover.

StorDown checks the Drive `capabilities.canDownload` metadata before enabling the Download button.

Google Workspace-native files (Docs, Sheets, Slides and similar) are intentionally not sent through the Range engine because Drive export operations do not support partial Range downloads. Export support remains a separate roadmap item.

The current Drive download job receives an access token when it is queued. Extremely long transfers that need to open new Drive requests after that access token expires may need token-refresh integration in the active worker; this is a follow-up hardening item.


## Google Workspace export

The Cloud workspace can now export native Google Workspace files that cannot use byte-range downloads.

Supported defaults and selectable formats include:

- Google Docs -> DOCX, PDF, TXT, Markdown, EPUB;
- Google Sheets -> XLSX, PDF, CSV (first sheet);
- Google Slides -> PPTX, PDF, TXT;
- Google Drawings -> PDF, PNG, JPEG, SVG;
- Apps Script -> JSON.

The desktop queries the supported StorDown export choices for the source MIME type, asks for a Windows destination path, then queues the export as a normal Google Drive transfer.

Export requests use:

```text
GET https://www.googleapis.com/drive/v3/files/<fileId>/export?mimeType=<target>
Authorization: Bearer <access token>
```

Unlike blob downloads, Google Workspace export does **not** support HTTP Range. StorDown therefore deliberately uses a direct single-request transfer path for exports instead of performing a wasteful Range probe.

The classic `files.export` endpoint currently has a 10 MB exported-content limit. Google Vids is also not handled by this path; it requires the newer long-running `files.download` flow and remains future work.


## Persistent resumable-upload sessions

StorDown now persists enough metadata for an interrupted Google Drive upload to continue after the application or Windows restarts.

For every local file in a Drive upload batch, SQLite stores only non-secret resume metadata:

- transfer ID and local source path;
- parent folder ID;
- remote name and MIME type;
- total size and resumable chunk size;
- last confirmed byte offset;
- completed state;
- a credential reference key.

The resumable **session URI itself is not stored in SQLite**. It is stored in the Windows credential store under a per-file credential key because Google treats the resumable session URI as sensitive.

Checkpoint flow:

```text
Google confirms chunk
        |
        +-> confirmed offset -> SQLite
        |
        +-> resumable session URI -> Windows Credential Manager
```

On restart, StorDown marks an in-flight upload as `interrupted`. After the Google account is restored, the queue exposes **Retomar upload**. StorDown loads the saved session URI from Windows Credential Manager, asks Google for the authoritative current offset, and continues from that byte.

If the saved resumable session has expired or is no longer valid, StorDown creates a fresh resumable session for that file instead of failing the entire batch. Files already marked complete are skipped.

Completed session credentials are removed from Windows Credential Manager. Removing transfer history also cleans up any remaining upload-session credentials.
