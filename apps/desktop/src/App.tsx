import { FormEvent, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

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
  const [driveToken, setDriveToken] = useState("");
  const [chunkMiB, setChunkMiB] = useState(8);

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

  async function submitDownload(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setStatus("Iniciando download segmentado…");

    try {
      const result = await invoke<DownloadResult>("start_download", {
        url,
        output,
        connections,
        bindIps: links,
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
    setBusy(true);
    setStatus(`Enviando ${files.length} arquivo(s) ao Google Drive…`);

    try {
      const result = await invoke<DriveUploadResult[]>("start_drive_upload", {
        files,
        accessToken: driveToken,
        parentId: driveParentId || null,
        bindIps: links,
        chunkMib: chunkMiB,
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
                {(nic?.link_speed || probe?.public_ip || probe?.error) && (
                  <div className="linkMeta">
                    {nic?.link_speed && <span>{nic.link_speed}</span>}
                    {probe?.public_ip && (
                      <span>
                        WAN: {probe.public_ip} • {probe.latency_ms ?? "?"} ms
                      </span>
                    )}
                    {probe?.error && <span className="errorText">Sem saída</span>}
                  </div>
                )}
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

            <label>
              Google OAuth access token — temporário para desenvolvimento
              <input
                type="password"
                value={driveToken}
                onChange={(e) => setDriveToken(e.target.value)}
                placeholder="OAuth será integrado ao botão Conectar Google Drive"
                required
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

            <RoutePreview links={links} nics={nicByIp} probes={probeByIp} />

            <button
              className="primary"
              disabled={busy || links.length === 0 || files.length === 0}
            >
              {busy ? "Enviando…" : `Enviar ${files.length} arquivo(s)`}
            </button>
          </form>
        )}

        <section className="metrics">
          <article>
            <span>Motor</span>
            <strong>Rust</strong>
            <small>HTTP + Drive resumable</small>
          </article>
          <article>
            <span>Rede</span>
            <strong>Auto-detect</strong>
            <small>NIC + IP público por rota</small>
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
