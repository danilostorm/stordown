# StorDown Browser Extension

Chrome/Edge Manifest V3 integration for sending browser downloads to the StorDown desktop queue.

## Current features

- **Baixar com StorDown** in link context menus.
- Popup with desktop connection test.
- Optional automatic capture for direct HTTP/HTTPS browser downloads.
- Optional **Sites autenticados** mode that asks for cookie/site access only when the user enables it.
- Cookie, Referer and browser User-Agent handoff for captured authenticated downloads.
- Native Messaging bridge through `cloud.hoststorm.stordown`.
- Browser download is cancelled only after the StorDown desktop confirms the transfer was accepted into its queue.

Authenticated capture is opt-in. The extension does not request cookie/site access unless **Sites autenticados** is enabled. Captured authentication headers are forwarded in memory and are not written into StorDown's SQLite history.

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

The desktop capture listener binds only to loopback. It automatically uses the active physical Windows NIC IPv4 addresses and saves captured browser downloads in the user's Downloads folder with collision-safe filenames.

## Authenticated capture

When **Sites autenticados** is enabled from the extension popup, Chrome/Edge prompts for the optional `cookies` permission and HTTP/HTTPS site access. The permission can be removed again by turning the option off.

For a captured URL, the extension may send:

- `Cookie` from the browser cookie jar for that URL;
- `Referer` when the browser exposes one;
- the browser `User-Agent`.

The desktop side accepts only a small header whitelist and size limits. These headers are passed directly to the in-memory HTTP transfer request and are **not persisted** in SQLite.

## Next browser milestone

- per-site capture rules and exclusions;
- packaged extension/native-host installer;
- download-all-links support.
