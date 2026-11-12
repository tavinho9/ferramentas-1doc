"""
SCRIPT: Processos que estão no 1Doc ANTES da "Pesquisa de Mercado" e
        FALTAM na planilha de acompanhamento
====================================================================

ENTRADA
-------
A planilha de acompanhamento (.xlsx) com uma coluna de número de processo
NN.NNN/AAAA (ex.: Nº Processo | Prioridade | Objeto | Etapa | Secretaria).
A coluna é achada pelo cabeçalho ou, sem cabeçalho reconhecível, pela
coluna em que mais células têm cara de número de processo.

O QUE CONTA COMO "FALTANDO"
---------------------------
Proc. Administrativo das caixas do setor (CAIXAS_BUSCA), emitido no
período escolhido (padrão: de 1º/jan do ano mais antigo da planilha até
hoje; ver PERÍODO DE EMISSÃO), que:
  1. não está na planilha (nem ele, nem o processo de origem, nem o
     processo novo emitido a partir dele);
  2. é de abertura, fase preparatória ou elaboração de documentação
     preparatória (etiqueta ou id do assunto) ou tem a
     Pesquisa de Mercado no próprio fluxo;
  3. está ANTES da Pesquisa de Mercado:
       - fluxo com a etapa: etapa atual antes dela e nenhuma coluna a
         partir dela com marca de tempo (não "retornou");
       - fluxo sem a etapa (elaboração/fase preparatória): o processo novo
         emitido a partir dele, se houver, também não chegou a ela;
  4. o(s) último(s) despacho(s) (N_ULTIMOS_DESPACHOS) NÃO mencionam
     cancelamento (cancelar, cancelado, cancelamento...). Sem despacho,
     vale o texto de abertura do processo.
Elaboração parada na "Etapa Final" sem processo novo encontrado sai como
concluída (EXCLUIR_ETAPA_FINAL_SEM_NOVO).
As caixas incluem Arquivados: o setor arquiva elaborações e fases
preparatórias após a análise. Quem já passou da Pesquisa fica no cache e não
é reaberto nas execuções seguintes.
Se a elaboração e o processo novo dela faltarem os dois, fica só o novo,
com a origem anotada na Observação.

SAÍDA
-----
<planilha>_faltantes_AAAA-MM-DD_HHMM.xlsx na pasta de saída (padrão: Área de
Trabalho; a original não é alterada), aberta ao final, com as colunas
  Nº Processo | Prioridade | Objeto | Etapa | Secretaria |
  Caixa | Último despacho | Trecho do último despacho | Menções | Observação
(as cinco primeiras no formato da planilha, para colar direto nela).
Menções: todos os processos citados no texto de abertura e nos despachos
(menção do 1Doc, link ou número com "processo", "nº"… antes), mesmo
quando não dá para dizer se são o pai ou o filho; "(pai)" é o processo de
origem, "(filho)" o processo novo achado e "(número no texto)" o citado só
pelo número digitado.
Se houver processos excluídos só por citarem cancelamento, vão para a aba
"Cancelados (conferir)", com o trecho da menção. Se não houver nem
faltantes nem cancelados, nenhum arquivo é gerado. O resumo do terminal mostra
quantos ficaram de fora por cada motivo.

LOGIN
-----
Feito por você, na janela do navegador; o programa espera e reaproveita a
sessão (ferramentas1doc.sessao). Usuário e senha nunca passam por aqui.

USO
---
    No programa: aba "Faltantes na planilha".
    Linha de comando:
    python -m ferramentas1doc.faltantes                 # abre seletor de arquivo
    python -m ferramentas1doc.faltantes planilha.xlsx --aba "Planilha1"
    python -m ferramentas1doc.faltantes planilha.xlsx --de 01/01/2025 --ate 30/06/2025
    python -m ferramentas1doc.faltantes planilha.xlsx --de -90      # últimos 90 dias
"""

import argparse
import html as _html
import json
import os
import re
import threading
import time
import unicodedata
from concurrent.futures import ThreadPoolExecutor
from copy import copy
from datetime import date, datetime, timedelta

import requests
from bs4 import BeautifulSoup
from openpyxl import load_workbook
from openpyxl.styles import Font, PatternFill
from openpyxl.utils import column_index_from_string, get_column_letter
from openpyxl.worksheet.table import Table, TableStyleInfo
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry

from . import busca, caminhos, config, navegador, processos_do_dia, sessao as sessao_1doc


# ============================================================
# CONFIGURAÇÃO
# ============================================================

URL_1DOC    = sessao_1doc.URL_1DOC
TIMEOUT     = 20
WORKERS     = 4           # detalhes baixados em paralelo
PAUSA_REQ   = 0.3         # segundos após cada requisição (por thread)

ETAPA_CORTE = "Pesquisa de Mercado"

# Caixas do setor varridas atrás de processos que faltam na planilha.
# Arquivados entra: o setor arquiva elaborações e fases preparatórias depois
# da análise, e aberturas paradas antes da Pesquisa também vão para lá
# (em 05/11/2026, 20 dos 84 processos da planilha estavam arquivados, e
# nenhuma elaboração estava em Em aberto/Saída). O custo da 1ª execução é
# abrir os ~900 arquivados de assunto alvo; a partir da 2ª, os que já
# passaram da Pesquisa saem do cache (chave "passou:").
CAIXAS_BUSCA = (("entrada", "Em aberto"), ("saida", "Caixa de saída"),
                ("arquivo", "Arquivados"), ("favoritos", "Favoritos"))
ANO_INICIAL = None            # início padrão do período: None = ano mais antigo da planilha
MAX_PAGINAS_CAIXA = 300       # por caixa

# Quantos despachos do fim examinar à procura de cancelamento
N_ULTIMOS_DESPACHOS = 1
# Termos de cancelamento (procurados no texto sem acento e em minúsculas)
_RE_CANCEL = re.compile(r"\bcancel\w*")
# Negação logo antes do termo ("não há cancelamento", "sem cancelamento"):
# a menção não conta. Os casos que escapam (ex.: "cancelar a requisição
# anterior e seguir com esta") vão para a aba "Cancelados (conferir)".
_RE_CANCEL_NEG = re.compile(r"\b(nao ha|nao houve|nao havera|nao se trata de|nao e caso de|"
                            r"sem|nenhum|nenhuma|inexiste|inexistencia de)( \w+)? ?$")
JANELA_NEG = 30           # caracteres antes do termo examinados
# Assuntos (etiquetas) que entram na busca, além de qualquer processo cujo
# fluxo tenha a etapa de corte
# (o 1Doc renomeou "Elaboração de Documentos Preparatórios" para "Elaboração
# de Documentação Preparatória"; o padrão aceita as duas formas)
_RE_ETIQ_ALVO = re.compile(r"abertura de procedimento|fase preparatoria|"
                           r"elaboracao de document\w* preparatori")
# Os mesmos assuntos pelo id (h4.tit_assunto[data-id_assunto]), que não muda
# quando o nome do assunto é editado. Lidos nas fontes de 05/11/2026.
IDS_ASSUNTO_ALVO = {"961721",   # Abertura de Procedimento Licitatório
                    "962809",   # Fase Preparatória de Procedimento Licitatório
                    "962265"}   # Elaboração de Documentação Preparatória
# Elaboração na "Etapa Final" sem processo novo encontrado = concluída
EXCLUIR_ETAPA_FINAL_SEM_NOVO = False

# Processos na fase de elaboração (fluxo sem a etapa de corte, ou sem quadro):
# procura o novo processo emitido a partir deles (normalmente a Abertura de
# Procedimento Licitatório) e usa a etapa DELE para decidir.
VERIFICAR_NOVO_PROCESSO = True
# Índice reverso origem → processo novo, lido das caixas do setor (ver a
# seção NOVO PROCESSO). False = só os despachos da elaboração.
USAR_INDICE_ORIGEM = True
CAIXAS_INDICE = (("entrada", "Em aberto"), ("saida", "Caixa de saída"),
                 ("arquivo", "Arquivados"), ("favoritos", "Favoritos"))
MAX_PAGINAS_INDICE = MAX_PAGINAS_CAIXA
# Etiquetas que entram no índice (comparação sem acento e sem caixa)
_RE_ETIQ_INDICE = re.compile(r"abertura de procedimento|fase preparatoria")

# Tipo de documento no "Localizar" do painel: 3 = Proc. Administrativo
ID_DOCUMENTO = "3"

# Ao achar a coluna pelo cabeçalho, aceita qualquer um destes (sem acento/caixa)
CABECALHOS_NUMERO = ("n processo", "no processo", "numero processo",
                     "numero do processo", "processo", "numero", "n")

# Na pasta de dados do programa (compartilhado com o verificar_planilha)
ARQ_CACHE = caminhos.arquivo_dados("hash_cache_1doc.json")

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
# PERÍODO DE EMISSÃO
# ============================================================
#
# Só entram os processos emitidos no período, de `de` a `ate` (as duas
# pontas incluídas). Escolhido na aba ou por --de/--ate; sem data inicial,
# vale 1º/jan do ano mais antigo da planilha (ou de ANO_INICIAL); sem data
# final, hoje. O índice de origem (NOVO PROCESSO) vai do início do período
# até hoje, qualquer que seja o fim: o processo novo é emitido depois da
# elaboração, às vezes depois do fim do período.

FORMATOS_DATA = "DD/MM/AAAA, DD/MM/AA, DD/MM, hoje, ontem ou -N (N dias atrás)"


def ler_data(txt, hoje=None):
    """Data digitada (FORMATOS_DATA) → date; vazio → None. ValueError se não
    for uma data."""
    hoje = hoje or date.today()
    t = re.sub(r"\s+", " ", (txt or "").strip().lower())
    if not t:
        return None
    if t == "hoje":
        return hoje
    if t == "ontem":
        return hoje - timedelta(days=1)
    m = re.fullmatch(r"- ?(\d+)", t)
    if m:
        try:
            return hoje - timedelta(days=int(m[1]))
        except OverflowError:
            raise ValueError(f"data fora do calendário: {txt.strip()!r}") from None
    d = processos_do_dia.interpretar_data(t, hoje)
    if d is None:
        raise ValueError(f"data não reconhecida: {txt.strip()!r}")
    return d


def ler_periodo(de_txt, ate_txt, hoje=None):
    """Datas digitadas → (de, ate), cada uma date ou None (= padrão).
    ValueError, com a mensagem para o usuário, se o período não servir."""
    hoje = hoje or date.today()
    datas = []
    for nome, txt in (("inicial", de_txt), ("final", ate_txt)):
        try:
            datas.append(ler_data(txt, hoje))
        except ValueError:
            raise ValueError(f"Data {nome} não reconhecida: \"{(txt or '').strip()}\".") from None
    de, ate = datas
    if de and de > hoje:
        raise ValueError(f"A data inicial ({de:%d/%m/%Y}) ainda não chegou.")
    if de and ate and de > ate:
        raise ValueError(f"A data inicial ({de:%d/%m/%Y}) é depois da final ({ate:%d/%m/%Y}).")
    return de, ate


def resolver_periodo(de, ate, itens, hoje=None):
    """Completa o período com o padrão: sem `de`, 1º/jan do ano mais antigo
    da planilha (ou de ANO_INICIAL); sem `ate`, ou com ele no futuro, hoje."""
    hoje = hoje or date.today()
    if de is None:
        de = date(ANO_INICIAL or min((int(it["ano"]) for it in itens), default=hoje.year), 1, 1)
    ate = min(ate or hoje, hoje)
    if de > ate:
        raise SystemExit(f"✗ Período vazio: a data final ({ate:%d/%m/%Y}) é anterior ao início "
                         f"({de:%d/%m/%Y}). Informe também a data inicial.")
    return de, ate


def texto_periodo(de, ate, sep=" a "):
    return f"{de:%d/%m/%Y}{sep}{ate:%d/%m/%Y}"


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
    adaptador = HTTPAdapter(max_retries=retry, pool_connections=WORKERS,
                            pool_maxsize=WORKERS * 2)  # índice abre um pool dentro dos workers
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


def localizar_navegador(driver, item):
    """Preenche o 'Localizar' do painel no navegador e lê o hash da URL."""
    try:
        url = navegador.localizar_no_painel(
            driver, URL_PAINEL, ID_DOCUMENTO, item["digitos"], item["ano"], TIMEOUT,
            achou=lambda u, antes: hash_de(u) or (u != antes and "pg=doc/ver" in u))
    except navegador.ErroNavegador:
        return []
    return [h for h in [hash_de(url or "")] if h]


_RE_NOME_EM = re.compile(r"""<input\b[^>]*\bid\s*=\s*["']nome_em["'][^>]*>""", re.I)
_RE_VALUE = re.compile(r"""\bvalue\s*=\s*["']([^"']*)["']""", re.I)


def ler_nome_em(html):
    """'Proc. Administrativo 31.243/2026' ou None."""
    m = _RE_NOME_EM.search(html)
    if not m:
        return None
    v = _RE_VALUE.search(m.group())
    return v.group(1) if v else None


def confere(html, item):
    """True = o detalhe é do processo procurado; None = página sem #nome_em."""
    nome = ler_nome_em(html)
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
    'marca' é o registro de passagem: na etapa atual vem em p.kanban_item
    ("Esta aqui 1 hora"); nas etapas já percorridas, em .kanban-time
    ("Ficou aqui 6 dias"). Etapa nunca visitada fica com marca vazia."""
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
        item = est.select_one(".kanban_itens .kanban_item")
        tempo = est.select_one(".kanban_itens .kanban-time")
        marca = (_limpo(item.get_text()) if item else "") or \
                (_limpo(tempo.get_text()) if tempo else "")
        etapas.append({
            "nome": _limpo(titulo.get_text()) if titulo else "",
            "atual": str(est.get("data-current", "")).lower() == "true",
            "marca": marca,
            "final": "estagio_final" in (est.get("class") or []),
        })
    return etapas


_RE_ID_ASSUNTO = re.compile(
    r"""class\s*=\s*["'][^"']*\btit_assunto\b[^"']*["'][^>]*\bdata-id_assunto\s*=\s*["'](\d+)["']""", re.I)


def ler_id_assunto(html):
    m = _RE_ID_ASSUNTO.search(html)
    return m.group(1) if m else ""


def assunto_alvo(etiqueta, id_assunto=""):
    return id_assunto in IDS_ASSUNTO_ALVO or bool(_RE_ETIQ_ALVO.search(_norm(etiqueta)))


def ler_etiqueta(html):
    """Assunto do badge em h4.tit_assunto, sem o prefixo 'Assunto:'."""
    i = html.find("tit_assunto\"")
    i = i if i >= 0 else html.find("tit_assunto ")
    while i >= 0:
        inicio = html.rfind("<", 0, i)
        soup = BeautifulSoup(html[inicio:inicio + 4000], PARSER)
        badge = soup.select_one(".tit_assunto .badge, .tit_assunto span[title]")
        if badge is not None:
            t = badge.get("title") or badge.get("data-original-title") or badge.get_text()
            return _limpo(re.sub(r"^\s*Assunto\s*:\s*", "", t))
        i = html.find("tit_assunto", i + 1)
    return ""


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
        obs = "Na etapa de corte" if atual_idx == idx_corte else f"Além de {ETAPA_CORTE}"
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
# O 1Doc registra o vínculo só no sentido filho → pai: o processo novo
# traz no cabeçalho o processo de onde foi emitido
#     <h2 class="tt pull-left emission_infos_item_from" title="...">
#       <a class="nd" href="./?pg=doc/ver&itd=3&hash=5077DF...&origem=base">
#         Proc. Administrativo <small>28.133/2026</small>
# A página da elaboração não lista os processos gerados a partir dela.
# Por isso os candidatos vêm de onde dá para tirá-los e cada um é aberto
# e conferido por esse cabeçalho:
#   1. cache de execuções anteriores
#   2. menções do 1Doc (data-mention="2", em qualquer lugar da página) e
#      links para outros processos dentro dos despachos (.texto_original)
#   3. números NN.NNN/AAAA citados nos despachos, com contexto de processo
#      ("processo", "abertura", "emitido", "nº"...) e sem contexto de lei,
#      requisição, contrato etc.
#   4. índice reverso origem → processo novo, montado uma vez por execução:
#      percorre as caixas do setor (Em aberto, Saída, Arquivados, Favoritos),
#      pega só os Proc. Administrativos com etiqueta de Abertura ou de Fase
#      Preparatória emitidos do início do período até hoje, e lê a origem de
#      cada um. A origem não muda, então fica no cache e cada processo só é
#      aberto na primeira execução em que aparece.
#      (O filtro pai_raiz da listagem foi testado e o servidor não o aplica.)
# Aceite:
#   • origem do candidato = a elaboração           → vínculo confirmado
#   • candidato sem origem, citado em despacho, com
#     fluxo que tem a etapa de corte (abertura)     → aceito como "citado"
#   • origem do candidato = outro processo          → descartado
# Se o processo novo também for de elaboração (ex.: Elaboração → Fase
# Preparatória → Abertura), a busca desce mais um nível.

_RE_ORIGEM = re.compile(r"""class\s*=\s*["'][^"']*\bemission_infos_item_from\b""", re.I)
_RE_ID_EMISSAO = re.compile(
    r"""<input\b[^>]*\bname\s*=\s*["']id_emissao_pai["'][^>]*\bvalue\s*=\s*["'](\d+)["']""", re.I)
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


def ler_id_emissao(html):
    m = _RE_ID_EMISSAO.search(html)
    return m.group(1) if m else None


# Menção do 1Doc (o documento escolhido na lista ao escrever o despacho):
#     <a class="mention label mention_2" data-mention="2" href="…&hash=5476D7…&origem=mention…">
#       Proc. Administrativo 26.831/2026 - …</a>
# no corpo do despacho e, repetida, na lista de documentos mencionados, que
# fica fora do .texto_original:
#     <a class=" pre_visualizar" data-mention="2" href="./?pg=doc/ver&hash=5476D7…&origem=emissao_anexada">
_RE_TAG_A = re.compile(r"<a\b[^>]*>", re.I)
_RE_EH_MENCAO = re.compile(r"""\bdata-mention\s*=\s*["']?2\b|\bmention_2\b""", re.I)


def citacoes(html, proprio):
    """Candidatos citados nos despachos: ([hashes de links], [números]).
    As menções do 1Doc (data-mention="2") vêm primeiro e valem em qualquer
    lugar da página; depois, os links e os números dos blocos .texto_original.
    A menção a uma pessoa (data-mention="1") não é link para processo: o href
    dela é a página que quem escreveu tinha aberta. O texto é lido como o
    navegador mostra, para o número partido pela formatação sair inteiro."""
    from bs4 import SoupStrainer
    # Função, não texto: no bs4 4.13+ o SoupStrainer(class_="texto_original")
    # compara a string de classe inteira e perde <div class="texto_original fr-view">.
    blocos = BeautifulSoup(html, PARSER, parse_only=SoupStrainer(
        class_=lambda c: bool(c) and "texto_original" in str(c).split()))
    hashes, numeros = [], []
    ano_min = int(proprio["ano"])
    for tag in _RE_TAG_A.findall(html):
        h = hash_de(tag) if _RE_EH_MENCAO.search(tag) else None
        if h and h not in hashes:
            hashes.append(h)
    for a in blocos.select("a[href]"):
        if busca.mencao_de_outro_tipo(a):
            continue
        h = hash_de(a["href"])
        if h and h not in hashes:
            hashes.append(h)
    texto = busca.texto_corrido(blocos)
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


_RE_DATA_LINHA = re.compile(r"(\d{2})/(\d{2})/(\d{4})")


def _data_emissao_linha(a):
    """Data de emissão da linha (<small class="data"> dentro do link), ou None."""
    el = a.select_one("small.data")
    m = _RE_DATA_LINHA.search(el.get_text(" ") if el is not None else "")
    try:
        return date(int(m[3]), int(m[2]), int(m[1])) if m else None
    except ValueError:
        return None


def linhas_listagem(html):
    """[{hash, numero, etiqueta, prioridade, emissao}] das linhas da listagem.
    A etiqueta é o title do badge de assunto da linha ("Abertura de
    Procedimento Licitatório", "Fase Preparatória de Procedimento
    Licitatório", "Elaboração de Documentos Preparatórios"); emissao é a
    data (date) lida na linha, ou None."""
    soup = BeautifulSoup(html, PARSER)
    linhas = []
    for a in soup.select("a.link_emissao_a"):
        h = hash_de(a.get("href", ""))
        tr = a.find_parent("tr")
        num = ler_numero(a.get_text(" "))
        if not h or tr is None or not num:
            continue
        etiqueta = prioridade = ""
        for el in tr.select("[title], [data-original-title]"):
            t_ = _limpo(el.get("title") or el.get("data-original-title") or "")
            if t_ == "Prioridade":
                prioridade = prioridade or _limpo(el.get_text())
            elif not etiqueta and (_RE_ETIQ_ALVO.search(_norm(t_))
                                   or _RE_ETIQ_INDICE.search(_norm(t_))):
                etiqueta = t_
        linhas.append({"hash": h, "numero": num[0], "etiqueta": etiqueta,
                       "prioridade": prioridade, "emissao": _data_emissao_linha(a)})
    return linhas


def listar_caixa(ctx, caixa, de, ate):
    """Todas as páginas de uma caixa (só Proc. Administrativos) emitidos de
    `de` a `ate` (date, as duas pontas incluídas). Quem filtra é o servidor
    (parâmetro intervalo); a data de emissão lida na linha confere de novo,
    para o período valer mesmo se o servidor um dia ignorar o filtro.
    Guardada em ctx: cada caixa é listada uma vez por período."""
    chave = (caixa, de, ate)
    with ctx.lock_listagem:
        if chave in ctx.listagens:
            return ctx.listagens[chave]
        linhas, pagina, total = [], 1, None
        while pagina <= min(total or MAX_PAGINAS_CAIXA, MAX_PAGINAS_CAIXA):
            params = {"chamada": "caixa", "pg": "painel/listar", "caixa": caixa,
                      "num_pagina": str(pagina), "ordenar": "1", "ordem": "desc",
                      "modo_visualizacao": "1", "intervalo": texto_periodo(de, ate, " - "),
                      "emissoes_arquivado_prazos": "0", "itd": ID_DOCUMENTO}
            html = _html_listagem(baixar(ctx.sessao, f"{URL_1DOC}/link.php", params=params,
                                         xhr=True, referer=URL_PAINEL))
            achadas = linhas_listagem(html)
            if not achadas:
                break
            linhas += [ln for ln in achadas if ln["emissao"] is None or de <= ln["emissao"] <= ate]
            m = _RE_TOTAL_PAG.search(html)
            if m:
                total = int(m.group(2))
            elif total is None and len(achadas) < 10:     # sem contador: página curta = última
                break
            pagina += 1
        ctx.listagens[chave] = linhas
        return linhas


def montar_indice(ctx):
    """origem (hash e número) → [hashes dos processos emitidos a partir dela].
    Montado uma vez; as outras threads esperam e reaproveitam."""
    with ctx.lock_indice:
        if ctx.indice is not None:
            return ctx.indice
        de, ate = ctx.de, ctx.hoje         # até hoje: o processo novo vem depois da elaboração
        print(f"\n  … montando o índice de origem (caixas do setor, emissões de {texto_periodo(de, ate)})")
        candidatos, falhas = {}, []
        for caixa, rotulo in CAIXAS_INDICE:
            try:
                for ln in listar_caixa(ctx, caixa, de, ate):
                    if _RE_ETIQ_INDICE.search(_norm(ln["etiqueta"])):
                        candidatos[ln["hash"]] = ln["etiqueta"]
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
    hashes, numeros = citacoes(html, elab)

    def avaliar_candidato(h, pagina, citado):
        """`citado=None`: candidato guardado no cache. Vale como citado só se
        ainda for citado na página, para não repetir um vínculo achado por uma
        leitura que já foi corrigida (ex.: menção a uma pessoa)."""
        if h in vistos:
            return
        vistos.add(h)
        nome = ler_numero(ler_nome_em(pagina) or "")
        if not nome or nome[0] == elab["numero"]:
            return
        if citado is None:
            citado = h in hashes or nome[0] in numeros
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
        v, etapa, obs = avaliar(etapas, ler_etiqueta(pagina))
        novo = {"numero": nome[0], "ano": nome[2], "hash": h, "url": url_detalhe(h), "veredito": v,
                "etapa": etapa, "obs": obs, "vinculo": vinculo, "via": None}
        if sem_etapa_corte(etapas) and nivel < 2:       # elaboração → fase prep. → abertura
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
        por_numero(numero, citado=None)
    for h in hashes:
        por_hash(h, citado=True)
    for numero in numeros:
        if not any(a["numero"] == numero for a in achados):
            por_numero(numero, citado=True)
    if USAR_INDICE_ORIGEM:
        indice = montar_indice(ctx)
        for h in indice.get(elab["hash"], []) + indice.get(elab["numero"], []):
            por_hash(h, citado=False)

    if achados:
        with ctx.lock_cache:
            ctx.cache[chave_cache] = sorted({a["numero"] for a in achados})

    def efetivo(a):                     # o neto conta pelo filho, se for mais adiantado
        return a["via"] if a["via"] and _ORDEM[a["via"]["veredito"]] < _ORDEM[a["veredito"]] else a
    achados.sort(key=lambda a: (_ORDEM[efetivo(a)["veredito"]], a["vinculo"] != "origem no 1Doc"))
    return achados


class Ctx:
    """O que todas as threads compartilham."""
    def __init__(self, sessao, cache, driver, de=None, ate=None):
        self.sessao, self.cache, self.driver = sessao, cache, driver
        self.lock_cache, self.lock_driver = threading.Lock(), threading.Lock()
        self.lock_indice, self.lock_listagem = threading.Lock(), threading.Lock()
        self.hoje = date.today()       # fixo na execução, mesmo se ela virar a meia-noite
        self.de = de or date(self.hoje.year, 1, 1)
        self.ate = ate or self.hoje
        self.indice, self.indice_info = None, {}
        self.listagens = {}
        self.planilha = set()


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
                        hashes = localizar_navegador(drv, item) if drv is not None else []
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


# ============================================================
# CAMPOS DO DETALHE (prioridade, objeto, secretaria, despachos)
# ============================================================

_RE_PRIORIDADE = re.compile(
    r"""<span\b[^>]*\btitle\s*=\s*["']Prioridade["'][^>]*>\s*([^<]*?)\s*<""", re.I)
_RE_TIT_ASSUNTO = re.compile(r"""<h4\b[^>]*class\s*=\s*["'][^"']*\btit_assunto\b""", re.I)
_RE_ID_SETOR_ORIGEM = re.compile(
    r"""<input\b[^>]*\bid\s*=\s*["']id_setor_origem["'][^>]*>""", re.I)
_RE_BADGE_SETOR = re.compile(r"""<span\b[^>]*\bdata-id_setor\s*=\s*["'](\d+)["'][^>]*>""", re.I)
_RE_TITLE = re.compile(r"""\btitle\s*=\s*["']([^"']*)["']""", re.I)
_RE_DATA_DESP = re.compile(r"(\d{2}/\d{2}/\d{4})(?:\s+(\d{2}:\d{2}))?")


def ler_prioridade(html):
    """1ª ocorrência = rótulo do cabeçalho do processo (Urgente, Média...)."""
    m = _RE_PRIORIDADE.search(html)
    return _limpo(_html.unescape(m.group(1))) if m else ""


def ler_objeto(html):
    """Texto do h4.tit_assunto sem o badge da etiqueta."""
    m = _RE_TIT_ASSUNTO.search(html)
    if not m:
        return ""
    soup = BeautifulSoup(html[m.start():m.start() + 4000], PARSER)
    h4 = soup.select_one("h4.tit_assunto")
    if h4 is None:
        return ""
    for el in h4.select(".badge, span[title]"):
        el.decompose()
    return _limpo(h4.get_text(" "))


_RE_CAMPO_OBJETO = re.compile(
    r"""<div\b[^>]*\bid\s*=\s*["']campo_adicional_descricao_objeto["']""", re.I)
_RE_PROX_CAMPO = re.compile(r"""\bid\s*=\s*["']campo_adicional_""", re.I)


def ler_objeto_abertura(html):
    """Campo 'Descrição do objeto' do formulário da Abertura de Procedimento
    Licitatório (div#campo_adicional_descricao_objeto). O valor vem completo
    no data-valor do <span> do rótulo e repetido no <small> logo abaixo.
    Processos de elaboração não têm esse campo: devolve ''."""
    m = _RE_CAMPO_OBJETO.search(html)
    if not m:
        return ""
    prox = _RE_PROX_CAMPO.search(html, m.end())
    fim = prox.start() if prox else m.start() + 20000
    soup = BeautifulSoup(html[m.start():fim], PARSER)
    campo = soup.select_one("#campo_adicional_descricao_objeto") or soup
    span = campo.select_one("[data-valor]")
    valor = _limpo(span.get("data-valor", "")) if span is not None else ""
    if not valor:
        small = campo.select_one("small")
        valor = _limpo(small.get_text(" ")) if small is not None else ""
    return valor


def eh_abertura(etapas, etiqueta):
    """Processo na fase de abertura: etiqueta de Abertura de Procedimento
    Licitatório ou fluxo que tem a etapa de corte."""
    return bool(re.search(r"abertura de procedimento", _norm(etiqueta))) or not sem_etapa_corte(etapas)


def ler_secretaria(html):
    """Secretaria do setor de origem: o badge de 'setores envolvidos' cujo
    data-id_setor é o #id_setor_origem; o title vem como
    'Secretaria de Saúde » SS-DAF » ...' e fica só o 1º nível."""
    m = _RE_ID_SETOR_ORIGEM.search(html)
    v = _RE_VALUE.search(m.group()) if m else None
    alvo = v.group(1) if v else None
    primeiro = None
    for b in _RE_BADGE_SETOR.finditer(html):
        t = _RE_TITLE.search(b.group())
        if not t:
            continue
        nome = _limpo(_html.unescape(t.group(1)).split("»")[0])
        if alvo is None or b.group(1) == alvo:
            return nome
        primeiro = primeiro or nome
    return primeiro or ""


def ler_despachos(html):
    """Despachos em ordem cronológica: [{'rotulo', 'data', 'texto'}].
    Cada despacho é um table.despacho, em ordem cronológica na página;
    o texto vem do td.celula_despacho por textContent, sem o menu de ações
    (que tem 'Solicitar/Assinar...' e não é conteúdo), mais valores de
    textarea e title/data-original-title. Sem despacho nenhum, devolve o
    texto de abertura do processo como único item."""
    from bs4 import SoupStrainer
    # Filtra a classe em Python: no bs4 4.13+ o SoupStrainer(class_=...) não
    # casa atributo class com vários valores ("relative despacho ...").
    soup = BeautifulSoup(html, PARSER, parse_only=SoupStrainer("table"))
    itens = []
    for t in soup.find_all("table"):
        if "despacho" not in (t.get("class") or []):
            continue
        cel = t.select_one("td.celula_despacho")
        if cel is None:
            continue
        for lixo in cel.select(".dropdown-menu, .btn-group, script, style"):
            lixo.decompose()
        partes = [cel.get_text(" ")]
        partes += [el.get("title") or el.get("data-original-title") or ""
                   for el in cel.select("[title], [data-original-title]")]
        cab = t.select_one(".rf_header strong")
        data = t.select_one("abbr.despacho_data[title]")
        md = _RE_DATA_DESP.search(data["title"] if data else "")
        itens.append({"rotulo": _limpo(cab.get_text(" ")) if cab else "",
                      "data": " ".join(g for g in md.groups() if g) if md else "",
                      "texto": _limpo(" ".join(partes))})
    if itens:
        return itens
    m = re.search(r"""class\s*=\s*["'][^"']*\bemissao_conteudo\b""", html)
    if m:
        frag = BeautifulSoup(html[m.start() - 200:m.start() + 30000], PARSER)
        orig = frag.select_one(".texto_original")
        if orig is not None:
            return [{"rotulo": "Abertura (sem despachos)", "data": "",
                     "texto": _limpo(orig.get_text(" "))}]
    return []


def posicao_cancelamento(texto):
    """Posição (no texto original) da 1ª menção a cancelamento sem negação
    imediatamente antes, ou None. A busca é feita numa cópia sem acento e em
    minúsculas de mesmo comprimento, para a posição valer no original."""
    t = unicodedata.normalize("NFC", texto or "")
    base = "".join(_sem_acento(ch) or ch for ch in t).lower()      # 1 char → 1 char
    for m in _RE_CANCEL.finditer(base):
        antes = _norm(base[max(0, m.start() - JANELA_NEG):m.start()])
        if not _RE_CANCEL_NEG.search(antes):
            return m.start()
    return None


def menciona_cancelamento(texto):
    return posicao_cancelamento(texto) is not None


def cancelamento_em(despachos):
    """Despacho (entre os N últimos) que menciona cancelamento, ou None."""
    for d in despachos[-N_ULTIMOS_DESPACHOS:]:
        if menciona_cancelamento(d["texto"]):
            return d
    return None


def trecho_em_volta(texto, pos, raio=150):
    """Trecho de ~2*raio caracteres em volta de `pos`, cortado em palavra."""
    t = unicodedata.normalize("NFC", texto or "")
    ini, fim = max(0, pos - raio), min(len(t), pos + raio)
    trecho = t[ini:fim]
    if ini > 0:
        trecho = "… " + trecho.split(" ", 1)[-1]
    if fim < len(t):
        trecho = trecho.rsplit(" ", 1)[0] + " …"
    return trecho


# ============================================================
# CLASSIFICAÇÃO DE CADA PROCESSO DAS CAIXAS
# ============================================================

FALTA       = "Faltando na planilha"
FORA        = "Fora do escopo (assunto)"
PASSOU      = f"Na {ETAPA_CORTE} ou além"
PASSOU_NOVO = f"Processo novo na {ETAPA_CORTE} ou além"
CANCELADO   = "Cancelamento no último despacho"
VINCULADO   = "Vinculado a processo da planilha"
CONCLUIDO   = "Etapa Final sem processo novo"
INDETERM    = "Indeterminado"
SEM_ASSUNTO = "Assunto não lido"
ERRO        = "Erro ao abrir"


def efetivo(n):
    """O neto conta pelo filho, se for mais adiantado (mesma regra de buscar_novos)."""
    return n["via"] if n["via"] and _ORDEM[n["via"]["veredito"]] < _ORDEM[n["veredito"]] else n


def classificar(ctx, ln):
    """ln: linha da listagem (hash, numero, etiqueta, prioridade, caixas)."""
    num = ler_numero(ln["numero"])
    res = dict(ln, ano=num[2], url=url_detalhe(ln["hash"]), situacao=ERRO,
               objeto="", etapa="", secretaria="", ult_data="", ult_texto="",
               obs="", origem_num="", novos=[], mencoes="", mencoes_url="")
    # Já passou da Pesquisa numa execução anterior: estado terminal (as marcas
    # de tempo das etapas percorridas não somem, então "retornou" continua
    # SIM). Evita reabrir centenas de arquivados a cada execução.
    passou = ctx.cache.get(f"passou:{ln['hash']}")
    if passou:
        sit, obs = passou if isinstance(passou, list) else (PASSOU, str(passou))
        res.update(situacao=sit if sit in (PASSOU, PASSOU_NOVO) else PASSOU, obs=obs)
        return res
    chave_etiq = f"etiq:{ln['hash']}"
    etiq_cache = ctx.cache.get(chave_etiq)
    # Só confia no "fora do escopo" guardado se houver um assunto registrado e
    # ele continuar fora pelas regras atuais (o cache antigo marcou as
    # elaborações como fora quando o 1Doc renomeou o assunto; e "FORA|" vazio
    # vem de página cujo assunto não foi lido, não de assunto fora).
    if etiq_cache is not None and etiq_cache.startswith("FORA|") \
            and etiq_cache[5:].strip() and not assunto_alvo(etiq_cache[5:]):
        res.update(situacao=FORA, etiqueta=etiq_cache[5:])
        return res
    try:
        html = baixar(ctx.sessao, res["url"])
    except requests.RequestException as e:
        res["obs"] = str(e)[:80]
        return res

    etiqueta = ler_etiqueta(html) or ln["etiqueta"]
    etapas = ler_quadro(html)
    tem_corte = not sem_etapa_corte(etapas)
    # Abertura: objeto do campo do formulário (o título costuma ser um
    # resumo); sem o campo, ou na elaboração, fica o título do processo.
    objeto = (ler_objeto_abertura(html) if eh_abertura(etapas, etiqueta) else "") or ler_objeto(html)
    res.update(etiqueta=etiqueta, objeto=objeto, secretaria=ler_secretaria(html),
               prioridade=ler_prioridade(html) or ln["prioridade"])
    id_assunto = ler_id_assunto(html)
    if not tem_corte and not assunto_alvo(etiqueta, id_assunto):
        if etiqueta or id_assunto:      # o assunto não muda: não abre de novo
            with ctx.lock_cache:
                ctx.cache[chave_etiq] = "FORA|" + etiqueta
            res["situacao"] = FORA
        else:                           # nada lido: não grava, e lista no resumo
            res.update(situacao=SEM_ASSUNTO,
                       obs="Assunto não lido na página (etiqueta e id vazios)")
        return res

    def marcar_passou(sit, obs_):
        with ctx.lock_cache:
            ctx.cache[f"passou:{ln['hash']}"] = [sit, obs_]

    veredito, etapa, obs = avaliar(etapas, etiqueta)
    res.update(etapa=etapa or ("(sem quadro de etapas)" if not etapas else ""), obs=obs)
    if veredito == SIM:
        res["situacao"] = PASSOU
        marcar_passou(PASSOU, obs)
        return res
    if veredito == INDET:
        res["situacao"] = INDETERM
        return res

    despachos = ler_despachos(html)
    if despachos:
        res.update(ult_data=despachos[-1]["data"], ult_texto=despachos[-1]["texto"])
    # As linhas que podem ir para a planilha (faltante, cancelado) levam as menções
    citados = busca.mencoes_da_pagina(html, ln["numero"], ln["hash"])
    anotar_mencoes(ctx, res, citados)
    desp_cancel = cancelamento_em(despachos)
    if desp_cancel:
        onde = desp_cancel["rotulo"] or desp_cancel["data"] or "último despacho"
        res.update(situacao=CANCELADO, obs=f"Cancelamento citado em {onde}",
                   trecho_cancel=trecho_em_volta(desp_cancel["texto"],
                                                 posicao_cancelamento(desp_cancel["texto"])))
        return res

    _o_hash, o_num = ler_origem(html)
    res["origem_num"] = o_num or ""
    anotar_mencoes(ctx, res, citados)
    if o_num and o_num in ctx.planilha:
        res.update(situacao=VINCULADO, obs=f"Emitido a partir de {o_num}, que está na planilha")
        return res

    if not tem_corte and VERIFICAR_NOVO_PROCESSO:
        novos = buscar_novos(ctx, res, html)
        todos = []
        for n in novos:
            todos += [n["numero"]] + ([n["via"]["numero"]] if n["via"] else [])
        res["novos"] = todos
        anotar_mencoes(ctx, res, citados)
        na_pl = [n for n in todos if n in ctx.planilha]
        if na_pl:
            res.update(situacao=VINCULADO,
                       obs=f"Processo novo {', '.join(na_pl)} está na planilha")
            return res
        if novos:
            alvo = efetivo(novos[0])
            cadeia = (f"{novos[0]['numero']} → {alvo['numero']}" if alvo is not novos[0]
                      else alvo["numero"])
            if alvo["veredito"] == SIM:
                res.update(situacao=PASSOU_NOVO,
                           obs=f"Novo processo {cadeia}: {alvo['etapa'] or alvo['obs']}")
                marcar_passou(PASSOU_NOVO, res["obs"])
                return res
            res["obs"] = (f"Novo processo {cadeia} ({novos[0]['vinculo']}), "
                          f"etapa {alvo['etapa'] or '-'}")
        else:
            atual = next((e for e in etapas if e["atual"]), None)
            if EXCLUIR_ETAPA_FINAL_SEM_NOVO and atual and atual["final"]:
                res.update(situacao=CONCLUIDO, obs="Etapa Final, nenhum processo novo encontrado")
                return res
            res["obs"] = f"{obs}; nenhum processo novo emitido encontrado"

    res["situacao"] = FALTA
    return res


def anotar_mencoes(ctx, res, citados):
    """Coluna Menções: os processos citados na página, com (pai) para a
    origem e (filho) para o processo novo achado, se já se sabe. Link sem
    número no texto vira número pelo cache, quando o hash já foi localizado
    antes; o link da célula leva à primeira menção."""
    por_hash = None

    def achar(c):
        nonlocal por_hash
        if por_hash is None:
            with ctx.lock_cache:
                por_hash = {v: k for k, v in ctx.cache.items() if isinstance(v, str) and ler_numero(k)}
        return por_hash.get(c["hash"])
    res["mencoes"] = busca.texto_mencoes(citados, res.get("origem_num", ""), res.get("novos") or [], achar)
    res["mencoes_url"] = next((url_detalhe(c["hash"]) for c in citados if c["hash"]), "")


def deduplicar(faltantes):
    """Elaboração e processo novo dela, ambos faltando: fica só o novo."""
    nums = {r["numero"] for r in faltantes}
    sai = set()
    for r in faltantes:
        if r["origem_num"] in nums:
            sai.add(r["origem_num"])
            r["obs"] = f"Origem {r['origem_num']} também fora da planilha; " + r["obs"]
    for r in faltantes:
        if r["numero"] not in sai and any(n in nums and n != r["numero"] for n in r["novos"]):
            sai.add(r["numero"])
    return [r for r in faltantes if r["numero"] not in sai], sorted(sai)


# ============================================================
# EXPORTAÇÃO
# ============================================================

AZUL = PatternFill("solid", start_color="1F4E78")
COLUNAS_BASE = ("Nº Processo", "Prioridade", "Objeto", "Etapa", "Secretaria", "Caixa", "Último despacho")
COLUNAS = COLUNAS_BASE + ("Trecho do último despacho", "Menções", "Observação")
TRECHO_MAX = 300


COLUNAS_CANCEL = COLUNAS_BASE + ("Trecho com a menção a cancelamento", "Menções", "Observação")


def _data_despacho(txt):
    """'05/11/2026 10:00' → datetime (ordena por data, não por texto); o texto
    original se não casar com o formato; None se vazio."""
    if not txt:
        return None
    for fmt in ("%d/%m/%Y %H:%M", "%d/%m/%Y"):
        try:
            return datetime.strptime(txt.strip(), fmt)
        except ValueError:
            pass
    return txt


def _preencher_aba(ws, linhas, colunas, trecho_de, nome_tabela):
    for c, titulo in enumerate(colunas, 1):
        cel = ws.cell(1, c, titulo)
        cel.font, cel.fill = Font(bold=True, color="FFFFFF"), AZUL
    ordem = {"urgente": 0, "alta": 1, "media": 2, "baixa": 3}
    linhas = sorted(linhas, key=lambda r: (ordem.get(_norm(r["prioridade"]), 9),
                                           int(r["ano"]), int(r["numero"].split("/")[0].replace(".", ""))))
    for i, r in enumerate(linhas, 2):
        data = _data_despacho(r["ult_data"])
        valores = (r["numero"], r["prioridade"], r["objeto"], r["etapa"], r["secretaria"],
                   ", ".join(r["caixas"]), data, trecho_de(r), r.get("mencoes") or None, r["obs"])
        for c, v in enumerate(valores, 1):
            ws.cell(i, c, v)
        if r.get("mencoes_url"):
            cel = ws.cell(i, 9)
            cel.hyperlink = r["mencoes_url"]
            cel.font = Font(color="0563C1", underline="single")
        if isinstance(data, datetime):
            ws.cell(i, 7).number_format = "dd/mm/yyyy hh:mm" if (data.hour or data.minute) else "dd/mm/yyyy"
        cel = ws.cell(i, 1)
        cel.hyperlink = r["url"]
        cel.font = Font(color="0563C1", underline="single")
    ultima = len(linhas) + 1
    ajustar_layout(ws, 1, ultima, len(colunas))
    # Filtro como Tabela (ListObject) com linha de cabeçalho declarada, e não
    # como autoFilter solto: com o autoFilter solto, o LibreOffice não guarda
    # que a 1ª linha é cabeçalho e, como todas as colunas são texto, o
    # "Classificar" e o "Filtro padrão" a tratam como dado (o cabeçalho ia
    # parar no meio das linhas). A Tabela vira, no LibreOffice, um intervalo
    # de banco de dados com "contém rótulos de coluna"; no Excel, uma tabela.
    # A tabela precisa de ao menos uma linha de dados: sem linhas, cobre a 2ª,
    # vazia.
    ref = f"A1:{get_column_letter(len(colunas))}{max(ultima, 2)}"
    tabela = Table(displayName=nome_tabela, ref=ref, headerRowCount=1)
    tabela.tableStyleInfo = TableStyleInfo(name="TableStyleLight1", showRowStripes=False,
                                           showColumnStripes=False)
    ws.add_table(tabela)
    ws.freeze_panes = "B2"
    return len(linhas)


def _trecho_ultimo(r):
    trecho = r["ult_texto"]
    if len(trecho) > TRECHO_MAX:
        trecho = trecho[:TRECHO_MAX].rsplit(" ", 1)[0] + " …"
    return trecho


def exportar_faltantes(faltantes, destino, cancelados=()):
    """Aba 'Faltantes' (formato da planilha nas 5 primeiras colunas) e, se
    houver, aba 'Cancelados (conferir)': processos excluídos só por citarem
    cancelamento no último despacho, com o trecho da menção, para conferência."""
    from openpyxl import Workbook
    wb = Workbook()
    ws = wb.active
    ws.title = "Faltantes"
    n = _preencher_aba(ws, faltantes, COLUNAS, _trecho_ultimo, "Faltantes")
    if cancelados:
        _preencher_aba(wb.create_sheet("Cancelados (conferir)"), cancelados, COLUNAS_CANCEL,
                       lambda r: r.get("trecho_cancel") or _trecho_ultimo(r), "Cancelados")
    wb.save(destino)
    return n


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


def executar(caminho, aba, cred, pasta_saida=None, abrir=None, de=None, ate=None):
    """Roda a verificação da planilha `caminho` (aba `aba`, None = a ativa)
    com as credenciais da sessão (sessao.obter_sessao), olhando os processos
    emitidos de `de` a `ate` (date; None = o padrão, ver resolver_periodo).
    Devolve o caminho da planilha gerada, ou None se nenhuma foi gerada."""
    cfg = config.carregar()
    pasta_saida = pasta_saida or config.pasta_saida(cfg)
    abrir = cfg["abrir_planilha"] if abrir is None else abrir
    if not caminho or not os.path.isfile(caminho):
        raise SystemExit(f"✗ Arquivo não encontrado: {caminho!r}")
    args = argparse.Namespace(planilha=caminho, aba=aba or None)

    _wb, ws, _lin_cab, col, itens, ignoradas = ler_planilha(caminho, args.aba)
    na_planilha = {it["numero"] for it in itens}
    inicio_padrao = de is None
    de, ate = resolver_periodo(de, ate, itens)
    print("=" * 64)
    print(f"Planilha: {os.path.basename(caminho)}  |  aba: {ws.title}")
    print(f"Coluna do número: {get_column_letter(col)}  |  processos na planilha: {len(na_planilha)}")
    if ignoradas:
        print(f"  ⚠ {len(ignoradas)} célula(s) sem número reconhecível: "
              + ", ".join(f"linha {r}" for r, _ in ignoradas[:8]))
    print(f"Período de emissão: {texto_periodo(de, ate)}"
          + ("  (início padrão: 1º/jan do ano mais antigo da planilha)"
             if inicio_padrao and not ANO_INICIAL else ""))
    print("=" * 64)

    t0 = time.time()
    cache = carregar_cache()

    driver = sessao_1doc.NavegadorSobDemanda(cred)   # só abre se o Localizar por HTTP falhar
    ctx = None
    linhas, resultados, falhas_caixa, nada_a_verificar = {}, [], [], False
    try:
        sessao = sessao_http(cred.cookies, cred.ua)
        baixar(sessao, URL_PAINEL)        # fixa o contexto do setor e testa os cookies
        print("✓ Sessão HTTP válida")
        ctx = Ctx(sessao, cache, driver, de=de, ate=ate)
        ctx.planilha = na_planilha

        print(f"\nCaixas do setor (Proc. Administrativos emitidos de {texto_periodo(de, ate)}):")
        for caixa, rotulo in CAIXAS_BUSCA:
            try:
                achadas = listar_caixa(ctx, caixa, de, ate)
            except requests.RequestException as e:
                falhas_caixa.append(rotulo)
                print(f"  ✗ {rotulo}: {str(e)[:60]}")
                continue
            for ln in achadas:
                d = linhas.setdefault(ln["hash"], dict(ln, caixas=[]))
                if rotulo not in d["caixas"]:
                    d["caixas"].append(rotulo)
            print(f"  {rotulo:<16} {len(achadas):>5} processo(s)")

        fora_da_planilha = [ln for ln in linhas.values() if ln["numero"] not in na_planilha]
        nada_a_verificar = not fora_da_planilha and len(falhas_caixa) < len(CAIXAS_BUSCA)
        print(f"  {'únicos':<16} {len(linhas):>5}  |  já na planilha: "
              f"{len(linhas) - len(fora_da_planilha)}  |  a verificar: {len(fora_da_planilha)}\n")

        with ThreadPoolExecutor(max_workers=WORKERS) as ex:
            futuros = [ex.submit(classificar, ctx, ln) for ln in fora_da_planilha]
            for i, f in enumerate(futuros, 1):
                r = f.result()
                resultados.append(r)
                if r["situacao"] in (FORA, PASSOU, PASSOU_NOVO):
                    if i % 100 == 0 or i == len(futuros):   # com Arquivados são centenas
                        print(f"  [{i:>4}/{len(futuros)}] … verificados")
                    continue                      # não polui o terminal
                icone = {FALTA: "✚", CANCELADO: "×", INDETERM: "?", SEM_ASSUNTO: "?",
                         ERRO: "✗"}.get(r["situacao"], "·")
                print(f"  [{i:>4}/{len(futuros)}] {icone} {r['numero']:<12} {r['situacao'][:34]:<34} "
                      f"{(r['etapa'] or '-')[:28]:<28} {r['obs'][:60]}")
    except SessaoExpirada as e:
        sessao_1doc.esquecer()
        print(f"\n✗ Sessão recusada: {e}. Execute de novo: o navegador vai abrir para o login.")
    finally:
        salvar_cache(cache)
        if ctx is not None:
            ctx.driver = None
        driver.fechar()

    if nada_a_verificar:               # período sem processo, ou todos já na planilha: não é erro
        print("\n" + "=" * 64)
        if linhas:
            print(f"○ Todos os processos emitidos de {texto_periodo(de, ate)} nas caixas "
                  f"({len(linhas)}) já estão na planilha; nenhuma planilha exportada.")
        else:
            print(f"○ Nenhum Proc. Administrativo emitido de {texto_periodo(de, ate)} nas caixas "
                  "do setor; nenhuma planilha exportada.")
        if falhas_caixa:
            print(f"  ⚠ caixas não lidas: {', '.join(falhas_caixa)} (resultado incompleto)")
        print("=" * 64)
        return None
    if not resultados:
        raise SystemExit("✗ Nenhum processo verificado; planilha não gerada.")

    faltantes, absorvidos = deduplicar([r for r in resultados if r["situacao"] == FALTA])
    cancelados = [r for r in resultados if r["situacao"] == CANCELADO]
    nome = os.path.splitext(os.path.basename(caminho))[0]
    destino = os.path.join(pasta_saida,
                           f"{nome}_faltantes_{datetime.now():%Y-%m-%d_%H%M}.xlsx")
    gerou = bool(faltantes or cancelados)
    n = exportar_faltantes(faltantes, destino, cancelados) if gerou else 0

    contagem = {}
    for r in resultados:
        contagem[r["situacao"]] = contagem.get(r["situacao"], 0) + 1

    def lista(sit):
        return ", ".join(r["numero"] for r in resultados if r["situacao"] == sit)

    print("\n" + "=" * 64)
    print(f"FALTANDO NA PLANILHA: {len(faltantes)}"
          + (f"  (+{len(absorvidos)} elaboração(ões) representada(s) pelo processo novo: "
             f"{', '.join(absorvidos)})" if absorvidos else ""))
    print("Verificados e deixados de fora:")
    for sit in (PASSOU, PASSOU_NOVO, CANCELADO, VINCULADO, CONCLUIDO, FORA):
        if contagem.get(sit):
            print(f"  {sit:<42} {contagem[sit]:>4}")
    for sit in (CANCELADO, INDETERM, SEM_ASSUNTO, ERRO):
        if contagem.get(sit):
            print(f"  {sit} — {lista(sit)}")
    if falhas_caixa:
        print(f"  ⚠ caixas não lidas: {', '.join(falhas_caixa)} (resultado incompleto)")
    print(f"Tempo: {time.time() - t0:.0f}s")
    if gerou:
        print(f"✓ Planilha: {destino}"
              + (f"  ({len(cancelados)} cancelado(s) na aba 'Cancelados (conferir)')"
                 if cancelados else ""))
        if not n:
            print("○ Nenhum processo faltando; a planilha traz só os cancelados para conferência.")
        if abrir:
            abrir_arquivo(destino)
    else:
        print("○ Nenhum processo faltando; nenhuma planilha exportada.")
    print("=" * 64)
    return destino if gerou else None


def main(argv=None):
    """Linha de comando: python -m ferramentas1doc.faltantes [planilha.xlsx] [--aba ABA]
    [--de DATA] [--ate DATA]"""
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("planilha", nargs="?")
    ap.add_argument("--aba", help="nome da aba (padrão: a ativa)")
    ap.add_argument("--de", metavar="DATA",
                    help=f"só processos emitidos a partir desta data ({FORMATOS_DATA}); "
                         "padrão: 1º/jan do ano mais antigo da planilha")
    ap.add_argument("--ate", metavar="DATA", help="só processos emitidos até esta data; padrão: hoje")
    args = ap.parse_args(argv)
    try:
        de, ate = ler_periodo(args.de, args.ate)
    except ValueError as e:
        ap.error(str(e))
    caminho = args.planilha or escolher_arquivo()
    cred = sessao_1doc.obter_sessao()
    return executar(caminho, args.aba, cred, de=de, ate=ate)


if __name__ == "__main__":
    main()
