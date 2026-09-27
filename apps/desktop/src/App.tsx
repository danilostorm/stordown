import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type DownloadResult = {
  output: string;
  bytes_written: number;
  segments: number;
  links_used: string[];
};

type DriveUploadResult = {
  id: string;
  name: string;
  size: number;
  web_view_link?: string | null;
  bytes_uploaded: number;
  link_used: string;
  local_ip: string;
};

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

type SpeedWindow = {
  startedAt: number;
  bytes: number;
};

type View = "download" | "upload";

export default function App() {
  const [view, setView] = useState<View>("download");
  const [url, setUrl] = useState("");
  const [output, setOutput] = useState("C:\\Downloads\\arquivo.bin");
  const [connections, setConnections] = useState(8);
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

  const [activeTransferId, setActiveTransferId] = useState<string | null>(null);
  const [progressByItem, setProgressByItem] = useState<Record<string, TransferProgress>>({});
  const [linkSpeeds, setLinkSpeeds] = useState<Record<string, number>>({});
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

  const activeProgress = useMemo(
    () =>
      Object.values(progressByItem)
        .filter((item) => item.transfer_id === activeTransferId)
        .sort((a, b) => a.item.localeCompare(b.item)),
    [progressByItem, activeTransferId],
  );

  const totalTransferred = useMemo(
    () => activeProgress.reduce((sum, item) => sum + item.bytes_transferred, 0),
    [activeProgress],
  );

  const totalBytes = useMemo(
    () =>
      activeProgress.reduce(
        (sum, item) => sum + (item.total_bytes ?? item.bytes_transferred),
        0,
      ),
    [activeProgress],
  );

  useEffect(() => {
    let stop: undefined | (() => void);

    listen<TransferProgress>("transfer-progress", ({ payload }) => {
      const progressKey = `${payload.transfer_id}:${payload.item}`;

      setProgressByItem((current) => ({
        ...current,
        [progressKey]: payload,
      }));

      const now = Date.now();
      const key = payload.local_ip;
      const currentWindow = speedWindows.current[key] ?? {
        startedAt: now,
        bytes: 0,
      };

      currentWindow.bytes += payload.bytes_delta;
      const elapsed = now - currentWindow.startedAt;

      if (elapsed >= 750 || payload.completed) {
        const bytesPerSecond =
          elapsed > 0 ? (currentWindow.bytes * 1000) / elapsed : 0;

        setLinkSpeeds((current) => ({
          ...current,
          [key]: bytesPerSecond,
        }));

        speedWindows.current[key] = {
          startedAt: now,
          bytes: 0,
        };
      } else {
        speedWindows.current[key] = currentWindow;
      }
    }).then((unlisten) => {
      stop = unlisten;
    });

    return () => stop?.();
  }, []);

  function beginTelemetry(transferId: string) {
    setActiveTransferId(transferId);
    setLinkSpeeds({});
    speedWindows.current = {};
  }

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
      const probes = await invoke<LinkProbeStatus[]>("test_routes", {
        bindIps: links,
      });
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

  async function submitDownload(event: FormEvent) {
    event.preventDefault();
    const transferId = crypto.randomUUID();
    beginTelemetry(transferId);
    setBusy(true);
    setStatus("Iniciando download segmentado…");

    try {
      const result = await invoke<DownloadResult>("start_download", {
        url,
        output,
        connections,
        bindIps: links,
        transferId,
      });

      setStatus(
        `Concluído: ${result.segments} segmentos • ${result.links_used.join(" + ")}`,
      );
    } catch (error) {
      setStatus(`Erro: ${String(error)}`);
    } finally {
      setBusy(false);
    }
  }

  async function submitUpload(event: FormEvent) {
    event.preventDefault();
    const transferId = crypto.randomUUID();
    beginTelemetry(transferId);
    setBusy(true);
    setStatus(`Enviando ${files.length} arquivo(s) ao Google Drive…`);

    try {
      const result = await invoke<DriveUploadResult[]>("start_drive_upload", {
        files,
        parentId: driveParentId || null,
        bindIps: links,
        chunkMib: chunkMiB,
        transferId,
      });

      const routes = [...new Set(result.map((item) => item.link_used))];
      setStatus(
        `Upload concluído: ${result.length} arquivo(s) • ${routes.join(" + ")}`,
      );
    } catch (error) {
      setStatus(`Erro: ${String(error)}`);
    } finally {
      setBusy(false);
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
          <button className="navItem">Finalizados</button>
          <button className="navItem">Cloud</button>
          <button className="navItem">Agendador</button>
          <button className="navItem">Configurações</button>
        </nav>

        <div className="networkCard">
          <span>Multi-Link</span>
          <strong>{links.length} links configurados</strong>

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
            <h1>{view === "download" ? "Novo download" : "Upload para Google Drive"}</h1>
            <p className="subtitle">
              {view === "download"
                ? "Segmentação HTTP Range com saída por múltiplas interfaces de rede."
                : "Upload retomável em blocos, com distribuição de vários arquivos entre as WANs."}
            </p>
          </div>
          <div className="statusPill">{status}</div>
        </header>

        {view === "download" ? (
          <form className="downloadCard" onSubmit={submitDownload}>
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
              <input
                value={output}
                onChange={(e) => setOutput(e.target.value)}
                required
              />
            </label>

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
              {busy ? "Transferindo…" : "Iniciar com Multi-Link"}
            </button>
          </form>
        ) : (
          <form className="downloadCard" onSubmit={submitUpload}>
            <div className="notice">
              <strong>Google Drive + duas WANs</strong>
              <span>
                Vários arquivos são enviados em paralelo por links diferentes. Um único arquivo
                usa uma sessão retomável sequencial; o modo Relay para somar duas WANs em um único
                arquivo será uma etapa separada.
              </span>
            </div>

            <label>
              Arquivos locais — um caminho por linha
              <textarea
                rows={5}
                value={uploadFiles}
                onChange={(e) => setUploadFiles(e.target.value)}
                placeholder={"C:\\Uploads\\filme1.mkv\nC:\\Uploads\\filme2.mkv"}
                required
              />
            </label>

            <div className="grid2 uploadGrid">
              <label>
                Bloco
                <select
                  value={chunkMiB}
                  onChange={(e) => setChunkMiB(Number(e.target.value))}
                >
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
              {busy ? "Enviando…" : `Enviar ${files.length} arquivo(s)`}
            </button>
          </form>
        )}

        {activeProgress.length > 0 && (
          <TransferTelemetry
            items={activeProgress}
            totalTransferred={totalTransferred}
            totalBytes={totalBytes}
            linkSpeeds={linkSpeeds}
          />
        )}

        <section className="metrics">
          <article>
            <span>Motor</span>
            <strong>Rust</strong>
            <small>HTTP + Drive resumable</small>
          </article>
          <article>
            <span>Telemetria</span>
            <strong>Tempo real</strong>
            <small>progresso + velocidade por WAN</small>
          </article>
          <article>
            <span>Cloud</span>
            <strong>Google Drive</strong>
            <small>download + upload</small>
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

function TransferTelemetry({
  items,
  totalTransferred,
  totalBytes,
  linkSpeeds,
}: {
  items: TransferProgress[];
  totalTransferred: number;
  totalBytes: number;
  linkSpeeds: Record<string, number>;
}) {
  const overallPercent =
    totalBytes > 0 ? Math.min(100, (totalTransferred / totalBytes) * 100) : 0;

  return (
    <section className="telemetryPanel">
      <div className="telemetryHeader">
        <div>
          <span>TRANSFERÊNCIA ATIVA</span>
          <strong>{overallPercent.toFixed(1)}%</strong>
        </div>
        <div className="aggregateSpeed">
          {formatSpeed(Object.values(linkSpeeds).reduce((sum, speed) => sum + speed, 0))}
        </div>
      </div>

      <div className="progressTrack">
        <div className="progressFill" style={{ width: `${overallPercent}%` }} />
      </div>

      <div className="telemetrySummary">
        <span>{formatBytes(totalTransferred)} transferidos</span>
        <span>{totalBytes > 0 ? formatBytes(totalBytes) : "Tamanho desconhecido"}</span>
      </div>

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

      <div className="wanTelemetry">
        {Object.entries(linkSpeeds).map(([ip, speed]) => (
          <div key={ip}>
            <span>{ip}</span>
            <strong>{formatSpeed(speed)}</strong>
          </div>
        ))}
      </div>
    </section>
  );
}

function formatBytes(value: number) {
  if (!Number.isFinite(value) || value <= 0) return "0 B";

  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(
    units.length - 1,
    Math.floor(Math.log(value) / Math.log(1024)),
  );
  const amount = value / 1024 ** index;

  return `${amount >= 100 || index === 0 ? amount.toFixed(0) : amount.toFixed(1)} ${units[index]}`;
}

function formatSpeed(bytesPerSecond: number) {
  return `${formatBytes(bytesPerSecond)}/s`;
}
