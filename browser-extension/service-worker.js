const HOST = "cloud.hoststorm.stordown";

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "stordown-download-link",
      title: "Baixar com StorDown",
      contexts: ["link"],
    });
  });

  chrome.storage.local.get("autoCapture").then((settings) => {
    if (settings.autoCapture === undefined) {
      chrome.storage.local.set({ autoCapture: false });
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

chrome.contextMenus.onClicked.addListener(async (info) => {
  if (info.menuItemId !== "stordown-download-link" || !info.linkUrl) {
    return;
  }

  const response = await sendToStorDown({
    type: "download",
    url: info.linkUrl,
    source: "context-menu",
  });

  await flashBadge(response.ok ? "✓" : "!");
});

chrome.downloads.onCreated.addListener(async (item) => {
  const { autoCapture = false } = await chrome.storage.local.get("autoCapture");

  if (!autoCapture || !item.url || !/^https?:\/\//i.test(item.url)) {
    return;
  }

  const response = await sendToStorDown({
    type: "download",
    url: item.url,
    filename: item.filename || null,
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
