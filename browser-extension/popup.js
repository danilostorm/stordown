const autoCapture = document.getElementById("autoCapture");
const desktopStatus = document.getElementById("desktopStatus");
const testButton = document.getElementById("testButton");
const siteName = document.getElementById("siteName");
const siteAuthStatus = document.getElementById("siteAuthStatus");
const siteAuthButton = document.getElementById("siteAuthButton");
const sitePolicy = document.getElementById("sitePolicy");
const policySiteName = document.getElementById("policySiteName");
const downloadAllButton = document.getElementById("downloadAllButton");
const batchStatus = document.getElementById("batchStatus");

let activeOrigin = null;
let activePattern = null;
let activeTabId = null;
let activePageUrl = null;

function permissionPattern(url) {
  try {
    const parsed = new URL(url);
    if (!["http:", "https:"].includes(parsed.protocol)) return null;
    return {
      origin: parsed.origin,
      pattern: `${parsed.origin}/*`,
      host: parsed.host,
    };
  } catch {
    return null;
  }
}

async function loadSettings() {
  const settings = await chrome.storage.local.get(["autoCapture", "authorizedOrigins"]);
  autoCapture.checked = settings.autoCapture === true;
}

async function testDesktop() {
  desktopStatus.textContent = "Verificando…";
  desktopStatus.dataset.state = "checking";

  try {
    const response = await chrome.runtime.sendMessage({ type: "stordown-ping" });

    if (response?.ok) {
      desktopStatus.textContent = "Conectado";
      desktopStatus.dataset.state = "ok";
    } else {
      desktopStatus.textContent = response?.error || "Indisponível";
      desktopStatus.dataset.state = "error";
    }
  } catch (error) {
    desktopStatus.textContent = String(error);
    desktopStatus.dataset.state = "error";
  }
}

async function loadActiveSite() {
  const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
  const tab = tabs[0];
  activeTabId = tab?.id ?? null;
  activePageUrl = tab?.url ?? null;

  const permission = permissionPattern(activePageUrl || "");

  if (!permission) {
    activeOrigin = null;
    activePattern = null;
    siteName.textContent = "Página não suportada";
    policySiteName.textContent = "Página não suportada";
    siteAuthStatus.textContent = "Abra um site HTTP/HTTPS.";
    siteAuthButton.disabled = true;
    sitePolicy.disabled = true;
    downloadAllButton.disabled = true;
    return;
  }

  activeOrigin = permission.origin;
  activePattern = permission.pattern;
  siteName.textContent = permission.host;
  policySiteName.textContent = permission.host;
  siteAuthButton.disabled = false;
  sitePolicy.disabled = false;
  downloadAllButton.disabled = false;

  await Promise.all([refreshSiteAuth(), refreshSitePolicy()]);
}

async function refreshSitePolicy() {
  if (!activeOrigin) return;

  const { sitePolicies = {} } = await chrome.storage.local.get("sitePolicies");
  sitePolicy.value = sitePolicies[activeOrigin] || "inherit";
}

async function saveSitePolicy() {
  if (!activeOrigin) return;

  const { sitePolicies = {} } = await chrome.storage.local.get("sitePolicies");
  const next = { ...sitePolicies };

  if (sitePolicy.value === "inherit") {
    delete next[activeOrigin];
  } else {
    next[activeOrigin] = sitePolicy.value;
  }

  await chrome.storage.local.set({ sitePolicies: next });
}

async function refreshSiteAuth() {
  if (!activeOrigin || !activePattern) return;

  const granted = await chrome.permissions.contains({
    permissions: ["cookies"],
    origins: [activePattern],
  });

  const { authorizedOrigins = [] } = await chrome.storage.local.get("authorizedOrigins");
  const enabled = granted && authorizedOrigins.includes(activeOrigin);

  siteAuthStatus.textContent = enabled
    ? "Autorizado: cookies podem acompanhar downloads deste site."
    : "Não autorizado: downloads serão enviados sem cookies.";
  siteAuthStatus.dataset.state = enabled ? "ok" : "off";
  siteAuthButton.textContent = enabled ? "Revogar" : "Autorizar site";
}

async function toggleSiteAuthorization() {
  if (!activeOrigin || !activePattern) return;

  const { authorizedOrigins = [] } = await chrome.storage.local.get("authorizedOrigins");
  const currentlyGranted = await chrome.permissions.contains({
    permissions: ["cookies"],
    origins: [activePattern],
  });

  if (currentlyGranted && authorizedOrigins.includes(activeOrigin)) {
    await chrome.permissions.remove({ origins: [activePattern] });
    await chrome.storage.local.set({
      authorizedOrigins: authorizedOrigins.filter((origin) => origin !== activeOrigin),
    });
    await refreshSiteAuth();
    return;
  }

  const granted = await chrome.permissions.request({
    permissions: ["cookies"],
    origins: [activePattern],
  });

  if (granted) {
    await chrome.storage.local.set({
      authorizedOrigins: [...new Set([...authorizedOrigins, activeOrigin])],
    });
  }

  await refreshSiteAuth();
}

async function downloadAllLinks() {
  if (!Number.isInteger(activeTabId)) return;

  downloadAllButton.disabled = true;
  batchStatus.textContent = "Lendo links e enviando para o StorDown…";
  batchStatus.dataset.state = "checking";

  try {
    const response = await chrome.runtime.sendMessage({
      type: "stordown-download-all",
      tabId: activeTabId,
      pageUrl: activePageUrl,
    });

    if (!response) {
      batchStatus.textContent = "Sem resposta da extensão.";
      batchStatus.dataset.state = "error";
      return;
    }

    if (response.total === 0 && response.ok) {
      batchStatus.textContent = "Nenhum link HTTP/HTTPS encontrado.";
      batchStatus.dataset.state = "off";
      return;
    }

    batchStatus.textContent = response.failed
      ? `${response.accepted} enviados • ${response.failed} falharam`
      : `${response.accepted} link(s) adicionados à fila`;

    batchStatus.dataset.state = response.failed ? "error" : "ok";
  } catch (error) {
    batchStatus.textContent = String(error);
    batchStatus.dataset.state = "error";
  } finally {
    downloadAllButton.disabled = false;
  }
}

autoCapture.addEventListener("change", async () => {
  await chrome.storage.local.set({ autoCapture: autoCapture.checked });
});

sitePolicy.addEventListener("change", saveSitePolicy);
testButton.addEventListener("click", testDesktop);
siteAuthButton.addEventListener("click", toggleSiteAuthorization);
downloadAllButton.addEventListener("click", downloadAllLinks);

loadSettings();
loadActiveSite();
testDesktop();
