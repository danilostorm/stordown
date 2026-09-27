const HOST = "cloud.hoststorm.stordown";

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "stordown-download-link",
    title: "Baixar com StorDown",
    contexts: ["link"],
  });
});

function sendToStorDown(payload) {
  chrome.runtime.sendNativeMessage(HOST, payload, (response) => {
    if (chrome.runtime.lastError) {
      console.warn("StorDown native host unavailable:", chrome.runtime.lastError.message);
      return;
    }

    console.debug("StorDown response:", response);
  });
}

chrome.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId === "stordown-download-link" && info.linkUrl) {
    sendToStorDown({
      type: "download",
      url: info.linkUrl,
      source: "context-menu",
    });
  }
});

chrome.downloads.onCreated.addListener(async (item) => {
  const { autoCapture = false } = await chrome.storage.local.get("autoCapture");

  if (!autoCapture || !item.url) {
    return;
  }

  sendToStorDown({
    type: "download",
    url: item.url,
    filename: item.filename || null,
    source: "browser-download",
  });
});
