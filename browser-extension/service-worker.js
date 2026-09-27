const HOST = "cloud.hoststorm.stordown";

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "stordown-download-link",
      title: "Baixar com StorDown",
      contexts: ["link"],
    });
  });

  chrome.storage.local.get(["autoCapture", "authorizedOrigins"]).then((settings) => {
    const updates = {};

    if (settings.autoCapture === undefined) {
      updates.autoCapture = false;
    }

    if (!Array.isArray(settings.authorizedOrigins)) {
      updates.authorizedOrigins = [];
    }

    if (Object.keys(updates).length > 0) {
      chrome.storage.local.set(updates);
    }
  });
});

function sendToStorDown(payload) {
  return new Promise((resolve) => {
    chrome.runtime.sendNativeMessage(HOST, payload, (response) => {
      if (chrome.runtime.lastError) {
        resolve({
          ok: false,
          error: chrome.runtime.lastError.message,
        });
        return;
      }

      resolve(response || { ok: false, error: "StorDown não respondeu." });
    });
  });
}

async function flashBadge(text) {
  await chrome.action.setBadgeText({ text });
  setTimeout(() => chrome.action.setBadgeText({ text: "" }), 1800);
}

function permissionPattern(url) {
  try {
    const parsed = new URL(url);
    if (!["http:", "https:"].includes(parsed.protocol)) return null;
    return `${parsed.origin}/*`;
  } catch {
    return null;
  }
}

async function buildAuthHeaders(url, referrer) {
  const pattern = permissionPattern(url);
  if (!pattern) return {};

  const origin = new URL(url).origin;
  const { authorizedOrigins = [] } = await chrome.storage.local.get("authorizedOrigins");

  if (!authorizedOrigins.includes(origin)) {
    return {};
  }

  const granted = await chrome.permissions.contains({
    permissions: ["cookies"],
    origins: [pattern],
  });

  if (!granted) {
    return {};
  }

  const headers = {};
  const cookies = await chrome.cookies.getAll({ url });

  if (cookies.length > 0) {
    headers.Cookie = cookies
      .map((cookie) => `${cookie.name}=${cookie.value}`)
      .join("; ");
  }

  if (referrer && /^https?:\/\//i.test(referrer)) {
    headers.Referer = referrer;
  }

  return headers;
}

chrome.contextMenus.onClicked.addListener(async (info) => {
  if (info.menuItemId !== "stordown-download-link" || !info.linkUrl) {
    return;
  }

  const headers = await buildAuthHeaders(info.linkUrl, info.pageUrl || null);
  const response = await sendToStorDown({
    type: "download",
    url: info.linkUrl,
    source: "context-menu",
    headers,
  });

  await flashBadge(response.ok ? "✓" : "!");
});

chrome.downloads.onCreated.addListener(async (item) => {
  const { autoCapture = false } = await chrome.storage.local.get("autoCapture");

  if (!autoCapture || !item.url || !/^https?:\/\//i.test(item.url)) {
    return;
  }

  const headers = await buildAuthHeaders(item.url, item.referrer || null);
  const response = await sendToStorDown({
    type: "download",
    url: item.url,
    filename: item.filename || null,
    source: "browser-download",
    headers,
  });

  if (!response.ok) {
    await flashBadge("!");
    return;
  }

  // StorDown only acknowledges after the transfer was accepted into its queue.
  // Cancel the browser copy after that ACK so the same file is not downloaded twice.
  try {
    await chrome.downloads.cancel(item.id);
    await chrome.downloads.erase({ id: item.id });
  } catch (error) {
    console.warn("StorDown captured the download, but Chrome cleanup failed:", error);
  }

  await flashBadge("✓");
});

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type === "stordown-ping") {
    sendToStorDown({ type: "ping" }).then(sendResponse);
    return true;
  }

  if (message?.type === "stordown-capture") {
    sendToStorDown(message.payload).then(sendResponse);
    return true;
  }

  return false;
});
