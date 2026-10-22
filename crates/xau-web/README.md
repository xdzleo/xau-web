# xau-web

A versão web do app: o mesmo núcleo (`xau-api`, o que o app Android também chama) rodando num
servidor local, com a interface no navegador. Tudo que o app faz, a página faz.

```
xau-web.exe                 # abre o navegador já com a chave
xau-web.exe --port 8790     # porta fixa
xau-web.exe --data-dir D:\xau   # onde ficam contas e CSVs
xau-web.exe --lan           # aceita outros aparelhos da rede (ver abaixo)
xau-web.exe --no-open       # só imprime o link
```

## Como funciona

- **Login e geração do XTKS** são os mesmos do app: o núcleo (`windows_login_*`) faz a troca de
  tokens. A página de login da Microsoft roda num WebView2 escondido (fora da tela), configurado
  com a mesma identidade do WebView do app (celular, `pt-BR`, sem `chrome.webview`), e é desenhada
  dentro do site, em tela cheia como no app; cliques e teclas são repassados como eventos reais.
  A senha vai direto para a página da Microsoft e não é guardada. `XAU_LOGIN_WINDOW=1` volta a
  mostrar a janela nativa no lugar (para depurar).
- **Checkout da Loja** abre a página de compra da Microsoft numa janela nativa; a compra é
  confirmada por você lá.
- O **farm roda no servidor**: fechar a aba não para nada. A página só mostra e controla.
- A interface é a do app (mesmas telas, textos, cores e fontes). Início tem a barra inferior; as
  demais telas são páginas com seta de voltar. Segurar (ou botão direito) num card pede para
  remover a conta.

### A página de login dentro do site

O WebView2 escondido roda a página no tamanho exato da tela do site. O motor empurra um quadro JPEG
a cada repintura (`Page.startScreencast`, confirmado na hora), e um WebSocket leva esses quadros ao
site, que desenha sempre só o mais novo num `<canvas>`. Toques, cliques e teclas voltam pelo mesmo
WebSocket e viram eventos reais (`Input.dispatchTouchEvent`/`dispatchMouseEvent`/`insertText`).
Se o WebSocket falhar, o site cai para fotos por HTTP.

O screencast do Chromium sai em pixels CSS (borrado numa tela de densidade 2). Por isso, assim que a
página fica parada por 120 ms, o servidor manda uma captura na densidade real da tela. Enquanto a
pessoa não está mexendo, mudanças pequenas (o cursor piscando) já saem só nítidas; durante o toque e
em animações saem os quadros rápidos. Quadros repetidos e o eco da captura nítida não são reenviados,
então com a tela parada não sai nada.

- Links que abrem janela nova (`target=_blank`) abrem na própria página, como no WebView do app.
- "Create one!" abre o cadastro da Microsoft no modo web normal, numa janela limpa própria (o modo
  "hospedado pelo Windows" do cadastro fica esperando o host do Windows para sempre). Ao voltar
  ("Sign in" ou fim do cadastro), o site mostra o login de novo, recarregado.

Benchmark (release, página local animada; `XAU_WEB_BENCH=1` liga `/api/bench/start`):

| Versão | Toque → quadro p50 / p95 | Quadros/s em animação |
|---|---|---|
| Foto a cada 150 ms (primeira versão) | 115 / 135 ms | 4,8 |
| Foto contínua | 58 / 84 ms | 16 |
| Screencast + MJPEG | 23 / 33 ms | ~90–100 |
| Screencast + WebSocket (atual), no servidor | 9 / 17 ms | 100 |
| Screencast + WebSocket, toque → pixel na tela do navegador | 13,5 / 22,7 ms | 98 desenhados |
| Atual (com quadro nítido ao parar), toque → desenho no navegador | 14,8 / 29 ms | 99,9 desenhados |

Atual, com a tela parada: 0 quadros. Cursor piscando sem interação: 2 quadros/s, todos nítidos.

`XAU_SCREENCAST_QUALITY` (30–95, padrão 72) muda a qualidade do JPEG; no benchmark, 50 a 85 não
mudou a latência.

## Segurança

- Escuta só em `127.0.0.1` por padrão. O link impresso traz uma chave aleatória de 256 bits que
  vira um cookie `HttpOnly`/`SameSite=Strict`; quem tem o link controla as contas.
- Cabeçalho `Host` validado (anti DNS rebinding), `Origin` igual ao do servidor e `x-xau: 1` em
  todo POST, CSP restritiva.
- `/api/call/<método>` só aceita uma lista fixa; passos de login e o token de checkout não são
  alcançáveis por ela.
- `--lan` expõe o servidor à rede local: use só em rede confiável. A janela de login continua
  abrindo no computador do servidor.

## Desenvolvimento

```
cargo test -p xau-web
XAU_WEB_UI_DIR=crates/xau-web/ui cargo run -p xau-web   # edita a UI sem recompilar
```

`?mock=1` na URL troca os dados das contas por dados de exemplo (só para ver a interface).
