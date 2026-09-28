# StorDown Browser Extension

Chrome/Edge Manifest V3 integration for sending browser downloads to the StorDown desktop queue.

## Current features

- **Baixar com StorDown** in link context menus.
- Popup with desktop connection test.
- Optional automatic capture for direct HTTP/HTTPS browser downloads.
- Native Messaging bridge through `cloud.hoststorm.stordown`.
- Browser download is cancelled only after the StorDown desktop confirms the transfer was accepted into its queue.
- **Per-site authenticated download support** with explicit cookie permission.
- Per-site automatic-capture policies: inherit, always capture or never capture.
- **Baixar todos os links** from the current page, with up to 250 unique HTTP/HTTPS links per batch.

## Authenticated downloads

StorDown does not request access to browser cookies for every website by default.

Open the StorDown popup while you are on a site that requires login and click **Autorizar site**. Chrome/Edge then asks for the optional `cookies` permission and host access only for that origin.

When StorDown captures a download from an authorized origin:

```text
authorized site
  -> browser cookie jar
  -> Cookie header + Referer
  -> Native Messaging
  -> StorDown Desktop
  -> in-memory HTTP request headers
  -> Multi-WAN download
```

The desktop accepts only a small header allowlist and rejects CR/LF injection. Browser authentication headers are used in memory for the captured transfer and are not written into the normal SQLite transfer-history record.

If the site is not authorized, capture still works but no cookies are sent to StorDown.

## Development install

1. Build the native host:

```powershell
cargo build -p stordown-native-host --release
```

2. In Chrome or Edge, enable Developer Mode and load the `browser-extension` folder as an unpacked extension.
3. Copy the extension ID shown by the browser.
4. Register the native host:

```powershell
powershell -ExecutionPolicy Bypass -File .\browser-extension\install-windows.ps1 -ExtensionId "YOUR_EXTENSION_ID"
```

The installer registers the same host for the current Windows user in both Chrome and Microsoft Edge.

5. Start StorDown Desktop.
6. Open the extension popup and click **Testar**. It should show **Conectado**.

## Capture path

```text
Chrome / Edge
    |
    | Native Messaging framing
    v
stordown-native-host.exe
    |
    | localhost 127.0.0.1:17832
    v
StorDown Desktop
    |
    v
Persistent queue -> Multi-WAN engine
```

The desktop capture listener binds only to loopback. It automatically uses the active physical Windows NIC IPv4 addresses and applies configured category/destination rules.

## Next browser milestone

- packaged extension/native-host installer;
- batch link review/filter UI before enqueue;
- richer per-site filename/capture filters.


## Site-specific capture policies

The global automatic-capture switch can now be overridden for the active origin:

- **Seguir configuração global** keeps the global behavior.
- **Sempre capturar neste site** intercepts HTTP/HTTPS downloads for that site even when global capture is off.
- **Nunca capturar neste site** leaves browser downloads from that site alone.

Policies are stored locally in the extension and do not grant cookie access by themselves.

## Download all links

The popup and page context menu can scan the active page for unique HTTP/HTTPS anchors and send them to StorDown in a bounded batch.

The current implementation accepts up to 250 links and uses four concurrent Native Messaging submissions. Category/destination rules are applied independently by StorDown Desktop for every accepted link.
