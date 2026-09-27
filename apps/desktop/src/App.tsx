import { FormEvent, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type DownloadResult = {
  output: string;
  bytes_written: number;
  segments: number;
  links_used: string[];
};

export default function App() {
  const [url, setUrl] = useState("");
  const [output, setOutput] = useState("C:\\Downloads\\arquivo.bin");
  const [connections, setConnections] = useState(8);
  const [bindIps, setBindIps] = useState("192.168.30.101, 192.168.30.102");
  const [status, setStatus] = useState("Pronto");
  const [busy, setBusy] = useState(false);

  const links = useMemo(
    () => bindIps.split(",").map((ip) => ip.trim()).filter(Boolean),
    [bindIps],
  );

  async function submit(event: FormEvent) {
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

  return (
    <main className="shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brandMark">S</span>
          <div>
            <strong>StorDown</strong>
            <small>Multi-Link Download Manager</small>
          </div>
        </div>

        <nav>
          <button className="navItem active">Downloads</button>
          <button className="navItem">Finalizados</button>
          <button className="navItem">Cloud</button>
          <button className="navItem">Agendador</button>
          <button className="navItem">Configurações</button>
        </nav>

        <div className="networkCard">
          <span>Multi-Link</span>
          <strong>{links.length} links ativos</strong>
          {links.map((ip, index) => (
            <div className="linkRow" key={ip}>
              <i />
              <span>Ethernet {index + 1}</span>
              <code>{ip}</code>
            </div>
          ))}
        </div>
      </aside>

      <section className="content">
        <header>
          <div>
            <p className="eyebrow">STORDOWN V0.1</p>
            <h1>Novo download</h1>
            <p className="subtitle">
              Segmentação HTTP Range com saída por múltiplas interfaces de rede.
            </p>
          </div>
          <div className="statusPill">{status}</div>
        </header>

        <form className="downloadCard" onSubmit={submit}>
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
                onChange={(e) => setBindIps(e.target.value)}
                placeholder="192.168.30.101, 192.168.30.102"
              />
            </label>
          </div>

          <div className="routePreview">
            {links.map((ip, index) => (
              <div key={ip}>
                <span>Worker {index + 1}</span>
                <strong>{ip}</strong>
                <small>→ WAN {index + 1}</small>
              </div>
            ))}
          </div>

          <button className="primary" disabled={busy || links.length === 0}>
            {busy ? "Baixando…" : "Iniciar com Multi-Link"}
          </button>
        </form>

        <section className="metrics">
          <article>
            <span>Motor</span>
            <strong>Rust</strong>
            <small>HTTP/HTTPS + Range</small>
          </article>
          <article>
            <span>Estratégia</span>
            <strong>Multi-NIC</strong>
            <small>bind por IP local</small>
          </article>
          <article>
            <span>Próximo</span>
            <strong>Browser</strong>
            <small>captura automática</small>
          </article>
        </section>
      </section>
    </main>
  );
}
