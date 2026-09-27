const autoCapture = document.getElementById("autoCapture");
const authenticatedCapture = document.getElementById("authenticatedCapture");
const desktopStatus = document.getElementById("desktopStatus");
const testButton = document.getElementById("testButton");

const authPermissions = {
  permissions: ["cookies"],
  origins: ["http://*/*", "https://*/*"],
};

async function loadSettings() {
  const settings = await chrome.storage.local.get([
    "autoCapture",
    "authenticatedCapture",
  ]);

  autoCapture.checked = settings.autoCapture === true;

  const permissionGranted = await chrome.permissions.contains(authPermissions);
  authenticatedCapture.checked =
    settings.authenticatedCapture === true && permissionGranted;

  if (settings.authenticatedCapture === true && !permissionGranted) {
    await chrome.storage.local.set({ authenticatedCapture: false });
  }
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

autoCapture.addEventListener("change", async () => {
  await chrome.storage.local.set({ autoCapture: autoCapture.checked });
});

authenticatedCapture.addEventListener("change", async () => {
  if (authenticatedCapture.checked) {
    const granted = await chrome.permissions.request(authPermissions);

    if (!granted) {
      authenticatedCapture.checked = false;
      await chrome.storage.local.set({ authenticatedCapture: false });
      return;
    }

    await chrome.storage.local.set({ authenticatedCapture: true });
    return;
  }

  await chrome.storage.local.set({ authenticatedCapture: false });
  await chrome.permissions.remove(authPermissions);
});

testButton.addEventListener("click", testDesktop);

loadSettings();
testDesktop();
