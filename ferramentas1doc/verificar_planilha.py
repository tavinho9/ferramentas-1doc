"""
SCRIPT: Verifica se os processos de uma planilha já chegaram à
        "Pesquisa de Mercado" (ou a etapa posterior) no 1Doc
====================================================================

ENTRADA
-------
Uma planilha .xlsx com uma coluna de número de processo no formato
NN.NNN/AAAA (ex.: a exportada pelos scripts anteriores, com as colunas
Nº Processo | Prioridade | Objeto | Etapa | Secretaria). A coluna é
achada pelo cabeçalho ("Nº Processo", "Processo", "Número"...) ou, se
não houver cabeçalho reconhecível, pela coluna em que mais células
têm cara de número de processo.

SAÍDA
-----
<planilha>_verificada_AAAA-MM-DD_HHMM.xlsx, na pasta de saída (padrão: Área
de Trabalho; a original não é alterada), aberta depois de salva,
contendo SÓ os processos que chegaram à Pesquisa de Mercado ou
passaram dela:
  • a aba original, formatação preservada, sem as linhas dos demais
    processos, mais as colunas
        Etapa atual (1Doc) | Processo pai | Processo filho |
        Objeto (abertura) | Observação
    - Processo pai: de onde o processo foi emitido (cabeçalho de origem)
    - Processo filho: o emitido a partir dele. Para a elaboração/fase
      preparatória, o processo conferido que decidiu o veredito, com a
      cadeia quando houver mais de um nível (37.923/2026 → 39.144/2026);
      para os demais, os listados em "É base de" na página (só a tabela
      "É base de" e os eventos "emitiu o …", nunca links em despachos)
    - Objeto (abertura): campo adicional "Objeto" (descricao_objeto) do
      processo de abertura (fluxo com a Pesquisa de Mercado ou etiqueta
      "Abertura de Procedimento Licitatório"): o PRÓPRIO processo da linha,
      se ele for a abertura; senão, o filho de abertura; vazio se nenhum
      dos dois chegou à fase de abertura
    Se a planilha de entrada for uma saída anterior, as colunas com esses
    títulos (e a antiga "Novo processo") saem e o bloco é refeito no fim,
    com os valores atuais — anotações feitas à mão nelas não se mantêm.
  • o número do processo, o pai e o filho viram link para o 1Doc
  • autofiltro em todas as colunas e cabeçalho congelado
Os processos que ficaram de fora (ainda antes, indeterminados ou não
localizados) são listados no resumo do terminal. Se nenhum processo
chegou, nenhum arquivo é gerado.

COMO O PROCESSO É LOCALIZADO (do mais barato ao mais caro)
---------------------------------------------------------
  1. Cache local (hash_cache_1doc.json, na pasta de dados do programa): o hash de
     um processo não muda, então cada número só é procurado uma vez.
  2. Hiperlink já existente na célula do número (se a planilha tiver).
  3. Formulário "Localizar" do painel (formEmissao.php?formAcessoRapido,
     tipo Proc. Administrativo + número + ano) enviado por HTTP.
  4. O mesmo formulário preenchido num navegador invisível (Selenium), com
     os cookies da sessão, caso o servidor não devolva o hash por HTTP. Ele
     só é aberto se este passo for necessário.
Todo hash obtido em 2–4 é conferido no detalhe pelo campo #nome_em
("Proc. Administrativo 31.243/2026") antes de ser aceito.

COMO A ETAPA É AVALIADA
-----------------------
Lê o quadro .workflow_v2_kanban VISÍVEL (a página traz uma segunda
cópia, oculta, com tempos desatualizados). As colunas .kanban_estagios
estão na ordem do fluxo; a atual tem data-current="true".

  Fluxo com "Pesquisa de Mercado" (Abertura de Procedimento Licitatório):
    Validação de Requisição → Correção de Documentação → PESQUISA DE
    MERCADO → Consulta Orçamentária → Análise do Controle Interno →
    Aprovação do COMPAC → Etapa Final
    - etapa atual na posição da Pesquisa de Mercado ou depois → SIM
    - etapa atual antes, mas alguma coluna a partir da Pesquisa de
      Mercado guarda tempo de permanência (.kanban-time, "Ficou aqui
      15 horas") → SIM (retornou); protege o caso de voltar para
      "Correção de Documentação".
    - senão → NÃO
  Quadro sem etapa atual marcada → INDETERMINADO

PROCESSOS NA FASE DE ELABORAÇÃO (fluxo sem a etapa, ou sem quadro)
------------------------------------------------------------------
O próprio fluxo deles não tem Pesquisa de Mercado: ela acontece no
processo NOVO emitido a partir deles (a Abertura de Procedimento
Licitatório). O script procura esse processo e usa a etapa dele:
  - processo novo na Pesquisa de Mercado ou além → SIM (a linha da
    elaboração entra, com a coluna "Novo processo" e o link para ele)
  - processo novo antes dela → NÃO (ou entra mesmo assim, se
    INCLUIR_NOVO_ANTES_CORTE = True)
  - nenhum processo novo encontrado → NÃO
Os candidatos saem primeiro da lista "É base de" da própria página; o
vínculo é conferido pelo cabeçalho do processo novo, que mostra o
processo de origem (detalhes na seção NOVO PROCESSO, mais abaixo).

LOGIN
-----
Feito por você, na janela do navegador; o programa espera e reaproveita a
sessão (ferramentas1doc.sessao). Usuário e senha nunca passam por aqui.

USO
---
    No programa: aba "Verificar planilha".
    Linha de comando:
    python -m ferramentas1doc.verificar_planilha                 # abre seletor de arquivo
    python -m ferramentas1doc.verificar_planilha planilha.xlsx --aba "Planilha1"
"""

import argparse
import json
import os
import re
import threading
import time
import unicodedata
from concurrent.futures import ThreadPoolExecutor
from copy import copy
from datetime import datetime

import requests
from bs4 import BeautifulSoup
from openpyxl import load_workbook
from openpyxl.styles import Font, PatternFill
from openpyxl.utils import column_index_from_string, get_column_letter
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry

from . import caminhos, config, sessao as sessao_1doc


# ============================================================
# CONFIGURAÇÃO
# ============================================================

URL_1DOC    = sessao_1doc.URL_1DOC
TIMEOUT     = 20
WORKERS     = 4           # detalhes baixados em paralelo
PAUSA_REQ   = 0.3         # segundos após cada requisição (por thread)

ETAPA_CORTE = "Pesquisa de Mercado"

# Processos na fase de elaboração (fluxo sem a etapa de corte, ou sem quadro):
# procura o novo processo emitido a partir deles (normalmente a Abertura de
# Procedimento Licitatório) e usa a etapa DELE para decidir.
VERIFICAR_NOVO_PROCESSO = True
# False = a elaboração só entra na planilha se o processo novo já chegou à
#         etapa de corte (mesma regra dos demais).
# True  = entra assim que o processo novo existir, em qualquer etapa.
INCLUIR_NOVO_ANTES_CORTE = True
# Quantos níveis descer na cadeia de emissões (DFD → Elaboração → Fase
# Preparatória → Abertura = 3).
PROFUNDIDADE_MAX = 3
# Índice reverso origem → processo novo, lido das caixas do setor (ver a
# seção NOVO PROCESSO). Só é montado se a lista "É base de" da página não
# confirmar nenhum filho. False = nunca montar.
USAR_INDICE_ORIGEM = True
CAIXAS_INDICE = (("entrada", "Em aberto"), ("saida", "Caixa de saída"),
                 ("arquivo", "Arquivados"), ("favoritos", "Favoritos"))
MAX_PAGINAS_INDICE = 300      # por caixa
# Etiquetas que entram no índice (comparação sem acento e sem caixa)
_RE_ETIQ_INDICE = re.compile(r"abertura de procedimento|fase preparatoria")

# Campo adicional com a descrição do objeto no processo de abertura
# (id="campo_adicional_descricao_objeto", rótulo "Objeto*:")
CAMPO_OBJETO = "descricao_objeto"

# Tipo de documento no "Localizar" do painel: 3 = Proc. Administrativo
ID_DOCUMENTO = "3"

# Ao achar a coluna pelo cabeçalho, aceita qualquer um destes (sem acento/caixa)
CABECALHOS_NUMERO = ("n processo", "no processo", "numero processo",
                     "numero do processo", "processo", "numero", "n")

# Na pasta de dados do programa (compartilhado com o faltantes)
ARQ_CACHE = caminhos.arquivo_dados("hash_cache_1doc.json")

PASTA_HTML = None     # definida por --html PASTA

URL_PAINEL = f"{URL_1DOC}/?pg=painel/listar&meu=0&trocar=1&caixa=entrada"

try:
    PARSER = "lxml"
    BeautifulSoup("<p></p>", PARSER)
except Exception:          # lxml ausente: cai no parser nativo
    PARSER = "html.parser"


# ============================================================
# TEXTO E NÚMEROS
# ============================================================

def _sem_acento(txt):
    return "".join(c for c in unicodedata.normalize("NFD", txt or "")
                   if unicodedata.category(c) != "Mn")


def _norm(txt):
    """minúsculas, sem acento, só letras/dígitos separados por 1 espaço."""
    return " ".join(re.sub(r"[^a-z0-9]+", " ", _sem_acento(txt).lower()).split())


def _limpo(txt):
    return " ".join((txt or "").split())


_RE_NUMERO = re.compile(r"(\d{1,3}(?:\.\d{3})+|\d+)\s*/\s*(\d{4})")


def ler_numero(valor):
    """'Proc. Administrativo 31.632/2026' | '31632/2026' → ('31.632/2026', '31632', '2026')."""
    if valor is None:
        return None
    m = _RE_NUMERO.search(str(valor))
    if not m:
        return None
    digitos = m.group(1).replace(".", "").lstrip("0") or "0"
    ano = m.group(2)
    return f"{int(digitos):,}".replace(",", ".") + f"/{ano}", digitos, ano


# ============================================================
# PLANILHA DE ENTRADA
# ============================================================

def achar_coluna_numero(ws, max_linhas_cab=10):
    """(linha_cabecalho, coluna) da coluna de número de processo."""
    alvos = set(CABECALHOS_NUMERO)
    for r in range(1, min(ws.max_row, max_linhas_cab) + 1):
        for c in range(1, ws.max_column + 1):
            v = ws.cell(r, c).value
            if isinstance(v, str) and _norm(v) in alvos:
                return r, c

    # Sem cabeçalho reconhecível: coluna com mais números NN.NNN/AAAA
    melhor, melhor_c = 0, None
    for c in range(1, ws.max_column + 1):
        n = sum(1 for r in range(1, ws.max_row + 1) if ler_numero(ws.cell(r, c).value))
        if n > melhor:
            melhor, melhor_c = n, c
    if not melhor_c:
        raise SystemExit("✗ Nenhuma coluna com números de processo (NN.NNN/AAAA) na aba.")
    # cabeçalho = linha acima do primeiro número (0 se o número está na linha 1)
    primeira = next(r for r in range(1, ws.max_row + 1) if ler_numero(ws.cell(r, melhor_c).value))
    return primeira - 1, melhor_c


def limpar_excesso(ws):
    """Apaga as células só de formatação que ficam além da última linha e da
    última coluna com conteúdo. Planilhas formatadas 'na coluna inteira'
    chegam com milhares de células vazias até XFD/1048576: o openpyxl as
    conta em max_column/max_row, as colunas novas iriam parar depois da
    última coluna que o Excel admite e o arquivo abriria com aviso.
    Devolve (última linha, última coluna) com conteúdo."""
    com_valor = [k for k, c in ws._cells.items() if c.value not in (None, "")]
    ult_lin = max((r for r, _ in com_valor), default=1)
    ult_col = max((c for _, c in com_valor), default=1)
    for k in [k for k in ws._cells if k[0] > ult_lin or k[1] > ult_col]:
        del ws._cells[k]
    # Dimensões de coluna agrupadas (ex.: um único <col min=1 max=16384>):
    # viram uma por coluna, só até a última com conteúdo. Agrupadas, uma
    # largura nova em B se sobreporia ao grupo A:XFD e o Excel acusaria erro.
    for letra, dim in list(ws.column_dimensions.items()):
        lo = dim.min or column_index_from_string(letra)
        hi = dim.max or lo
        del ws.column_dimensions[letra]
        for c in range(lo, min(hi, ult_col) + 1):
            nova = copy(dim)
            nova.index, nova.min, nova.max = get_column_letter(c), c, c
            ws.column_dimensions[nova.index] = nova
    for n in [n for n in ws.row_dimensions if n > ult_lin]:
        del ws.row_dimensions[n]
    ws.auto_filter.ref = None
    return ult_lin, ult_col


def ler_planilha(caminho, aba=None):
    wb = load_workbook(caminho)          # sem data_only: preserva fórmulas da original
    if aba and aba not in wb.sheetnames:
        raise SystemExit(f"✗ Aba {aba!r} não existe. Abas: {', '.join(wb.sheetnames)}")
    ws = wb[aba] if aba else wb.active
    limpar_excesso(ws)
    lin_cab, col = achar_coluna_numero(ws)
    itens, ignoradas = [], []
    for r in range(lin_cab + 1, ws.max_row + 1):
        cel = ws.cell(r, col)
        num = ler_numero(cel.value)
        if not num:
            if cel.value not in (None, ""):
                ignoradas.append((r, cel.value))
            continue
        link = cel.hyperlink.target if cel.hyperlink is not None else None
        itens.append({"linha": r, "numero": num[0], "digitos": num[1], "ano": num[2],
                      "link_planilha": link})
    return wb, ws, lin_cab, col, itens, ignoradas


# ============================================================
# SESSÃO HTTP (o login é feito por você no navegador: ver sessao.py)
# ============================================================

def sessao_http(cookies, user_agent):
    s = requests.Session()
    s.headers.update({
        "User-Agent": user_agent or "Mozilla/5.0",
        "Accept-Language": "pt-BR,pt;q=0.9",
    })
    for c in cookies:
        s.cookies.set(c["name"], c["value"], domain=c.get("domain"), path=c.get("path") or "/")
    retry = Retry(total=3, backoff_factor=0.6, status_forcelist=(429, 500, 502, 503, 504),
                  allowed_methods=("GET",))     # POST do Localizar não é repetido
    adaptador = HTTPAdapter(max_retries=retry, pool_connections=WORKERS, pool_maxsize=WORKERS)
    s.mount("https://", adaptador)
    s.mount("http://", adaptador)
    return s


class SessaoExpirada(RuntimeError):
    """A resposta veio com cara de tela de login."""


_MARCAS_APP = ("link_emissao_a", "caixa_parametro", "navega_caixa",
               "kanban_estagios", "nome_em", "tit_assunto")


def _parece_login(html):
    tem_senha = (re.search(r"""type\s*=\s*["']?password""", html, re.I)
                 or 'name="identifier"' in html)
    return bool(tem_senha) and not any(m in html for m in _MARCAS_APP)


def _texto(r):
    if "charset" not in r.headers.get("Content-Type", "").lower():
        r.encoding = "utf-8"
    return r.text


def baixar(sessao, url, params=None, xhr=False, referer=None):
    sessao_1doc.checar_cancelamento()     # botão Cancelar da interface
    cabecalhos = {}
    if xhr:
        cabecalhos["X-Requested-With"] = "XMLHttpRequest"   # como o jQuery da interface
    if referer:
        cabecalhos["Referer"] = referer
    r = sessao.get(url, params=params, headers=cabecalhos or None, timeout=TIMEOUT)
    r.raise_for_status()
    html = _texto(r)
    if _parece_login(html):
        raise SessaoExpirada(f"{url.split('?')[0]} devolveu a tela de login")
    if PASTA_HTML and hash_de(url):            # --html: guarda a página para diagnóstico
        try:
            with open(os.path.join(PASTA_HTML, hash_de(url) + ".html"), "w", encoding="utf-8") as f:
                f.write(html)
        except OSError:
            pass
    time.sleep(PAUSA_REQ)
    return html


# ============================================================
# LOCALIZAR O PROCESSO (número → hash)
# ============================================================

_RE_HASH = re.compile(r"[?&](?:amp;)?hash=([0-9A-Fa-f]{16,40})")


def url_detalhe(hash_):
    return f"{URL_1DOC}/?pg=doc/ver&hash={hash_}&itd={ID_DOCUMENTO}"


def hash_de(texto):
    m = _RE_HASH.search(texto or "")
    return m.group(1).upper() if m else None


def carregar_cache():
    try:
        with open(ARQ_CACHE, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def salvar_cache(cache):
    try:
        with open(ARQ_CACHE, "w", encoding="utf-8") as f:
            json.dump(cache, f, ensure_ascii=False, indent=1, sort_keys=True)
    except OSError as e:
        print(f"  ⚠ cache não salvo ({e})")


def _candidatos_resposta(r):
    """Hashes que a resposta do Localizar pode trazer, em ordem de confiança:
    redirecionamentos, URL final, campos JSON e, só se a resposta for curta
    (não é uma página inteira com a caixa de entrada), o corpo."""
    vistos = []

    def add(h):
        if h and h not in vistos:
            vistos.append(h)

    for resp in list(r.history) + [r]:
        add(hash_de(resp.headers.get("Location", "")))
    add(hash_de(r.url))
    corpo = _texto(r)
    try:
        dados = json.loads(corpo)
    except ValueError:
        dados = None
    if dados is not None:
        add(hash_de(json.dumps(dados)))
    elif len(corpo) < 5000:
        add(hash_de(corpo))
    return vistos


def localizar_http(sessao, item):
    sessao_1doc.checar_cancelamento()
    dados = {"id_documento": ID_DOCUMENTO, "id_assunto": "",
             "num_codigo": item["digitos"], "ano": item["ano"]}
    r = sessao.post(f"{URL_1DOC}/formEmissao.php?formAcessoRapido", data=dados,
                    headers={"Referer": URL_PAINEL, "X-Requested-With": "XMLHttpRequest"},
                    timeout=TIMEOUT, allow_redirects=True)
    r.raise_for_status()
    time.sleep(PAUSA_REQ)
    return _candidatos_resposta(r)


def localizar_selenium(driver, item):
    """Preenche o 'Localizar' do painel no navegador e lê o hash da URL."""
    from selenium.common.exceptions import TimeoutException, WebDriverException
    from selenium.webdriver.common.by import By
    from selenium.webdriver.support import expected_conditions as EC
    from selenium.webdriver.support.ui import WebDriverWait

    try:
        driver.get(URL_PAINEL)
        wait = WebDriverWait(driver, TIMEOUT)
        num = wait.until(EC.presence_of_element_located((By.ID, "num_codigo")))
        # O <select> é escondido pelo select2; define o valor no próprio
        # elemento nativo, que é o que o formulário envia.
        driver.execute_script(
            "var s=document.getElementById('id_documento_rapido');"
            "s.value=arguments[0];"
            "s.dispatchEvent(new Event('change',{bubbles:true}));", ID_DOCUMENTO)
        num.clear()
        num.send_keys(item["digitos"])
        ano = driver.find_element(By.ID, "localiza_ano")
        ano.clear()
        ano.send_keys(item["ano"])
        url_antes = driver.current_url
        driver.find_element(By.CSS_SELECTOR, "form.form_localiza button[type='submit']").click()
        WebDriverWait(driver, TIMEOUT).until(
            lambda d: hash_de(d.current_url) or (d.current_url != url_antes and "pg=doc/ver" in d.current_url))
        return [h for h in [hash_de(driver.current_url)] if h]
    except (TimeoutException, WebDriverException):
        return []


_RE_NOME_EM = re.compile(r"""<input\b[^>]*\bid\s*=\s*["']nome_em["'][^>]*>""", re.I)
_RE_VALUE = re.compile(r"""\bvalue\s*=\s*["']([^"']*)["']""", re.I)


def ler_nome_em(html):
    """'Proc. Administrativo 31.243/2026' ou None."""
    m = _RE_NOME_EM.search(html)
    if not m:
        return None
    v = _RE_VALUE.search(m.group())
    return v.group(1) if v else None


_RE_ITEM_ATUAL = re.compile(r"""class\s*=\s*["'][^"']*\bemission_infos_item_atual\b""", re.I)


def ler_identificacao(html):
    """'Proc. Administrativo 31.243/2026' ou None. O #nome_em (hidden do
    formulário de despacho) só vem quando o usuário pode despachar no
    processo; processos de outros setores/arquivados chegam sem ele. Nesse
    caso lê o cabeçalho h2.emission_infos_item_atual, que sempre existe:
    <span class="nd">Proc. Administrativo</span> <small class="nd_num">15.725/2026</small>"""
    nome = ler_nome_em(html)
    if nome:
        return nome
    m = _RE_ITEM_ATUAL.search(html)
    if not m:
        return None
    inicio = html.rfind("<", 0, m.start())
    fim = html.find("</h2>", m.end())
    h2 = BeautifulSoup(html[inicio:(fim + 5 if fim >= 0 else inicio + 3000)], PARSER)
    tipo, num = h2.select_one(".nd"), h2.select_one(".nd_num")
    if tipo is None or num is None:
        return None
    return f"{_limpo(tipo.get_text(' '))} {_limpo(num.get_text(' '))}"


def confere(html, item):
    """True = o detalhe é do processo procurado; None = página sem identificação."""
    nome = ler_identificacao(html)
    if nome is None:
        return None
    lido = ler_numero(nome)
    return bool(lido) and lido[1:] == (item["digitos"], item["ano"])


# ============================================================
# LEITURA DO DETALHE (kanban e etiqueta)
# ============================================================

def ler_quadro(html):
    """Lista de etapas do quadro VISÍVEL, em ordem:
    [{'nome', 'atual', 'marca', 'final'}]; [] se não houver quadro.
    'marca' é o tempo registrado na coluna: na etapa atual vem em
    .kanban_item ("Esta aqui 1 hora"); nas já percorridas, em .kanban-time
    ("Ficou aqui 15 horas"). A cópia oculta do quadro não traz este
    histórico, por isso só a visível serve."""
    # mira o atributo class (não o CSS/JS que citam o seletor); 1ª ocorrência = visível
    m = re.search(r"""class\s*=\s*["'][^"']*\bworkflow_v2_kanban\b""", html)
    if not m:
        return []
    inicio = html.rfind("<", 0, m.start())
    soup = BeautifulSoup(html[inicio:inicio + 60000], PARSER)
    quadro = soup.select_one(".workflow_v2_kanban")
    if quadro is None:
        return []
    etapas = []
    for est in quadro.select(".kanban_estagios"):
        titulo = est.select_one(".kanban_titulo")
        marcas = [_limpo(el.get_text()) for el in
                  est.select(".kanban_itens .kanban_item, .kanban_itens .kanban-time")]
        etapas.append({
            "nome": _limpo(titulo.get_text()) if titulo else "",
            "atual": str(est.get("data-current", "")).lower() == "true",
            "marca": "; ".join(m for m in marcas if m),
            "final": "estagio_final" in (est.get("class") or []),
        })
    return etapas


_RE_TIT_ASSUNTO = re.compile(r"""class\s*=\s*["'][^"']*\btit_assunto\b""", re.I)


def _h4_assunto(html):
    """O <h4 class="tit tit_assunto"> do cabeçalho (objeto + badge do
    assunto), já parseado; None se a página não tiver."""
    for m in _RE_TIT_ASSUNTO.finditer(html):      # mira o atributo, não o CSS
        inicio = html.rfind("<", 0, m.start())
        fim = html.find("</h4>", m.end())
        fim = fim + 5 if fim >= 0 else inicio + 8000
        h4 = BeautifulSoup(html[inicio:fim], PARSER).select_one(".tit_assunto")
        if h4 is not None:
            return h4
    return None


def ler_etiqueta(html):
    """Assunto do badge em h4.tit_assunto, sem o prefixo 'Assunto:'."""
    h4 = _h4_assunto(html)
    badge = h4.select_one(".badge, span[title]") if h4 is not None else None
    if badge is None:
        return ""
    t = badge.get("title") or badge.get("data-original-title") or badge.get_text()
    return _limpo(re.sub(r"^\s*Assunto\s*:\s*", "", t))


_RE_CAMPO = re.compile(r"""id\s*=\s*["']campo_adicional_([A-Za-z0-9_]+)["']""")


def ler_objeto(html):
    """Campo adicional "Objeto" do processo (#campo_adicional_<CAMPO_OBJETO>),
    não o título: "Contratação de empresa especializada para a execução de
    OBRA DE ...". O texto completo vem no data-valor do rótulo (o <small>
    visível é o reserva). Vazio se o processo não tiver o campo."""
    campos = list(_RE_CAMPO.finditer(html))
    blocos = {}
    for k, m in enumerate(campos):
        inicio = html.rfind("<", 0, m.start())
        fim = campos[k + 1].start() if k + 1 < len(campos) else m.end() + 20000
        blocos[m.group(1)] = html[inicio:fim]
    trecho = blocos.get(CAMPO_OBJETO)
    if trecho is None:
        # outro modelo de formulário: o campo cujo rótulo é "Objeto"
        for chave, tr in blocos.items():
            sp = BeautifulSoup(tr, PARSER).select_one("[data-valor], span[for]")
            if sp is not None and _norm(sp.get_text(" ")) == "objeto":
                trecho = tr
                break
    if trecho is None:
        return ""
    bloco = BeautifulSoup(trecho, PARSER)
    rotulo = bloco.select_one("[data-valor]")
    texto = rotulo.get("data-valor", "") if rotulo is not None else ""
    if not texto.strip():
        small = bloco.select_one("small")
        texto = small.get_text(" ") if small is not None else ""
    # o data-valor pode trazer <br> (escapado na página, literal aqui)
    texto = re.sub(r"<br\s*/?>", "\n", texto, flags=re.I)
    texto = re.sub(r"<[^>]+>", " ", texto)
    return "\n".join(_limpo(l) for l in texto.splitlines() if l.strip())


SIM, NAO, INDET = "Sim", "Não", "Indeterminado"


def avaliar(etapas, etiqueta=""):
    """(veredito, etapa_atual, observação)."""
    corte = _norm(ETAPA_CORTE)
    if not etapas:
        obs = "Sem quadro de etapas"
        return NAO, "", obs + (f" — {etiqueta}" if etiqueta else "")

    nomes = [e["nome"] for e in etapas]
    atual_idx = next((i for i, e in enumerate(etapas) if e["atual"]), None)
    atual = nomes[atual_idx] if atual_idx is not None else ""
    idx_corte = next((i for i, n in enumerate(nomes) if corte in _norm(n)), None)

    if idx_corte is None:
        fluxo = " → ".join(nomes)
        return NAO, atual, f"Fluxo sem {ETAPA_CORTE} ({etiqueta or fluxo})"

    passou = [e["nome"] for e in etapas[idx_corte:] if e["marca"] and not e["atual"]]
    if atual_idx is None:
        if passou:
            return SIM, "", f"Quadro sem etapa atual; registro em {', '.join(passou)}"
        return INDET, "", "Quadro sem etapa atual marcada"
    if atual_idx >= idx_corte:
        if atual_idx == idx_corte:
            return SIM, atual, "Na etapa de corte"
        obs = f"Além de {ETAPA_CORTE}"
        # Sem tempo registrado em nenhuma coluna entre o corte e a atual: o
        # processo foi movido por cima delas (ex.: direto para a Etapa Final).
        # O veredito fica, mas a observação avisa para conferir.
        # (só avisa se o quadro tem histórico em outras colunas, para não
        # acusar processos cujo quadro não registra tempos)
        if (any(e["marca"] for e in etapas if not e["atual"])
                and not any(e["marca"] for e in etapas[idx_corte:atual_idx])):
            obs += f" — sem registro de passagem por {ETAPA_CORTE}, conferir"
        return SIM, atual, obs
    if passou:
        return SIM, atual, f"Retornou após passar por {', '.join(passou)}"
    return NAO, atual, f"Antes de {ETAPA_CORTE}"


# ============================================================
# PIPELINE POR PROCESSO
# ============================================================

def sem_etapa_corte(etapas):
    """Fase de elaboração: sem quadro, ou quadro cujo fluxo não tem a etapa de
    corte (Fase Preparatória, Elaboração de Documentos Preparatórios)."""
    corte = _norm(ETAPA_CORTE)
    return not any(corte in _norm(e["nome"]) for e in etapas)


# ============================================================
# NOVO PROCESSO EMITIDO A PARTIR DA ELABORAÇÃO
# ============================================================
#
# Filho → pai: o processo novo traz no cabeçalho o processo de onde foi
# emitido
#     <h2 class="tt pull-left emission_infos_item_from" title="...">
#       <a class="nd" href="./?pg=doc/ver&itd=3&hash=5077DF...&origem=base">
#         Proc. Administrativo <small>28.133/2026</small>
# Pai → filho: a página do processo de origem agora também lista os
# emitidos a partir dela, em dois lugares
#     <span class="base_de">É base de:</span>
#     <table class="... table_base"> <a href="./?pg=doc/ver&hash=65ED...
#         &origem=emissao_anexada">Proc. Administrativo 39.144/2026 - Abertura...
# e no evento da linha do tempo
#     <small class="marcou_quem">emitiu o <a href="./?pg=doc/ver&hash=65ED...
#         &origem=emissao_evento_base">Proc. Administrativo 39.144/2026</a>
#         a partir deste documento.
# Os candidatos vêm destas fontes e cada um é aberto e conferido pelo
# cabeçalho de origem:
#   1. cache de execuções anteriores
#   2. lista "É base de" e eventos "emitiu o … a partir deste documento"
#   3. links para outros processos dentro dos despachos (.texto_original)
#   4. números NN.NNN/AAAA citados nos despachos, com contexto de processo
#      ("processo", "abertura", "emitido", "nº"...) e sem contexto de lei,
#      requisição, contrato etc.
#   5. só se 2 não confirmar nenhum filho (página antiga ou sem a lista):
#      índice reverso origem → processo novo, montado uma vez por execução:
#      percorre as caixas do setor (Em aberto, Saída, Arquivados, Favoritos),
#      pega só os Proc. Administrativos com etiqueta de Abertura ou de Fase
#      Preparatória emitidos desde 1º/jan do ano mais antigo da planilha, e
#      lê a origem de cada um. A origem não muda, então fica no cache e cada
#      processo só é aberto na primeira execução em que aparece.
#      (O filtro pai_raiz da listagem foi testado e o servidor não o aplica.)
# Aceite:
#   • origem do candidato = a elaboração           → vínculo confirmado
#   • candidato sem origem, citado em despacho, com
#     fluxo que tem a etapa de corte (abertura)     → aceito como "citado"
#   • origem do candidato = outro processo          → descartado
# Se o processo novo também for de elaboração (ex.: DFD → Elaboração →
# Fase Preparatória → Abertura), a busca desce até PROFUNDIDADE_MAX níveis.

_RE_ORIGEM = re.compile(r"""class\s*=\s*["'][^"']*\bemission_infos_item_from\b""", re.I)
_RE_CTX_SIM = re.compile(r"(proc|abertura|emitid|gerad|autuad|instaurad|\bn[o]?\b|numero)")
_RE_CTX_NAO = re.compile(r"(lei|decreto|portaria|requisi|\brc\b|contrato|\bata\b|edital|pregao|"
                         r"oficio|memorando|resolucao|instrucao|acordao|cnpj|cpf)")


def ler_origem(html):
    """(hash, número) do processo de origem no cabeçalho, ou (None, None)."""
    m = _RE_ORIGEM.search(html)
    if not m:
        return None, None
    inicio = html.rfind("<", 0, m.start())
    soup = BeautifulSoup(html[inicio:inicio + 3000], PARSER)
    h2 = soup.select_one(".emission_infos_item_from")
    a = h2.select_one("a[href]") if h2 else None
    if a is None:
        return None, None
    num = ler_numero(a.get_text(" "))
    return hash_de(a["href"]), (num[0] if num else None)


def ler_filhos(html):
    """[(hash, número, rótulo)] dos Proc. Administrativos emitidos a partir
    desta página ("É base de" e eventos "emitiu o … a partir deste
    documento"), em ordem, sem repetir. A lista "É base de" também traz
    memorandos, pareceres, ofícios etc. emitidos do processo: esses ficam de
    fora. O rótulo é o assunto que o link mostra depois do número ("Abertura
    de Procedimento Licitatório"), quando houver.
    Só lê a tabela .table_base e os eventos .marcou_quem: um despacho que
    cita outro processo ("a demanda está sendo prosseguida através do
    Proc. Administrativo 30.161/2026") gera um link com o MESMO
    origem=emissao_anexada, e o regex na página inteira o tomava por filho."""
    from bs4 import SoupStrainer
    # Função, não lista: com lista o SoupStrainer compara a string de classe
    # inteira e perde <table class="menor table ... table_base">.
    alvo = {"table_base", "marcou_quem"}
    areas = BeautifulSoup(html, PARSER, parse_only=SoupStrainer(
        class_=lambda c: bool(c) and not alvo.isdisjoint(str(c).split())))
    filhos = {}
    for a in areas.select("a[href]"):
        if not re.search(r"origem=(?:emissao_anexada|emissao_evento_base)", a["href"]):
            continue
        h = hash_de(a["href"])
        texto = _limpo(a.get_text(" "))
        if not h or "proc administrativo" not in _norm(texto):
            continue
        num = ler_numero(texto)
        rotulo = _limpo(texto.split(" - ", 1)[1]) if " - " in texto else ""
        antigo = filhos.get(h)
        if antigo is None or (not antigo[0] and num) or (not antigo[1] and rotulo):
            filhos[h] = ((num[0] if num else (antigo or (None,))[0]),
                         rotulo or (antigo[1] if antigo else ""))
    return [(h, n, r) for h, (n, r) in filhos.items()]


def _tem_abertura(a):
    """A cadeia a → via → via… chega a um processo de abertura?"""
    while a:
        if a.get("abertura"):
            return True
        a = a.get("via")
    return False


def citacoes(html, proprio):
    """Candidatos citados nos despachos: ([hashes de links], [números])."""
    from bs4 import SoupStrainer
    blocos = BeautifulSoup(html, PARSER,
                           parse_only=SoupStrainer(class_="texto_original"))
    hashes, numeros = [], []
    ano_min = int(proprio["ano"])
    for a in blocos.select("a[href]"):
        h = hash_de(a["href"])
        if h and h not in hashes:
            hashes.append(h)
    texto = blocos.get_text(" ")
    for m in _RE_NUMERO.finditer(texto):
        num = ler_numero(m.group())
        if not num or num[0] == proprio["numero"] or int(num[2]) < ano_min:
            continue
        antes = _norm(texto[max(0, m.start() - 45):m.start()])
        if not _RE_CTX_SIM.search(antes) or _RE_CTX_NAO.search(antes[-30:]):
            continue
        if num[0] not in numeros:
            numeros.append(num[0])
    return hashes, numeros


_RE_TOTAL_PAG = re.compile(r"P(?:á|&aacute;|a)gina\s*(\d+)\s*/\s*(\d+)")


def _html_listagem(texto):
    """A listagem (link.php?chamada=caixa) vem como HTML ou embrulhada em
    JSON; neste caso junta as strings que contêm marcação."""
    if not texto.lstrip().startswith(("{", "[")):
        return texto
    try:
        dados = json.loads(texto)
    except ValueError:
        return texto
    pedacos = []

    def coletar(x):
        if isinstance(x, str):
            if "<" in x:
                pedacos.append(x)
        elif isinstance(x, dict):
            for v in x.values():
                coletar(v)
        elif isinstance(x, list):
            for v in x:
                coletar(v)
    coletar(dados)
    return "\n".join(pedacos)


def linhas_listagem(html):
    """[(hash, número, etiqueta)] das linhas da listagem. A etiqueta é o
    title do badge de assunto da linha ("Abertura de Procedimento
    Licitatório", "Fase Preparatória de Procedimento Licitatório")."""
    soup = BeautifulSoup(html, PARSER)
    linhas = []
    for a in soup.select("a.link_emissao_a"):
        h = hash_de(a.get("href", ""))
        tr = a.find_parent("tr")
        num = ler_numero(a.get_text(" "))
        if not h or tr is None or not num:
            continue
        etiqueta = ""
        for el in tr.select("[title], [data-original-title]"):
            t_ = el.get("title") or el.get("data-original-title") or ""
            if _RE_ETIQ_INDICE.search(_norm(t_)):
                etiqueta = _limpo(t_)
                break
        linhas.append((h, num[0], etiqueta))
    return linhas


def listar_caixa(ctx, caixa, intervalo):
    """Todas as páginas de uma caixa, filtradas por emissão no intervalo."""
    linhas, pagina, total = [], 1, None
    while pagina <= min(total or MAX_PAGINAS_INDICE, MAX_PAGINAS_INDICE):
        params = {"chamada": "caixa", "pg": "painel/listar", "caixa": caixa,
                  "num_pagina": str(pagina), "ordenar": "1", "ordem": "desc",
                  "modo_visualizacao": "1", "intervalo": intervalo,
                  "emissoes_arquivado_prazos": "0", "itd": ID_DOCUMENTO}
        html = _html_listagem(baixar(ctx.sessao, f"{URL_1DOC}/link.php", params=params,
                                     xhr=True, referer=URL_PAINEL))
        achadas = linhas_listagem(html)
        if not achadas:
            break
        linhas += achadas
        m = _RE_TOTAL_PAG.search(html)
        if m:
            total = int(m.group(2))
        elif total is None and len(achadas) < 10:     # sem contador: página curta = última
            break
        pagina += 1
    return linhas


def montar_indice(ctx):
    """origem (hash e número) → [hashes dos processos emitidos a partir dela].
    Montado uma vez; as outras threads esperam e reaproveitam."""
    with ctx.lock_indice:
        if ctx.indice is not None:
            return ctx.indice
        intervalo = f"01/01/{ctx.ano_min} - {datetime.now():%d/%m/%Y}"
        print(f"\n  … montando o índice de origem (caixas do setor, emissões de {intervalo})")
        candidatos, falhas = {}, []
        for caixa, rotulo in CAIXAS_INDICE:
            try:
                for h, _num, etiqueta in listar_caixa(ctx, caixa, intervalo):
                    if etiqueta:
                        candidatos[h] = etiqueta
            except requests.RequestException as e:
                falhas.append(f"{rotulo} ({str(e)[:40]})")

        faltam = [h for h in candidatos if f"origem:{h}" not in ctx.cache]

        def ler(h):
            try:
                o_hash, o_num = ler_origem(baixar(ctx.sessao, url_detalhe(h)))
            except requests.RequestException:
                return
            with ctx.lock_cache:
                ctx.cache[f"origem:{h}"] = [o_hash or "", o_num or ""]

        with ThreadPoolExecutor(max_workers=WORKERS) as ex:
            list(ex.map(ler, faltam))

        indice = {}
        for h in candidatos:
            o_hash, o_num = ctx.cache.get(f"origem:{h}", ["", ""])
            for chave in {o_hash, o_num} - {""}:
                indice.setdefault(chave, []).append(h)
        ctx.indice = indice
        ctx.indice_info = {"candidatos": len(candidatos), "lidos": len(faltam),
                           "com_origem": sum(1 for h in candidatos
                                             if ctx.cache.get(f"origem:{h}", [""])[0]),
                           "falhas": falhas}
        info = ctx.indice_info
        print(f"  ✓ índice: {info['candidatos']} processo(s) de abertura/fase preparatória nas caixas, "
              f"{info['lidos']} lido(s) agora, {info['com_origem']} com origem registrada"
              + (f"; ⚠ falha em {', '.join(falhas)}" if falhas else "") + "\n")
        return indice


_ORDEM = {SIM: 0, INDET: 1, NAO: 2}


def buscar_novos(ctx, elab, html, nivel=1):
    """Processos novos emitidos a partir de `elab` (dict com numero, ano,
    hash). Lista de dicts: numero, hash, url, veredito, etapa, obs, vinculo."""
    achados, vistos = [], {elab["hash"]}
    chave_cache = f"novo:{elab['numero']}"

    def avaliar_candidato(h, pagina, citado):
        if h in vistos:
            return
        vistos.add(h)
        nome_em = ler_identificacao(pagina) or ""   # sem #nome_em: lê o cabeçalho
        nome = ler_numero(nome_em)
        if not nome or nome[0] == elab["numero"]:
            return
        if not _norm(nome_em).startswith("proc administrativo"):
            return                       # memorando, parecer, ofício… emitido do processo
        origem_hash, origem_num = ler_origem(pagina)
        if origem_hash == elab["hash"] or origem_num == elab["numero"]:
            vinculo = "origem no 1Doc"
        elif origem_hash is None and citado:
            vinculo = "citado em despacho"
        else:
            return
        etapas = ler_quadro(pagina)
        if vinculo == "citado em despacho" and sem_etapa_corte(etapas):
            return                       # citação fraca só vale para processo de abertura
        etiqueta = ler_etiqueta(pagina)
        v, etapa, obs = avaliar(etapas, etiqueta)
        # "abertura" = fluxo com a etapa de corte ou etiqueta de Abertura de
        # Procedimento Licitatório; é dele que sai o objeto para a planilha
        abertura = (not sem_etapa_corte(etapas)) or "abertura de procedimento" in _norm(etiqueta)
        novo = {"numero": nome[0], "ano": nome[2], "hash": h, "url": url_detalhe(h), "veredito": v,
                "etapa": etapa, "obs": obs, "vinculo": vinculo, "via": None,
                "abertura": abertura, "objeto": ler_objeto(pagina) if abertura else ""}
        if sem_etapa_corte(etapas) and nivel < PROFUNDIDADE_MAX:   # desce a cadeia
            netos = buscar_novos(ctx, novo, pagina, nivel + 1)
            if netos:
                novo["via"] = netos[0]
        achados.append(novo)

    def por_hash(h, citado):
        if h in vistos:
            return
        try:
            avaliar_candidato(h, baixar(ctx.sessao, url_detalhe(h)), citado)
        except requests.RequestException:
            pass

    def por_numero(numero, citado):
        n = ler_numero(numero)
        item = {"numero": n[0], "digitos": n[1], "ano": n[2]}
        r = abrir(ctx, item)
        if r is not None:
            avaliar_candidato(r[0], r[1], citado)

    for numero in ctx.cache.get(chave_cache, []):
        por_numero(numero, citado=True)
    # "É base de" / "emitiu o … a partir deste documento": a própria página
    # lista os filhos; cada um ainda é conferido pelo cabeçalho de origem.
    for h, _num, _rot in ler_filhos(html):
        por_hash(h, citado=False)
    # O índice das caixas só é dispensado se a lista levou, com origem
    # conferida, a um processo de abertura. Um filho confirmado que não é
    # abertura (ou cadeia que não chega a uma) não basta: a abertura pode ter
    # sido emitida de outro jeito e só o índice a acha.
    confirmado = any(a["vinculo"] == "origem no 1Doc" and _tem_abertura(a) for a in achados)
    hashes, numeros = citacoes(html, elab)
    for h in hashes:
        por_hash(h, citado=True)
    for numero in numeros:
        if not any(a["numero"] == numero for a in achados):
            por_numero(numero, citado=True)
    if USAR_INDICE_ORIGEM and not confirmado:
        indice = montar_indice(ctx)
        for h in indice.get(elab["hash"], []) + indice.get(elab["numero"], []):
            por_hash(h, citado=False)

    if achados:
        with ctx.lock_cache:
            ctx.cache[chave_cache] = sorted({a["numero"] for a in achados})

    achados.sort(key=lambda a: (_ORDEM[efetivo(a)["veredito"]], a["vinculo"] != "origem no 1Doc"))
    return achados


def efetivo(a):
    """O descendente que decide pela cadeia a → via → via…: o mais
    adiantado (Sim antes de Indeterminado antes de Não); empate fica com o
    mais próximo."""
    melhor, atual = a, a.get("via")
    while atual:
        if _ORDEM[atual["veredito"]] < _ORDEM[melhor["veredito"]]:
            melhor = atual
        atual = atual.get("via")
    return melhor


def cadeia_ate(a, alvo):
    """'28.133/2026 → 31.002/2026 → 39.144/2026' de a até alvo."""
    nums, atual = [a["numero"]], a
    while atual is not alvo and atual.get("via"):
        atual = atual["via"]
        nums.append(atual["numero"])
    return " → ".join(nums)


def aplicar_novo_processo(ctx, res, html):
    """Transfere para a linha da elaboração o veredito do processo novo."""
    novos = buscar_novos(ctx, res, html)
    res["novos"] = [n["numero"] for n in novos]
    if not novos:
        res["obs"] += " — nenhum novo processo emitido encontrado"
        return
    principal = novos[0]
    alvo = efetivo(principal)
    res["novo"] = alvo
    cadeia = cadeia_ate(principal, alvo)
    res["cadeia"] = cadeia
    # objeto do processo de abertura da cadeia (o decisivo, se for de
    # abertura; senão o primeiro de abertura descendo a partir do filho)
    ab = alvo if alvo.get("abertura") else None
    no = principal
    while ab is None and no:
        ab = no if no.get("abertura") else None
        no = no.get("via")
    if ab and ab.get("objeto"):
        res["objeto_filho"] = ab["objeto"]
    outros = [n for n in res["novos"] if n != principal["numero"]]
    obs = f"Novo processo {cadeia} ({principal['vinculo']}): {alvo['obs']}"
    if outros:
        obs += f"; também emitido(s): {', '.join(outros)}"
    res.update(veredito=alvo["veredito"], etapa=alvo["etapa"], obs=obs)


class Ctx:
    """O que todas as threads compartilham."""
    def __init__(self, sessao, cache, driver, ano_min=None):
        self.sessao, self.cache, self.driver = sessao, cache, driver
        self.lock_cache, self.lock_driver = threading.Lock(), threading.Lock()
        self.lock_indice = threading.Lock()
        self.ano_min = ano_min or datetime.now().year
        self.indice, self.indice_info = None, {}


def abrir(ctx, item, link=None):
    """Localiza e baixa o detalhe do processo `item` (dict com numero,
    digitos, ano). Devolve (hash, html, fonte, aviso) ou None."""
    chave = item["numero"]
    tentativas = []
    if chave in ctx.cache:
        tentativas.append(("cache", [ctx.cache[chave]]))
    if link and hash_de(link):
        tentativas.append(("link", [hash_de(link)]))
    tentativas.append(("localizar (HTTP)", None))
    tentativas.append(("localizar (navegador)", None))

    for fonte, hashes in tentativas:
        if hashes is None:
            try:
                if fonte.endswith("(HTTP)"):
                    hashes = localizar_http(ctx.sessao, item)
                elif ctx.driver is not None:
                    # navegador invisível aberto só agora, na 1ª vez que precisar
                    with ctx.lock_driver:
                        drv = ctx.driver.obter()
                        hashes = localizar_selenium(drv, item) if drv is not None else []
                else:
                    hashes = []
            except requests.RequestException:
                hashes = []
        for h in hashes:
            try:
                pagina = baixar(ctx.sessao, url_detalhe(h))
            except requests.RequestException:
                continue
            ok = confere(pagina, item)
            if ok is False:
                continue
            with ctx.lock_cache:
                ctx.cache[chave] = h
            return h, pagina, fonte, ("número não confirmado no detalhe; " if ok is None else "")
    return None


def processar(ctx, item):
    res = dict(item, hash=None, url="", veredito=INDET, etapa="", obs="", fonte="",
               elaboracao=False, novos=[], novo=None, pai=None, filhos=[])
    achado = abrir(ctx, item, item["link_planilha"])
    if achado is None:
        res["obs"] = "Processo não localizado no 1Doc"
        return res
    h, html, fonte, aviso = achado
    res.update(hash=h, url=url_detalhe(h), fonte=fonte)

    # Vínculos para o controle da movimentação: de onde o processo veio
    # (cabeçalho de origem) e o que foi emitido a partir dele ("É base de").
    pai_hash, pai_num = ler_origem(html)
    if pai_num:
        res["pai"] = {"numero": pai_num, "url": url_detalhe(pai_hash) if pai_hash else ""}
    filhos_pag = ler_filhos(html)
    res["filhos"] = [{"numero": n, "url": url_detalhe(fh), "hash": fh, "rotulo": rot}
                     for fh, n, rot in filhos_pag if n]

    etapas = ler_quadro(html)
    etiqueta = ler_etiqueta(html)
    veredito, etapa, obs = avaliar(etapas, etiqueta)
    res.update(veredito=veredito, etapa=etapa, obs=aviso + obs)

    # O processo da linha JÁ é a abertura (fluxo com a Pesquisa de Mercado ou
    # etiqueta de Abertura): o objeto está na própria página. Antes o objeto
    # só era lido de filhos, e quase todas as linhas que chegam à Pesquisa de
    # Mercado são aberturas sem filho de abertura → coluna sempre vazia.
    if (not sem_etapa_corte(etapas)) or "abertura de procedimento" in _norm(etiqueta):
        res["objeto_filho"] = ler_objeto(html)

    if VERIFICAR_NOVO_PROCESSO and sem_etapa_corte(etapas):
        res["elaboracao"] = True
        aplicar_novo_processo(ctx, res, html)
    elif res["filhos"] and not res.get("objeto_filho"):
        objeto_dos_filhos(ctx, res)
    return res


def objeto_dos_filhos(ctx, res):
    """Linha que não é de elaboração mas tem filhos em "É base de": abre os
    que o link diz serem de abertura (no máximo 3) e leva o objeto deles."""
    objetos = []
    for f in [f for f in res["filhos"] if "abertura de procedimento" in _norm(f["rotulo"])][:3]:
        try:
            pagina = baixar(ctx.sessao, f["url"])
        except requests.RequestException:
            continue
        o_hash, o_num = ler_origem(pagina)
        if o_hash != res["hash"] and o_num != res["numero"]:
            continue
        obj = ler_objeto(pagina)
        if obj:
            objetos.append(obj if len(res["filhos"]) == 1 else f"{f['numero']}: {obj}")
    res["objeto_filho"] = "\n".join(objetos)


# ============================================================
# EXPORTAÇÃO
# ============================================================

AZUL = PatternFill("solid", start_color="1F4E78")
LINK = dict(color="0563C1", underline="single")

# Colunas acrescentadas no fim da aba, nesta ordem
COLUNAS_NOVAS = ("Etapa atual (1Doc)", "Processo pai", "Processo filho",
                 "Objeto (abertura)", "Observação")
# Colunas que este script (ou versão anterior) gera: se a planilha de
# entrada for uma saída anterior, saem e o bloco é refeito junto, na ordem
# acima, com valores atuais. Comparação sem acento e sem caixa.
COLUNAS_GERADAS = {_norm(t) for t in COLUNAS_NOVAS + ("Novo processo",
                                                       "Objeto do processo filho")}


def _blocos(linhas):
    """[5, 6, 7, 10] → [(5, 3), (10, 1)]: (início, quantidade) de faixas contíguas."""
    faixas = []
    for n in sorted(linhas):
        if faixas and n == faixas[-1][0] + faixas[-1][1]:
            faixas[-1][1] += 1
        else:
            faixas.append([n, 1])
    return [tuple(f) for f in faixas]


def entra(r):
    """A linha vai para a planilha?"""
    if not r["url"]:
        return False
    if r["veredito"] == SIM:
        return True
    return bool(INCLUIR_NOVO_ANTES_CORTE and r.get("novo"))


def exportar(wb, ws, lin_cab, col_num, resultados, destino):
    """Mantém na aba só as linhas que chegaram à etapa de corte, acrescenta
    Etapa atual, Processo pai, Processo filho e Observação e liga o
    autofiltro em todas as colunas.
    Devolve quantas linhas ficaram; com 0, não salva arquivo nenhum."""
    if not any(entra(r) for r in resultados):
        return 0
    _, n_orig = limpar_excesso(ws)     # última coluna com conteúdo, não a última formatada

    # Aba auxiliar de versões antigas do script, se a planilha de entrada for uma saída anterior
    if "Mudaram" in wb.sheetnames and wb["Mudaram"] is not ws:
        del wb["Mudaram"]

    # Tabelas do Excel têm filtro próprio e conflitam com o autofiltro da aba:
    # viram intervalo comum (a formatação das células continua).
    for nome in list(ws.tables):
        del ws.tables[nome]

    # Sem cabeçalho: cria um, porque o autofiltro precisa de linha de títulos
    if lin_cab < 1:
        ws.insert_rows(1)
        for c in range(1, n_orig + 1):
            ws.cell(1, c, "Nº Processo" if c == col_num else f"Coluna {get_column_letter(c)}")
            ws.cell(1, c).font = Font(bold=True, color="FFFFFF")
            ws.cell(1, c).fill = AZUL
        lin_cab = 1
        resultados = [dict(r, linha=r["linha"] + 1) for r in resultados]

    # Linhas que ficam, na ordem original (as alturas são recalculadas no fim)
    ficam = sorted((r for r in resultados if entra(r)), key=lambda r: r["linha"])
    manter = {r["linha"] for r in ficam}
    remover = [n for n in range(lin_cab + 1, ws.max_row + 1) if n not in manter]
    # Colunas geradas por execução anterior (nunca a do número)
    velhas = [c for c in range(1, n_orig + 1) if c != col_num
              and _norm(str(ws.cell(lin_cab, c).value or "")) in COLUNAS_GERADAS]
    # delete_rows/delete_cols não deslocam mesclagens: desfaz as que tocam a
    # área de dados ou as colunas que saem (o valor fica na célula superior
    # esquerda).
    for faixa in list(ws.merged_cells.ranges):
        if faixa.max_row > lin_cab or any(faixa.min_col <= c <= faixa.max_col for c in velhas):
            ws.unmerge_cells(str(faixa))
    for c in reversed(velhas):                          # da direita para a esquerda
        ws.delete_cols(c)
    col_num -= sum(1 for c in velhas if c < col_num)
    n_orig -= len(velhas)
    for inicio, qtd in reversed(_blocos(remover)):     # de baixo para cima
        ws.delete_rows(inicio, qtd)
    # ...nem a referência dos hiperlinks: cada um continuaria apontando para a
    # célula antiga e sairia no lugar errado (ou sobre outro link).
    for cel in ws._cells.values():
        if cel.hyperlink is not None:
            cel.hyperlink.ref = cel.coordinate
    for n in list(ws.row_dimensions):
        if n > lin_cab:
            del ws.row_dimensions[n]

    # Bloco das colunas novas no fim, com o estilo do cabeçalho do número
    ref = ws.cell(lin_cab, col_num)
    col = dict(zip(("etapa", "pai", "filho", "objeto_filho", "obs"),
                   range(n_orig + 1, n_orig + 1 + len(COLUNAS_NOVAS))))
    for c, titulo in zip(col.values(), COLUNAS_NOVAS):
        cel = ws.cell(lin_cab, c, titulo)
        cel.font, cel.fill, cel.border = copy(ref.font), copy(ref.fill), copy(ref.border)
        cel.alignment = copy(ref.alignment)

    def escrever(linha, chave, texto, url=""):
        cel = ws.cell(linha, col[chave], texto or None)
        if url:
            cel.hyperlink = url
            cel.font = Font(**LINK)

    for i, r in enumerate(ficam):
        linha = lin_cab + 1 + i
        escrever(linha, "etapa", r["etapa"])
        pai = r.get("pai") or {}
        escrever(linha, "pai", pai.get("numero"), pai.get("url"))
        if r.get("novo"):
            # filho conferido pela origem; a cadeia mostra os níveis até o
            # processo que decidiu o veredito, e o link leva a ele
            escrever(linha, "filho", r.get("cadeia") or r["novo"]["numero"], r["novo"]["url"])
        elif r.get("filhos"):
            # emitidos a partir dele, segundo a lista "É base de" da página
            fs = r["filhos"]
            escrever(linha, "filho", "; ".join(f["numero"] for f in fs), fs[0]["url"])
        else:
            escrever(linha, "filho", None)
        escrever(linha, "objeto_filho", r.get("objeto_filho"))
        escrever(linha, "obs", r["obs"])
        cel = ws.cell(linha, col_num)
        cel.hyperlink = r["url"]
        cel.font = Font(**LINK, bold=cel.font.bold, size=cel.font.size, name=cel.font.name)

    n_cols = max(col.values())
    ultima_lin = lin_cab + len(ficam)
    ajustar_layout(ws, lin_cab, ultima_lin, n_cols)
    ws.auto_filter.ref = f"A{lin_cab}:{get_column_letter(n_cols)}{ultima_lin}"
    ws.freeze_panes = f"A{lin_cab + 1}"

    wb.save(destino)
    return len(ficam)


# Medidas do Excel: largura de coluna em "caracteres do 0" da fonte padrão
# (Calibri 11); altura de linha em pontos, ~15 pt por linha de texto em 11 pt.
LARG_MIN, LARG_MAX = 8, 60      # colunas de texto longo param em 60 e quebram linha
PT_POR_LINHA = 15.0


def _texto_celula(v):
    if v is None:
        return ""
    if isinstance(v, datetime):
        return v.strftime("%d/%m/%Y %H:%M") if (v.hour or v.minute) else v.strftime("%d/%m/%Y")
    if isinstance(v, float):
        return f"{v:,.2f}"
    return str(v)


def _compr(s):
    """Comprimento visual em 'caracteres do 0': maiúsculas e M/W são mais
    largas, pontuação e i/l/1 mais estreitas (Calibri)."""
    total = 0.0
    for ch in s:
        if ch in "MWmw@%":
            total += 1.35
        elif ch.isupper():
            total += 1.08
        elif ch == " ":
            total += 0.45
        elif ch in ".,;:!|'ilj1()/-":
            total += 0.55
        elif ch.isdigit():
            total += 1.0
        else:
            total += 0.88
    return total


def _fator_fonte(cel):
    tam = (cel.font.size or 11) / 11
    return tam * (1.1 if cel.font.bold else 1.0)


def _linhas_quebradas(texto, largura, fator):
    """Quantas linhas o texto ocupa numa coluna de 'largura' caracteres,
    quebrando por palavra como o Excel faz."""
    util = max(largura - 1.5, 1) / fator          # descontando a margem interna
    total = 0
    for paragrafo in texto.split("\n"):
        linhas, atual = 1, 0
        for palavra in paragrafo.split():
            n = _compr(palavra)
            if atual == 0:
                atual = n
            elif atual + 0.45 + n <= util:
                atual += 0.45 + n
            else:
                linhas += 1
                atual = n
            while atual > util:                   # palavra maior que a coluna
                linhas += 1
                atual -= util
        total += linhas
    return total


def ajustar_layout(ws, lin_cab, ultima_lin, n_cols):
    """Largura de cada coluna pelo conteúdo (cabeçalho e dados), com teto;
    o que passa do teto quebra linha, e cada linha ganha a altura do seu
    texto mais alto. O openpyxl não faz auto-ajuste: isto o emula."""
    larguras = {}
    for c in range(1, n_cols + 1):
        dados = 0.0
        for r in range(lin_cab + 1, ultima_lin + 1):
            cel = ws.cell(r, c)
            txt = _texto_celula(cel.value)
            if txt:
                maior = max(_compr(p) for p in txt.split("\n"))
                dados = max(dados, maior * _fator_fonte(cel))
        cab = ws.cell(lin_cab, c)
        txt_cab = _texto_celula(cab.value)
        # o cabeçalho pode quebrar em 2 linhas, mas nunca no meio de uma palavra
        palavra_cab = max((_compr(p) for p in txt_cab.split()), default=0) * _fator_fonte(cab)
        cab_inteiro = _compr(txt_cab) * _fator_fonte(cab)
        alvo = max(dados, palavra_cab, min(cab_inteiro, max(dados, 14))) + 2.5  # + seta do filtro
        larguras[c] = round(min(max(alvo, LARG_MIN), LARG_MAX), 1)
        ws.column_dimensions[get_column_letter(c)].width = larguras[c]

    for r in range(lin_cab, ultima_lin + 1):
        maior = 1
        for c in range(1, n_cols + 1):
            cel = ws.cell(r, c)
            txt = _texto_celula(cel.value)
            if not txt:
                continue
            al = copy(cel.alignment)
            al.wrap_text = True
            al.vertical = al.vertical or ("center" if r == lin_cab else "top")
            cel.alignment = al
            n = _linhas_quebradas(txt, larguras[c], _fator_fonte(cel))
            maior = max(maior, n * ((cel.font.size or 11) / 11))
        ws.row_dimensions[r].height = round(maior * PT_POR_LINHA + 3, 1)


# ============================================================
# PRINCIPAL
# ============================================================

area_de_trabalho = caminhos.area_de_trabalho
abrir_arquivo = caminhos.abrir_arquivo


def escolher_arquivo():
    """Só para a linha de comando: no programa, a planilha vem da interface."""
    try:
        import tkinter as tk
        from tkinter import filedialog
        raiz = tk.Tk()
        raiz.withdraw()
        caminho = filedialog.askopenfilename(
            title="Planilha de processos", filetypes=[("Excel", "*.xlsx *.xlsm")])
        raiz.destroy()
        if caminho:
            return caminho
    except Exception:
        pass
    return input("Caminho da planilha .xlsx: ").strip().strip('"')


def executar(caminho, aba, cred, pasta_saida=None, abrir=None, pasta_html=None):
    """Verifica a planilha `caminho` (aba `aba`, None = a ativa) com as
    credenciais da sessão (sessao.obter_sessao). `pasta_html`: salva cada
    página de processo baixada (diagnóstico). Devolve o caminho da planilha
    gerada, ou None se nenhuma foi gerada."""
    global PASTA_HTML
    cfg = config.carregar()
    pasta_saida = pasta_saida or config.pasta_saida(cfg)
    abrir = cfg["abrir_planilha"] if abrir is None else abrir
    PASTA_HTML = None
    if pasta_html:
        os.makedirs(pasta_html, exist_ok=True)
        PASTA_HTML = pasta_html
    if not caminho or not os.path.isfile(caminho):
        raise SystemExit(f"✗ Arquivo não encontrado: {caminho!r}")
    args = argparse.Namespace(planilha=caminho, aba=aba or None)

    wb, ws, lin_cab, col, itens, ignoradas = ler_planilha(caminho, args.aba)
    print("=" * 64)
    print(f"Planilha: {os.path.basename(caminho)}  |  aba: {ws.title}")
    print(f"Coluna do número: {get_column_letter(col)}  |  processos: {len(itens)}")
    if ignoradas:
        print(f"  ⚠ {len(ignoradas)} célula(s) sem número reconhecível: "
              + ", ".join(f"linha {r}" for r, _ in ignoradas[:8]))
    print("=" * 64)
    if not itens:
        raise SystemExit("✗ Nada a verificar.")

    # mesma linha repetida na planilha: consulta uma vez só
    unicos = {}
    for it in itens:
        unicos.setdefault(it["numero"], it)

    t0 = time.time()
    cache = carregar_cache()

    driver = sessao_1doc.NavegadorSobDemanda(cred)   # só abre se o Localizar por HTTP falhar
    ctx = None
    por_numero = {}
    try:
        sessao = sessao_http(cred.cookies, cred.ua)
        baixar(sessao, URL_PAINEL)        # fixa o contexto do setor e testa os cookies
        print("✓ Sessão HTTP válida")
        ctx = Ctx(sessao, cache, driver, ano_min=min(int(it["ano"]) for it in itens))
        print()

        lista = list(unicos.values())
        with ThreadPoolExecutor(max_workers=WORKERS) as ex:
            futuros = [ex.submit(processar, ctx, it) for it in lista]
            for i, f in enumerate(futuros, 1):
                try:
                    r = f.result()
                except SessaoExpirada:
                    # sem isto o 'with' esperaria cada processo restante
                    # bater na tela de login antes de sair
                    ex.shutdown(wait=True, cancel_futures=True)
                    raise
                por_numero[r["numero"]] = r
                icone = {SIM: "✓", NAO: "·", INDET: "?"}[r["veredito"]] if r["url"] else "✗"
                if r.get("novo"):
                    icone += "↳"
                print(f"  [{i:>3}/{len(lista)}] {icone:<2} {r['numero']:<12} "
                      f"{r['veredito']:<13} {(r['etapa'] or '-')[:30]:<30} {r['obs'][:70]}")
    except SessaoExpirada as e:
        sessao_1doc.esquecer()
        print(f"\n✗ Sessão recusada: {e}. Execute de novo: o navegador vai abrir para o login.")
    finally:
        salvar_cache(cache)
        if ctx is not None:
            ctx.driver = None
        driver.fechar()

    if not por_numero:
        raise SystemExit("✗ Nenhum processo verificado; planilha não gerada.")

    resultados = [dict(por_numero[it["numero"]], linha=it["linha"])
                  for it in itens if it["numero"] in por_numero]
    nome = os.path.splitext(os.path.basename(caminho))[0]
    destino = os.path.join(pasta_saida,
                           f"{nome}_verificada_{datetime.now():%Y-%m-%d_%H%M}.xlsx")
    linhas = exportar(wb, ws, lin_cab, col, resultados, destino)   # 0 = nada salvo

    def numeros(cond):
        return [r["numero"] for r in por_numero.values() if cond(r)]

    na_planilha = numeros(entra)
    antes    = numeros(lambda r: r["url"] and not entra(r) and r["veredito"] == NAO)
    indet    = numeros(lambda r: r["url"] and not entra(r) and r["veredito"] == INDET)
    elab     = [r for r in por_numero.values() if r.get("elaboracao")]
    elab_com = [r for r in elab if r.get("novo")]
    perdidos = numeros(lambda r: not r["url"])
    sem_resp = [n for n in unicos if n not in por_numero]   # sessão caiu no meio
    fontes = {}
    for r in por_numero.values():
        if r["fonte"]:
            fontes[r["fonte"]] = fontes.get(r["fonte"], 0) + 1

    print("\n" + "=" * 64)
    if elab:
        print(f"Em elaboração: {len(elab)}  |  com novo processo emitido: {len(elab_com)}")
        for r in elab_com:
            n = r["novo"]
            print(f"  {r['numero']:<12} → {n['numero']:<12} {n['veredito']:<13} "
                  f"{(n['etapa'] or '-')[:30]}  [{n['vinculo']}]")
    if linhas:
        rotulo = (f"Chegaram à {ETAPA_CORTE} ou além"
                  + (" (ou com processo novo emitido)" if INCLUIR_NOVO_ANTES_CORTE else ""))
        print(f"{rotulo}: {len(na_planilha)}  → na planilha ({linhas} linha(s))")
        print("Fora da planilha:")
    else:
        print(f"Nenhum processo chegou à {ETAPA_CORTE} ou além.")
        print("Situação dos demais:")
    print(f"  ainda antes:      {len(antes)}")
    if indet:
        print(f"  indeterminados:   {len(indet)}  — {', '.join(indet)}")
    if perdidos:
        print(f"  não localizados:  {len(perdidos)}  — {', '.join(perdidos)}")
    if sem_resp:
        print(f"  não verificados:  {len(sem_resp)}  — {', '.join(sem_resp)}")
    if fontes:
        print("Localizados por: " + ", ".join(f"{k} {v}" for k, v in fontes.items()))
    print(f"Tempo: {time.time() - t0:.0f}s")
    if linhas:
        print(f"✓ Planilha: {destino}")
        if abrir:
            abrir_arquivo(destino)
    else:
        print("○ Nenhuma planilha exportada.")
    print("=" * 64)
    return destino if linhas else None


def main(argv=None):
    """Linha de comando: python -m ferramentas1doc.verificar_planilha [planilha.xlsx] [--aba ABA] [--html PASTA]"""
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("planilha", nargs="?")
    ap.add_argument("--aba", help="nome da aba (padrão: a ativa)")
    ap.add_argument("--html", metavar="PASTA",
                    help="salva cada página de processo baixada nesta pasta (diagnóstico)")
    args = ap.parse_args(argv)
    caminho = args.planilha or escolher_arquivo()
    cred = sessao_1doc.obter_sessao()
    return executar(caminho, args.aba, cred, pasta_html=args.html)


if __name__ == "__main__":
    main()
