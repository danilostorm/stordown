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

StorDown currently requests `https://www.googleapis.com/auth/drive.file`, keeping access narrower than full Drive-wide authorization. Broader scopes should only be introduced for features that truly require them.

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
