# Ferramentas 1Doc — versão Rust

A mesma ferramenta da raiz do repositório, reescrita em Rust: as quatro abas
(*Processos do dia*, *Busca*, *Faltantes na planilha*, *Verificar
planilha*), o login feito pela pessoa no Chrome/Edge e as mesmas planilhas.
A versão Python continua mantida; esta fica ao lado dela.

## O que é igual e o que muda

- **Regras, colunas e textos:** os mesmos. Cada módulo Rust espelha um
  módulo Python (tabela abaixo). Onde `faltantes.py` e `verificar_planilha.py`
  têm código idêntico, ele fica em `verificadores.rs`.
- **Pasta de dados:** a mesma (`%LOCALAPPDATA%\Ferramentas1Doc`), com os
  mesmos arquivos e formatos: `sessao.json`, `config.json`,
  `hash_cache_1doc.json`, `1doc_cache.json` e o perfil do navegador. As duas
  versões compartilham a sessão e os caches.
- **Instalação:** um executável (`Ferramentas1Doc.exe`) e a linha de comando
  (`ferramentas1doc-cli.exe`), em `%LOCALAPPDATA%\Programs\Ferramentas 1Doc
  Rust`, com o atalho *Ferramentas 1Doc (Rust)*. Não substitui a versão Python.
- **Janela:** egui em vez de Tk, com o mesmo arranjo. O desenho usa wgpu
  (DirectX 12, com o WARP em software onde não há placa de vídeo) ou, sem
  adaptador, OpenGL. `FERRAMENTAS1DOC_DESENHO=glow` ou `wgpu` força um deles.
- **Planilhas novas** (*Processos do dia*, *Busca*, *Faltantes*) são gravadas
  com rust_xlsxwriter. A planilha do *Verificar*, que parte da original e
  mantém a formatação, é editada com umya-spreadsheet. As larguras de coluna
  ficam ~0,7 maiores no arquivo: o rust_xlsxwriter grava a largura como o
  próprio Excel (com a margem interna), e a aparência é a mesma.

| Rust (`src/`) | Python (`ferramentas1doc/`) |
|---|---|
| `sessao.rs`, `navegador.rs` | `sessao.py`, `navegador.py` |
| `processos_do_dia.rs` | `processos_do_dia.py` |
| `busca/` (`args`, `parse`, `registro`, `saida`) | `busca.py` |
| `busca_argv.rs` | `busca_argv.py` |
| `faltantes.rs` | `faltantes.py` |
| `verificar_planilha.rs` | `verificar_planilha.py` |
| `verificadores.rs` | o que os dois acima têm de idêntico |
| `config.rs`, `caminhos.rs`, `saida.rs` | `config.py`, `caminhos.py`, `gui/log.py` |
| `gui/` (`janela`, `abas`, `calendario`) | `gui/` |
| `autoteste.rs`, `main.rs` | `app.py` |

## Compilar e rodar

Precisa do Rust estável (<https://rustup.rs>).

```bash
cargo run --release
```

```bash
cargo run --release --bin ferramentas1doc-cli -- busca --help
```

A linha de comando aceita os mesmos argumentos de `python -m
ferramentas1doc.<ferramenta>`: `processos-do-dia [DIA]`, `busca [opções]`,
`faltantes [PLANILHA] [--aba ABA] [--de DATA] [--ate DATA]` e
`verificar-planilha [PLANILHA] [--aba ABA] [--html PASTA]`.

## Testes

```bash
cargo test
```

Os testes usam um 1Doc de mentira num servidor HTTP local, com o mesmo HTML
de `tests/fake1doc.py`. Os do navegador de verdade rodam com
`AUTOTESTE_NAVEGADOR_REAL=1` (Chrome/Edge instalado) ou
`FERRAMENTAS1DOC_CHROME=/caminho/do/chrome`.

**Paridade com a versão Python.** `paridade/` roda as duas versões nas mesmas
planilhas de entrada (inclusive uma "difícil" para o *Verificar*: links,
mesclagens, tabela, fórmula e colunas de uma execução anterior), com o mesmo
HTML, e compara as planilhas célula a célula:

```bash
python paridade/gerar_python.py /tmp/paridade
```

```bash
PARIDADE_PASTA=/tmp/paridade cargo test --test paridade
```

```bash
python paridade/comparar.py /tmp/paridade
```

## Gerar o instalador

O GitHub Actions (`.github/workflows/build-windows-rust.yml`) roda os testes
no Linux e no Windows (com o Chrome real), a paridade com o Python, compila,
faz o autoteste do `.exe`, gera o instalador, instala, testa o instalado e
desinstala. O Setup.exe fica como artefato da execução.

Para publicar uma versão:

1. Mude `version` em `Cargo.toml` e descreva a versão em `NOTAS_DA_VERSAO.md`
   (os dois aqui em `rust/`).
2. No GitHub, crie a Release com a tag `rust-vX.Y.Z` (a mesma versão) a
   partir da `main`.
3. Em poucos minutos o workflow anexa `Ferramentas1Doc-Rust-Setup-X.Y.Z.exe`
   à Release.

As tags da versão Python continuam `vX.Y.Z`; uma não dispara a publicação da
outra.

Num PC Windows com Rust, `build\build.bat` faz o mesmo localmente e gera
`dist\Ferramentas1Doc-Rust-Setup-<versão>.exe` na raiz do repositório.
