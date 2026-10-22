# xau-web no Railway, com a saída pela Cloudflare

O site fica no ar 24 h, sem o seu computador. Um container roda três coisas juntas:

```
 navegador ──https──▶ Cloudflare (domínio e HTTPS) ──▶ Railway: container
                                                                 ├─ xau-web         (o servidor e as telas)
                                                                 ├─ Chromium        (roda a página de login da Microsoft)
                                                                 └─ Cloudflare WARP (modo proxy, 127.0.0.1:40000)
                                                                          │
   tudo que vai para a Microsoft (login, Xbox Live, Loja, farm) ──────────┘──▶ sai com o IP da Cloudflare
```

**A regra da saída é fail-closed.** O servidor só chama a Microsoft depois de confirmar que o proxy
sai pela Cloudflare (a página `cdn-cgi/trace` da Cloudflare, lida pelo proxy, tem de dizer `warp=on`
ou `plus`, e o endereço tem de ser diferente do da saída direta). A cada 30 s ele confere de novo.
Se o WARP cair, as chamadas passam a ser **recusadas**; nada sai pelo IP do Railway. O Chromium usa
`--proxy-server` e não resolve nomes sozinho; sem proxy ele mostra "No internet" em vez de sair direto.

## 1. Cloudflare Zero Trust: o token de serviço do WARP

O container entra no seu Zero Trust como um "dispositivo" sem tela, com um token de serviço.

1. **Zero Trust → Access → Service Auth → Create Service Token.** Nome: `xau-web`. Duração: sem
   expirar (ou renove). Copie o **Client ID** e o **Client Secret** (o secret só aparece uma vez).
2. **Zero Trust → Team & Resources → Devices → Management → Device enrollment permissions → Manage**
   → adicione uma regra: **Action = Service Auth**, **Include → Service Token → `xau-web`**.
   (Sem isso, "Allow" não vale para tokens de serviço.)
3. Anote o **nome da equipe** (o `hsplay` de `hsplay.cloudflareaccess.com`).

## 2. Railway: o serviço

1. Crie o projeto: **New Project → Deploy from GitHub repo → `xdzleo/xau-web`**. O Railway lê o
   `railway.toml` e constrói o `Dockerfile` (a primeira compilação demora, uns 10 a 15 min).
2. **Volume:** no serviço, **Settings → Volumes → Add Volume**, caminho de montagem **`/data`**
   (aí ficam as contas, os códigos e os perfis do navegador; sem volume tudo some a cada deploy).
3. **Variables:**

| Variável | Valor | Para quê |
|---|---|---|
| `XAU_PUBLIC_HOSTS` | `rewardsxb.site` | o nome pelo qual o servidor responde (outros nomes são recusados) |
| `XAU_PASSWORD` | a senha que você vai digitar (10+ caracteres) | pedida na página inicial; quem a tem controla as contas |
| `WARP_TEAM` | `hsplay` | a equipe do Zero Trust |
| `WARP_CLIENT_ID` | o Client ID do passo 1 | inscrição do WARP |
| `WARP_CLIENT_SECRET` | o Client Secret do passo 1 | inscrição do WARP |

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

* **Logs do Railway.** O entrypoint escreve `[entrypoint] ...`; se o proxy do WARP não subir em 90 s,
  ele mostra o fim do log do `warp-svc`. O servidor escreve `saída pela Cloudflare confirmada` quando
  tudo está certo, ou o motivo (`warp=off`, `o proxy não respondeu`).
* **"a saída pela Cloudflare (WARP) ainda não está confirmada"** nas telas: o WARP ainda não conectou
  ou o token não tem a regra Service Auth (passo 1.2).
* **WARP não sobe no Railway:** o `warp-svc` pode precisar de capacidades que a plataforma não dá. O
  plano B é um proxy WARP em outro lugar (uma VM pequena) e `XAU_EGRESS_PROXY=socks5h://host:porta`
  no Railway, sem as variáveis `WARP_*`.
* **A Microsoft recusa ou pede verificação.** O endereço de saída é da Cloudflare, compartilhado
  com outros usuários do WARP. Isso pode gerar verificações extras de segurança nas contas.

## Variáveis (referência)

| Variável | Efeito |
|---|---|
| `PORT` | porta de escuta, em todas as interfaces (o host define) |
| `XAU_PUBLIC_HOSTS` | nomes aceitos, separados por vírgula; liga o modo servidor |
| `XAU_PASSWORD` | a senha pedida na página inicial (10+ caracteres) |
| `XAU_KEY` | (opcional) a chave da sessão e do link de emergência; sem ela, uma é criada em `/data/web-key.txt` e aparece nos logs |
| `XAU_DATA_DIR` | pasta de dados (o volume), `/data` na imagem |
| `XAU_EGRESS_PROXY` | proxy de saída (`socks5h://127.0.0.1:40000`); o entrypoint define quando há `WARP_*` |
| `XAU_ALLOW_DIRECT_EGRESS` | **só testes**: aceita falar com a Microsoft sem o WARP |
| `XAU_CHROMIUM` | caminho do navegador (na imagem: `/usr/bin/chromium`) |
| `XAU_CHROMIUM_NO_SANDBOX` | roda o Chromium sem o sandbox (necessário como root, na imagem) |
| `XAU_WEB_BENCH` | **só testes**: liga a página de benchmark local |
