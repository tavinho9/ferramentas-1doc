# Ferramentas 1Doc

Programa para Windows que reúne os quatro scripts de consulta ao 1Doc da
Prefeitura de Montes Claros numa janela só:

| Aba | Script de origem | O que faz |
|---|---|---|
| **Processos do dia** | `processos_do_dia_todos_1doc.py` | Processos emitidos num dia, com a etapa atual → `processos novos DD.MM.xlsx` e `processos_do_dia.json` |
| **Busca** | `busca_1doc.py` | Busca por etapa, fase, datas e mudanças de etapa → `Busca 1Doc ….xlsx` |
| **Faltantes na planilha** | `verificar_faltantes_1doc.py` | Processos emitidos no período escolhido, antes da Pesquisa de Mercado, que não estão na planilha de acompanhamento → `<planilha>_faltantes_….xlsx` |
| **Verificar planilha** | `verificar_planilha_unificada_1doc.py` | Processos da planilha que já chegaram à Pesquisa de Mercado → `<planilha>_verificada_….xlsx` |

As regras de cada script (critérios de etapa, filtros, colunas das planilhas,
caches) são as mesmas de antes. O que mudou é a borda: login, entradas,
pastas e a janela.

## O login é seu: o programa só espera

O programa **não pede nem guarda usuário e senha**. Quando precisa de uma
sessão, ele abre o Chrome (ou o Edge) na página do 1Doc e espera. Você entra
do jeito de sempre, na janela do navegador; assim que o 1Doc mostra a caixa
de entrada, o programa copia a sessão, fecha o navegador e continua sozinho.

A sessão fica guardada no seu usuário do Windows e é reaproveitada nas
próximas vezes, até o 1Doc expirá-la; aí o navegador abre de novo. O botão
**Sair do 1Doc** apaga a sessão guardada.

## Instalar

1. Baixe `Ferramentas1Doc-Setup-<versão>.exe` em **Releases** (ou no artefato
   da última execução em **Actions**).
2. Execute. Não precisa ser administrador: o programa vai para
   `%LOCALAPPDATA%\Programs\Ferramentas 1Doc`, com atalho no menu Iniciar
   (e, se marcar, na Área de Trabalho).
3. O instalador não tem assinatura digital, então o Windows pode mostrar
   "O Windows protegeu o computador". Clique em **Mais informações →
   Executar assim mesmo**.

Para atualizar, instale a versão nova por cima. Para remover: Configurações
do Windows → Aplicativos → Ferramentas 1Doc → Desinstalar.

## Usar

1. Abra **Ferramentas 1Doc** pelo menu Iniciar.
2. Clique em **Entrar no 1Doc** (ou direto em **Executar** numa aba). O
   navegador abre; faça o login nele. A faixa amarela no programa indica que ele
   está esperando.
3. Na aba desejada, preencha os campos e clique em **Executar**. O andamento
   aparece em *Mensagens*; **Cancelar** interrompe na próxima requisição.
4. A planilha sai na pasta configurada (padrão: Área de Trabalho) e abre no
   Excel ao terminar. Isso pode ser desligado em **Configurações**.

Na aba **Busca**, os filtros mais usados têm campo próprio. Os demais (por
exemplo `--setor-etapa`, `--cita-setor`, `--ordenar`) vão em **Outras
opções**, no mesmo formato da linha de comando. **Ver todas as opções…**
mostra a lista completa.

Na aba **Faltantes na planilha**, os campos **Emitidos de** e **até** definem
o período de emissão dos processos procurados. Aceitam DD/MM/AAAA, DD/MM/AA,
DD/MM, `hoje`, `ontem` ou `-N` (N dias atrás; `-90` = últimos 90 dias). Com
os campos vazios vale o padrão: de 1º de janeiro do ano mais antigo da planilha
até hoje. Ao lado dos campos aparece o período que será usado. O programa
guarda o que foi digitado para as próximas vezes; `-90` continua contando a
partir do dia da execução. O botão **Padrão** limpa os dois campos.

## Onde ficam os arquivos

| O quê | Onde |
|---|---|
| Planilhas geradas | Pasta das planilhas (Configurações). Padrão: Área de Trabalho (inclusive se ela estiver no OneDrive) |
| Sessão, caches, configurações e logs | `%LOCALAPPDATA%\Ferramentas1Doc` (botão *Abrir pasta de dados* nas Configurações) |

Cada execução grava um log em `%LOCALAPPDATA%\Ferramentas1Doc\logs`, útil se
algo der errado.

**Aproveitar o cache dos scripts antigos:** se você já rodava os
verificadores, copie o `hash_cache_1doc.json` que ficava ao lado deles para
`%LOCALAPPDATA%\Ferramentas1Doc`. Assim a primeira execução de *Faltantes na
planilha* não precisa reabrir todos os arquivados.

## Se algo não funcionar

- **O navegador não abre.** O programa usa o Chrome; se não houver Chrome,
  usa o Edge. Na primeira vez, o Selenium baixa o "driver" do navegador da
  internet (como nos scripts antigos). Se a rede bloquear esse download,
  escolha o outro navegador em Configurações, ou peça liberação de
  `googlechromelabs.github.io` / `storage.googleapis.com` (Chrome) ou
  `msedgedriver.microsoft.com` (Edge).
- **"Login não concluído: tempo esgotado".** O programa espera 10 minutos por
  padrão. O tempo pode ser aumentado em Configurações.
- **"Sessão recusada" no meio da execução.** A sessão do 1Doc expirou. Execute
  de novo: o navegador abre para você entrar.
- **"Interface não reconhecida" / coleta vazia.** A conta está com o "novo
  visual da 1Doc" ligado. Desligue a chave no topo da caixa de entrada; os
  scripts leem o visual clássico.
- **Antivírus bloqueou.** Programas feitos com PyInstaller às vezes geram
  alarme falso. Peça ao suporte para liberar a pasta
  `%LOCALAPPDATA%\Programs\Ferramentas 1Doc`.

## Para quem for mexer no código

```
app.py                         ponto de entrada (janela) e --autoteste
ferramentas1doc/
  sessao.py                    login com espera, sessão guardada, cancelamento
  processos_do_dia.py          ← processos_do_dia_todos_1doc.py
  busca.py                     ← busca_1doc.py
  faltantes.py                 ← verificar_faltantes_1doc.py
  verificar_planilha.py        ← verificar_planilha_unificada_1doc.py
  busca_argv.py                formulário da aba Busca → argumentos do busca.py
  caminhos.py, config.py       pastas e configurações
  gui/                         janela, abas, mensagens
build/                         PyInstaller (.spec) e Inno Setup (.iss)
tests/                         pytest (inclui um 1Doc de mentira em fake1doc.py)
```

O histórico do git começa com os quatro scripts originais, sem mudanças. O
segundo commit mostra exatamente o que foi alterado em cada um.

Rodar sem instalar (Python 3.10+):

```bash
pip install -r requirements.txt
```

```bash
python app.py
```

Cada ferramenta também roda pela linha de comando, por exemplo
`python -m ferramentas1doc.busca --help` ou
`python -m ferramentas1doc.processos_do_dia ontem` ou
`python -m ferramentas1doc.faltantes planilha.xlsx --de 01/01/2025 --ate 30/06/2025`.

Testes:

```bash
pip install -r requirements-build.txt
```

```bash
python -m pytest -q tests
```

**Gerar o instalador.** O GitHub Actions (`.github/workflows/build-windows.yml`)
compila a cada push na `main`. Ele roda os testes no Windows, inclusive o login
com Chrome real e a janela. Depois empacota, faz o autoteste do `.exe`, gera o
instalador, instala, testa o programa instalado e desinstala. O Setup.exe fica
como artefato da execução.

Para publicar uma versão:

1. Mude `VERSAO` em `ferramentas1doc/__init__.py` e descreva a versão em
   `NOTAS_DA_VERSAO.md`.
2. No GitHub, abra **Releases → Draft a new release** e crie a tag `vX.Y.Z`
   (a mesma versão) a partir da `main`.
3. Publique. Em poucos minutos o workflow compila, testa e anexa
   `Ferramentas1Doc-Setup-X.Y.Z.exe` à Release.

Enviar a tag pelo git também funciona: a execução da tag cria a Release.

Num PC Windows com Python, `build.bat` faz o mesmo localmente e gera
`dist\Ferramentas1Doc-Setup-<versão>.exe`.
