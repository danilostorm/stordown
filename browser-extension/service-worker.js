const HOST = "cloud.hoststorm.stordown";

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "stordown-download-link",
      title: "Baixar com StorDown",
      contexts: ["link"],
    });
  });

  chrome.storage.local.get(["autoCapture", "authenticatedCapture"]).then((settings) => {
    const defaults = {};
    if (settings.autoCapture === undefined) defaults.autoCapture = false;
    if (settings.authenticatedCapture === undefined) defaults.authenticatedCapture = false;
    if (Object.keys(defaults).length > 0) chrome.storage.local.set(defaults);
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

async function buildCaptureHeaders(url, referrer) {
  const headers = {};

  if (referrer && /^https?:\/\//i.test(referrer)) {
    headers.Referer = referrer;
  }

  if (navigator.userAgent) {
    headers["User-Agent"] = navigator.userAgent;
  }

  const { authenticatedCapture = false } =
    await chrome.storage.local.get("authenticatedCapture");

  if (!authenticatedCapture) {
    return headers;
  }

  try {
    const allowed = await chrome.permissions.contains({
      permissions: ["cookies"],
      origins: ["http://*/*", "https://*/*"],
    });

    if (!allowed) {
      return headers;
    }

    const cookies = await chrome.cookies.getAll({ url });
    if (cookies.length > 0) {
      headers.Cookie = cookies
        .map((cookie) => `${cookie.name}=${cookie.value}`)
        .join("; ");
    }
  } catch (error) {
    console.warn("StorDown could not collect authenticated headers:", error);
  }

  return headers;
}

async function flashBadge(text) {
  await chrome.action.setBadgeText({ text });
  setTimeout(() => chrome.action.setBadgeText({ text: "" }), 1800);
}

chrome.contextMenus.onClicked.addListener(async (info) => {
  if (info.menuItemId !== "stordown-download-link" || !info.linkUrl) {
    return;
  }

  const headers = await buildCaptureHeaders(info.linkUrl, info.pageUrl || null);
  const response = await sendToStorDown({
    type: "download",
    url: info.linkUrl,
    headers,
    source: "context-menu",
  });

  await flashBadge(response.ok ? "✓" : "!");
});

chrome.downloads.onCreated.addListener(async (item) => {
  const { autoCapture = false } = await chrome.storage.local.get("autoCapture");

  if (!autoCapture || !item.url || !/^https?:\/\//i.test(item.url)) {
    return;
  }

  const headers = await buildCaptureHeaders(item.url, item.referrer || null);
  const response = await sendToStorDown({
    type: "download",
    url: item.url,
    filename: item.filename || null,
    headers,
    source: "browser-download",
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
