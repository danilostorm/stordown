import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type NetworkInterfaceInfo = {
  name: string;
  description: string;
  ipv4: string;
  gateway?: string | null;
  link_speed?: string | null;
  index: number;
};

type LinkProbeStatus = {
  name: string;
  local_ip: string;
  public_ip?: string | null;
  latency_ms?: number | null;
  error?: string | null;
};

type GoogleAuthStatus = {
  connected: boolean;
  client_id?: string | null;
  scope?: string | null;
  expires_in_seconds?: number | null;
};

type GoogleDriveFolder = {
  id: string;
  name: string;
  drive_id?: string | null;
};

type GoogleDriveItem = {
  id: string;
  name: string;
  mime_type: string;
  size?: number | null;
  drive_id?: string | null;
  resource_key?: string | null;
  can_download: boolean;
  md5_checksum?: string | null;
  is_folder: boolean;
  is_google_workspace: boolean;
};

type GoogleDriveExportFormat = {
  label: string;
  mime_type: string;
  extension: string;
};

type GoogleSharedDrive = {
  id: string;
  name: string;
};

type DriveBreadcrumb = {
  id: string | null;
  name: string;
  resource_key?: string | null;
};

type TransferProgress = {
  transfer_id: string;
  direction: "download" | "upload" | string;
  item: string;
  phase: string;
  bytes_delta: number;
  bytes_transferred: number;
  total_bytes?: number | null;
  link_name: string;
  local_ip: string;
  completed: boolean;
};

type TransferRecord = {
  id: string;
  direction: "download" | "upload" | string;
  name: string;
  source: string;
  destination: string;
  provider: string;
  status: string;
  bytes_transferred: number;
  total_bytes?: number | null;
  connections: number;
  bind_ips: string[];
  created_at: number;
  updated_at: number;
  scheduled_at?: number | null;
  max_bytes_per_second?: number | null;
  expected_sha256?: string | null;
  error?: string | null;
};

type DownloadRule = {
  id: number;
  name: string;
  extensions: string[];
  destination: string;
  enabled: boolean;
  priority: number;
  created_at: number;
  updated_at: number;
};

type BrowserIntegrationResult = {
  extension_id: string;
  manifest_path: string;
  native_host_path: string;
  chrome_registered: boolean;
  edge_registered: boolean;
};

type BrowserExtensionPrepared = {
  extension_dir: string;
};

type DesktopDefaults = {
  download_dir: string;
  interfaces: NetworkInterfaceInfo[];
};

type RuntimePreferences = {
  download_dir: string;
  selected_ips: string[];
  connections: number;
};

type DownloadProbe = {
  size?: number | null;
  accepts_ranges: boolean;
  content_type?: string | null;
  suggested_name?: string | null;
  final_url?: string | null;
};

type LiveTransferStats = {
  speed: number;
  bytesTransferred: number;
  totalBytes?: number | null;
  updatedAt: number;
};

type SpeedWindow = {
  startedAt: number;
  bytes: number;
};

type View = "home" | "download" | "upload" | "cloud" | "queue" | "scheduled" | "finished" | "extension" | "settings";

const activeStatuses = new Set(["scheduled", "queued", "running", "paused", "interrupted"]);
const finishedStatuses = new Set(["completed", "failed", "cancelled"]);

export default function App() {
  const [view, setView] = useState<View>("home");
  const [url, setUrl] = useState("");
  const [output, setOutput] = useState("");
  const [defaultDownloadDir, setDefaultDownloadDir] = useState("");
  const [downloadProbe, setDownloadProbe] = useState<DownloadProbe | null>(null);
  const [inspectingUrl, setInspectingUrl] = useState(false);
  const [outputManuallyEdited, setOutputManuallyEdited] = useState(false);
  const [connections, setConnections] = useState(8);
  const [downloadSchedule, setDownloadSchedule] = useState("");
  const [downloadSpeedLimit, setDownloadSpeedLimit] = useState(0);
  const [expectedSha256, setExpectedSha256] = useState("");
  const [bindIps, setBindIps] = useState("");
  const [status, setStatus] = useState("Inicializando…");
  const [busy, setBusy] = useState(false);
  const [networkBusy, setNetworkBusy] = useState(false);
  const [detectedNics, setDetectedNics] = useState<NetworkInterfaceInfo[]>([]);
  const [routeTests, setRouteTests] = useState<LinkProbeStatus[]>([]);

  const [uploadFiles, setUploadFiles] = useState("");
  const [driveParentId, setDriveParentId] = useState("");
  const [driveDestinationLabel, setDriveDestinationLabel] = useState("Meu Drive");
  const [driveBrowserOpen, setDriveBrowserOpen] = useState(false);
  const [driveBrowserBusy, setDriveBrowserBusy] = useState(false);
  const [driveFolders, setDriveFolders] = useState<GoogleDriveFolder[]>([]);
  const [sharedDrives, setSharedDrives] = useState<GoogleSharedDrive[]>([]);
  const [activeDriveId, setActiveDriveId] = useState<string | null>(null);
  const [driveBreadcrumbs, setDriveBreadcrumbs] = useState<DriveBreadcrumb[]>([
    { id: null, name: "Meu Drive" },
  ]);
  const [cloudItems, setCloudItems] = useState<GoogleDriveItem[]>([]);
  const [cloudBusy, setCloudBusy] = useState(false);
  const [sharedDriveLink, setSharedDriveLink] = useState("");
  const [sharedLinkBusy, setSharedLinkBusy] = useState(false);
  const [sharedLinkItem, setSharedLinkItem] = useState<GoogleDriveItem | null>(null);
  const [cloudDriveId, setCloudDriveId] = useState<string | null>(null);
  const [cloudBreadcrumbs, setCloudBreadcrumbs] = useState<DriveBreadcrumb[]>([
    { id: null, name: "Meu Drive" },
  ]);
  const [workspaceExportItem, setWorkspaceExportItem] = useState<GoogleDriveItem | null>(null);
  const [workspaceExportFormats, setWorkspaceExportFormats] = useState<GoogleDriveExportFormat[]>([]);
  const [workspaceExportBusy, setWorkspaceExportBusy] = useState(false);
  const [driveClientId, setDriveClientId] = useState("");
  const [driveAuth, setDriveAuth] = useState<GoogleAuthStatus>({ connected: false });
  const [authBusy, setAuthBusy] = useState(false);
  const [chunkMiB, setChunkMiB] = useState(8);
  const [uploadSpeedLimit, setUploadSpeedLimit] = useState(0);

  const [activeTransferId, setActiveTransferId] = useState<string | null>(null);
  const [progressByItem, setProgressByItem] = useState<Record<string, TransferProgress>>({});
  const [linkSpeeds, setLinkSpeeds] = useState<Record<string, number>>({});
  const [records, setRecords] = useState<TransferRecord[]>([]);
  const [downloadRules, setDownloadRules] = useState<DownloadRule[]>([]);
  const [ruleId, setRuleId] = useState<number | null>(null);
  const [ruleName, setRuleName] = useState("Vídeos");
  const [ruleExtensions, setRuleExtensions] = useState("mkv, mp4, avi, mov");
  const [ruleDestination, setRuleDestination] = useState("");
  const [ruleEnabled, setRuleEnabled] = useState(true);
  const [rulePriority, setRulePriority] = useState(100);
  const [browserExtensionId, setBrowserExtensionId] = useState("oiiogiiplcdekpofikkajmmgkjcpgojj");
  const [browserInstallBusy, setBrowserInstallBusy] = useState(false);
  const [browserIntegration, setBrowserIntegration] = useState<BrowserIntegrationResult | null>(null);
  const [browserExtensionPrepared, setBrowserExtensionPrepared] = useState<BrowserExtensionPrepared | null>(null);
  const [liveTransferStats, setLiveTransferStats] = useState<Record<string, LiveTransferStats>>({});
  const speedWindows = useRef<Record<string, SpeedWindow>>({});
  const transferSpeedWindows = useRef<Record<string, SpeedWindow>>({});

  const links = useMemo(
    () => bindIps.split(",").map((ip) => ip.trim()).filter(Boolean),
    [bindIps],
  );

  const files = useMemo(
    () => uploadFiles.split(/\r?\n/).map((path) => path.trim()).filter(Boolean),
    [uploadFiles],
  );

  const nicByIp = useMemo(
    () => new Map(detectedNics.map((nic) => [nic.ipv4, nic])),
    [detectedNics],
  );

  const probeByIp = useMemo(
    () => new Map(routeTests.map((probe) => [probe.local_ip, probe])),
    [routeTests],
  );

  const selectedRecord = useMemo(
    () => records.find((record) => record.id === activeTransferId) ?? null,
    [records, activeTransferId],
  );

  const activeProgress = useMemo(
    () =>
      Object.values(progressByItem)
        .filter((item) => item.transfer_id === activeTransferId)
        .sort((a, b) => a.item.localeCompare(b.item)),
    [progressByItem, activeTransferId],
  );

  const totalTransferred = useMemo(() => {
    if (activeProgress.length > 0) {
      return activeProgress.reduce((sum, item) => sum + item.bytes_transferred, 0);
    }

    return selectedRecord?.bytes_transferred ?? 0;
  }, [activeProgress, selectedRecord]);

  const totalBytes = useMemo(() => {
    if (activeProgress.length > 0) {
      return activeProgress.reduce(
        (sum, item) => sum + (item.total_bytes ?? item.bytes_transferred),
        0,
      );
    }

    return selectedRecord?.total_bytes ?? selectedRecord?.bytes_transferred ?? 0;
  }, [activeProgress, selectedRecord]);

  const queuedRecords = useMemo(
    () => records.filter((record) => activeStatuses.has(record.status)),
    [records],
  );

  const scheduledRecords = useMemo(
    () => records.filter((record) => record.status === "scheduled"),
    [records],
  );

  const finishedRecords = useMemo(
    () => records.filter((record) => finishedStatuses.has(record.status)),
    [records],
  );

  const aggregateLiveSpeed = useMemo(
    () =>
      queuedRecords.reduce(
        (sum, record) => sum + (liveTransferStats[record.id]?.speed ?? 0),
        0,
      ),
    [queuedRecords, liveTransferStats],
  );

  async function reloadTransfers() {
    try {
      const items = await invoke<TransferRecord[]>("list_transfers", { limit: 300 });
      setRecords(items);
    } catch (error) {
      setStatus(`Erro ao carregar fila: ${String(error)}`);
    }
  }

  async function reloadDownloadRules() {
    try {
      const rules = await invoke<DownloadRule[]>("list_download_rules");
      setDownloadRules(rules);
    } catch (error) {
      setStatus(`Erro ao carregar categorias: ${String(error)}`);
    }
  }

  useEffect(() => {
    initializeDesktop();
    reloadTransfers();
    reloadDownloadRules();

    let stopProgress: undefined | (() => void);
    let stopList: undefined | (() => void);
    const refreshTimer = window.setInterval(() => {
      reloadTransfers();
    }, 1500);

    listen<TransferProgress>("transfer-progress", ({ payload }) => {
      const progressKey = `${payload.transfer_id}:${payload.item}`;

      setProgressByItem((current) => {
        const previous = current[progressKey];
        const next =
          previous &&
          !payload.completed &&
          payload.bytes_transferred < previous.bytes_transferred
            ? { ...payload, bytes_transferred: previous.bytes_transferred }
            : payload;

        return { ...current, [progressKey]: next };
      });

      const now = Date.now();
      const key = payload.local_ip;
      const currentWindow = speedWindows.current[key] ?? {
        startedAt: now,
        bytes: 0,
      };

      currentWindow.bytes += payload.bytes_delta;
      const elapsed = now - currentWindow.startedAt;

      if (elapsed >= 500 || payload.completed) {
        const bytesPerSecond = elapsed > 0 ? (currentWindow.bytes * 1000) / elapsed : 0;
        setLinkSpeeds((current) => ({ ...current, [key]: bytesPerSecond }));
        speedWindows.current[key] = { startedAt: now, bytes: 0 };
      } else {
        speedWindows.current[key] = currentWindow;
      }

      const transferKey = payload.transfer_id;
      const transferWindow = transferSpeedWindows.current[transferKey] ?? {
        startedAt: now,
        bytes: 0,
      };
      transferWindow.bytes += payload.bytes_delta;
      const transferElapsed = now - transferWindow.startedAt;

      setLiveTransferStats((current) => {
        const previous = current[transferKey];
        const baseBytes = previous?.bytesTransferred ?? 0;
        const nextBytes = Math.max(
          baseBytes + payload.bytes_delta,
          payload.direction === "download" ? payload.bytes_transferred : baseBytes + payload.bytes_delta,
        );

        return {
          ...current,
          [transferKey]: {
            speed: previous?.speed ?? 0,
            bytesTransferred: nextBytes,
            totalBytes: payload.total_bytes ?? previous?.totalBytes ?? null,
            updatedAt: now,
          },
        };
      });

      if (transferElapsed >= 500 || payload.completed) {
        const speed = transferElapsed > 0 ? (transferWindow.bytes * 1000) / transferElapsed : 0;
        setLiveTransferStats((current) => ({
          ...current,
          [transferKey]: {
            speed: payload.completed ? 0 : speed,
            bytesTransferred:
              current[transferKey]?.bytesTransferred ?? payload.bytes_transferred,
            totalBytes: payload.total_bytes ?? current[transferKey]?.totalBytes ?? null,
            updatedAt: now,
          },
        }));
        transferSpeedWindows.current[transferKey] = { startedAt: now, bytes: 0 };
      } else {
        transferSpeedWindows.current[transferKey] = transferWindow;
      }

      setRecords((current) =>
        current.map((record) =>
          record.id === payload.transfer_id
            ? {
                ...record,
                status:
                  payload.completed && record.direction === "download"
                    ? "completed"
                    : record.status === "paused"
                      ? "paused"
                      : "running",
                bytes_transferred:
                  payload.direction === "download"
                    ? Math.max(record.bytes_transferred, payload.bytes_transferred)
                    : record.bytes_transferred,
                total_bytes: payload.total_bytes ?? record.total_bytes,
                updated_at: Math.floor(now / 1000),
              }
            : record,
        ),
      );

      if (payload.completed) {
        reloadTransfers();
      }
    }).then((unlisten) => {
      stopProgress = unlisten;
    });

    listen<string>("transfer-list-changed", () => {
      reloadTransfers();
    }).then((unlisten) => {
      stopList = unlisten;
    });

    return () => {
      window.clearInterval(refreshTimer);
      stopProgress?.();
      stopList?.();
    };
  }, []);

  async function syncRuntimePreferences(
    downloadDir: string,
    selectedIps: string[],
    connectionCount: number,
  ) {
    if (!downloadDir.trim()) return;

    try {
      await invoke<RuntimePreferences>("update_runtime_preferences", {
        downloadDir,
        selectedIps,
        connections: Math.min(64, Math.max(1, connectionCount || 1)),
      });
    } catch (error) {
      console.warn("Não foi possível sincronizar preferências do Native Host:", error);
    }
  }

  async function initializeDesktop() {
    try {
      const defaults = await invoke<DesktopDefaults>("get_desktop_defaults");
      setDetectedNics(defaults.interfaces);

      const detectedIps = defaults.interfaces.map((nic) => nic.ipv4);
      const savedIps = (window.localStorage.getItem("stordown.bindIps") ?? "")
        .split(",")
        .map((ip) => ip.trim())
        .filter((ip) => detectedIps.includes(ip));
      const selectedIps = savedIps.length > 0 ? savedIps : detectedIps;

      if (selectedIps.length > 0) {
        setBindIps(selectedIps.join(", "));
      }

      const savedDownloadDir = window.localStorage.getItem("stordown.downloadDir");
      const downloadDir = savedDownloadDir?.trim() || defaults.download_dir;
      setDefaultDownloadDir(downloadDir);

      const savedConnections = Number(window.localStorage.getItem("stordown.connections") ?? "8");
      const connectionCount =
        Number.isFinite(savedConnections) && savedConnections >= 1 && savedConnections <= 64
          ? savedConnections
          : 8;
      setConnections(connectionCount);

      await syncRuntimePreferences(downloadDir, selectedIps, connectionCount);

      if (!output) {
        setOutput(joinWindowsPath(downloadDir, "download.bin"));
      }

      if (!ruleDestination) {
        setRuleDestination(downloadDir);
      }

      setStatus(
        detectedIps.length > 1
          ? `${detectedIps.length} interfaces detectadas — teste as WANs para confirmar saídas independentes`
          : detectedIps.length === 1
            ? "1 interface detectada — StorDown funcionará normalmente em modo single-link"
            : "Nenhuma interface física foi detectada automaticamente",
      );
    } catch (error) {
      setStatus(`Falha ao inicializar este computador: ${String(error)}`);
    }
  }

  async function inspectDownloadUrl() {
    if (!/^https?:\/\//i.test(url.trim())) {
      setDownloadProbe(null);
      return;
    }

    setInspectingUrl(true);

    try {
      const probe = await invoke<DownloadProbe>("inspect_download_url", { url: url.trim() });
      setDownloadProbe(probe);

      const name = probe.suggested_name || suggestedDownloadName(probe.final_url || url);
      if (!outputManuallyEdited && name) {
        const directory = parentDirectory(output) || defaultDownloadDir;
        if (directory) {
          setOutput(joinWindowsPath(directory, name));
        }
      }

      setStatus(
        `URL analisada: ${probe.size ? formatBytes(probe.size) : "tamanho desconhecido"} • ${probe.accepts_ranges ? "segmentação disponível" : "download direto"}`,
      );
    } catch (error) {
      setDownloadProbe(null);
      setStatus(`Não foi possível analisar a URL: ${String(error)}`);
    } finally {
      setInspectingUrl(false);
    }
  }

  async function detectNetworks() {
    setNetworkBusy(true);
    setStatus("Detectando placas de rede físicas do Windows…");

    try {
      const nics = await invoke<NetworkInterfaceInfo[]>("list_network_interfaces");
      setDetectedNics(nics);
      setRouteTests([]);

      if (nics.length > 0) {
        const detected = nics.map((nic) => nic.ipv4);
        setBindIps(detected.join(", "));
        window.localStorage.setItem("stordown.bindIps", detected.join(","));
        await syncRuntimePreferences(defaultDownloadDir, detected, connections);
        setStatus(`${nics.length} interface(s) física(s) detectada(s)`);
      } else {
        setStatus("Nenhuma interface física ativa com IPv4 foi encontrada");
      }
    } catch (error) {
      setStatus(`Erro ao detectar interfaces: ${String(error)}`);
    } finally {
      setNetworkBusy(false);
    }
  }

  async function toggleNetworkInterface(ip: string, enabled: boolean) {
    const selected = new Set(links);

    if (enabled) {
      selected.add(ip);
    } else {
      selected.delete(ip);
    }

    const next = Array.from(selected);
    setBindIps(next.join(", "));
    setRouteTests([]);
    window.localStorage.setItem("stordown.bindIps", next.join(","));
    await syncRuntimePreferences(defaultDownloadDir, next, connections);
  }

  async function testRoutes() {
    if (links.length === 0) return;

    setNetworkBusy(true);
    setStatus("Testando a saída de Internet de cada interface…");

    try {
      const probes = await invoke<LinkProbeStatus[]>("test_routes", { bindIps: links });
      setRouteTests(probes);

      const publicIps = new Set(
        probes.map((probe) => probe.public_ip).filter((ip): ip is string => Boolean(ip)),
      );

      if (publicIps.size >= 2) {
        setStatus("Multi-WAN confirmado: as interfaces estão saindo por IPs públicos diferentes");
      } else if (probes.some((probe) => probe.error)) {
        setStatus("Teste concluído com falha em uma ou mais interfaces");
      } else {
        setStatus("As interfaces responderam, mas estão usando o mesmo IP público");
      }
    } catch (error) {
      setStatus(`Erro no teste Multi-WAN: ${String(error)}`);
    } finally {
      setNetworkBusy(false);
    }
  }

  async function connectDrive() {
    setAuthBusy(true);
    setStatus("Abrindo login seguro do Google Drive no navegador…");

    try {
      const auth = await invoke<GoogleAuthStatus>("connect_google_drive", {
        clientId: driveClientId || null,
      });
      setDriveAuth(auth);
      setStatus("Google Drive conectado com sucesso");
    } catch (error) {
      setStatus(`Erro ao conectar Google Drive: ${String(error)}`);
    } finally {
      setAuthBusy(false);
    }
  }

  async function restoreDrive() {
    setAuthBusy(true);
    setStatus("Restaurando sessão segura do Google Drive…");

    try {
      const auth = await invoke<GoogleAuthStatus>("restore_google_drive", {
        clientId: driveClientId || null,
      });
      setDriveAuth(auth);
      setStatus("Sessão Google Drive restaurada");
    } catch (error) {
      setStatus(`Não foi possível restaurar a sessão: ${String(error)}`);
    } finally {
      setAuthBusy(false);
    }
  }

  async function disconnectDrive() {
    setAuthBusy(true);

    try {
      const auth = await invoke<GoogleAuthStatus>("disconnect_google_drive", {
        clientId: driveClientId || null,
      });
      setDriveAuth(auth);
      setStatus("Google Drive desconectado deste Windows");
    } catch (error) {
      setStatus(`Erro ao desconectar Google Drive: ${String(error)}`);
    } finally {
      setAuthBusy(false);
    }
  }

  async function loadDriveFolders(
    parentId: string | null,
    driveId: string | null,
  ) {
    setDriveBrowserBusy(true);

    try {
      const folders = await invoke<GoogleDriveFolder[]>("browse_google_drive_folders", {
        parentId,
        driveId,
      });
      setDriveFolders(folders);
    } catch (error) {
      const message = String(error);
      setStatus(
        message.includes("403") || message.toLowerCase().includes("scope")
          ? "O Google Drive precisa de nova autorização para navegar pastas. Desconecte e conecte novamente."
          : `Erro ao listar pastas do Google Drive: ${message}`,
      );
      setDriveFolders([]);
    } finally {
      setDriveBrowserBusy(false);
    }
  }

  async function openDriveBrowser() {
    if (!driveAuth.connected) {
      setStatus("Conecte o Google Drive antes de escolher uma pasta");
      return;
    }

    setDriveBrowserOpen(true);
    setDriveBrowserBusy(true);

    try {
      const drives = await invoke<GoogleSharedDrive[]>("list_google_drive_roots");
      setSharedDrives(drives);
      setActiveDriveId(null);
      setDriveBreadcrumbs([{ id: null, name: "Meu Drive" }]);
      await loadDriveFolders(null, null);
    } catch (error) {
      const message = String(error);
      setStatus(
        message.includes("403") || message.toLowerCase().includes("scope")
          ? "Reconecte o Google Drive para liberar o navegador de pastas."
          : `Erro ao abrir Google Drive: ${message}`,
      );
      setDriveBrowserBusy(false);
    }
  }

  async function switchDrive(drive: GoogleSharedDrive | null) {
    const driveId = drive?.id ?? null;
    setActiveDriveId(driveId);
    setDriveBreadcrumbs([
      { id: driveId, name: drive?.name ?? "Meu Drive" },
    ]);
    await loadDriveFolders(driveId, driveId);
  }

  async function enterDriveFolder(folder: GoogleDriveFolder) {
    setDriveBreadcrumbs((current) => [
      ...current,
      { id: folder.id, name: folder.name },
    ]);
    await loadDriveFolders(folder.id, activeDriveId);
  }

  async function driveBrowserBack() {
    if (driveBreadcrumbs.length <= 1) return;

    const next = driveBreadcrumbs.slice(0, -1);
    const parent = next[next.length - 1];
    setDriveBreadcrumbs(next);
    await loadDriveFolders(parent.id, activeDriveId);
  }

  function selectCurrentDriveFolder() {
    const current = driveBreadcrumbs[driveBreadcrumbs.length - 1];
    const parentId = current.id ?? "";
    setDriveParentId(parentId);
    setDriveDestinationLabel(driveBreadcrumbs.map((item) => item.name).join(" / "));
    setDriveBrowserOpen(false);
    setStatus(`Destino do Google Drive: ${driveBreadcrumbs.map((item) => item.name).join(" / ")}`);
  }

  async function loadCloudItems(
    parentId: string | null,
    driveId: string | null,
    resourceKey: string | null = null,
  ) {
    if (!driveAuth.connected) {
      setCloudItems([]);
      return;
    }

    setCloudBusy(true);

    try {
      const items = await invoke<GoogleDriveItem[]>("browse_google_drive_items", {
        parentId,
        driveId,
        resourceKey,
      });
      setCloudItems(items);
    } catch (error) {
      const message = String(error);
      setCloudItems([]);
      setStatus(
        message.includes("403") || message.toLowerCase().includes("scope")
          ? "Reconecte o Google Drive para liberar leitura e download dos arquivos."
          : `Erro ao listar arquivos do Google Drive: ${message}`,
      );
    } finally {
      setCloudBusy(false);
    }
  }

  async function openCloudWorkspace() {
    setView("cloud");

    if (!driveAuth.connected) {
      setStatus("Conecte sua conta Google Drive para navegar e baixar arquivos");
      return;
    }

    setCloudBusy(true);

    try {
      const drives = await invoke<GoogleSharedDrive[]>("list_google_drive_roots");
      setSharedDrives(drives);
      setCloudDriveId(null);
      setCloudBreadcrumbs([{ id: null, name: "Meu Drive" }]);
      await loadCloudItems(null, null, null);
      setStatus("Google Drive carregado");
    } catch (error) {
      setStatus(`Erro ao abrir Google Drive: ${String(error)}`);
      setCloudBusy(false);
    }
  }

  async function switchCloudDrive(drive: GoogleSharedDrive | null) {
    const driveId = drive?.id ?? null;
    setCloudDriveId(driveId);
    setCloudBreadcrumbs([{ id: driveId, name: drive?.name ?? "Meu Drive" }]);
    await loadCloudItems(driveId, driveId, null);
  }

  async function enterCloudFolder(item: GoogleDriveItem) {
    if (!item.is_folder) return;

    setCloudBreadcrumbs((current) => [
      ...current,
      { id: item.id, name: item.name, resource_key: item.resource_key ?? null },
    ]);
    await loadCloudItems(item.id, cloudDriveId, item.resource_key ?? null);
  }

  async function cloudBack() {
    if (cloudBreadcrumbs.length <= 1) return;

    const next = cloudBreadcrumbs.slice(0, -1);
    const parent = next[next.length - 1];
    setCloudBreadcrumbs(next);
    await loadCloudItems(parent.id, cloudDriveId, parent.resource_key ?? null);
  }

  async function importSharedDriveLink() {
    if (!sharedDriveLink.trim()) {
      setStatus("Cole um link compartilhado do Google Drive.");
      return;
    }

    if (!driveAuth.connected) {
      setStatus("Conecte o Google Drive antes de importar um link compartilhado.");
      return;
    }

    setSharedLinkBusy(true);

    try {
      const item = await invoke<GoogleDriveItem>("resolve_drive_shared_link", {
        sharedLink: sharedDriveLink.trim(),
      });
      setSharedLinkItem(item);
      setStatus(`Link reconhecido: ${item.name}`);
    } catch (error) {
      setSharedLinkItem(null);
      setStatus(`Erro ao abrir link compartilhado: ${String(error)}`);
    } finally {
      setSharedLinkBusy(false);
    }
  }

  async function openSharedDriveItem(item: GoogleDriveItem) {
    if (item.is_folder) {
      setCloudDriveId(item.drive_id ?? null);
      setCloudBreadcrumbs([
        {
          id: item.id,
          name: item.name,
          resource_key: item.resource_key ?? null,
        },
      ]);
      await loadCloudItems(
        item.id,
        item.drive_id ?? null,
        item.resource_key ?? null,
      );
      setSharedLinkItem(null);
      setStatus(`Pasta compartilhada aberta: ${item.name}`);
      return;
    }

    await downloadCloudItem(item);
  }

  async function openWorkspaceExport(item: GoogleDriveItem) {
    if (!item.is_google_workspace || !item.can_download) {
      setStatus("Este item não pode ser exportado pela sua permissão atual.");
      return;
    }

    setWorkspaceExportBusy(true);

    try {
      const formats = await invoke<GoogleDriveExportFormat[]>("google_drive_export_options", {
        mimeType: item.mime_type,
      });
      setWorkspaceExportItem(item);
      setWorkspaceExportFormats(formats);

      if (formats.length === 0) {
        setStatus("Este tipo do Google Workspace ainda não tem exportação direta no StorDown.");
      } else {
        setStatus(`Escolha o formato para exportar ${item.name}`);
      }
    } catch (error) {
      setStatus(`Erro ao carregar formatos de exportação: ${String(error)}`);
    } finally {
      setWorkspaceExportBusy(false);
    }
  }

  async function exportWorkspaceItem(format: GoogleDriveExportFormat) {
    const item = workspaceExportItem;
    if (!item) return;

    setWorkspaceExportBusy(true);

    try {
      const suggestedName = item.name.toLowerCase().endsWith(format.extension.toLowerCase())
        ? item.name
        : `${item.name}${format.extension}`;
      const picked = await invoke<string | null>("pick_download_destination", {
        suggestedName,
      });
      if (!picked) return;

      const transferId = crypto.randomUUID();
      const record = await invoke<TransferRecord>("enqueue_drive_export", {
        fileId: item.id,
        fileName: item.name,
        sourceMime: item.mime_type,
        exportMime: format.mime_type,
        extension: format.extension,
        resourceKey: item.resource_key ?? null,
        output: picked,
        bindIps: links,
        speedLimitMbps: downloadSpeedLimit > 0 ? downloadSpeedLimit : null,
        transferId,
      });

      setActiveTransferId(record.id);
      setLinkSpeeds({});
      speedWindows.current = {};
      setWorkspaceExportItem(null);
      setWorkspaceExportFormats([]);
      await reloadTransfers();
      setStatus(`${item.name} exportado para a fila como ${format.extension}`);
    } catch (error) {
      setStatus(`Erro ao exportar documento Google: ${String(error)}`);
    } finally {
      setWorkspaceExportBusy(false);
    }
  }

  async function downloadCloudItem(item: GoogleDriveItem) {
    if (item.is_folder) {
      await enterCloudFolder(item);
      return;
    }

    if (item.is_google_workspace) {
      await openWorkspaceExport(item);
      return;
    }

    if (!item.can_download) {
      setStatus("Sua permissão do Google Drive não permite baixar este arquivo.");
      return;
    }

    try {
      const picked = await invoke<string | null>("pick_download_destination", {
        suggestedName: item.name,
      });
      if (!picked) return;

      const transferId = crypto.randomUUID();
      const record = await invoke<TransferRecord>("enqueue_drive_download", {
        fileId: item.id,
        fileName: item.name,
        mimeType: item.mime_type,
        resourceKey: item.resource_key ?? null,
        output: picked,
        connections,
        bindIps: links,
        speedLimitMbps: downloadSpeedLimit > 0 ? downloadSpeedLimit : null,
        transferId,
      });

      setActiveTransferId(record.id);
      setLinkSpeeds({});
      speedWindows.current = {};
      await reloadTransfers();
      setStatus(`${item.name} adicionado à fila Multi-WAN`);
    } catch (error) {
      setStatus(`Erro ao baixar do Google Drive: ${String(error)}`);
    }
  }

  async function chooseDownloadDestination() {
    try {
      const picked = await invoke<string | null>("pick_download_folder");
      if (picked) {
        const name = outputManuallyEdited
          ? fileNameFromPath(output) || "download.bin"
          : downloadProbe?.suggested_name ||
            suggestedDownloadName(downloadProbe?.final_url || url) ||
            fileNameFromPath(output) ||
            "download.bin";
        setDefaultDownloadDir(picked);
        window.localStorage.setItem("stordown.downloadDir", picked);
        await syncRuntimePreferences(picked, links, connections);
        setOutput(joinWindowsPath(picked, name));
        setOutputManuallyEdited(false);
        setStatus(`Pasta de destino: ${picked}`);
      }
    } catch (error) {
      setStatus(`Erro ao abrir seletor do Windows: ${String(error)}`);
    }
  }

  async function chooseUploadFiles() {
    try {
      const picked = await invoke<string[]>("pick_upload_files");
      if (picked.length > 0) {
        setUploadFiles(picked.join("\n"));
        setStatus(`${picked.length} arquivo(s) selecionado(s)`);
      }
    } catch (error) {
      setStatus(`Erro ao abrir seletor do Windows: ${String(error)}`);
    }
  }

  async function submitDownload(event: FormEvent) {
    event.preventDefault();
    const transferId = crypto.randomUUID();
    setBusy(true);
    setStatus("Adicionando download à fila…");

    try {
      const record = await invoke<TransferRecord>("enqueue_download", {
        url,
        output,
        connections,
        bindIps: links,
        transferId,
        scheduledAt: downloadSchedule
          ? Math.floor(new Date(downloadSchedule).getTime() / 1000)
          : null,
        speedLimitMbps: downloadSpeedLimit > 0 ? downloadSpeedLimit : null,
        expectedSha256: expectedSha256.trim() || null,
      });

      setActiveTransferId(record.id);
      setLinkSpeeds({});
      speedWindows.current = {};
      await reloadTransfers();
      setStatus(downloadSchedule ? "Download agendado" : "Download adicionado à fila");
    } catch (error) {
      setStatus(`Erro: ${String(error)}`);
    } finally {
      setBusy(false);
    }
  }

  async function submitUpload(event: FormEvent) {
    event.preventDefault();
    const transferId = crypto.randomUUID();
    setBusy(true);
    setStatus(`Adicionando ${files.length} arquivo(s) à fila…`);

    try {
      const record = await invoke<TransferRecord>("enqueue_drive_upload", {
        files,
        parentId: driveParentId || null,
        bindIps: links,
        chunkMib: chunkMiB,
        speedLimitMbps: uploadSpeedLimit > 0 ? uploadSpeedLimit : null,
        transferId,
      });

      setActiveTransferId(record.id);
      setLinkSpeeds({});
      speedWindows.current = {};
      await reloadTransfers();
      setStatus("Upload adicionado à fila");
    } catch (error) {
      setStatus(`Erro: ${String(error)}`);
    } finally {
      setBusy(false);
    }
  }

  async function pauseTransfer(transferId: string) {
    try {
      await invoke("pause_transfer", { transferId });
      setStatus("Transferência pausada");
      await reloadTransfers();
    } catch (error) {
      setStatus(`Erro ao pausar: ${String(error)}`);
    }
  }

  async function resumeTransfer(transferId: string) {
    try {
      const record = records.find((item) => item.id === transferId);

      if (
        record?.status === "interrupted" &&
        record.provider === "google_drive" &&
        record.direction === "upload"
      ) {
        if (!driveAuth.connected) {
          setStatus("Conecte ou restaure o Google Drive antes de retomar este upload.");
          setView("upload");
          return;
        }

        await invoke<TransferRecord>("resume_drive_upload", { transferId });
        setStatus("Upload do Google Drive retomado da sessão salva");
      } else {
        await invoke("resume_transfer", { transferId });
        setStatus("Transferência retomada");
      }

      setActiveTransferId(transferId);
      setLinkSpeeds({});
      speedWindows.current = {};
      await reloadTransfers();
    } catch (error) {
      setStatus(`Erro ao retomar: ${String(error)}`);
    }
  }

  async function cancelTransfer(transferId: string) {
    try {
      await invoke("cancel_transfer", { transferId });
      setStatus("Cancelamento solicitado");
      await reloadTransfers();
    } catch (error) {
      setStatus(`Erro ao cancelar: ${String(error)}`);
    }
  }

  async function deleteHistory(transferId: string) {
    try {
      await invoke("delete_transfer_history", { transferId });
      if (activeTransferId === transferId) setActiveTransferId(null);
      await reloadTransfers();
    } catch (error) {
      setStatus(`Erro ao remover histórico: ${String(error)}`);
    }
  }

  async function clearFinished() {
    try {
      const removed = await invoke<number>("clear_finished_history");
      await reloadTransfers();
      setStatus(`${removed} item(ns) removido(s) do histórico`);
    } catch (error) {
      setStatus(`Erro ao limpar histórico: ${String(error)}`);
    }
  }

  async function prepareBrowserExtension() {
    setBrowserInstallBusy(true);
    setStatus("Preparando extensão e Native Host do StorDown…");

    try {
      const prepared = await invoke<BrowserExtensionPrepared>("prepare_browser_extension");
      setBrowserExtensionPrepared(prepared);

      const integration = await invoke<BrowserIntegrationResult>("install_browser_integration", {
        extensionId: null,
      });
      setBrowserIntegration(integration);
      setBrowserExtensionId(integration.extension_id);

      setStatus(
        "Extensão preparada e Native Host registrado. Agora carregue a pasta no Chrome/Edge uma única vez.",
      );
    } catch (error) {
      setStatus(`Erro ao preparar extensão: ${String(error)}`);
    } finally {
      setBrowserInstallBusy(false);
    }
  }

  async function installBrowserIntegration() {
    setBrowserInstallBusy(true);
    setStatus("Registrando novamente a integração Chrome/Edge…");

    try {
      const result = await invoke<BrowserIntegrationResult>("install_browser_integration", {
        extensionId: browserExtensionId || null,
      });
      setBrowserIntegration(result);
      setBrowserExtensionId(result.extension_id);
      setStatus("Integração do navegador registrada neste Windows");
    } catch (error) {
      setStatus(`Erro ao instalar integração: ${String(error)}`);
    } finally {
      setBrowserInstallBusy(false);
    }
  }

  async function openExtensionsPage(browser: "chrome" | "edge") {
    try {
      await invoke("open_browser_extensions", { browser });
    } catch (error) {
      setStatus(`Erro ao abrir extensões: ${String(error)}`);
    }
  }

  function resetRuleForm() {
    setRuleId(null);
    setRuleName("Vídeos");
    setRuleExtensions("mkv, mp4, avi, mov");
    setRuleDestination(defaultDownloadDir || parentDirectory(output));
    setRuleEnabled(true);
    setRulePriority(100);
  }

  function editRule(rule: DownloadRule) {
    setRuleId(rule.id);
    setRuleName(rule.name);
    setRuleExtensions(rule.extensions.join(", "));
    setRuleDestination(rule.destination);
    setRuleEnabled(rule.enabled);
    setRulePriority(rule.priority);
    setView("settings");
  }

  async function chooseRuleFolder() {
    try {
      const picked = await invoke<string | null>("pick_download_rule_folder");
      if (picked) setRuleDestination(picked);
    } catch (error) {
      setStatus(`Erro ao selecionar pasta: ${String(error)}`);
    }
  }

  async function saveRule(event: FormEvent) {
    event.preventDefault();

    try {
      await invoke<DownloadRule>("save_download_rule", {
        id: ruleId,
        name: ruleName,
        extensions: ruleExtensions
          .split(/[;,\s]+/)
          .map((value) => value.trim())
          .filter(Boolean),
        destination: ruleDestination,
        enabled: ruleEnabled,
        priority: rulePriority,
      });

      await reloadDownloadRules();
      setStatus(ruleId ? "Categoria atualizada" : "Categoria criada");
      resetRuleForm();
    } catch (error) {
      setStatus(`Erro ao salvar categoria: ${String(error)}`);
    }
  }

  async function removeRule(id: number) {
    try {
      await invoke("delete_download_rule", { id });
      await reloadDownloadRules();
      if (ruleId === id) resetRuleForm();
      setStatus("Categoria removida");
    } catch (error) {
      setStatus(`Erro ao remover categoria: ${String(error)}`);
    }
  }

  return (
    <main className="shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brandMark">S</span>
          <div>
            <strong>StorDown</strong>
            <small>Multi-Link Transfer Manager</small>
          </div>
        </div>

        <nav>
          <button
            className={`navItem ${view === "home" ? "active" : ""}`}
            onClick={() => setView("home")}
          >
            <span className="navIcon">⌂</span> Visão geral
          </button>
          <button
            className={`navItem ${view === "download" ? "active" : ""}`}
            onClick={() => setView("download")}
          >
            <span className="navIcon">↓</span> Novo download
          </button>
          <button
            className={`navItem ${view === "upload" ? "active" : ""}`}
            onClick={() => setView("upload")}
          >
            <span className="navIcon">↑</span> Novo upload
          </button>
          <button
            className={`navItem navCount ${view === "queue" ? "active" : ""}`}
            onClick={() => setView("queue")}
          >
            <span><span className="navIcon">≡</span> Transferências</span>
            <b>{queuedRecords.length}</b>
          </button>
          <button
            className={`navItem navCount ${view === "scheduled" ? "active" : ""}`}
            onClick={() => setView("scheduled")}
          >
            <span><span className="navIcon">◷</span> Agendados</span>
            <b>{scheduledRecords.length}</b>
          </button>
          <button
            className={`navItem navCount ${view === "finished" ? "active" : ""}`}
            onClick={() => setView("finished")}
          >
            <span><span className="navIcon">✓</span> Histórico</span>
            <b>{finishedRecords.length}</b>
          </button>
          <button
            className={`navItem ${view === "cloud" ? "active" : ""}`}
            onClick={openCloudWorkspace}
          >
            <span className="navIcon">☁</span> Google Drive
          </button>
          <button
            className={`navItem ${view === "extension" ? "active" : ""}`}
            onClick={() => setView("extension")}
          >
            <span className="navIcon">⊕</span> Extensão
          </button>
          <button
            className={`navItem ${view === "settings" ? "active" : ""}`}
            onClick={() => setView("settings")}
          >
            <span className="navIcon">⚙</span> Configurações
          </button>
        </nav>

        <div className="networkCard">
          <span>Conexões de rede</span>
          <strong>{links.length} selecionada(s)</strong>
          <div className="smartWanBadge">
            {links.length > 1 ? "Multi-WAN disponível para testar" : "Modo single-link disponível"}
          </div>

          {detectedNics.length > 0 ? (
            detectedNics.map((nic, index) => {
              const ip = nic.ipv4;
              const probe = probeByIp.get(ip);
              const selected = links.includes(ip);

              return (
                <label className={`networkLink networkChoice ${selected ? "selected" : ""}`} key={ip}>
                  <input
                    type="checkbox"
                    checked={selected}
                    onChange={(event) => toggleNetworkInterface(ip, event.target.checked)}
                  />
                  <div className="networkChoiceBody">
                    <div className="linkRow">
                      <i className={probe?.error ? "bad" : ""} />
                      <span>{nic.name || `Interface ${index + 1}`}</span>
                      <code>{ip}</code>
                    </div>
                    <div className="linkMeta">
                      {nic.link_speed && <span>{nic.link_speed}</span>}
                      {probe?.public_ip && (
                        <span>
                          Internet: {probe.public_ip} • {probe.latency_ms ?? "?"} ms
                        </span>
                      )}
                      {probe?.error && <span className="errorText">Sem saída</span>}
                      {linkSpeeds[ip] !== undefined && (
                        <span className="speedText">{formatSpeed(linkSpeeds[ip])}</span>
                      )}
                    </div>
                  </div>
                </label>
              );
            })
          ) : (
            <div className="networkEmpty">Nenhuma placa ativa detectada.</div>
          )}

          <div className="networkActions">
            <button type="button" onClick={detectNetworks} disabled={networkBusy}>
              Redetectar
            </button>
            <button
              type="button"
              onClick={testRoutes}
              disabled={networkBusy || links.length === 0}
            >
              Testar saídas
            </button>
          </div>
        </div>
      </aside>

      <section className="content">
        <header>
          <div>
            <p className="eyebrow">STORDOWN 0.1 ALPHA</p>
            <h1>{viewTitle(view)}</h1>
            <p className="subtitle">{viewSubtitle(view)}</p>
          </div>
          <div className="statusPill">{status}</div>
        </header>

        <div className="managerToolbar">
          <button type="button" className={view === "download" ? "active" : ""} onClick={() => setView("download")}>
            <span>＋</span> Download
          </button>
          <button type="button" className={view === "upload" ? "active" : ""} onClick={() => setView("upload")}>
            <span>↑</span> Upload
          </button>
          <button type="button" className={view === "queue" ? "active" : ""} onClick={() => setView("queue")}>
            <span>≡</span> Transferências
          </button>
          <button type="button" className={view === "extension" ? "active" : ""} onClick={() => setView("extension")}>
            <span>⊕</span> Navegador
          </button>
          <div className="toolbarSummary">
            <strong>{formatSpeed(aggregateLiveSpeed)}</strong>
            <span>{records.filter((record) => record.status === "running").length} ativa(s)</span>
          </div>
        </div>

        <ActiveTransferDock
          records={queuedRecords}
          liveStats={liveTransferStats}
          selectedId={activeTransferId}
          onSelect={(id) => {
            setActiveTransferId(id);
            setView("queue");
          }}
          onPause={pauseTransfer}
          onResume={resumeTransfer}
          onCancel={cancelTransfer}
        />

        {view === "home" && (
          <section className="managerHome">
            <div className="quickActions">
              <button type="button" className="primary actionTile" onClick={() => setView("download")}>
                <span>↓</span>
                <div>
                  <strong>Novo download</strong>
                  <small>HTTP/HTTPS, Range e Multi-WAN</small>
                </div>
              </button>
              <button type="button" className="actionTile" onClick={() => setView("upload")}>
                <span>↑</span>
                <div>
                  <strong>Novo upload</strong>
                  <small>Google Drive resumível</small>
                </div>
              </button>
              <button type="button" className="actionTile" onClick={openCloudWorkspace}>
                <span>☁</span>
                <div>
                  <strong>Google Drive</strong>
                  <small>Navegar, baixar e exportar</small>
                </div>
              </button>
              <button type="button" className="actionTile" onClick={() => setView("extension")}>
                <span>⊕</span>
                <div>
                  <strong>Extensão</strong>
                  <small>Chrome / Edge</small>
                </div>
              </button>
            </div>

            <div className="managerStats">
              <article>
                <span>Velocidade total</span>
                <strong>{formatSpeed(aggregateLiveSpeed)}</strong>
                <small>somando transferências ativas</small>
              </article>
              <article>
                <span>Em andamento</span>
                <strong>{records.filter((record) => record.status === "running").length}</strong>
                <small>{queuedRecords.length} item(ns) na fila</small>
              </article>
              <article>
                <span>Links ativos</span>
                <strong>{links.length}</strong>
                <small>{routeTests.length ? "rotas testadas" : "teste as WANs nas configurações de rede"}</small>
              </article>
              <article>
                <span>Concluídos</span>
                <strong>{records.filter((record) => record.status === "completed").length}</strong>
                <small>histórico persistente</small>
              </article>
            </div>

            <section className="managerPanel">
              <div className="managerPanelHeader">
                <div>
                  <strong>Transferências</strong>
                  <small>Progresso, velocidade e tempo restante em tempo real</small>
                </div>
                <button type="button" onClick={() => setView("queue")}>Ver fila completa</button>
              </div>
              <TransferList
                records={queuedRecords}
                emptyText="Nenhuma transferência ativa. Adicione um download para começar."
                selectedId={activeTransferId}
                onSelect={setActiveTransferId}
                onPause={pauseTransfer}
                onResume={resumeTransfer}
                onCancel={cancelTransfer}
                onDelete={deleteHistory}
                liveStats={liveTransferStats}
              />
            </section>

            <section className="managerPanel">
              <div className="managerPanelHeader">
                <div>
                  <strong>Recentes</strong>
                  <small>Últimas transferências finalizadas</small>
                </div>
                <button type="button" onClick={() => setView("finished")}>Abrir histórico</button>
              </div>
              <TransferList
                records={finishedRecords.slice(0, 5)}
                emptyText="Ainda não há downloads concluídos."
                selectedId={activeTransferId}
                onSelect={setActiveTransferId}
                onPause={pauseTransfer}
                onResume={resumeTransfer}
                onCancel={cancelTransfer}
                onDelete={deleteHistory}
                liveStats={liveTransferStats}
              />
            </section>
          </section>
        )}

        {view === "download" && (
          <form className="downloadCard" onSubmit={submitDownload}>
            <div className="notice">
              <strong>Smart Multi-WAN ativo</strong>
              <span>
                O StorDown mede o desempenho real das interfaces durante o arquivo, entrega mais
                blocos ao link mais rápido e move novas tentativas para outra WAN quando uma rota falha.
              </span>
            </div>

            <label>
              URL
              <div className="fieldWithButton">
                <input
                  value={url}
                  onChange={(e) => {
                    setUrl(e.target.value);
                    setDownloadProbe(null);
                    setOutputManuallyEdited(false);
                  }}
                  onBlur={inspectDownloadUrl}
                  placeholder="https://servidor/arquivo.iso"
                  required
                />
                <button type="button" onClick={inspectDownloadUrl} disabled={inspectingUrl || !url.trim()}>
                  {inspectingUrl ? "Analisando…" : "Analisar"}
                </button>
              </div>
              {downloadProbe && (
                <small className="fieldHint downloadProbe">
                  {downloadProbe.suggested_name ?? "Nome não informado"} •{" "}
                  {downloadProbe.size ? formatBytes(downloadProbe.size) : "tamanho desconhecido"} •{" "}
                  {downloadProbe.accepts_ranges ? "HTTP Range / Multi-WAN" : "download direto"}
                </small>
              )}
            </label>

            <label>
              Arquivo de destino
              <div className="fieldWithButton">
                <input
                  value={output}
                  onChange={(e) => {
                    setOutput(e.target.value);
                    setOutputManuallyEdited(true);
                  }}
                  placeholder="Pasta Downloads deste Windows"
                  required
                />
                <button type="button" onClick={chooseDownloadDestination}>
                  Escolher pasta…
                </button>
              </div>
              <small className="fieldHint">
                O StorDown usa a pasta Downloads deste computador e tenta obter o nome real pelo servidor.
              </small>
            </label>

            <label>
              Agendar início — opcional
              <div className="scheduleField">
                <input
                  type="datetime-local"
                  value={downloadSchedule}
                  min={toLocalDateTimeInput(new Date())}
                  onChange={(event) => setDownloadSchedule(event.target.value)}
                />
                {downloadSchedule && (
                  <button type="button" onClick={() => setDownloadSchedule("")}>
                    Agora
                  </button>
                )}
              </div>
              <small className="fieldHint">
                Se preenchido, o download fica salvo no SQLite e inicia automaticamente no horário,
                inclusive após reiniciar o StorDown.
              </small>
            </label>

            <div className="grid2">
              <label>
                Limite de velocidade
                <input
                  type="number"
                  min={0}
                  value={downloadSpeedLimit}
                  onChange={(event) => setDownloadSpeedLimit(Number(event.target.value))}
                  placeholder="0 = ilimitado"
                />
                <small className="fieldHint">Mbps totais somando todas as WANs. 0 = ilimitado.</small>
              </label>

              <label>
                SHA256 esperado — opcional
                <input
                  value={expectedSha256}
                  onChange={(event) => setExpectedSha256(event.target.value)}
                  placeholder="64 caracteres hexadecimais"
                />
                <small className="fieldHint">Se informado, o arquivo só conclui se o hash bater.</small>
              </label>
            </div>

            <div className="grid2">
              <label>
                Conexões simultâneas por arquivo
                <input
                  type="number"
                  min={1}
                  max={64}
                  value={connections}
                  onChange={(e) => {
                    const value = Number(e.target.value);
                    setConnections(value);
                    window.localStorage.setItem("stordown.connections", String(value));
                    void syncRuntimePreferences(defaultDownloadDir, links, value);
                  }}
                />
                <small className="fieldHint">
                  O StorDown distribui os blocos entre as interfaces selecionadas automaticamente.
                </small>
              </label>

              <div className="selectedLinksSummary">
                <span>Interfaces usadas</span>
                <strong>{links.length || 0}</strong>
                <small>
                  {links.length > 1
                    ? "Multi-link ativo. Use 'Testar saídas' para confirmar Internet independente."
                    : links.length === 1
                      ? "Single-link ativo. Funciona normalmente em qualquer PC."
                      : "Selecione ao menos uma interface na barra lateral."}
                </small>
              </div>
            </div>

            <RoutePreview links={links} nics={nicByIp} probes={probeByIp} />

            <button className="primary" disabled={busy || links.length === 0}>
              {busy ? "Adicionando…" : "Adicionar à fila"}
            </button>
          </form>
        )}

        {view === "upload" && (
          <form className="downloadCard" onSubmit={submitUpload}>
            <div className="notice">
              <strong>Google Drive + Smart Multi-WAN</strong>
              <span>
                Lotes usam um pool adaptativo entre as WANs. Cada bloco resumível pode trocar de
                interface após falha, enquanto vários arquivos continuam em paralelo para somar upload.
              </span>
            </div>

            <label>
              Arquivos locais — um caminho por linha
              <div className="uploadPicker">
                <textarea
                  rows={5}
                  value={uploadFiles}
                  onChange={(e) => setUploadFiles(e.target.value)}
                  placeholder={"C:\\Uploads\\filme1.mkv\nC:\\Uploads\\filme2.mkv"}
                  required
                />
                <button type="button" onClick={chooseUploadFiles}>
                  Selecionar arquivos…
                </button>
              </div>
            </label>

            <div className="grid2 uploadGrid">
              <label>
                Limite de upload
                <input
                  type="number"
                  min={0}
                  value={uploadSpeedLimit}
                  onChange={(event) => setUploadSpeedLimit(Number(event.target.value))}
                  placeholder="0 = ilimitado"
                />
                <small className="fieldHint">Mbps totais do lote, compartilhados entre as WANs.</small>
              </label>
              <div />
            </div>

            <div className="grid2 uploadGrid">
              <label>
                Bloco
                <select value={chunkMiB} onChange={(e) => setChunkMiB(Number(e.target.value))}>
                  <option value={1}>1 MiB</option>
                  <option value={4}>4 MiB</option>
                  <option value={8}>8 MiB</option>
                  <option value={16}>16 MiB</option>
                  <option value={32}>32 MiB</option>
                </select>
              </label>

              <label>
                Pasta de destino
                <div className="driveDestinationField">
                  <div>
                    <strong>{driveDestinationLabel}</strong>
                    <small>{driveParentId ? `ID: ${driveParentId}` : "Raiz do Meu Drive"}</small>
                  </div>
                  <button
                    type="button"
                    onClick={openDriveBrowser}
                    disabled={!driveAuth.connected}
                  >
                    Escolher pasta…
                  </button>
                </div>
              </label>
            </div>

            {driveBrowserOpen && (
              <section className="driveBrowser">
                <div className="driveBrowserHeader">
                  <div>
                    <span>DESTINO GOOGLE DRIVE</span>
                    <strong>{driveBreadcrumbs.map((item) => item.name).join(" / ")}</strong>
                  </div>
                  <button type="button" onClick={() => setDriveBrowserOpen(false)}>
                    Fechar
                  </button>
                </div>

                <div className="driveRootTabs">
                  <button
                    type="button"
                    className={activeDriveId === null ? "active" : ""}
                    onClick={() => switchDrive(null)}
                  >
                    Meu Drive
                  </button>
                  {sharedDrives.map((drive) => (
                    <button
                      type="button"
                      key={drive.id}
                      className={activeDriveId === drive.id ? "active" : ""}
                      onClick={() => switchDrive(drive)}
                    >
                      {drive.name}
                    </button>
                  ))}
                </div>

                <div className="driveBrowserToolbar">
                  <button
                    type="button"
                    onClick={driveBrowserBack}
                    disabled={driveBreadcrumbs.length <= 1 || driveBrowserBusy}
                  >
                    ← Voltar
                  </button>
                  <button
                    type="button"
                    className="primary"
                    onClick={selectCurrentDriveFolder}
                    disabled={driveBrowserBusy}
                  >
                    Usar esta pasta
                  </button>
                </div>

                <div className="driveFolderList">
                  {driveBrowserBusy ? (
                    <div className="driveBrowserEmpty">Carregando pastas…</div>
                  ) : driveFolders.length === 0 ? (
                    <div className="driveBrowserEmpty">Nenhuma subpasta aqui.</div>
                  ) : (
                    driveFolders.map((folder) => (
                      <button
                        type="button"
                        className="driveFolderRow"
                        key={folder.id}
                        onClick={() => enterDriveFolder(folder)}
                      >
                        <span>📁</span>
                        <strong>{folder.name}</strong>
                        <small>Abrir →</small>
                      </button>
                    ))
                  )}
                </div>
              </section>
            )}

            <div className="driveAuthCard">
              <div>
                <span className="driveAuthLabel">CONTA GOOGLE DRIVE</span>
                <strong>
                  {driveAuth.connected ? "Conectado com OAuth + PKCE" : "Conta não conectada"}
                </strong>
                <small>
                  {driveAuth.connected
                    ? "Refresh token protegido no armazenamento seguro do Windows."
                    : "O login abre no navegador padrão; a senha nunca passa pelo StorDown."}
                </small>
              </div>

              <div className="driveAuthActions">
                {!driveAuth.connected ? (
                  <>
                    <button type="button" onClick={connectDrive} disabled={authBusy}>
                      {authBusy ? "Conectando…" : "Conectar Google Drive"}
                    </button>
                    <button type="button" onClick={restoreDrive} disabled={authBusy}>
                      Restaurar sessão
                    </button>
                  </>
                ) : (
                  <button type="button" onClick={disconnectDrive} disabled={authBusy}>
                    Desconectar
                  </button>
                )}
              </div>
            </div>

            {!driveAuth.connected && (
              <label>
                Google OAuth Client ID — desenvolvimento
                <input
                  value={driveClientId}
                  onChange={(e) => setDriveClientId(e.target.value)}
                  placeholder="Na versão distribuída, o Client ID do StorDown virá configurado"
                />
              </label>
            )}

            <label>
              IPs das interfaces
              <input
                value={bindIps}
                onChange={(e) => {
                  setBindIps(e.target.value);
                  setRouteTests([]);
                }}
                placeholder="Use Detectar placas ou informe os IPs"
              />
            </label>

            <RoutePreview links={links} nics={nicByIp} probes={probeByIp} />

            <button
              className="primary"
              disabled={busy || links.length === 0 || files.length === 0 || !driveAuth.connected}
            >
              {busy ? "Adicionando…" : `Adicionar ${files.length} arquivo(s) à fila`}
            </button>
          </form>
        )}

        {view === "cloud" && (
          <section className="cloudWorkspace">
            <div className="notice">
              <strong>Google Drive → download Multi-WAN</strong>
              <span>
                Arquivos binários usam o endpoint alt=media com HTTP Range, então o mesmo motor
                adaptativo do StorDown pode dividir o arquivo entre WAN1/WAN2, retomar partes e
                fazer failover de segmentos.
              </span>
            </div>

            <section className="sharedLinkImporter">
              <div className="sharedLinkFields">
                <label>
                  Link compartilhado do Google Drive
                  <input
                    value={sharedDriveLink}
                    onChange={(event) => setSharedDriveLink(event.target.value)}
                    placeholder="https://drive.google.com/file/d/.../view"
                  />
                </label>
                <button
                  type="button"
                  onClick={importSharedDriveLink}
                  disabled={sharedLinkBusy || !driveAuth.connected || !sharedDriveLink.trim()}
                >
                  {sharedLinkBusy ? "Abrindo…" : "Importar link"}
                </button>
              </div>

              {sharedLinkItem && (
                <article className="sharedLinkResult">
                  <div className="cloudItemIcon">{sharedLinkItem.is_folder ? "📁" : "🔗"}</div>
                  <div className="cloudItemInfo">
                    <strong>{sharedLinkItem.name}</strong>
                    <small>
                      {sharedLinkItem.is_folder
                        ? "Pasta compartilhada"
                        : sharedLinkItem.is_google_workspace
                          ? "Documento Google Workspace compartilhado"
                          : `${formatBytes(sharedLinkItem.size ?? 0)} • arquivo compartilhado`}
                    </small>
                  </div>
                  <div className="rowActions">
                    {sharedLinkItem.is_folder ? (
                      <button type="button" onClick={() => openSharedDriveItem(sharedLinkItem)}>
                        Abrir pasta
                      </button>
                    ) : sharedLinkItem.is_google_workspace ? (
                      <button
                        type="button"
                        onClick={() => openWorkspaceExport(sharedLinkItem)}
                        disabled={!sharedLinkItem.can_download}
                      >
                        Exportar
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="primary"
                        onClick={() => openSharedDriveItem(sharedLinkItem)}
                        disabled={!sharedLinkItem.can_download || links.length === 0}
                      >
                        Baixar
                      </button>
                    )}
                    <button type="button" onClick={() => setSharedLinkItem(null)}>
                      Limpar
                    </button>
                  </div>
                </article>
              )}
            </section>

            {!driveAuth.connected ? (
              <section className="driveAuthCard">
                <div>
                  <span className="driveAuthLabel">CONTA GOOGLE DRIVE</span>
                  <strong>Conta não conectada</strong>
                  <small>
                    O download de conteúdo exige nova autorização de leitura do Drive.
                  </small>
                </div>
                <div className="driveAuthActions">
                  <button type="button" onClick={connectDrive} disabled={authBusy}>
                    {authBusy ? "Conectando…" : "Conectar Google Drive"}
                  </button>
                  <button type="button" onClick={restoreDrive} disabled={authBusy}>
                    Restaurar sessão
                  </button>
                </div>
              </section>
            ) : (
              <>
                <div className="cloudToolbar">
                  <div className="driveRootTabs">
                    <button
                      type="button"
                      className={cloudDriveId === null ? "active" : ""}
                      onClick={() => switchCloudDrive(null)}
                    >
                      Meu Drive
                    </button>
                    {sharedDrives.map((drive) => (
                      <button
                        type="button"
                        key={drive.id}
                        className={cloudDriveId === drive.id ? "active" : ""}
                        onClick={() => switchCloudDrive(drive)}
                      >
                        {drive.name}
                      </button>
                    ))}
                  </div>
                  <button type="button" onClick={() => {
                    const current = cloudBreadcrumbs[cloudBreadcrumbs.length - 1];
                    loadCloudItems(
                      current?.id ?? null,
                      cloudDriveId,
                      current?.resource_key ?? null,
                    );
                  }}>
                    Atualizar
                  </button>
                </div>

                <div className="cloudPathBar">
                  <button
                    type="button"
                    onClick={cloudBack}
                    disabled={cloudBreadcrumbs.length <= 1 || cloudBusy}
                  >
                    ← Voltar
                  </button>
                  <strong>{cloudBreadcrumbs.map((item) => item.name).join(" / ")}</strong>
                </div>

                <div className="cloudDownloadOptions">
                  <label>
                    Conexões por arquivo
                    <input
                      type="number"
                      min={1}
                      max={64}
                      value={connections}
                      onChange={(event) => setConnections(Number(event.target.value))}
                    />
                  </label>
                  <label>
                    Limite de download
                    <input
                      type="number"
                      min={0}
                      value={downloadSpeedLimit}
                      onChange={(event) => setDownloadSpeedLimit(Number(event.target.value))}
                    />
                    <small className="fieldHint">Mbps totais. 0 = ilimitado.</small>
                  </label>
                </div>

                {workspaceExportItem && (
                  <section className="workspaceExportPanel">
                    <div>
                      <span>EXPORTAR GOOGLE WORKSPACE</span>
                      <strong>{workspaceExportItem.name}</strong>
                      <small>
                        O Drive não aceita HTTP Range em exportações; este item usa uma conexão única.
                        O endpoint clássico de exportação do Google limita o resultado a 10 MB.
                      </small>
                    </div>

                    <div className="workspaceExportFormats">
                      {workspaceExportFormats.length === 0 ? (
                        <span className="workspaceExportUnavailable">
                          Formato ainda não suportado pelo export direto.
                        </span>
                      ) : (
                        workspaceExportFormats.map((format) => (
                          <button
                            type="button"
                            key={format.mime_type}
                            onClick={() => exportWorkspaceItem(format)}
                            disabled={workspaceExportBusy || links.length === 0}
                          >
                            {format.label}
                          </button>
                        ))
                      )}
                      <button
                        type="button"
                        className="dangerButton"
                        onClick={() => {
                          setWorkspaceExportItem(null);
                          setWorkspaceExportFormats([]);
                        }}
                        disabled={workspaceExportBusy}
                      >
                        Fechar
                      </button>
                    </div>
                  </section>
                )}

                <div className="cloudItemList">
                  {cloudBusy ? (
                    <div className="driveBrowserEmpty">Carregando Google Drive…</div>
                  ) : cloudItems.length === 0 ? (
                    <div className="driveBrowserEmpty">Esta pasta está vazia.</div>
                  ) : (
                    cloudItems.map((item) => (
                      <article className="cloudItemRow" key={item.id}>
                        <div className="cloudItemIcon">{item.is_folder ? "📁" : "📄"}</div>
                        <div className="cloudItemInfo">
                          <strong title={item.name}>{item.name}</strong>
                          <small>
                            {item.is_folder
                              ? "Pasta"
                              : item.is_google_workspace
                                ? "Documento Google Workspace"
                                : `${formatBytes(item.size ?? 0)} • ${item.mime_type}`}
                          </small>
                        </div>
                        <div className="rowActions">
                          {item.is_folder ? (
                            <button type="button" onClick={() => enterCloudFolder(item)}>
                              Abrir
                            </button>
                          ) : item.is_google_workspace ? (
                            <button
                              type="button"
                              disabled={!item.can_download || workspaceExportBusy}
                              onClick={() => openWorkspaceExport(item)}
                            >
                              Exportar
                            </button>
                          ) : (
                            <button
                              type="button"
                              className="primary"
                              disabled={!item.can_download || links.length === 0}
                              onClick={() => downloadCloudItem(item)}
                            >
                              Baixar
                            </button>
                          )}
                        </div>
                      </article>
                    ))
                  )}
                </div>
              </>
            )}
          </section>
        )}

        {view === "queue" && (
          <TransferList
            records={queuedRecords}
            emptyText="A fila está vazia."
            selectedId={activeTransferId}
            onSelect={setActiveTransferId}
            onPause={pauseTransfer}
            onResume={resumeTransfer}
            onCancel={cancelTransfer}
            onDelete={deleteHistory}
            liveStats={liveTransferStats}
          />
        )}

        {view === "scheduled" && (
          <>
            <div className="listToolbar">
              <span>{scheduledRecords.length} download(s) agendado(s)</span>
              <small>Os horários ficam persistidos e são restaurados ao iniciar o StorDown.</small>
            </div>
            <TransferList
              records={scheduledRecords}
              emptyText="Nenhum download agendado."
              selectedId={activeTransferId}
              onSelect={setActiveTransferId}
              onPause={pauseTransfer}
              onResume={resumeTransfer}
              onCancel={cancelTransfer}
              onDelete={deleteHistory}
              liveStats={liveTransferStats}
            />
          </>
        )}

        {view === "finished" && (
          <>
            <div className="listToolbar">
              <span>{finishedRecords.length} registro(s)</span>
              <button type="button" onClick={clearFinished} disabled={finishedRecords.length === 0}>
                Limpar finalizados
              </button>
            </div>
            <TransferList
              records={finishedRecords}
              emptyText="Ainda não há transferências finalizadas."
              selectedId={activeTransferId}
              onSelect={setActiveTransferId}
              onPause={pauseTransfer}
              onResume={resumeTransfer}
              onCancel={cancelTransfer}
              onDelete={deleteHistory}
              liveStats={liveTransferStats}
            />
          </>
        )}

        {view === "extension" && (
          <section className="extensionWorkspace">
            <section className="downloadCard browserInstaller">
              <div className="notice">
                <strong>Extensão Chrome / Edge incluída</strong>
                <span>
                  O instalador do StorDown agora leva a extensão e o Native Messaging Host junto.
                  Não é necessário baixar outro pacote para começar.
                </span>
              </div>

              <div className="extensionSteps">
                <span><b>1</b> Clique em <strong>Preparar e conectar</strong>. O StorDown extrai a extensão e registra o Native Host automaticamente.</span>
                <span><b>2</b> No Chrome/Edge, ative o modo desenvolvedor, escolha <strong>Carregar sem compactação</strong> e selecione a pasta aberta pelo StorDown.</span>
                <span><b>3</b> Pronto. O ID da extensão é fixo e igual em qualquer computador; não precisa copiar nem configurar manualmente.</span>
              </div>

              <div className="browserInstallerActions">
                <button
                  type="button"
                  className="primary"
                  onClick={prepareBrowserExtension}
                  disabled={browserInstallBusy}
                >
                  {browserInstallBusy ? "Preparando…" : "Preparar e conectar"}
                </button>
                <button type="button" onClick={() => openExtensionsPage("chrome")}>
                  Chrome
                </button>
                <button type="button" onClick={() => openExtensionsPage("edge")}>
                  Edge
                </button>
              </div>

              {browserExtensionPrepared && (
                <div className="browserInstallResult extensionPrepared">
                  <strong>Extensão pronta para carregar</strong>
                  <code>{browserExtensionPrepared.extension_dir}</code>
                </div>
              )}

              <div className="browserInstallResult extensionIdentity">
                <strong>ID fixo da extensão</strong>
                <code>{browserExtensionId}</code>
                <span>Esse mesmo ID é usado no Chrome e Edge em qualquer PC.</span>
              </div>

              <div className="browserInstallerActions">
                <button
                  type="button"
                  onClick={installBrowserIntegration}
                  disabled={browserInstallBusy}
                >
                  Registrar novamente
                </button>
              </div>

              {browserIntegration && (
                <div className="browserInstallResult">
                  <strong>Extensão conectada ao aplicativo</strong>
                  <span>ID: {browserIntegration.extension_id}</span>
                  <span>Chrome: {browserIntegration.chrome_registered ? "registrado" : "não registrado"}</span>
                  <span>Edge: {browserIntegration.edge_registered ? "registrado" : "não registrado"}</span>
                  <code>{browserIntegration.native_host_path}</code>
                </div>
              )}
            </section>

            <section className="extensionHelp">
              <div>
                <strong>Integração do navegador</strong>
                <span>
                  Depois de conectar, o botão direito ganha “Baixar com StorDown” e a captura
                  automática pode substituir o download do Chrome/Edge.
                </span>
              </div>
              <div className="extensionFeatureGrid">
                <span>✓ captura de links</span>
                <span>✓ downloads autenticados por site</span>
                <span>✓ cookies somente com sua autorização</span>
                <span>✓ lote de links da página</span>
                <span>✓ fila e progresso no desktop</span>
                <span>✓ usa as mesmas interfaces escolhidas no StorDown</span>
              </div>
            </section>
          </section>
        )}

        {view === "settings" && (
          <section className="settingsGrid">
            <form className="downloadCard ruleEditor" onSubmit={saveRule}>
              <div className="notice">
                <strong>Categorias automáticas de download</strong>
                <span>
                  Downloads capturados pelo Chrome/Edge podem ir automaticamente para uma pasta
                  conforme a extensão do arquivo.
                </span>
              </div>

              <div className="grid2">
                <label>
                  Categoria
                  <input
                    value={ruleName}
                    onChange={(event) => setRuleName(event.target.value)}
                    placeholder="Vídeos"
                    required
                  />
                </label>

                <label>
                  Extensões
                  <input
                    value={ruleExtensions}
                    onChange={(event) => setRuleExtensions(event.target.value)}
                    placeholder="mkv, mp4, avi"
                    required
                  />
                </label>
              </div>

              <label>
                Pasta de destino
                <div className="fieldWithButton">
                  <input
                    value={ruleDestination}
                    onChange={(event) => setRuleDestination(event.target.value)}
                    required
                  />
                  <button type="button" onClick={chooseRuleFolder}>
                    Selecionar…
                  </button>
                </div>
              </label>

              <div className="ruleOptions">
                <label className="checkRow">
                  <input
                    type="checkbox"
                    checked={ruleEnabled}
                    onChange={(event) => setRuleEnabled(event.target.checked)}
                  />
                  Regra ativa
                </label>

                <label>
                  Prioridade
                  <input
                    type="number"
                    min={0}
                    max={9999}
                    value={rulePriority}
                    onChange={(event) => setRulePriority(Number(event.target.value))}
                  />
                </label>
              </div>

              <div className="ruleFormActions">
                <button className="primary" type="submit">
                  {ruleId ? "Salvar alterações" : "Criar categoria"}
                </button>
                {ruleId !== null && (
                  <button type="button" onClick={resetRuleForm}>
                    Nova categoria
                  </button>
                )}
              </div>
            </form>

            <section className="ruleList">
              <div className="listToolbar">
                <span>{downloadRules.length} categoria(s)</span>
                <small>Menor prioridade numérica é aplicada primeiro.</small>
              </div>

              {downloadRules.length === 0 ? (
                <div className="emptyState">
                  Nenhuma categoria criada. O navegador continuará usando a pasta Downloads padrão.
                </div>
              ) : (
                downloadRules.map((rule) => (
                  <article className="ruleRow" key={rule.id}>
                    <div className="ruleInfo">
                      <div className="transferTitle">
                        <strong>{rule.name}</strong>
                        <span className={`ruleState ${rule.enabled ? "enabled" : "disabled"}`}>
                          {rule.enabled ? "Ativa" : "Desativada"}
                        </span>
                      </div>
                      <div className="ruleMeta">
                        <span>{rule.extensions.map((ext) => `.${ext}`).join("  ")}</span>
                        <span>{rule.destination}</span>
                        <span>Prioridade {rule.priority}</span>
                      </div>
                    </div>
                    <div className="rowActions">
                      <button type="button" onClick={() => editRule(rule)}>Editar</button>
                      <button
                        type="button"
                        className="dangerButton"
                        onClick={() => removeRule(rule.id)}
                      >
                        Remover
                      </button>
                    </div>
                  </article>
                ))
              )}
            </section>
          </section>
        )}

        {selectedRecord && (activeProgress.length > 0 || activeStatuses.has(selectedRecord.status)) && (
          <TransferTelemetry
            record={selectedRecord}
            items={activeProgress}
            totalTransferred={totalTransferred}
            totalBytes={totalBytes}
            linkSpeeds={linkSpeeds}
            onPause={() => pauseTransfer(selectedRecord.id)}
            onResume={() => resumeTransfer(selectedRecord.id)}
            onCancel={() => cancelTransfer(selectedRecord.id)}
          />
        )}

        {view === "home" && (
          <section className="metrics">
            <article>
              <span>Fila</span>
              <strong>{queuedRecords.length}</strong>
              <small>download + upload unificados</small>
            </article>
            <article>
              <span>Execução</span>
              <strong>2 simultâneas</strong>
              <small>com Multi-WAN e telemetria</small>
            </article>
            <article>
              <span>Histórico</span>
              <strong>{finishedRecords.length}</strong>
              <small>persistente em SQLite</small>
            </article>
          </section>
        )}

      </section>
    </main>
  );
}

function RoutePreview({
  links,
  nics,
  probes,
}: {
  links: string[];
  nics: Map<string, NetworkInterfaceInfo>;
  probes: Map<string, LinkProbeStatus>;
}) {
  return (
    <div className="routePreview">
      {links.map((ip, index) => {
        const nic = nics.get(ip);
        const probe = probes.get(ip);

        return (
          <div key={ip}>
            <span>{nic?.name ?? `Link ${index + 1}`}</span>
            <strong>{ip}</strong>
            <small>
              {probe?.public_ip
                ? `WAN ${index + 1}: ${probe.public_ip} • ${probe.latency_ms ?? "?"} ms`
                : "Saída ainda não testada"}
            </small>
          </div>
        );
      })}
    </div>
  );
}

function ActiveTransferDock({
  records,
  liveStats,
  selectedId,
  onSelect,
  onPause,
  onResume,
  onCancel,
}: {
  records: TransferRecord[];
  liveStats: Record<string, LiveTransferStats>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onCancel: (id: string) => void;
}) {
  if (records.length === 0) return null;

  const record =
    records.find((item) => item.id === selectedId) ??
    records.find((item) => item.status === "running") ??
    records[0];

  const live = liveStats[record.id];
  const transferred = Math.max(record.bytes_transferred, live?.bytesTransferred ?? 0);
  const total = live?.totalBytes ?? record.total_bytes ?? null;
  const percent = total && total > 0 ? Math.min(100, (transferred / total) * 100) : 0;
  const speed = record.status === "running" ? live?.speed ?? 0 : 0;
  const eta =
    total && speed > 0 && transferred < total ? (total - transferred) / speed : null;

  return (
    <section className="activeTransferDock">
      <button className="dockMain" type="button" onClick={() => onSelect(record.id)}>
        <div className="dockIcon">{record.direction === "upload" ? "↑" : "↓"}</div>
        <div className="dockBody">
          <div className="dockTitle">
            <strong title={record.name}>{record.name}</strong>
            <span>{percent > 0 ? `${percent.toFixed(1)}%` : statusShortLabel(record.status)}</span>
          </div>
          <div className="dockTrack">
            <div className="dockFill" style={{ width: `${percent}%` }} />
          </div>
          <div className="dockMeta">
            <span>{formatBytes(transferred)}{total ? ` / ${formatBytes(total)}` : ""}</span>
            <span>{formatSpeed(speed)}</span>
            {eta !== null && <span>ETA {formatDuration(eta)}</span>}
            <span>{record.bind_ips.length || 1} link(s)</span>
          </div>
        </div>
      </button>

      <div className="dockActions">
        {record.status === "running" && (
          <button type="button" onClick={() => onPause(record.id)}>Ⅱ</button>
        )}
        {(record.status === "paused" || record.status === "interrupted") && (
          <button type="button" onClick={() => onResume(record.id)}>▶</button>
        )}
        {activeStatuses.has(record.status) && (
          <button type="button" className="dangerButton" onClick={() => onCancel(record.id)}>×</button>
        )}
      </div>

      {records.length > 1 && <span className="dockQueueCount">+{records.length - 1} na fila</span>}
    </section>
  );
}

function statusShortLabel(status: string) {
  const labels: Record<string, string> = {
    queued: "Na fila",
    running: "Transferindo",
    paused: "Pausado",
    scheduled: "Agendado",
    interrupted: "Interrompido",
  };
  return labels[status] ?? status;
}

function TransferList({
  records,
  emptyText,
  selectedId,
  onSelect,
  onPause,
  onResume,
  onCancel,
  onDelete,
  liveStats = {},
}: {
  records: TransferRecord[];
  emptyText: string;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onCancel: (id: string) => void;
  onDelete: (id: string) => void;
  liveStats?: Record<string, LiveTransferStats>;
}) {
  if (records.length === 0) {
    return <div className="emptyState">{emptyText}</div>;
  }

  return (
    <section className="transferList managerTransferList">
      {records.map((record) => {
        const live = liveStats[record.id];
        const currentBytes = Math.max(record.bytes_transferred, live?.bytesTransferred ?? 0);
        const total = live?.totalBytes ?? record.total_bytes ?? null;
        const percent =
          total && total > 0
            ? Math.min(100, (currentBytes / total) * 100)
            : 0;
        const speed = record.status === "running" ? live?.speed ?? 0 : 0;
        const remaining = total ? Math.max(0, total - currentBytes) : 0;
        const etaSeconds = speed > 0 && remaining > 0 ? remaining / speed : null;

        return (
          <article
            key={record.id}
            className={`transferRow managerTransferRow ${selectedId === record.id ? "selected" : ""}`}
            onClick={() => onSelect(record.id)}
          >
            <div className="transferKind">
              <span>{record.direction === "upload" ? "↑" : "↓"}</span>
            </div>

            <div className="transferMain">
              <div className="transferTitle">
                <strong title={record.name}>{record.name}</strong>
                <StatusBadge status={record.status} />
              </div>

              <div className="miniTrack">
                <div className="miniFill" style={{ width: `${percent}%` }} />
              </div>

              <div className="transferMeta transferMetaManager">
                <span>{record.provider === "google_drive" ? "Google Drive" : "HTTP/HTTPS"}</span>
                <span>
                  {formatBytes(currentBytes)}
                  {total ? ` / ${formatBytes(total)}` : ""}
                </span>
                <span>{percent > 0 ? `${percent.toFixed(1)}%` : "—"}</span>
                <span className="liveSpeed">{speed > 0 ? formatSpeed(speed) : "0 B/s"}</span>
                {etaSeconds !== null && <span>Restante {formatDuration(etaSeconds)}</span>}
                <span>{record.bind_ips.length || 1} link(s)</span>
                {record.max_bytes_per_second ? (
                  <span>Limite {formatMbps(record.max_bytes_per_second)}</span>
                ) : null}
                {record.expected_sha256 ? <span>SHA256 ✓</span> : null}
                <span>
                  {record.status === "scheduled" && record.scheduled_at
                    ? `Inicia ${formatDate(record.scheduled_at)}`
                    : formatDate(record.updated_at)}
                </span>
              </div>

              {record.error && <div className="rowError">{record.error}</div>}
            </div>

            <div className="rowActions" onClick={(event) => event.stopPropagation()}>
              {record.status === "running" && (
                <button type="button" onClick={() => onPause(record.id)}>Pausar</button>
              )}
              {record.status === "paused" && (
                <button type="button" onClick={() => onResume(record.id)}>Retomar</button>
              )}
              {record.status === "interrupted" &&
                record.provider === "google_drive" &&
                record.direction === "upload" && (
                  <button type="button" onClick={() => onResume(record.id)}>
                    Retomar upload
                  </button>
                )}
              {(record.status === "running" || record.status === "paused" || record.status === "queued" || record.status === "scheduled") && (
                <button type="button" className="dangerButton" onClick={() => onCancel(record.id)}>
                  Cancelar
                </button>
              )}
              {finishedStatuses.has(record.status) && (
                <button type="button" onClick={() => onDelete(record.id)}>Remover</button>
              )}
            </div>
          </article>
        );
      })}
    </section>
  );
}

function StatusBadge({ status }: { status: string }) {
  const label: Record<string, string> = {
    queued: "Na fila",
    running: "Transferindo",
    paused: "Pausado",
    scheduled: "Agendado",
    interrupted: "Interrompido",
    completed: "Concluído",
    failed: "Falhou",
    cancelled: "Cancelado",
  };

  return <span className={`statusBadge status-${status}`}>{label[status] ?? status}</span>;
}

function TransferTelemetry({
  record,
  items,
  totalTransferred,
  totalBytes,
  linkSpeeds,
  onPause,
  onResume,
  onCancel,
}: {
  record: TransferRecord;
  items: TransferProgress[];
  totalTransferred: number;
  totalBytes: number;
  linkSpeeds: Record<string, number>;
  onPause: () => void;
  onResume: () => void;
  onCancel: () => void;
}) {
  const overallPercent =
    totalBytes > 0 ? Math.min(100, (totalTransferred / totalBytes) * 100) : 0;

  return (
    <section className="telemetryPanel">
      <div className="telemetryHeader">
        <div>
          <span>TRANSFERÊNCIA SELECIONADA</span>
          <strong>{overallPercent.toFixed(1)}%</strong>
        </div>

        <div className="telemetryActions">
          <div className="aggregateSpeed">
            {record.status === "paused"
              ? "Pausado"
              : formatSpeed(Object.values(linkSpeeds).reduce((sum, speed) => sum + speed, 0))}
          </div>

          {record.status === "running" && (
            <button type="button" onClick={onPause}>Pausar</button>
          )}
          {record.status === "paused" && (
            <button type="button" onClick={onResume}>Retomar</button>
          )}
          {record.status === "interrupted" &&
            record.provider === "google_drive" &&
            record.direction === "upload" && (
              <button type="button" onClick={onResume}>Retomar upload</button>
            )}
          {(record.status === "running" || record.status === "paused" || record.status === "queued") && (
            <button type="button" className="dangerButton" onClick={onCancel}>Cancelar</button>
          )}
        </div>
      </div>

      <div className="progressTrack">
        <div className="progressFill" style={{ width: `${overallPercent}%` }} />
      </div>

      <div className="telemetrySummary">
        <span>{formatBytes(totalTransferred)} transferidos</span>
        <span>{totalBytes > 0 ? formatBytes(totalBytes) : "Tamanho desconhecido"}</span>
      </div>

      {items.length > 0 && (
        <div className="telemetryItems">
          {items.map((item) => {
            const percent =
              item.total_bytes && item.total_bytes > 0
                ? Math.min(100, (item.bytes_transferred / item.total_bytes) * 100)
                : 0;

            return (
              <article key={`${item.transfer_id}:${item.item}`}>
                <div className="itemTop">
                  <strong title={item.item}>{item.item}</strong>
                  <span>{item.completed ? "Concluído" : `${percent.toFixed(1)}%`}</span>
                </div>
                <div className="miniTrack">
                  <div className="miniFill" style={{ width: `${percent}%` }} />
                </div>
                <div className="itemMeta">
                  <span>
                    {item.direction === "upload" ? "Upload" : "Download"} • {item.link_name}
                    {item.phase.startsWith("failover") ? " • Failover" : ""}
                  </span>
                  <span>
                    {formatBytes(item.bytes_transferred)}
                    {item.total_bytes ? ` / ${formatBytes(item.total_bytes)}` : ""}
                  </span>
                </div>
              </article>
            );
          })}
        </div>
      )}

      <div className="wanTelemetry">
        {record.bind_ips.map((ip) => (
          <div key={ip}>
            <span>{ip}</span>
            <strong>{formatSpeed(linkSpeeds[ip] ?? 0)}</strong>
          </div>
        ))}
      </div>
    </section>
  );
}

function viewTitle(view: View) {
  if (view === "home") return "Gerenciador";
  if (view === "download") return "Novo download";
  if (view === "upload") return "Upload para Google Drive";
  if (view === "cloud") return "Google Drive";
  if (view === "queue") return "Fila de transferências";
  if (view === "scheduled") return "Agendador";
  if (view === "extension") return "Extensão do navegador";
  if (view === "settings") return "Configurações";
  return "Histórico";
}

function viewSubtitle(view: View) {
  if (view === "home") {
    return "Acompanhe downloads e uploads em tempo real, com velocidade, progresso, ETA e uso das conexões.";
  }
  if (view === "download") {
    return "Adicione downloads HTTP/HTTPS segmentados à fila Multi-WAN.";
  }
  if (view === "upload") {
    return "Uploads resumíveis do Google Drive com sessão persistente, Multi-WAN e retomada após reiniciar o Windows.";
  }
  if (view === "cloud") {
    return "Navegue no Drive, importe links compartilhados, baixe blobs com Multi-WAN e exporte Workspace.";
  }
  if (view === "queue") {
    return "Downloads e uploads em uma fila única, com até duas transferências simultâneas.";
  }
  if (view === "scheduled") {
    return "Programe downloads HTTP/HTTPS para começar automaticamente mais tarde.";
  }
  if (view === "extension") {
    return "Instale e conecte a extensão Chrome/Edge que acompanha o StorDown.";
  }
  if (view === "settings") {
    return "Preferências e categorias automáticas para organizar downloads.";
  }
  return "Transferências concluídas, canceladas e com falha ficam salvas entre reinicializações.";
}

function toLocalDateTimeInput(date: Date) {
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

function suggestedDownloadName(url: string) {
  try {
    const parsed = new URL(url);
    const last = parsed.pathname.split("/").filter(Boolean).pop();
    return last ? decodeURIComponent(last) : "download.bin";
  } catch {
    return "download.bin";
  }
}

function formatBytes(value: number) {
  if (!Number.isFinite(value) || value <= 0) return "0 B";

  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(units.length - 1, Math.floor(Math.log(value) / Math.log(1024)));
  const amount = value / 1024 ** index;

  return `${amount >= 100 || index === 0 ? amount.toFixed(0) : amount.toFixed(1)} ${units[index]}`;
}

function formatMbps(bytesPerSecond: number) {
  return `${((bytesPerSecond * 8) / 1_000_000).toFixed(0)} Mbps`;
}

function formatSpeed(bytesPerSecond: number) {
  return `${formatBytes(bytesPerSecond)}/s`;
}

function fileNameFromPath(path: string) {
  const normalized = path.replace(/\//g, "\\");
  return normalized.split("\\").filter(Boolean).pop() ?? "";
}

function parentDirectory(path: string) {
  const normalized = path.replace(/\//g, "\\");
  const index = normalized.lastIndexOf("\\");
  return index > 0 ? normalized.slice(0, index) : "";
}

function joinWindowsPath(directory: string, name: string) {
  const cleanDir = directory.trim().replace(/[\\/]+$/, "");
  const cleanName = name.trim().replace(/[\\/:*?"<>|]/g, "_") || "download.bin";
  return cleanDir ? `${cleanDir}\\${cleanName}` : cleanName;
}

function formatDuration(seconds: number) {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0s";
  const rounded = Math.ceil(seconds);
  const hours = Math.floor(rounded / 3600);
  const minutes = Math.floor((rounded % 3600) / 60);
  const secs = rounded % 60;

  if (hours > 0) return `${hours}h ${minutes}min`;
  if (minutes > 0) return `${minutes}min ${secs}s`;
  return `${secs}s`;
}

function formatDate(epochSeconds: number) {
  if (!epochSeconds) return "—";
  return new Date(epochSeconds * 1000).toLocaleString("pt-BR", {
    day: "2-digit",
    month: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}
