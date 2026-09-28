# StorDown v0.1.0-alpha.2

Alpha focada em transformar o StorDown de uma tela técnica de teste em um gerenciador de downloads utilizável em outros computadores.

## Interface e acompanhamento em tempo real

- nova tela **Visão geral** com aparência de gerenciador de downloads;
- transferências ativas aparecem diretamente na tela inicial;
- velocidade individual por transferência em tempo real;
- velocidade total agregada;
- porcentagem, bytes transferidos e ETA;
- ações rápidas para novo download, upload, Google Drive e extensão;
- fila e histórico continuam acessíveis pela barra lateral;
- atualização periódica da fila para recuperar eventos perdidos e manter a tela sincronizada.

## Portabilidade

O StorDown não inicia mais com IPs, caminhos ou arquivos específicos da máquina de desenvolvimento.

Ao abrir em outro Windows ele agora:

- encontra a pasta Downloads do próprio usuário;
- detecta as placas físicas ativas;
- funciona normalmente com apenas uma interface;
- permite selecionar quais interfaces participarão do Multi-Link;
- salva as interfaces escolhidas, pasta padrão e número de conexões localmente;
- não exibe mais textos assumindo que existe UDM Pro;
- permite testar se duas interfaces realmente saem por IPs públicos diferentes.

O Multi-WAN continua dependendo de roteamento real independente. Em um computador com uma única conexão o StorDown opera como um gerenciador normal.

## Nomes de arquivo

- o StorDown consulta metadados HTTP antes do download;
- usa `Content-Disposition` quando o servidor fornece o nome real;
- segue redirecionamentos e usa a URL final como fallback;
- o botão de destino agora escolhe **uma pasta**, em vez de abrir um diálogo que podia sugerir nomes estranhos;
- downloads capturados pelo navegador também tentam resolver o nome real pelo servidor;
- nomes duplicados recebem apenas o sufixo `(1)`, `(2)` etc.

## Chrome / Edge

A extensão passa a fazer parte do próprio instalador.

Em **Configurações > Extensão Chrome / Edge**:

1. clique em **Preparar extensão**;
2. StorDown extrai e abre a pasta da extensão;
3. carregue a pasta em modo desenvolvedor no Chrome/Edge;
4. copie o ID mostrado pelo navegador;
5. clique em **Conectar ao StorDown**.

O Native Messaging Host também continua incluído no pacote.

## Google Drive e Multi-WAN

Mantidos nesta alpha:

- OAuth + PKCE;
- Meu Drive e Shared Drives;
- links compartilhados e resource keys;
- upload resumível com sessão persistente;
- download HTTP Range Multi-WAN;
- exportação Google Workspace;
- StorDown Relay v1.

## Arquivos da release

- `StorDown-v0.1.0-alpha.2-Windows-x64-Setup.exe`
- `StorDown-Browser-Extension-v0.1.0-alpha.2.zip`
- `StorDown-Native-Host-v0.1.0-alpha.2.zip`
- `StorDown-Relay-v0.1.0-alpha.2-Windows-x64.zip`
- `SHA256SUMS.txt`

## Observações

Esta ainda é uma alpha. O instalador não possui assinatura Authenticode comercial e o Windows SmartScreen pode exibir aviso.
