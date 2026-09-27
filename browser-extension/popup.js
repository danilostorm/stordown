const autoCapture = document.getElementById("autoCapture");
const desktopStatus = document.getElementById("desktopStatus");
const testButton = document.getElementById("testButton");

async function loadSettings() {
  const settings = await chrome.storage.local.get("autoCapture");
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

autoCapture.addEventListener("change", async () => {
  await chrome.storage.local.set({ autoCapture: autoCapture.checked });
});

testButton.addEventListener("click", testDesktop);

loadSettings();
testDesktop();
