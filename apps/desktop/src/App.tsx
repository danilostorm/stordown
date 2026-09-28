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

type SpeedWindow = {
  startedAt: number;
  bytes: number;
};

type View = "download" | "upload" | "queue" | "scheduled" | "finished" | "settings";

const activeStatuses = new Set(["scheduled", "queued", "running", "paused", "interrupted"]);
const finishedStatuses = new Set(["completed", "failed", "cancelled"]);

export default function App() {
  const [view, setView] = useState<View>("download");
  const [url, setUrl] = useState("");
  const [output, setOutput] = useState("C:\\Downloads\\arquivo.bin");
  const [connections, setConnections] = useState(8);
  const [downloadSchedule, setDownloadSchedule] = useState("");
  const [downloadSpeedLimit, setDownloadSpeedLimit] = useState(0);
  const [expectedSha256, setExpectedSha256] = useState("");
  const [bindIps, setBindIps] = useState("192.168.30.101, 192.168.30.102");
  const [status, setStatus] = useState("Pronto");
  const [busy, setBusy] = useState(false);
  const [networkBusy, setNetworkBusy] = useState(false);
  const [detectedNics, setDetectedNics] = useState<NetworkInterfaceInfo[]>([]);
  const [routeTests, setRouteTests] = useState<LinkProbeStatus[]>([]);

  const [uploadFiles, setUploadFiles] = useState(
    "C:\\Uploads\\arquivo1.mkv\nC:\\Uploads\\arquivo2.mkv",
  );
  const [driveParentId, setDriveParentId] = useState("");
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
  const [ruleDestination, setRuleDestination] = useState("C:\\Downloads\\Vídeos");
  const [ruleEnabled, setRuleEnabled] = useState(true);
  const [rulePriority, setRulePriority] = useState(100);
  const speedWindows = useRef<Record<string, SpeedWindow>>({});

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
    reloadTransfers();
    reloadDownloadRules();

    let stopProgress: undefined | (() => void);
    let stopList: undefined | (() => void);

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

      if (elapsed >= 750 || payload.completed) {
        const bytesPerSecond = elapsed > 0 ? (currentWindow.bytes * 1000) / elapsed : 0;
        setLinkSpeeds((current) => ({ ...current, [key]: bytesPerSecond }));
        speedWindows.current[key] = { startedAt: now, bytes: 0 };
      } else {
        speedWindows.current[key] = currentWindow;
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
      stopProgress?.();
      stopList?.();
    };
  }, []);

  async function detectNetworks() {
    setNetworkBusy(true);
    setStatus("Detectando placas de rede físicas do Windows…");

    try {
      const nics = await invoke<NetworkInterfaceInfo[]>("list_network_interfaces");
      setDetectedNics(nics);
      setRouteTests([]);

      if (nics.length > 0) {
        setBindIps(nics.map((nic) => nic.ipv4).join(", "));
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

  async function chooseDownloadDestination() {
    try {
      const picked = await invoke<string | null>("pick_download_destination", {
        suggestedName: suggestedDownloadName(url),
      });
      if (picked) {
        setOutput(picked);
        setStatus("Destino selecionado");
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
      await invoke("resume_transfer", { transferId });
      setStatus("Transferência retomada");
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

  function resetRuleForm() {
    setRuleId(null);
    setRuleName("Vídeos");
    setRuleExtensions("mkv, mp4, avi, mov");
    setRuleDestination("C:\\Downloads\\Vídeos");
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
            className={`navItem ${view === "download" ? "active" : ""}`}
            onClick={() => setView("download")}
          >
            Downloads
          </button>
          <button
            className={`navItem ${view === "upload" ? "active" : ""}`}
            onClick={() => setView("upload")}
          >
            Uploads
          </button>
          <button
            className={`navItem navCount ${view === "queue" ? "active" : ""}`}
            onClick={() => setView("queue")}
          >
            <span>Fila</span>
            <b>{queuedRecords.length}</b>
          </button>
          <button
            className={`navItem navCount ${view === "scheduled" ? "active" : ""}`}
            onClick={() => setView("scheduled")}
          >
            <span>Agendador</span>
            <b>{scheduledRecords.length}</b>
          </button>
          <button
            className={`navItem navCount ${view === "finished" ? "active" : ""}`}
            onClick={() => setView("finished")}
          >
            <span>Finalizados</span>
            <b>{finishedRecords.length}</b>
          </button>
          <button className="navItem" disabled>Cloud</button>
          <button
            className={`navItem ${view === "settings" ? "active" : ""}`}
            onClick={() => setView("settings")}
          >
            Configurações
          </button>
        </nav>

        <div className="networkCard">
          <span>Multi-Link</span>
          <strong>{links.length} links configurados</strong>
          <div className="smartWanBadge">Smart balance + failover automático</div>

          {links.map((ip, index) => {
            const nic = nicByIp.get(ip);
            const probe = probeByIp.get(ip);

            return (
              <div className="networkLink" key={ip}>
                <div className="linkRow">
                  <i className={probe?.error ? "bad" : ""} />
                  <span>{nic?.name ?? `Ethernet ${index + 1}`}</span>
                  <code>{ip}</code>
                </div>
                <div className="linkMeta">
                  {nic?.link_speed && <span>{nic.link_speed}</span>}
                  {probe?.public_ip && (
                    <span>
                      WAN: {probe.public_ip} • {probe.latency_ms ?? "?"} ms
                    </span>
                  )}
                  {probe?.error && <span className="errorText">Sem saída</span>}
                  {linkSpeeds[ip] !== undefined && (
                    <span className="speedText">{formatSpeed(linkSpeeds[ip])}</span>
                  )}
                </div>
              </div>
            );
          })}

          <div className="networkActions">
            <button type="button" onClick={detectNetworks} disabled={networkBusy}>
              Detectar placas
            </button>
            <button
              type="button"
              onClick={testRoutes}
              disabled={networkBusy || links.length === 0}
            >
              Testar WANs
            </button>
          </div>
        </div>
      </aside>

      <section className="content">
        <header>
          <div>
            <p className="eyebrow">STORDOWN V0.2 DEV</p>
            <h1>{viewTitle(view)}</h1>
            <p className="subtitle">{viewSubtitle(view)}</p>
          </div>
          <div className="statusPill">{status}</div>
        </header>

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
              <input
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://servidor/arquivo.iso"
                required
              />
            </label>

            <label>
              Salvar em
              <div className="fieldWithButton">
                <input value={output} onChange={(e) => setOutput(e.target.value)} required />
                <button type="button" onClick={chooseDownloadDestination}>
                  Procurar…
                </button>
              </div>
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
                Conexões
                <input
                  type="number"
                  min={1}
                  max={64}
                  value={connections}
                  onChange={(e) => setConnections(Number(e.target.value))}
                />
              </label>

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
                Pasta de destino (ID opcional)
                <input
                  value={driveParentId}
                  onChange={(e) => setDriveParentId(e.target.value)}
                  placeholder="ID da pasta no Google Drive"
                />
              </label>
            </div>

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
            />
          </>
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
                : `→ regra UDM → WAN ${index + 1}`}
            </small>
          </div>
        );
      })}
    </div>
  );
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
}: {
  records: TransferRecord[];
  emptyText: string;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onCancel: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  if (records.length === 0) {
    return <div className="emptyState">{emptyText}</div>;
  }

  return (
    <section className="transferList">
      {records.map((record) => {
        const percent =
          record.total_bytes && record.total_bytes > 0
            ? Math.min(100, (record.bytes_transferred / record.total_bytes) * 100)
            : 0;

        return (
          <article
            key={record.id}
            className={`transferRow ${selectedId === record.id ? "selected" : ""}`}
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

              <div className="transferMeta">
                <span>{record.provider === "google_drive" ? "Google Drive" : "HTTP/HTTPS"}</span>
                <span>
                  {formatBytes(record.bytes_transferred)}
                  {record.total_bytes ? ` / ${formatBytes(record.total_bytes)}` : ""}
                </span>
                <span>{record.bind_ips.length} link(s)</span>
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
  if (view === "download") return "Novo download";
  if (view === "upload") return "Upload para Google Drive";
  if (view === "queue") return "Fila de transferências";
  if (view === "scheduled") return "Agendador";
  if (view === "settings") return "Configurações";
  return "Histórico";
}

function viewSubtitle(view: View) {
  if (view === "download") {
    return "Adicione downloads HTTP/HTTPS segmentados à fila Multi-WAN.";
  }
  if (view === "upload") {
    return "Adicione lotes de upload do Google Drive à mesma fila do StorDown.";
  }
  if (view === "queue") {
    return "Downloads e uploads em uma fila única, com até duas transferências simultâneas.";
  }
  if (view === "scheduled") {
    return "Programe downloads HTTP/HTTPS para começar automaticamente mais tarde.";
  }
  if (view === "settings") {
    return "Categorias e regras automáticas para organizar downloads capturados pelo navegador.";
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

function formatDate(epochSeconds: number) {
  if (!epochSeconds) return "—";
  return new Date(epochSeconds * 1000).toLocaleString("pt-BR", {
    day: "2-digit",
    month: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}
