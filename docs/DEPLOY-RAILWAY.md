# xau-web no Railway, com a saída pela Cloudflare

O site fica no ar 24 h, sem o seu computador. Um container roda três coisas juntas:

```
 navegador ──https──▶ Cloudflare (domínio e HTTPS) ──▶ Railway: container
                                                                 ├─ xau-web         (o servidor e as telas)
                                                                 ├─ Chromium        (roda a página de login da Microsoft)
                                                                 └─ Cloudflare WARP (usque, proxy 127.0.0.1:40000)
                                                                          │
   tudo que vai para a Microsoft (login, Xbox Live, Loja, farm) ──────────┘──▶ sai com o IP da Cloudflare
```

**A regra da saída é fail-closed.** O servidor só chama a Microsoft depois de confirmar que o proxy
sai pela Cloudflare (a página `cdn-cgi/trace` da Cloudflare, lida pelo proxy, tem de dizer `warp=on`
ou `plus`, e o endereço tem de ser diferente do da saída direta). A cada 10 s ele confere de novo.
Se o WARP ficar mais de 45 s sem responder, as chamadas passam a ser **recusadas**; nada sai pelo IP
do Railway. O Chromium usa
`--proxy-server` e não resolve nomes sozinho; sem proxy ele mostra "No internet" em vez de sair direto.

## O login: no navegador de quem entra, ou no servidor

Há duas maneiras de abrir a página de login da Microsoft, e o site escolhe sozinho:

* **No navegador da pessoa** (padrão, quando o navegador deixa): a página roda como uma página de
  verdade do navegador de quem está entrando. O teclado, o gerenciador de senhas, a rolagem e os
  toques são os do próprio navegador, sem atraso de rede, e o servidor não gasta memória por login
  (~3 MB em vez de ~500 MB de um Chromium). Quem faz isso possível é o
  [Scramjet](https://github.com/MercuryWorkshop/scramjet) (um service worker que reescreve a
  página) com o transporte libcurl (o TLS roda em WebAssembly, no navegador). O que sai do
  navegador vai por **um WebSocket** (`/wisp/`) até o servidor, que só repassa bytes, **só para
  os domínios da Microsoft** (e dos captchas que ela usa), **só nas portas 80 e 443**, **só TCP**,
  e **só pela Cloudflare** (cada conexão é um SOCKS5 para o proxy do WARP; se a saída não estiver
  confirmada, nada sai). O núcleo continua no servidor: é ele que registra o dispositivo Windows, lê
  o token do login e guarda a conta. A página é carregada **antes**, escondida, enquanto a pessoa
  ainda está na tela de boas-vindas, então o "Entrar" mostra uma página que já está pronta.
* **No servidor** (reserva): o Chromium do container abre a página e o site desenha a imagem dela. É
  usado quando o navegador não suporta o modo acima (sem service worker, contexto não seguro), ou
  quando a página não abre a tempo (25 s). Dá para forçar com `?login=stream` na URL.

Os arquivos do Scramjet entram na imagem na hora da montagem (`docker/scramjet`: versões travadas pelo
`package-lock.json`) e ficam em `/opt/scramjet` (`XAU_SCRAMJET_DIR`). Sem essa pasta o modo no
navegador fica desligado e todos usam o do servidor. O Scramjet é AGPL-3.0; **o código do site é
publicado no GitHub**, junto com a única correção que aplicamos a ele (`docker/scramjet/layout.mjs`: o
`history.replaceState` sem URL levava a página para `/undefined`; corrigido por eles depois dessa
versão). Se uma versão nova do Scramjet mudar esse trecho, a montagem da imagem **falha** em vez de
publicar uma página que se comporta mal.

Domínios extras que o relé deve aceitar (separados por vírgula): `XAU_WISP_ALLOW`. Os recusados
aparecem no log como `wisp: recusado ...`.

## 1. O WARP

Não precisa configurar nada. O container usa o **usque** (open source, o mesmo protocolo MASQUE do
WARP) como proxy local: na primeira inicialização ele registra um dispositivo WARP e guarda o registro
no volume (`/data/warp-usque.json`), para os próximos deploys usarem o mesmo.

Por que não o cliente oficial (`warp-svc`): mesmo em modo proxy ele mexe no firewall do Linux
(nftables), o que exige a permissão `NET_ADMIN`, que o Railway não dá. Ele derrubava o túnel a cada
~15 s (`FirewallRepairFailed`). O usque roda inteiro em espaço de usuário.

O túnel vai por **HTTP/2 sobre TCP** (não depende de UDP). `XAU_WARP_TRANSPORT=quic` troca para HTTP/3.

## 2. Railway: o serviço

1. Crie o projeto: **New Project → Deploy from GitHub repo → `xdzleo/xau-web`**. O Railway lê o
   `railway.toml` e constrói o `Dockerfile` (a primeira compilação demora, uns 10 a 15 min).
2. **Volume:** no serviço, **Settings → Volumes → Add Volume**, caminho de montagem **`/data`**
   (aí ficam as contas, os códigos e os perfis do navegador; sem volume tudo some a cada deploy).
3. **Variables:**

| Variável | Valor | Para quê |
|---|---|---|
| `XAU_PUBLIC_HOSTS` | `rewardsxb.site` | o nome pelo qual o servidor responde (outros nomes são recusados) |
| `XAU_PASSWORD` | uma senha, ou várias separadas por vírgula (`senha1,senha2,senha3`), cada uma com 4+ caracteres | pedida na página inicial; qualquer uma delas entra, e quem a tem controla as contas |

   O Railway já define `PORT`. Não defina `XAU_ALLOW_DIRECT_EGRESS`: ele existe só para testes e
   desliga a regra da saída pela Cloudflare.
4. **Settings → Networking → Custom Domain → `rewardsxb.site`.** O Railway mostra um destino
   (`xxxx.up.railway.app`) para o CNAME.

## 3. Cloudflare: domínio

1. **DNS** do `rewardsxb.site`: um registro **CNAME** `@` (a Cloudflare achata o CNAME na raiz)
   apontando para o destino do Railway, **Proxied (nuvem laranja)**.
2. **SSL/TLS → Overview → Full (strict).**

Não há login da Cloudflare na frente: o acesso é só a senha do site (abaixo).

## 4. Acesso

Abra `https://rewardsxb.site/`, digite a senha e entre (o navegador guarda o acesso por um ano).

* **Tentativas limitadas.** 5 senhas erradas do mesmo endereço bloqueiam esse endereço por 5 minutos;
  20 erradas, de qualquer lugar, bloqueiam todas as tentativas por 10 minutos (mesmo a senha certa).
  Assim ninguém consegue ficar chutando a senha.
* **Se alguém bloquear você de propósito:** o servidor também aceita `https://rewardsxb.site/?key=<chave>`;
  a chave aparece nos logs do Railway na primeira inicialização (ou fica em `/data/web-key.txt`).
* `https://rewardsxb.site/api/info` (depois de entrar) mostra o estado da saída pela Cloudflare:
  `egress: {ok: true, warp: on, colo: GRU, ...}`.
* Escolha uma senha longa e só sua: ela é a única barreira entre a internet e as suas contas.
## 5. Se algo não funcionar

* **Logs do Railway.** O entrypoint escreve `[entrypoint] ...` e o proxy, `[warp] ...`. O servidor escreve `saída pela Cloudflare confirmada` quando
  tudo está certo, ou o motivo (`warp=off`, `o proxy não respondeu`).
* **"a saída pela Cloudflare (WARP) ainda não está confirmada"** nas telas: o WARP ainda não conectou
  (veja as linhas `[warp]`). Se o registro falhou, apague `/data/warp-usque.json` e reinicie.
* **Proxy em outro lugar:** `XAU_EGRESS_PROXY=socks5h://host:porta` usa um proxy WARP externo no
  lugar do usque.
* **A Microsoft recusa ou pede verificação.** O endereço de saída é da Cloudflare, compartilhado
  com outros usuários do WARP. Isso pode gerar verificações extras de segurança nas contas.

## Variáveis (referência)

| Variável | Efeito |
|---|---|
| `PORT` | porta de escuta, em todas as interfaces (o host define) |
| `XAU_PUBLIC_HOSTS` | nomes aceitos, separados por vírgula; liga o modo servidor |
| `XAU_PASSWORD` | a senha pedida na página inicial; várias separadas por vírgula, qualquer uma entra (4+ caracteres cada, sem vírgula dentro) |
| `XAU_KEY` | (opcional) a chave da sessão e do link de emergência; sem ela, uma é criada em `/data/web-key.txt` e aparece nos logs |
| `XAU_DATA_DIR` | pasta de dados (o volume), `/data` na imagem |
| `XAU_EGRESS_PROXY` | proxy de saída; sem ela, o entrypoint sobe o usque e usa `socks5h://127.0.0.1:40000` |
| `XAU_WARP` | `off` desliga o WARP (as chamadas à Microsoft são recusadas) |
| `XAU_WARP_TRANSPORT` | `http2` (padrão, TCP) ou `quic` (HTTP/3, UDP) |
| `XAU_ALLOW_DIRECT_EGRESS` | **só testes**: aceita falar com a Microsoft sem o WARP |
| `XAU_SCRAMJET_DIR` | pasta com os arquivos do login no navegador (na imagem: `/opt/scramjet`); sem ela, esse modo fica desligado |
| `XAU_WISP_ALLOW` | domínios a mais que o relé do login no navegador aceita, separados por vírgula |
| `XAU_CHROMIUM` | caminho do navegador (na imagem: `/usr/bin/chromium`) |
| `XAU_CHROMIUM_NO_SANDBOX` | roda o Chromium sem o sandbox (necessário como root, na imagem) |
| `XAU_WEB_BENCH` | **só testes**: liga a página de benchmark local |
