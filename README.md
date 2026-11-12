# Ferramentas 1Doc

Programa para Windows que reúne os quatro scripts de consulta ao 1Doc da
Prefeitura de Montes Claros numa janela só:

| Aba | Script de origem | O que faz |
|---|---|---|
| **Processos do dia** | `processos_do_dia_todos_1doc.py` | Processos emitidos num dia, com a etapa atual → `processos novos DD.MM.xlsx` e `processos_do_dia.json` |
| **Busca** | `busca_1doc.py` | Busca por etapa, fase, datas, mudanças de etapa e novo processo emitido → `Busca 1Doc ….xlsx` |
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

Os campos de data têm, ao lado, um botão de calendário: ele abre o mês logo
abaixo do campo, e clicar num dia escreve a data (DD/MM/AAAA). Para ir a outro
mês ou ano sem passar mês a mês, clique no título do calendário (*março de
2025 ▾*): ele mostra os 12 meses do ano. Clicando no título de novo (*2025 ▾*),
aparecem os anos da década. Escolha o ano, depois o mês, e o calendário volta
aos dias. Nessas grades, ◀ ▶ e a roda do mouse passam o ano ou a década. No
teclado, as setas mudam o dia, PageUp/PageDown o mês, Shift+PageUp/PageDown o
ano e Enter escolhe. Esc ou um clique fora fecham o calendário sem mudar nada,
e **Limpar** esvazia o campo. Onde uma data futura não vale (o dia em
*Processos do dia* e o início do período em *Faltantes*), os dias, meses e anos
depois de hoje aparecem apagados. Digitar a data continua valendo, com os
mesmos formatos de antes.

Se a janela estiver pequena (ou o Windows usar escala de 125–150%), o
formulário da aba ganha barra de rolagem, e a roda do mouse rola o formulário.
O botão **Executar** e as *Mensagens* continuam sempre à vista. O divisor entre
a aba e as *Mensagens* pode ser arrastado.

Na aba **Busca**, os campos de etapa (*Etapa atual contém*, *Exceto a etapa*,
*Ainda não chegou a*, *Já chegou a*, *Já passou por*, *…para a etapa*,
*…saindo da etapa*) têm uma lista para escolher a etapa sem digitar. A lista
traz o fluxo da Abertura e as etapas dos quadros que as buscas anteriores já
leram, então cresce com o uso. Ainda dá para digitar parte do nome. Para juntar
mais de uma etapa no mesmo campo, digite `;` depois da primeira e escolha a
próxima na lista.

Ainda na **Busca**, **Emitiu novo processo** separa os processos de onde já
saiu um novo Proc. Administrativo (por exemplo, a Abertura emitida a partir
da Fase Preparatória): *Sim* fica só com eles, *Não* fica só com os que ainda
não emitiram nenhum. Em **…emitido de / até** dá para pedir que o novo
processo tenha sido emitido num período (com as datas, vale *Sim*).
Memorandos, ofícios etc. emitidos do processo não contam.

Nem sempre o processo novo é emitido do jeito normal ("emitir a partir deste
documento"): às vezes a Abertura é criada à parte e só cita o número da Fase
Preparatória, ou um despacho da fase avisa em que processo a demanda seguiu.
Por isso a busca acha o processo novo de quatro jeitos, mostrados na coluna
*Novos processos (vínculo)* e na coluna *Vínculo* da aba *Novos processos*:

| Vínculo | De onde vem | Data usada |
|---|---|---|
| *emissão* | evento "emitiu o Proc. Administrativo N/AAAA a partir deste documento" da linha do tempo | a do evento |
| *origem* | o processo novo traz este no cabeçalho de origem, sem o evento aqui | a de emissão do processo novo |
| *citado neste* | este processo cita o novo, no texto de abertura ou num despacho | a de emissão do processo novo |
| *cita este* | o processo novo cita este, no texto de abertura ou num despacho | a de emissão do processo novo |

A coluna *Citação* da aba *Novos processos* diz como a citação foi lida, da
mais confiável para a menos: *menção do 1Doc* (o processo escolhido na lista
ao escrever o despacho, que a página marca com `data-mention="2"`, no texto e
na lista de documentos mencionados), *link no texto* ou *número no texto*
(um número NN.NNN/AAAA digitado, com "processo", "nº", "abertura"… antes e
sem "lei", "requisição", "contrato"…). Vale conferir primeiro as que vieram
só do número digitado. O *Faltantes* e o *Verificar planilha* também passam
a usar a menção do 1Doc para achar o processo novo.

**Coluna Menções.** A planilha da *Busca*, a do *Faltantes* e a do
*Verificar planilha* têm a coluna *Menções*, com todos os processos citados
no texto de abertura e nos despachos (menção do 1Doc, link ou número com
"processo", "nº"… antes), mesmo quando não dá para dizer se são o processo
pai ou o filho. *(pai)* marca o processo de origem, *(filho)* o processo novo
achado e *(número no texto)* o citado só pelo número digitado; a célula leva
à primeira menção. Um link cujo texto não traz o número (um "clique aqui")
aparece com o número do processo quando o hash dele já está no cache, e como
*link ABCD…* quando não está: na página só há o hash. A menção a uma pessoa (o @ do despacho) não conta: o link dela leva à
página que quem escreveu tinha aberta, não a um processo. O número partido
pela formatação do editor (`40.<span>001/2026</span>`) é lido inteiro, como o
navegador mostra. A primeira *Busca* depois de atualizar reabre os processos
guardados no cache por versão anterior, para ler as menções; no `--offline`,
eles saem com "menções não lidas" na *Observação*.

As menções (*citado neste* e *cita este*) só valem para um Proc.
Administrativo que não tem outra origem registrada, foi emitido depois deste
e está adiante na licitação (Elaboração de Documentação Preparatória → Fase
Preparatória → Abertura). Assim o pagamento ou o contrato que citam o número
da licitação, a fase semelhante citada como referência e o processo antigo
ficam de fora. Para achar quem cita o processo, a busca lê, uma vez, os
processos da licitação de todas as caixas do setor emitidos desde o processo
mais antigo do resultado, e abre os processos citados que não estão nas
caixas. O que ela abre fica no cache; nas próximas buscas só é reaberto o que
se movimentou. O resumo fica na aba *Parâmetros* (linha *Menções*). Sem o
filtro de novo processo, as colunas mostram as menções entre os processos
já abertos, sem abrir outros.

A planilha também ganha a coluna *Novo processo emitido em* (o mais recente;
com período, o mais recente dentro dele) e, na aba *Novos processos*, cada
processo novo, o vínculo, como foi citado, a data, quem emitiu (na emissão normal), a etapa
em que ele está e o link para abri-lo. Na linha de comando:
`--novo-processo`, `--sem-novo-processo`, `--novo-processo-de` e
`--novo-processo-ate`; `--sem-mencoes` (em **Outras opções**) volta a
considerar só a emissão normal e não lê as outras caixas.

Os filtros mais usados têm campo próprio. Os demais (por
exemplo `--setor-etapa`, `--cita-setor`, `--ordenar`) vão em **Outras
opções**, no mesmo formato da linha de comando. **Ver todas as opções…**
mostra a lista completa. As datas, explicadas no próprio quadro de datas,
vão como DD/MM/AAAA com o ano completo (`05/11/2026`), AAAA-MM-DD, `hoje`,
`ontem` ou `-N` (N dias atrás: `-7` = uma semana). O dia informado entra na
busca: "até 31/10/2026" inclui o dia 31.

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
  usa o Edge (que já vem no Windows). Ele usa o navegador instalado e não
  baixa nada da internet: não há "driver". Se o Chrome não abrir, escolha o
  Edge em Configurações. A mensagem em *Mensagens* mostra o motivo de cada
  tentativa.
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
