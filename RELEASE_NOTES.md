# StorDown v0.1.0-alpha.3

Alpha focada nos problemas encontrados durante o uso real: acompanhamento em tempo real, aparência de gerenciador de downloads, integração do navegador, nomes de arquivo e portabilidade entre computadores e redes diferentes.

## Gerenciador e acompanhamento em tempo real

- barra de ações no estilo de um gerenciador de downloads desktop, sempre acessível;
- dock de transferência ativa visível nas diferentes telas do StorDown;
- progresso, porcentagem, bytes transferidos, velocidade e ETA no dock;
- pausa, retomada e cancelamento sem precisar procurar a transferência em outra tela;
- contador da fila e velocidade agregada;
- tela dedicada **Extensão** na navegação principal;
- detalhes técnicos foram afastados do fluxo principal para a interface ficar menos parecida com uma ferramenta de desenvolvimento.

## Portabilidade entre computadores

A configuração de rede não é mais tratada como algo específico da máquina original.

Cada instalação:

- detecta as interfaces físicas do próprio Windows;
- funciona com uma única conexão normalmente;
- permite escolher as interfaces que participarão das transferências;
- mantém pasta padrão, interfaces escolhidas e número de conexões como preferências daquele computador;
- sincroniza essas preferências também com os downloads capturados pelo navegador;
- não exige UDM Pro: Multi-WAN funciona quando o sistema/rede realmente oferece rotas independentes; caso contrário o StorDown funciona em modo single-link.

Os downloads enviados pela extensão agora usam as mesmas interfaces e a mesma pasta selecionadas no aplicativo, em vez de selecionar todas as placas físicas por conta própria.

## Pasta e nome do arquivo

A tela **Novo download** separa claramente:

- **Pasta de destino**;
- **Nome do arquivo**.

O StorDown tenta resolver o nome nessa ordem:

1. `Content-Disposition` do servidor;
2. nome legível da URL final após redirecionamentos;
3. extensão/nome coerente com o `Content-Type`;
4. fallback seguro.

URLs cujo último segmento parece UUID, hash ou token deixam de ser usadas automaticamente como nome visível. O nome continua editável antes do download.

Downloads capturados pelo navegador também rejeitam nomes opacos gerados pelo site quando conseguem obter metadados melhores do servidor.

## Extensão Chrome / Edge

A extensão continua incluída no instalador, mas a configuração ficou mais simples.

- a extensão agora possui um **ID estável** entre computadores;
- **Preparar e conectar** extrai a extensão e registra o Native Messaging Host automaticamente;
- não é mais necessário copiar o ID do Chrome/Edge e colar no StorDown;
- o aplicativo mostra o ID fixo apenas para diagnóstico;
- há opção de registrar novamente a integração caso o navegador seja reinstalado.

Por segurança do Chrome/Edge, uma extensão não publicada não pode ser instalada silenciosamente como extensão normal. Nesta alpha ainda é necessário ativar modo desenvolvedor e usar **Carregar sem compactação** uma vez. A publicação futura na Chrome Web Store / Edge Add-ons elimina essa etapa manual.

## Mantido da alpha.2

- downloads HTTP/HTTPS segmentados e Smart Multi-WAN;
- failover entre interfaces;
- fila, agendamento, histórico e categorias;
- Google Drive OAuth + PKCE;
- Meu Drive, Shared Drives e links compartilhados;
- downloads HTTP Range do Drive;
- upload resumível e retomada de sessão após reiniciar;
- exportação Google Workspace;
- StorDown Relay v1;
- Native Host e extensão incluídos no pacote Windows.

## Arquivos da release

- `StorDown-v0.1.0-alpha.3-Windows-x64-Setup.exe`
- `StorDown-Browser-Extension-v0.1.0-alpha.3.zip`
- `StorDown-Native-Host-v0.1.0-alpha.3.zip`
- `StorDown-Relay-v0.1.0-alpha.3-Windows-x64.zip`
- `SHA256SUMS.txt`

## Observações

Esta ainda é uma alpha. O instalador não possui assinatura Authenticode comercial e o Windows SmartScreen pode exibir aviso.
