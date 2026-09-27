use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

const MAX_NATIVE_MESSAGE: usize = 1024 * 1024;
const DESKTOP_CAPTURE_ADDR: &str = "127.0.0.1:17832";

#[derive(Debug, Deserialize)]
struct NativeRequest {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Serialize)]
struct NativeResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    transfer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn main() {
    if let Err(error) = run() {
        let _ = write_native_response(&NativeResponse {
            ok: false,
            transfer_id: None,
            status: None,
            error: Some(error.to_string()),
        });
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut input = stdin.lock();

    loop {
        let Some(payload) = read_native_message(&mut input)? else {
            break;
        };

        let response = handle_message(&payload);
        write_native_response(&response)?;
    }

    Ok(())
}

fn handle_message(payload: &[u8]) -> NativeResponse {
    let parsed: NativeRequest = match serde_json::from_slice(payload) {
        Ok(value) => value,
        Err(error) => {
            return NativeResponse {
                ok: false,
                transfer_id: None,
                status: None,
                error: Some(format!("Mensagem inválida da extensão: {error}")),
            };
        }
    };

    if parsed.kind == "ping" {
        return forward_to_desktop(json!({ "type": "ping" }));
    }

    if parsed.kind != "download" {
        return NativeResponse {
            ok: false,
            transfer_id: None,
            status: None,
            error: Some(format!("Tipo não suportado: {}", parsed.kind)),
        };
    }

    let Some(url) = parsed.url.filter(|value| !value.trim().is_empty()) else {
        return NativeResponse {
            ok: false,
            transfer_id: None,
            status: None,
            error: Some("URL ausente".to_string()),
        };
    };

    forward_to_desktop(json!({
        "type": "download",
        "url": url,
        "filename": parsed.filename,
        "source": parsed.source.unwrap_or_else(|| "browser".to_string())
    }))
}

fn forward_to_desktop(payload: Value) -> NativeResponse {
    let addr: SocketAddr = match DESKTOP_CAPTURE_ADDR.parse() {
        Ok(value) => value,
        Err(error) => {
            return NativeResponse {
                ok: false,
                transfer_id: None,
                status: None,
                error: Some(format!("Endereço local inválido: {error}")),
            };
        }
    };

    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(stream) => stream,
        Err(_) => {
            return NativeResponse {
                ok: false,
                transfer_id: None,
                status: None,
                error: Some(
                    "StorDown Desktop não está em execução. Abra o StorDown e tente novamente."
                        .to_string(),
                ),
            };
        }
    };

    let _ = stream.set_read_timeout(Some(Duration::from_secs(8)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(8)));

    let mut line = match serde_json::to_vec(&payload) {
        Ok(value) => value,
        Err(error) => {
            return NativeResponse {
                ok: false,
                transfer_id: None,
                status: None,
                error: Some(error.to_string()),
            };
        }
    };
    line.push(b'\n');

    if let Err(error) = stream.write_all(&line) {
        return NativeResponse {
            ok: false,
            transfer_id: None,
            status: None,
            error: Some(format!("Falha ao enviar captura ao StorDown: {error}")),
        };
    }

    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();

    if let Err(error) = reader.read_line(&mut response_line) {
        return NativeResponse {
            ok: false,
            transfer_id: None,
            status: None,
            error: Some(format!("Falha ao ler resposta do StorDown: {error}")),
        };
    }

    match serde_json::from_str::<NativeResponse>(&response_line) {
        Ok(response) => response,
        Err(error) => NativeResponse {
            ok: false,
            transfer_id: None,
            status: None,
            error: Some(format!("Resposta inválida do StorDown: {error}")),
        },
    }
}

fn read_native_message(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length_bytes = [0u8; 4];

    match reader.read_exact(&mut length_bytes) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }

    let length = u32::from_le_bytes(length_bytes) as usize;
    if length == 0 || length > MAX_NATIVE_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("native message size out of bounds: {length}"),
        ));
    }

    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    Ok(Some(payload))
}

fn write_native_response(response: &NativeResponse) -> io::Result<()> {
    let payload = serde_json::to_vec(response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    if payload.len() > MAX_NATIVE_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native response too large",
        ));
    }

    let stdout = io::stdout();
    let mut output = stdout.lock();
    output.write_all(&(payload.len() as u32).to_le_bytes())?;
    output.write_all(&payload)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::{read_native_message, MAX_NATIVE_MESSAGE};
    use std::io::Cursor;

    #[test]
    fn reads_chrome_native_message_frame() {
        let payload = br#"{"type":"ping"}"#;
        let mut framed = Vec::new();
        framed.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        framed.extend_from_slice(payload);

        let mut cursor = Cursor::new(framed);
        assert_eq!(read_native_message(&mut cursor).unwrap().unwrap(), payload);
    }

    #[test]
    fn rejects_oversized_message() {
        let mut cursor = Cursor::new(((MAX_NATIVE_MESSAGE as u32) + 1).to_le_bytes());
        assert!(read_native_message(&mut cursor).is_err());
    }
}
