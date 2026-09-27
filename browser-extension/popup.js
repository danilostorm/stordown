const autoCapture = document.getElementById("autoCapture");
const desktopStatus = document.getElementById("desktopStatus");
const testButton = document.getElementById("testButton");
const siteName = document.getElementById("siteName");
const siteAuthStatus = document.getElementById("siteAuthStatus");
const siteAuthButton = document.getElementById("siteAuthButton");

let activeOrigin = null;
let activePattern = null;

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
  const permission = permissionPattern(tab?.url || "");

  if (!permission) {
    activeOrigin = null;
    activePattern = null;
    siteName.textContent = "Página não suportada";
    siteAuthStatus.textContent = "Abra um site HTTP/HTTPS para autorizar cookies.";
    siteAuthButton.disabled = true;
    return;
  }

  activeOrigin = permission.origin;
  activePattern = permission.pattern;
  siteName.textContent = permission.host;
  siteAuthButton.disabled = false;
  await refreshSiteAuth();
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
    await chrome.permissions.remove({
      permissions: ["cookies"],
      origins: [activePattern],
    });

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

autoCapture.addEventListener("change", async () => {
  await chrome.storage.local.set({ autoCapture: autoCapture.checked });
});

testButton.addEventListener("click", testDesktop);
siteAuthButton.addEventListener("click", toggleSiteAuthorization);

loadSettings();
loadActiveSite();
testDesktop();
