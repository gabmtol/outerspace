<p align="center">
  <img src="assets/icon/outerspace-1024.png" width="128" alt="Ícone do OuterSpace">
</p>

<h1 align="center">OuterSpace</h1>

<p align="center">
  Um monitor virtual para agentes de IA trabalharem sem atrapalhar você.<br>
  <sub>Windows 11 · Rust + Tauri 2 · MCP · Liquid Glass (macOS 27 Golden Gate)</sub>
</p>

<p align="center">
  <a href="https://github.com/gabmtol/outerspace/releases/latest/download/OuterSpace-setup.exe"><b>Baixar para Windows</b></a> ·
  <a href="https://gabmtol.github.io/outerspace/">Site</a> ·
  <a href="docs/PLANO.md">Plano</a>
</p>

---

> **Status:** MVP completo (F0–F5): instalador, monitor virtual, servidor MCP com 10 ferramentas, ponte stdio e setup dos clientes. Agentes reais já usam o OuterSpace pelo Claude Code. Próximos passos na seção v1 do [plano de implementação](docs/PLANO.md).

## O que é

O OuterSpace cria um **monitor virtual de verdade** no Windows (um display que a GPU renderiza normalmente) e entrega ele a um agente de IA via **MCP**. O agente abre programas lá, tira screenshots, clica e digita — enquanto você continua usando o seu monitor sem perder o mouse, o teclado ou o foco.

- **Fica na bandeja.** Você inicia uma vez e ele fica em segundo plano (e o agente o abre sozinho quando precisa).
- **Um switch.** Ligado: o monitor virtual aparece e o servidor MCP aceita chamadas. Desligado: o monitor some e tudo é bloqueado.
- **Setup em um clique** para **Claude Code**, **Codex CLI** e **OpenCode** (ou qualquer cliente MCP).
- **Sem problemas de GPU.** Como é um display ativo, Chrome, VS Code e apps DirectX renderizam como em qualquer monitor — diferente de janelas minimizadas ou desktops ocultos.

<p align="center">
  <img src="docs/images/ui-preview.png" alt="Janela do OuterSpace: desligado, ativo, driver ausente (escuro) e ativo (escuro)">
  <br><sub>Prévia da UI com dados de exemplo: desligado, ativo, driver ausente e ativo no tema escuro.</sub>
</p>

## Instalar

1. Baixe o [`OuterSpace-setup.exe`](https://github.com/gabmtol/outerspace/releases/latest/download/OuterSpace-setup.exe) da última release (ou gere com `scripts/build-installer.ps1`, veja abaixo) e rode. Instala só para o seu usuário, em `%LOCALAPPDATA%\OuterSpace`, sem pedir administrador. O instalador ainda não é assinado: se o Windows avisar, use *Mais informações → Executar assim mesmo*.
2. Abra o OuterSpace e ligue o switch. Na primeira vez ele pede para instalar o driver de monitor virtual (**Parsec VDD**, assinado pela Parsec): baixa o instalador oficial, confere o SHA-256 e pede permissão de administrador uma única vez.
3. Em **Connect an agent**, clique em **Add** no Claude Code, Codex ou OpenCode. Reinicie o cliente se ele estiver aberto.

Pronto: peça ao agente algo como *"use o OuterSpace para abrir o Edge em example.com e me diga o que aparece"*.

Os dados do app (token, configurações, logs) ficam em `%LOCALAPPDATA%\com.gabmtol.outerspace`.

## Como funciona

```
Claude Code ─┐
Codex CLI  ──┼─ stdio ─▶ outerspace-mcp.exe ─ HTTP local + token ─▶ OuterSpace.exe (bandeja)
OpenCode   ──┘          (abre o app se preciso)                     ├─ monitor virtual (driver IddCx)
                                                                    ├─ abre/move janelas sem roubar foco
                                                                    ├─ screenshot do monitor virtual
                                                                    └─ clique/teclado sem mover seu cursor
```

Ferramentas MCP: `status`, `launch`, `list_windows`, `screenshot`, `click`, `type_text`, `press_keys`, `scroll`, `move_window`, `close_window`. Detalhes, decisões técnicas, resultados medidos e limitações conhecidas estão no [plano](docs/PLANO.md).

Configuração manual (o botão **Add** faz isso por você):

```sh
# Claude Code
claude mcp add --transport stdio --scope user outerspace -- "%LOCALAPPDATA%\OuterSpace\outerspace-mcp.exe"

# Codex CLI (o app também ajusta startup_timeout_sec = 20 no config.toml)
codex mcp add outerspace -- "%LOCALAPPDATA%\OuterSpace\outerspace-mcp.exe"
```

OpenCode: o app edita o seu `opencode.jsonc`/`opencode.json` preservando comentários (veja §7 do plano). Qualquer outro cliente MCP pode usar o `outerspace-mcp.exe` por stdio, ou falar direto com `http://127.0.0.1:7342/mcp` com o cabeçalho `Authorization: Bearer <token>` (o token está em `config.json` na pasta de dados).

## Desenvolvimento

Requisitos: Windows 11, Rust 1.88+ (MSVC), Node 20.19+, WebView2 (já vem no Windows 11).

```sh
cd app
npm install
npm run tauri dev                 # app com recarga ao vivo da UI

# instalador (app em release + outerspace-mcp.exe ao lado), em target/release/bundle/nsis
powershell -ExecutionPolicy Bypass -File scripts/build-installer.ps1
```

```sh
cargo test --workspace                                 # testes (não precisam do driver)
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p outerspace-core -- --ignored --nocapture # testes de hardware: precisam do driver Parsec e plugam o monitor de verdade
```

Testes ponta a ponta com o SDK oficial de MCP:

```sh
cd tests/mcp
npm install
node shim.mjs            # pelo outerspace-mcp.exe, como os clientes usam (abre o app se estiver fechado)
npm run smoke            # direto no HTTP, com o monitor ligado; --apps abre Edge, Firefox e VS Code
node smoke.mjs --input   # clique/digitação/teclas/scroll no Edge em modo post
```

A UI também abre num navegador comum com dados de exemplo (`npm run dev` e depois `http://localhost:5173/?state=on`; outros estados: `off`, `starting`, `error&code=driverMissing`).

| Pasta | O que tem |
|---|---|
| `crates/outerspace-core` | Lógica sem UI: liga/desliga, driver (trait `VirtualDisplayBackend`), janelas, captura, entrada, setup dos clientes |
| `crates/outerspace-mcp` | Servidor MCP (rmcp) com as ferramentas, token e log de ações |
| `crates/outerspace-shim` | `outerspace-mcp.exe`: ponte stdio ⇄ HTTP que os clientes executam |
| `app/src-tauri` | App Tauri: bandeja, janela, comandos |
| `app/ui` | Interface (Vite + TypeScript, textos em `src/i18n/en.json`) |
| `tests/mcp` | Testes ponta a ponta com o SDK oficial de MCP |
| `assets/icon`, `scripts` | Ícone (gerado por `scripts/gen-app-icon.mjs`, empacotado por `scripts/build-icons.ps1`) e o build do instalador |
| `site` | O site em [gabmtol.github.io/outerspace](https://gabmtol.github.io/outerspace/), publicado pelo workflow `pages.yml` |

## Design

A UI e o ícone seguem o **Liquid Glass do macOS 27 Golden Gate**: vidro refinado com refração uniforme, contraste priorizando legibilidade (texto secundário ≥ 4,5:1 mesmo sobre papéis de parede extremos), cantos consistentes, toolbar unificada e ícone em camadas com profundidade. O acento é o laranja "International Orange" da ponte. Tokens e regras estão em §6.3 do plano. A UI é em inglês por padrão.

O ícone é o **Twin**: uma tela laranja e a sua gêmea virtual de vidro fosco. Na bandeja, a tela de trás muda de cor com o estado (laranja ligado, cinza desligado, vermelho com erro). Os candidatos descartados estão em `assets/icon/minimal/` e `assets/icon/variants/`.

## Roadmap

| Fase | Entrega | Status |
|---|---|---|
| F0 | Esqueleto Tauri 2 com bandeja e janela de vidro | ✅ |
| F1 | Monitor virtual liga/desliga pelo switch | ✅ liga em ~0,3 s, desliga em ~1,8 s |
| F2 | Abrir/listar/mover janelas + screenshot + servidor MCP | ✅ testado com Bloco de Notas, Edge, Firefox e VS Code |
| F3 | Clique, teclado e scroll sem roubar cursor/foco; cerca de mouse | ✅ post (Win32 e Chromium) e borrow verificados |
| F4 | Shim stdio + setup de Claude Code, Codex e OpenCode | ✅ agente real no Claude Code |
| F5 | Polimento, instalador, iniciar com o Windows | ✅ instalador sem admin; 0,04% CPU e 40 MB com o monitor ligado |

## Licença

[MIT](LICENSE) · avisos de terceiros em [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)
