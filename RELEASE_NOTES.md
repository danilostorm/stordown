# StorDown v0.1.0-alpha.1

Primeiro release instalável do StorDown para Windows.

## Destaques

- downloads HTTP/HTTPS segmentados com múltiplas conexões;
- uso de múltiplas placas de rede e políticas Multi-WAN do UDM;
- balanceamento adaptativo e failover automático entre links;
- pause, resume, cancelamento, retry e retomada por partes;
- fila unificada, histórico SQLite, categorias e agendamento;
- limite de velocidade e verificação SHA-256 opcional;
- integração Chrome/Edge via Native Messaging;
- Google Drive com OAuth + PKCE, Meu Drive, Shared Drives e links compartilhados;
- upload resumível do Google Drive com retomada após reiniciar o StorDown/Windows;
- download de blobs do Drive por HTTP Range usando o motor Multi-WAN;
- exportação de Docs, Sheets, Slides, Drawings e Apps Script;
- StorDown Relay v1 para dividir um arquivo grande em chunks simultâneos por WAN1/WAN2 até um relay remoto;
- Relay self-hosted para VPS/Unraid, com persistência de sessão e SHA-256.

## Arquivos do release

- `StorDown-v0.1.0-alpha.1-Windows-x64-Setup.exe`: instalador principal do StorDown;
- `StorDown-Browser-Extension-v0.1.0-alpha.1.zip`: extensão Chrome/Edge para carregar em modo de desenvolvedor;
- `StorDown-Native-Host-v0.1.0-alpha.1.zip`: ponte Native Messaging separada para diagnóstico/instalação manual;
- `StorDown-Relay-v0.1.0-alpha.1-Windows-x64.zip`: relay Windows;
- `SHA256SUMS.txt`: hashes dos artefatos.

O instalador principal também carrega o Native Host como recurso do aplicativo para que a tela de integração do navegador consiga registrá-lo no Windows.

## Google Drive

Esta build suporta um Google OAuth Client ID fornecido pelo usuário na tela de desenvolvimento. Se o repositório tiver o secret `STORDOWN_GOOGLE_CLIENT_ID` configurado no GitHub, o workflow também o incorpora como fallback de build.

Como o download completo do Drive usa `drive.readonly`, uma distribuição pública ampla do OAuth ainda depende da verificação correspondente do Google.

## StorDown Relay

O Relay v1 já soma WANs no trecho PC -> Relay:

```text
arquivo grande
  -> NIC1/WAN1 --+
  -> NIC2/WAN2 --+-> StorDown Relay -> arquivo montado/verificado
```

Nesta alpha o Relay é um endpoint de staging. O adaptador Relay -> Google Drive fica para uma atualização posterior e não bloqueia o gerenciador desktop, Drive direto ou os testes Multi-WAN.

## Observações da alpha

- instalador ainda não possui assinatura Authenticode comercial, então o Windows SmartScreen pode exibir aviso;
- extensão ainda não está publicada na Chrome Web Store / Edge Add-ons; use o ZIP incluído;
- foco deste release é Windows x64.
