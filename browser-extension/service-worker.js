const HOST = "cloud.hoststorm.stordown";
const MAX_BATCH_LINKS = 250;
const BATCH_CONCURRENCY = 4;

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "stordown-download-link",
      title: "Baixar com StorDown",
      contexts: ["link"],
    });

    chrome.contextMenus.create({
      id: "stordown-download-all",
      title: "Baixar todos os links com StorDown",
      contexts: ["page"],
    });
  });

  chrome.storage.local.get(["autoCapture", "authorizedOrigins", "sitePolicies"]).then((settings) => {
    const updates = {};

    if (settings.autoCapture === undefined) updates.autoCapture = false;
    if (!Array.isArray(settings.authorizedOrigins)) updates.authorizedOrigins = [];
    if (!settings.sitePolicies || typeof settings.sitePolicies !== "object") {
      updates.sitePolicies = {};
    }

    if (Object.keys(updates).length > 0) chrome.storage.local.set(updates);
  });
});

function sendToStorDown(payload) {
  return new Promise((resolve) => {
    chrome.runtime.sendNativeMessage(HOST, payload, (response) => {
      if (chrome.runtime.lastError) {
        resolve({ ok: false, error: chrome.runtime.lastError.message });
        return;
      }

      resolve(response || { ok: false, error: "StorDown não respondeu." });
    });
  });
}

async function flashBadge(text) {
  await chrome.action.setBadgeText({ text: String(text).slice(0, 4) });
  setTimeout(() => chrome.action.setBadgeText({ text: "" }), 2200);
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

function originOf(url) {
  try {
    const parsed = new URL(url);
    if (!["http:", "https:"].includes(parsed.protocol)) return null;
    return parsed.origin;
  } catch {
    return null;
  }
}

async function shouldAutoCapture(url) {
  const origin = originOf(url);
  const { autoCapture = false, sitePolicies = {} } =
    await chrome.storage.local.get(["autoCapture", "sitePolicies"]);

  const policy = origin ? sitePolicies[origin] : null;
  if (policy === "capture") return true;
  if (policy === "ignore") return false;
  return autoCapture === true;
}

async function buildAuthHeaders(url, referrer) {
  const pattern = permissionPattern(url);
  if (!pattern) return {};

  const origin = new URL(url).origin;
  const { authorizedOrigins = [] } = await chrome.storage.local.get("authorizedOrigins");

  if (!authorizedOrigins.includes(origin)) return {};

  const granted = await chrome.permissions.contains({
    permissions: ["cookies"],
    origins: [pattern],
  });

  if (!granted) return {};

  const headers = {};
  const cookies = await chrome.cookies.getAll({ url });

  if (cookies.length > 0) {
    headers.Cookie = cookies.map((cookie) => `${cookie.name}=${cookie.value}`).join("; ");
  }

  if (referrer && /^https?:\/\//i.test(referrer)) {
    headers.Referer = referrer;
  }

  return headers;
}

async function captureOne(url, filename, referrer, source) {
  const headers = await buildAuthHeaders(url, referrer);
  return sendToStorDown({
    type: "download",
    url,
    filename: filename || null,
    source,
    headers,
  });
}

async function discoverPageLinks(tabId) {
  const results = await chrome.scripting.executeScript({
    target: { tabId },
    func: (limit) => {
      const seen = new Set();
      const links = [];

      for (const anchor of document.querySelectorAll("a[href]")) {
        if (links.length >= limit) break;

        let url;
        try {
          url = new URL(anchor.href, document.baseURI);
        } catch {
          continue;
        }

        if (!["http:", "https:"].includes(url.protocol)) continue;
        const normalized = url.href;
        if (seen.has(normalized)) continue;
        seen.add(normalized);

        const downloadName =
          anchor.getAttribute("download") ||
          decodeURIComponent(url.pathname.split("/").filter(Boolean).pop() || "") ||
          null;

        links.push({
          url: normalized,
          filename: downloadName,
        });
      }

      return links;
    },
    args: [MAX_BATCH_LINKS],
  });

  return results?.[0]?.result || [];
}

async function capturePageLinks(tabId, pageUrl, source = "download-all") {
  let links;

  try {
    links = await discoverPageLinks(tabId);
  } catch (error) {
    return { ok: false, total: 0, accepted: 0, failed: 0, error: String(error) };
  }

  if (!links.length) {
    return { ok: true, total: 0, accepted: 0, failed: 0, errors: [] };
  }

  let cursor = 0;
  let accepted = 0;
  let failed = 0;
  const errors = [];

  async function worker() {
    while (true) {
      const index = cursor++;
      if (index >= links.length) return;

      const link = links[index];
      const response = await captureOne(link.url, link.filename, pageUrl, source);

      if (response?.ok) {
        accepted += 1;
      } else {
        failed += 1;
        if (errors.length < 10) {
          errors.push({ url: link.url, error: response?.error || "Falha desconhecida" });
        }
      }
    }
  }

  await Promise.all(
    Array.from({ length: Math.min(BATCH_CONCURRENCY, links.length) }, () => worker()),
  );

  await flashBadge(failed === 0 ? accepted : "!");
  return {
    ok: failed === 0,
    total: links.length,
    accepted,
    failed,
    errors,
    truncated: links.length >= MAX_BATCH_LINKS,
  };
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === "stordown-download-link" && info.linkUrl) {
    const response = await captureOne(
      info.linkUrl,
      null,
      info.pageUrl || null,
      "context-menu",
    );
    await flashBadge(response.ok ? "✓" : "!");
    return;
  }

  if (info.menuItemId === "stordown-download-all" && tab?.id) {
    await capturePageLinks(tab.id, info.pageUrl || tab.url || null, "context-menu-all");
  }
});

chrome.downloads.onCreated.addListener(async (item) => {
  if (!item.url || !/^https?:\/\//i.test(item.url)) return;

  const capture = await shouldAutoCapture(item.url);
  if (!capture) return;

  const response = await captureOne(
    item.url,
    item.filename || null,
    item.referrer || null,
    "browser-download",
  );

  if (!response.ok) {
    await flashBadge("!");
    return;
  }

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

  if (message?.type === "stordown-download-all" && Number.isInteger(message.tabId)) {
    capturePageLinks(message.tabId, message.pageUrl || null, "popup-download-all")
      .then(sendResponse);
    return true;
  }

  return false;
});
