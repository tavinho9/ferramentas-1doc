"""
SCRIPT: Extrator de processos do dia no 1Doc — varredura por requisição
=========================================================================
O dia vem da interface do programa (aba "Processos do dia") ou, rodando pela
linha de comando, é pedido no terminal (Enter = hoje).

O navegador só entra para o login, e quem digita usuário e senha é a pessoa,
na janela do navegador: o programa espera o login (ferramentas1doc.sessao).
Em seguida os cookies da sessão passam para um requests.Session, o navegador é
fechado e todo o resto sai por HTTP:

  LISTAGEM  Chama direto o endpoint que a própria interface usa para trocar
            de aba e paginar (link.php?chamada=caixa). O parâmetro
            `intervalo` filtra pela data de EMISSÃO no servidor, então um dia
            comum cabe em uma requisição por caixa — sem clicar em "próxima",
            sem esperar o AJAX redesenhar a tabela.

  DETALHE   A página de cada processo é baixada por GET, sem renderizar, em
            paralelo, e o kanban é lido do HTML (etapa data-current="true").

O 1Doc responde com fragmento HTML (o mesmo que o jQuery injeta na tela), não
com JSON; se um dia passar a responder JSON, o script extrai o HTML de dentro.
O que sai em JSON é a coleta: processos_do_dia.json, ao lado da planilha.

Critério de inclusão — todo processo exportado foi emitido no dia escolhido,
e além disso:
  COM QUADRO   dia = hoje: a etapa atual do kanban marca menos de 24h
               ("Esta aqui 1 hora" — o 1Doc escreve sem acento —,
               "Aqui há alguns segundos"). Vale só o quadro do cabeçalho
               (#emissao_principal): a página traz uma segunda cópia, no
               modelo oculto do despacho (#parent_wa), cujo tempo é sempre
               "Aqui há alguns segundos" e aprovaria todo processo.
               dia passado: entra com a etapa atual, qualquer que seja o tempo
               nela. Toda etapa de um processo emitido no dia D começou em D
               ou depois, então o teste não teria o que filtrar — e a janela
               fixa de 24h excluiria quase tudo ("Está aqui 3 dias").
  SEM QUADRO   é Proc. Administrativo com etiqueta de fase de elaboração
               (ex.: "Elaboração de Documentação Preparatória"). Não há
               "Está aqui" para medir; a emissão no dia já é a recência.

Coluna Etapa:
  ELABORAÇÃO   "Elaboração" — processos sem quadro ("Elaboração de
               Documentação Preparatória") e os da "Fase Preparatória de
               Procedimento Licitatório", e processos de outras fases (exceto
               abertura) cuja etapa atual no kanban seja "Etapa Final".
  ABERTURA     "<etapa atual> — Abertura" ("Validação de Requisição — Abertura",
               "Etapa Final — Abertura").
  OUTRAS       "<etapa atual> — <etiqueta do assunto>".
  O "Está aqui…" só decide a inclusão e aparece no terminal e no JSON
  (tempo_etapa), não na planilha; a etiqueta completa fica no JSON ("fase").

Coluna Objeto:
  ABERTURA     processo cuja etiqueta do assunto casa com ETIQUETAS_ABERTURA
               ("Abertura de Procedimento Licitatório"): o texto vem do campo
               adicional #campo_adicional_descricao_objeto, lido na página de
               detalhe. Se o campo não existir ou vier vazio, fica o título
               do processo, e o número aparece no resumo.
  DEMAIS       título do processo, como na listagem.

COMO USAR
    No programa: aba "Processos do dia" → Dia (DD/MM/AAAA, DD/MM/AA, DD/MM,
    "ontem" ou vazio para hoje) → Executar.
    Linha de comando: python -m ferramentas1doc.processos_do_dia [DIA]
    Saída na pasta configurada (padrão: Área de Trabalho): "processos novos
    DD.MM.xlsx" (dia e mês do dia escolhido), aberta ao final, e
    processos_do_dia.json (nome fixo; o JSON registra o dia em "dia")
"""

import copy
import json
import os
import re
import sys
import threading
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from datetime import date, datetime, timedelta
from importlib.util import find_spec
from urllib.parse import urljoin

import requests
from bs4 import BeautifulSoup, NavigableString, SoupStrainer
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry

from openpyxl import Workbook
from openpyxl.styles import Font, PatternFill, Alignment

from . import caminhos, config, sessao as sessao_1doc

PARSER = "lxml" if find_spec("lxml") else "html.parser"   # lxml é opcional, só mais rápido


# ============================================================
# CONFIGURAÇÃO
# ============================================================

URL_1DOC  = sessao_1doc.URL_1DOC
TIMEOUT   = 15

# Caixas varridas. Com o filtro de data, cada uma custa ~1 requisição num dia
# comum. O script anterior olhava só "entrada" e "arquivo"; para voltar a esse
# comportamento, apague as outras duas linhas.
CAIXAS = [
    ("entrada",   "Em aberto"),
    ("saida",     "Caixa de saída"),
    ("arquivo",   "Arquivados"),
    ("favoritos", "Favoritos"),
]
CAIXA_DO_SETOR = True   # True = caixa do setor (meu=0); False = caixa pessoal (meu=1)
ITD            = ""     # "" = todos os tipos de documento; "3" = só Proc. Administrativo
WORKERS        = 4      # páginas de detalhe baixadas em paralelo
PAUSA_REQ      = 0.15   # respiro entre requisições de cada worker (gentileza com o servidor)
MAX_PAGINAS    = 60     # teto de segurança por caixa

# Processos SEM quadro de etapas entram na planilha quando forem Proc.
# Administrativo e a etiqueta do assunto casar com ETIQUETAS_ELABORACAO (e não
# com ETIQUETAS_ABERTURA). Os que ficarem de fora aparecem no resumo com a
# etiqueta lida, para você ajustar o padrão se precisar.
INCLUIR_SEM_QUADRO   = True
ETIQUETAS_ELABORACAO = (r"Elabora[çc][ãa]o de Document|Documenta[çc][ãa]o Preparat[óo]ria"
                        r"|Fase Preparat[óo]ria|Aditivo")
ETIQUETAS_ABERTURA   = r"Abertura de Procedimento Licitat"

# Processos de abertura (etiqueta casa com ETIQUETAS_ABERTURA): a coluna Objeto
# usa o campo adicional abaixo em vez do título. False = título para todos.
OBJETO_DA_ABERTURA  = True
ID_DESCRICAO_OBJETO = "campo_adicional_descricao_objeto"
LIMITE_CELULA       = 32000   # Excel aceita até 32.767 caracteres por célula

NOME_PLANILHA = "processos novos {dia:%d.%m}.xlsx"   # → "processos novos 29.10.xlsx"
NOME_JSON     = "processos_do_dia.json"             # None = não grava
# Pasta de saída e "abrir a planilha ao terminar" vêm das Configurações do programa

URL_LINK  = f"{URL_1DOC}/link.php"
URL_CAIXA = (f"{URL_1DOC}/?pg=painel/listar&meu={0 if CAIXA_DO_SETOR else 1}"
             f"&trocar=1&caixa=entrada")


# ============================================================
# CRITÉRIOS DE TEMPO (inalterados)
# ============================================================

def eh_sub_dia(texto):
    """
    Texto relativo de movimentação ('3 horas', '45 minutos', '2 dias').
    True se indica menos de 1 dia. Formato desconhecido → True (não
    interrompe a paginação por excesso de cautela).
    """
    texto = texto.lower().strip()
    if re.search(r'\bdia(s)?\b|\bsemana(s)?\b|\bm[eê]s(es)?\b|\bano(s)?\b', texto):
        return False
    if re.search(r'\bsegundo(s)?\b|\bminuto(s)?\b|\bhora(s)?\b', texto):
        return True
    return True


def etapa_dentro_24h(texto):
    """
    Tempo na etapa atual ('Está aqui 3 dias', 'Aqui há alguns segundos').
    True se indica menos de 24h. Formato desconhecido → False (exclui).
    """
    texto = texto.lower()
    if re.search(r'\bdia(s)?\b|\bsemana(s)?\b|\bm[eê]s(es)?\b|\bano(s)?\b', texto):
        return False
    if re.search(r'\bsegundo(s)?\b|\bminuto(s)?\b|\bhora(s)?\b', texto):
        return True
    return False


# Mapeamento sigla → nome da secretaria (Prefeitura de Montes Claros)
SIGLAS_SECRETARIAS = {
    "AD":        "Assinatura Digital",
    "AC":        "Autorização para Compras",
    "AMASBE":    "Agência Municipal de Água e Saneamento Básico de Montes Claros",
    "CGM":       "Controladoria Geral do Município",
    "CIMP":      "Comissão de Implantação",
    "CM":        "Câmara Municipal",
    "ESURB":     "ESURB",
    "GAB":       "Gabinete do Prefeito",
    "GABP":      "Prefeito",
    "GABVP":     "Gabinete do Vice-Prefeito",
    "PGM":       "Procuradoria Geral do Município",
    "SA":        "Secretaria de Administração",
    "SABEAS":    "Secretaria de Ambiente, Bem-Estar Animal e Sustentabilidade",
    "SAE":       "Secretaria de Aceleração Econômica",
    "SAPDR":     "Secretaria de Agricultura, Pecuária e Desenvolvimento Rural",
    "SC":        "Secretaria de Comunicação",
    "SCT":       "Secretaria de Cultura e Turismo",
    "SDC":       "Secretaria de Defesa Civil",
    "SDS":       "Secretaria de Desenvolvimento Social",
    "SE":        "Secretaria de Educação",
    "SEJ":       "Secretaria de Esporte e Juventude e Lazer",
    "SEJL":      "Secretaria de Esporte e Juventude e Lazer",
    "SELJ":      "Secretaria de Esporte e Juventude e Lazer",
    "SF":        "Secretaria de Finanças",
    "SGRI":      "Secretaria de Governo e Relações Institucionais",
    "SIPE":      "Secretaria de Inovação e Projetos Especiais",
    "SIPU":      "Secretaria de Infraestrutura e Planejamento Urbano",
    "SPOT":      "Secretaria de Planejamento, Orçamento e Tecnologia",
    "SS":        "Secretaria de Saúde",
    "SSI":       "Secretaria de Segurança Integrada",
    "SSU":       "Secretaria de Serviços Urbanos",
    "SUPERMOC":  "Superintendência de Administração de Estádios e Estabelecimentos de Montes Claros",
}


def normalizar_sigla(texto):
    """Remove espaços e hífens para comparação."""
    return re.sub(r'[\s\-]', '', texto).upper()


def resolver_secretaria(sigla_bruta):
    """Extrai a sigla (letras antes do primeiro hífen) e converte para nome completo."""
    parte = sigla_bruta.split("-")[0].strip()
    chave = normalizar_sigla(parte)
    return SIGLAS_SECRETARIAS.get(chave, sigla_bruta.strip())


# ============================================================
# DATA DOS PROCESSOS (perguntada no terminal)
# ============================================================

def interpretar_data(txt, hoje):
    """
    'DD/MM/AAAA', 'DD/MM/AA' ou 'DD/MM' → date. Aceita '/', '-', '.' ou
    espaço como separador, e também os dígitos colados (19082026, 190826).
    Sem ano: o corrente — ou o anterior, se a data cairia no futuro
    (digitar 20/12 em janeiro). None se não for uma data válida.
    """
    txt = txt.strip()
    if re.fullmatch(r"\d{6}|\d{8}", txt):
        partes = [txt[:2], txt[2:4], txt[4:]]
    else:
        partes = [p for p in re.split(r"[/\-.\s]+", txt) if p]
    sem_ano = len(partes) == 2
    if sem_ano:
        partes.append(str(hoje.year))
    if len(partes) != 3 or not all(p.isdigit() for p in partes) or len(partes[2]) not in (2, 4):
        return None
    d, m, a = (int(p) for p in partes)
    if len(partes[2]) == 2:
        a += 2000
    try:
        alvo = date(a, m, d)
        if sem_ano and alvo > hoje:
            alvo = date(a - 1, m, d)
    except ValueError:          # 31/02, 29/02 em ano não bissexto, mês 13...
        return None
    return alvo


def pedir_data(hoje):
    """Pergunta o dia de emissão no terminal até receber uma data válida."""
    while True:
        txt = input(f"Dia de emissão dos processos [Enter = hoje, {hoje:%d/%m/%Y}]: ").strip().lower()
        if txt in ("", "hoje"):
            return hoje
        if txt == "ontem":
            return hoje - timedelta(days=1)
        alvo = interpretar_data(txt, hoje)
        if alvo is None:
            print("  ✗ Data não reconhecida. Use DD/MM/AAAA, DD/MM/AA, DD/MM ou 'ontem'.")
        elif alvo > hoje:
            print(f"  ✗ {alvo:%d/%m/%Y} ainda não chegou.")
        else:
            return alvo


def _data_br(txt):
    """'17/08/2026' ou '17/08/2026 13:09' → date (None se não casar)."""
    m = re.match(r"\s*(\d{2})/(\d{2})/(\d{4})", txt or "")
    if not m:
        return None
    try:
        return date(int(m.group(3)), int(m.group(2)), int(m.group(1)))
    except ValueError:
        return None


# ============================================================
# SESSÃO HTTP (o login é feito pela pessoa no navegador: ver sessao.py)
# ============================================================

def sessao_http(cookies, user_agent):
    """Cookies e User-Agent do navegador logado → requests.Session."""
    s = requests.Session()
    s.headers.update({
        "User-Agent": user_agent or "Mozilla/5.0",
        "Accept-Language": "pt-BR,pt;q=0.9",
    })
    for c in cookies:
        s.cookies.set(c["name"], c["value"], domain=c.get("domain"), path=c.get("path") or "/")

    # Repete sozinho em erro transitório (5xx, 429, conexão), com espera crescente
    retry = Retry(total=3, backoff_factor=0.6, status_forcelist=(429, 500, 502, 503, 504))
    adaptador = HTTPAdapter(max_retries=retry, pool_connections=WORKERS, pool_maxsize=WORKERS)
    s.mount("https://", adaptador)
    s.mount("http://", adaptador)
    return s


# ============================================================
# CAMADA HTTP
# ============================================================

class SessaoExpirada(RuntimeError):
    """A resposta veio com cara de tela de login: os cookies não valem (ou caíram)."""


class LayoutDesconhecido(RuntimeError):
    """A página carregou logada, mas sem as marcas da interface clássica do
    1Doc — o caso típico é a "Nova 1Doc" (facelift) ativada na conta, que
    troca o HTML de painel/listar e doc/ver."""


_MARCAS_APP = ("link_emissao_a", "caixa_parametro", "navega_caixa",
               "kanban_estagios", "emission_infos")

_lock = threading.Lock()
CONTAGEM = {"listagem": 0, "detalhe": 0, "outras": 0}


def _parece_login(html):
    tem_campo_senha = (re.search(r"""type\s*=\s*["']?password""", html, re.I)
                       or 'name="identifier"' in html)
    return bool(tem_campo_senha) and not any(m in html for m in _MARCAS_APP)


def _html_da_resposta(r):
    """Texto da resposta como HTML. Se o servidor devolver JSON, junta as
    strings de dentro que contenham marcação."""
    ctype = r.headers.get("Content-Type", "").lower()
    if "charset" not in ctype:
        r.encoding = "utf-8"   # o 1Doc declara utf-8 no <meta>; sem isso o requests assume latin-1
    texto = r.text

    if "json" in ctype or texto.lstrip().startswith(("{", "[")):
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
    return texto


def baixar(sessao, url, params=None, tipo="outras", xhr=False, referer=None):
    sessao_1doc.checar_cancelamento()     # botão Cancelar da interface
    cabecalhos = {}
    if xhr:
        cabecalhos["X-Requested-With"] = "XMLHttpRequest"   # como o jQuery da interface
    if referer:
        cabecalhos["Referer"] = referer
    r = sessao.get(url, params=params, headers=cabecalhos, timeout=TIMEOUT)
    r.raise_for_status()
    with _lock:
        CONTAGEM[tipo] += 1
    html = _html_da_resposta(r)
    if _parece_login(html):
        raise SessaoExpirada(f"{url.split('?')[0]} devolveu a tela de login")
    time.sleep(PAUSA_REQ)
    return html


def preparar_sessao(sessao):
    """Abre a caixa do setor uma vez: fixa o contexto (meu/trocar) na sessão
    do servidor e prova que os cookies do navegador valem por requisição."""
    html = baixar(sessao, URL_CAIXA)   # tela de login aqui já levanta SessaoExpirada
    if "caixa_parametro" not in html and "tab-content" not in html:
        # Não é login (baixar teria detectado), então é a interface que mudou
        raise LayoutDesconhecido(
            "a caixa do setor abriu, mas sem os campos caixa_parametro_* da interface "
            "clássica — a 'Nova 1Doc' foi ativada? Desligue a chave no topo da caixa.")
    print("✓ Sessão HTTP válida — cookies do login reaproveitados")


# ============================================================
# LISTAGEM (link.php?chamada=caixa)
# ============================================================

def _limpo(txt):
    return " ".join((txt or "").split())


def _texto_direto(tag):
    """Só os nós de texto filhos diretos — o equivalente ao filtro
    nodeType === TEXT_NODE que o script antigo fazia via execute_script.
    Espaços e quebras internos são colapsados: o HTML indenta o tipo em
    várias linhas ("Proc.\\n   Administrativo"), o que quebrava o teste
    startswith("Proc. Administrativo") e levava quebras para a planilha."""
    return _limpo(" ".join(s for s in tag.children if type(s) is NavigableString))


def _titulo(tag):
    if tag is None:
        return ""
    # Sem JS rodando, o title fica no lugar; no navegador o tooltip do
    # Bootstrap o move para data-original-title. Aceita os dois.
    return tag.get("title") or tag.get("data-original-title") or ""


def params_caixa(caixa, dia, num_pagina):
    p = {
        "chamada": "caixa", "pg": "painel/listar", "caixa": caixa,
        "num_pagina": str(num_pagina), "ordenar": "1", "ordem": "desc",
        "modo_visualizacao": "1", "intervalo": f"{dia} - {dia}",
        "emissoes_arquivado_prazos": "0",
    }
    if ITD:
        p["itd"] = ITD
    return p


def ler_total_paginas(html):
    """'Página 1/15' → 15 (None se o contador não vier no fragmento)."""
    m = re.search(r"P(?:á|&aacute;|a)gina\s*(\d+)\s*/\s*(\d+)", html)
    return int(m.group(2)) if m else None


def extrair_linhas(html):
    """Uma entrada por linha da tabela (tr que contém a.link_emissao_a)."""
    soup = BeautifulSoup(html, PARSER)
    linhas = []
    for a in soup.select("a.link_emissao_a"):
        tr = a.find_parent("tr")
        href = a.get("href")
        if tr is None or not href:
            continue

        texto_link = _texto_direto(a)                    # "Proc. Administrativo 31.632/2026"
        m = re.search(r"\d[\d.]*/\d+", texto_link)
        numero = m.group() if m else texto_link
        tipo_doc = texto_link[:m.start()].strip() if m else ""

        el = a.select_one("small.data")
        emissao = _limpo(el.get_text()) if el else ""     # "17/08/2026 13:09"

        # Última movimentação: o title traz a data absoluta
        # ("Última movimentação em 17/08/2026 13:09:56"); o texto, a relativa.
        el = tr.select_one(".tt.data")
        m_mov = re.search(r"\d{2}/\d{2}/\d{4}", _titulo(el))
        mov_data = m_mov.group() if m_mov else ""
        mov_rel = _limpo(el.get_text()) if el else ""

        el = tr.select_one("span.label[title='Prioridade']") or tr.select_one(".tt.label")
        prioridade = _limpo(el.get_text()) if el else ""

        # Objeto: 1º texto direto das células .link_emissao.td.ignora.normal,
        # fora remetente e destinatário (mesma regra do script antigo).
        # Para processos de abertura ele é substituído no detalhe pelo campo
        # adicional de descrição do objeto (ver processar_processo).
        objeto, td_assunto = "", None
        for td in tr.select(".link_emissao.td.ignora.normal"):
            if "celula_para" in (td.get("class") or []) or td.find_parent(class_="td_remetente"):
                continue
            texto = _texto_direto(td)
            if texto:
                objeto, td_assunto = texto, td
                break

        # Etiqueta do assunto (badge sob o objeto): "Fase Preparatória de
        # Procedimento Licitatório", "Elaboração de Documentação Preparatória"...
        etiqueta = ""
        if td_assunto is not None:
            b = td_assunto.select_one("span.badge") or td_assunto.select_one("span[title]:not(.label)")
            if b is not None:
                etiqueta = _limpo(b.get("data-original-title") or b.get("title") or b.get_text())

        setor_de, de = "", ""
        rem = tr.select_one(".td_remetente")
        if rem is not None:
            b = rem.select_one(".badge.badge_env.tt") or rem.select_one("span.badge")
            setor_de = _limpo(b.get_text()) if b else ""
            # Nome completo no popover (span.pp[data-content]); o texto visível
            # vem abreviado ("Suelen A."). Sem o popover, o texto do bloco.
            pp = rem.select_one("span.pp[data-content]")
            clone = copy.copy(rem)
            for e in clone.select("span.badge, i, .label"):
                e.decompose()
            partes = [p.strip() for p in clone.get_text("\n").split("\n") if p.strip()]
            de = (_limpo(pp["data-content"]) if pp is not None and pp["data-content"].strip()
                  else partes[0] if partes else "")[:60]

        m_hash = re.search(r"[?&]hash=([0-9A-Za-z]+)", href)
        m_itd = re.search(r"[?&]itd=(\d+)", href)          # 3 = Proc. Administrativo
        linhas.append({
            # Chave de deduplicação: o hash do documento. A URL não serve,
            # porque muda o parâmetro 'origem' conforme a caixa.
            "chave": m_hash.group(1) if m_hash else (numero or href),
            "numero": numero,
            "tipo_doc": tipo_doc,
            "itd": m_itd.group(1) if m_itd else "",
            # Link canônico: só hash (+ itd). O href da listagem carrega ainda o
            # contexto de navegação de quem gerou (origem=painel_setor, ordenar,
            # página...), que varia por caixa e não tem a ver com o documento.
            "url": (f"{URL_1DOC}/?pg=doc/ver&hash={m_hash.group(1)}"
                    + (f"&itd={m_itd.group(1)}" if m_itd else "")
                    if m_hash else urljoin(URL_1DOC + "/", href)),
            "emissao": emissao,
            "mov_data": mov_data,
            "mov_rel": mov_rel,
            "prioridade": prioridade,
            "objeto": objeto,
            "etiqueta": etiqueta,
            "de": de,
            "setor_de": setor_de,
            "secretaria": resolver_secretaria(setor_de) if setor_de else "",
        })
    return linhas


def _pode_ser_da_data(linha, dia):
    """Linha emitida ou movimentada no dia pedido ou depois dele — sinal de
    que a paginação (mais recentes primeiro) ainda não passou desse dia. Só
    pesa na rede de segurança, caso o servidor ignore o filtro de data. Para
    hoje equivale ao critério antigo (emitida ou movimentada hoje)."""
    alvo = _data_br(dia)
    emissao, mov = _data_br(linha["emissao"]), _data_br(linha["mov_data"])
    if (emissao and emissao >= alvo) or (mov and mov >= alvo):
        return True
    if mov is None:
        # Sem data absoluta da movimentação: para hoje vale o texto relativo
        # ('3 horas'); para dia passado, na dúvida continua paginando
        # (MAX_PAGINAS segura o teto).
        return eh_sub_dia(linha["mov_rel"]) or alvo < date.today()
    return False


def listar_caixa(sessao, caixa, rotulo, dia):
    achados, vistos = [], set()
    total_pag, lidas = None, 0

    for num in range(1, MAX_PAGINAS + 1):
        html = baixar(sessao, URL_LINK, params=params_caixa(caixa, dia, num),
                      tipo="listagem", xhr=True, referer=URL_CAIXA)
        lidas += 1
        linhas = extrair_linhas(html)
        if total_pag is None:
            total_pag = ler_total_paginas(html)

        novas = [l for l in linhas if l["chave"] not in vistos]
        if not novas:          # página vazia, ou o servidor devolvendo a última de novo
            break
        vistos.update(l["chave"] for l in novas)

        for l in novas:
            if l["emissao"].startswith(dia):   # confere a data lida, não só o filtro
                l["caixa"] = rotulo
                achados.append(l)

        if total_pag is not None and num >= total_pag:
            break
        # Rede de segurança: se o servidor um dia ignorar `intervalo`, vale o
        # critério antigo — página sem nada do dia pedido (ou posterior)
        # encerra a caixa.
        if not any(_pode_ser_da_data(l, dia) for l in linhas):
            break

    return achados, lidas, total_pag


def _data_emissao(proc):
    try:
        return datetime.strptime(proc["emissao"][:16], "%d/%m/%Y %H:%M")
    except ValueError:
        return datetime.min


def coletar_do_dia(sessao, dia, caixas=None):
    por_chave, caixas_com_falha = {}, []
    for caixa, rotulo in caixas or CAIXAS:
        try:
            achados, lidas, total = listar_caixa(sessao, caixa, rotulo, dia)
        except SessaoExpirada:
            raise
        except requests.RequestException as e:
            print(f"  ⚠ {rotulo}: falha na listagem ({str(e)[:80]})")
            caixas_com_falha.append(rotulo)
            continue

        pags = "1 requisição" if lidas == 1 else f"{lidas} requisições"
        if total:
            pags += f", {total} pág. no filtro"
        print(f"  {rotulo:<15} {len(achados):>3} emitido(s) no dia  ({pags})")

        for l in achados:
            if l["chave"] in por_chave:
                por_chave[l["chave"]]["caixa"] += f", {rotulo}"
            else:
                por_chave[l["chave"]] = l

    todos = sorted(por_chave.values(), key=_data_emissao, reverse=True)
    return todos, caixas_com_falha


# ============================================================
# DETALHE (etapa atual no kanban)
# ============================================================

_RE_ATUAL = re.compile(r"""data-current\s*=\s*["']?true""", re.I)
# Início do modelo oculto do despacho (div#parent_wa dentro de .tm_conteudo).
# O \b separa de "data-current_idph", atributo da mesma div.
_RE_MODELO_DESPACHO = re.compile(r"""\bid\s*=\s*["']?parent_wa\b""")


def _antes_do_modelo(html):
    """HTML até o modelo oculto do despacho. O 1Doc renderiza o kanban duas
    vezes: no cabeçalho (#emissao_principal), com o tempo real na etapa
    ("Esta aqui 1 hora"), e dentro de #parent_wa, um modelo do formulário de
    despacho cujo tempo é sempre "Aqui há alguns segundos". Ler a segunda
    cópia faria todo processo passar na janela de 24h."""
    m = _RE_MODELO_DESPACHO.search(html)
    return html[:m.start()] if m else html


def _etapa(atual):
    """(nome, tempo) de um nó data-current, ou None se não tiver título."""
    titulo = atual.select_one(".kanban_titulo p") if atual is not None else None
    if titulo is None or not _limpo(titulo.get_text()):
        return None
    return _limpo(titulo.get_text()), _tempo_da_etapa(atual)


def _tempo_da_etapa(atual):
    """'Esta aqui 1 hora' / 'Aqui há alguns segundos' ('' se não houver).
    A "Etapa Final" (div.estagio_final) não tem p.kanban_item — o bloco vem
    vazio —, e o tempo é opcional. Plano B: o div.kanban-time, onde o 1Doc
    passou a escrever o tempo das etapas já concluídas ("Ficou aqui 6 dias")
    e que hoje vem vazio na etapa atual."""
    for sel in (".kanban_itens p.kanban_item", ".kanban_itens .kanban-time"):
        el = atual.select_one(sel)
        if el is not None and _limpo(el.get_text()):
            return _limpo(el.get_text())
    return ""


def ler_etapa_atual(html):
    """
    (nome, tempo) da etapa marcada data-current="true" no quadro do
    cabeçalho, ou (None, None).
    Caminho rápido: a primeira marca ANTES do modelo de despacho, num
    recorte de 8 kB (não parseia centenas de kB). Caminho lento, se o
    rápido falhar ou a ordem da página mudar: a página inteira, ignorando
    qualquer marca dentro de #parent_wa.
    """
    antes = _antes_do_modelo(html)
    m = _RE_ATUAL.search(antes)
    if m:
        inicio = max(antes.rfind("<", 0, m.start()), 0)
        achado = _etapa(BeautifulSoup(antes[inicio:inicio + 8000], PARSER)
                        .select_one("[data-current='true']"))
        if achado:
            return achado
    if not _RE_ATUAL.search(html):
        return None, None
    for atual in BeautifulSoup(html, PARSER).select("[data-current='true']"):
        if atual.find_parent(id="parent_wa") is None:
            achado = _etapa(atual)
            if achado:
                return achado
    return None, None


_RE_ASSUNTO = re.compile(r"""class\s*=\s*["'][^"']*\btit_assunto\b""", re.I)


def ler_etiqueta(html):
    """
    Etiqueta do assunto no cabeçalho do processo — o badge dentro de
    h4.tit_assunto, com title "Assunto: Elaboração de Documentação
    Preparatória" —, sem o prefixo "Assunto:". "" se não achar.
    Mesmo esquema de recorte do kanban; a regex mira o atributo class para
    não parar no CSS ou no JS que citam .tit_assunto.
    """
    m = _RE_ASSUNTO.search(html)
    if not m:
        return ""
    inicio = max(html.rfind("<", 0, m.start()), 0)
    for fonte in (html[inicio:inicio + 4000], html):
        h4 = BeautifulSoup(fonte, PARSER).select_one(".tit_assunto")
        b = h4.select_one("span.badge") if h4 is not None else None
        if b is not None:
            txt = b.get("title") or b.get("data-original-title") or b.get_text()
            return _limpo(re.sub(r"^\s*Assunto:\s*", "", txt))
    return ""


# Mira o atributo id (e não o CSS/JS que citem o nome); o \b impede casar com
# um id mais longo, como "campo_adicional_descricao_objeto_2".
_RE_DESC_OBJETO = re.compile(r"""\bid\s*=\s*["']?""" + re.escape(ID_DESCRICAO_OBJETO) + r"""\b""")
_BLOCOS = ("p", "div", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6")


def ler_descricao_objeto(html):
    """
    Texto do elemento #campo_adicional_descricao_objeto ("" se não houver).
    Funciona se o campo vier como input (value), textarea ou bloco de texto.
    Quebras de linha de <br> e de parágrafos são preservadas; espaços
    internos de cada linha são normalizados. Aqui não há recorte por
    tamanho, como no kanban: a descrição pode ser longa, e um recorte a
    cortaria sem aviso. O SoupStrainer monta só a subárvore do campo.
    """
    if not _RE_DESC_OBJETO.search(html):
        return ""
    soup = BeautifulSoup(html, PARSER, parse_only=SoupStrainer(id=ID_DESCRICAO_OBJETO))
    el = soup.find(id=ID_DESCRICAO_OBJETO)
    if el is None:
        return ""

    # Estrutura do 1Doc: div#campo… > div.cp > span[data-valor] (valor no
    # atributo) + small (o mesmo valor como texto), ao lado do rótulo do
    # campo. data-valor é a fonte mais limpa; o small vem em seguida.
    valor = el.select_one("[data-valor]")
    small = el.select_one("small")
    if valor is not None and valor.get("data-valor", "").strip():
        txt = valor["data-valor"]
        if "<" in txt:   # valor com marcação (ex.: <br>) → texto com quebras
            frag = BeautifulSoup(txt.replace("<br", "\n<br"), PARSER)
            txt = frag.get_text()
    elif small is not None and small.get_text(strip=True):
        el = small
        for br in el.find_all("br"):
            br.replace_with("\n")
        txt = el.get_text()
    elif el.name == "input":
        txt = el.get("value", "")
    elif el.name == "textarea":
        txt = el.get_text()
    else:
        for e in el.find_all(["script", "style"]):
            e.decompose()
        for br in el.find_all("br"):
            br.replace_with("\n")
        for bloco in el.find_all(_BLOCOS):
            bloco.append("\n")
        txt = el.get_text()

    linhas = (_limpo(l) for l in txt.splitlines())
    txt = "\n".join(l for l in linhas if l)
    # Se o rótulo do campo vier junto no mesmo elemento, sai daqui. O rótulo
    # atual é "Objeto*:" (o asterisco de obrigatório vem num span à parte);
    # o antigo, "Descrição do objeto:".
    txt = re.sub(r"^\s*(?:Descri[çc][ãa]o do )?objeto\s*\*?\s*:\s*", "", txt, flags=re.I)
    return txt[:LIMITE_CELULA]


ETIQUETAS_PREPARATORIA = r"Fase Preparat[óo]ria"
ETAPAS_COMO_ELABORACAO = r"^\s*Etapa Final\s*$"   # etapas do kanban rotuladas "Elaboração" (fora da abertura)


def rotulo_etapa(nome, etiqueta, sem_quadro):
    """
    Texto da coluna Etapa:
      fase de elaboração — sem quadro ("Elaboração de Documentação
        Preparatória") ou "Fase Preparatória de Procedimento Licitatório"
        → "Elaboração"
      abertura → "<etapa do kanban> — Abertura"
      outra fase com quadro → "<etapa do kanban> — <etiqueta>"
    """
    # Abertura vem antes: nela até a "Etapa Final" mantém o nome do kanban
    if not sem_quadro and re.search(ETIQUETAS_ABERTURA, etiqueta or "", re.I):
        return f"{nome} — Abertura"
    if (sem_quadro or re.search(ETIQUETAS_PREPARATORIA, etiqueta or "", re.I)
            or re.search(ETAPAS_COMO_ELABORACAO, nome or "", re.I)):
        return "Elaboração"
    return f"{nome} — {etiqueta}" if etiqueta else nome


def classificar_sem_quadro(proc, etiqueta):
    """(entra na planilha?, motivo se não entra) para documento sem quadro."""
    if not INCLUIR_SEM_QUADRO:
        return False, "sem quadro de etapas"
    if proc.get("itd") != "3" and not proc.get("tipo_doc", "").startswith("Proc. Administrativo"):
        return False, f"sem quadro — não é Proc. Administrativo ({proc.get('tipo_doc') or '?'})"
    if not etiqueta:
        return False, "sem quadro — etiqueta do assunto não encontrada"
    if re.search(ETIQUETAS_ABERTURA, etiqueta, re.I) or not re.search(ETIQUETAS_ELABORACAO, etiqueta, re.I):
        return False, f"sem quadro — etiqueta fora da elaboração: {etiqueta}"
    return True, ""


def processar_processo(sessao, proc, exigir_24h=True):
    """exigir_24h=True (dia = hoje): etapa atual com menos de 24h.
    False (dia passado): entra com a etapa atual, qualquer que seja o tempo."""
    res = dict(proc, etapa="", etapa_nome="", tempo_etapa="", sem_quadro=False,
               incluir=False, motivo="", erro="",
               titulo=proc.get("objeto", ""), objeto_fonte="título", abertura=False)
    try:
        html = baixar(sessao, proc["url"], tipo="detalhe", referer=URL_CAIXA)
        # Sem cabeçalho de assunto nem quadro, a página não é o doc/ver que o
        # script conhece. Sem esta trava o processo cairia, calado, em
        # "sem quadro — etiqueta não encontrada"; assim vai para as falhas.
        if "tit_assunto" not in html and "kanban_estagios" not in html:
            raise LayoutDesconhecido("detalhe sem h4.tit_assunto nem kanban (layout mudou?)")
        nome, tempo = ler_etapa_atual(html)

        # Etiqueta do cabeçalho do detalhe; a da listagem é o plano B
        etiqueta = ler_etiqueta(html) or proc.get("etiqueta", "")
        res["etiqueta"] = etiqueta

        # Fase de abertura: objeto = campo adicional de descrição do objeto
        if etiqueta and re.search(ETIQUETAS_ABERTURA, etiqueta, re.I):
            res["abertura"] = True
            if OBJETO_DA_ABERTURA:
                descricao = ler_descricao_objeto(html)
                if descricao:
                    res.update(objeto=descricao, objeto_fonte=ID_DESCRICAO_OBJETO)

        if nome:                                   # COM QUADRO, etapa "Está aqui"
            # O tempo ("Está aqui 3 horas") só decide a inclusão; na coluna
            # Etapa sai a fase do processo (a etiqueta do assunto).
            # Sem "Está aqui" (Etapa Final) não há o que medir: vale a emissão
            # no dia, como nos processos sem quadro.
            dentro = etapa_dentro_24h(tempo) if (exigir_24h and tempo) else True
            res.update(etapa=rotulo_etapa(nome, etiqueta, False), etapa_nome=nome, tempo_etapa=tempo,
                       incluir=dentro, motivo="" if dentro else "etapa atual há 1 dia ou mais")

        elif "kanban_estagios" in html:            # quadro existe, mas nenhuma etapa atual
            res["motivo"] = "quadro sem etapa atual (saiu de uma etapa, não entrou na seguinte)"

        else:                                      # SEM QUADRO de etapas (fase de elaboração)
            entra, motivo = classificar_sem_quadro(proc, etiqueta)
            res.update(sem_quadro=True, incluir=entra, motivo=motivo,
                       etapa=rotulo_etapa("", etiqueta, True))
    except SessaoExpirada:
        raise
    except Exception as e:   # falha de leitura ≠ processo excluído: vai para o resumo
        res["erro"] = str(e)[:120]
    return res


# ============================================================
# EXPORTAÇÃO
# ============================================================

def exportar_xlsx(processos, arquivo):
    wb = Workbook()
    ws = wb.active
    ws.title = "Processos Recentes"

    cabecalho = ["Nº Processo", "Prioridade", "Objeto", "Etapa", "Secretaria"]
    hdr_font = Font(bold=True, color="FFFFFF", name="Arial", size=11)
    hdr_fill = PatternFill("solid", fgColor="2F5496")
    hdr_align = Alignment(horizontal="center", vertical="center", wrap_text=True)
    for col, titulo in enumerate(cabecalho, 1):
        c = ws.cell(row=1, column=col, value=titulo)
        c.font, c.fill, c.alignment = hdr_font, hdr_fill, hdr_align

    data_font = Font(name="Arial", size=10)
    link_font = Font(name="Arial", size=10, color="0563C1", underline="single")
    wrap = Alignment(vertical="top", wrap_text=True)
    for i, proc in enumerate(processos, 2):
        c = ws.cell(row=i, column=1, value=proc.get("numero", ""))
        url = proc.get("url", "")
        if url:
            # Abre o processo no 1Doc (no navegador padrão — exige estar logado nele)
            c.hyperlink, c.font = url, link_font
        else:
            c.font = data_font
        ws.cell(row=i, column=2, value=proc.get("prioridade", "")).font = data_font
        c = ws.cell(row=i, column=3, value=proc.get("objeto", ""))
        c.font, c.alignment = data_font, wrap
        c = ws.cell(row=i, column=4, value=proc.get("etapa", ""))
        c.font, c.alignment = data_font, wrap
        ws.cell(row=i, column=5, value=proc.get("secretaria", "")).font = data_font

    for col, larg in zip("ABCDE", (30, 14, 55, 40, 50)):
        ws.column_dimensions[col].width = larg
    ws.auto_filter.ref = f"A1:E{len(processos) + 1}"
    ws.freeze_panes = "A2"

    salvo = _salvar(arquivo, wb.save)
    print(f"  Processos exportados: {len(processos)}")
    return salvo


def exportar_json(resultados, dia, arquivo, caixas=None):
    """Coleta completa (inclusive os excluídos, com o motivo). As chaves de
    'itens' seguem as da listagem da skill do 1Doc (numero, data, hora,
    assunto, tipo, prioridade, de, setor_de, caixa). 'assunto' é o mesmo
    texto da coluna Objeto; 'titulo' guarda o título original e
    'objeto_fonte' diz de onde o objeto veio."""
    itens = []
    for r in resultados:
        data, _, hora = r["emissao"].partition(" ")
        itens.append({
            "numero": r["numero"], "data": data, "hora": hora,
            "assunto": r["objeto"], "titulo": r["titulo"], "objeto_fonte": r["objeto_fonte"],
            "tipo": r["etiqueta"] or r["tipo_doc"],
            "prioridade": r["prioridade"] or None,
            "de": r["de"], "setor_de": r["setor_de"], "secretaria": r["secretaria"],
            "caixa": r["caixa"], "url": r["url"],
            "etapa": r["etapa_nome"], "fase": r["etiqueta"] or None,
            "tempo_etapa": r["tempo_etapa"],
            "sem_quadro": r["sem_quadro"], "abertura": r["abertura"], "incluir": r["incluir"],
            "motivo": r["motivo"] or None, "erro": r["erro"] or None,
        })
    dados = {
        "dia": dia,
        "gerado_em": datetime.now().isoformat(timespec="seconds"),
        "caixas": [rotulo for _, rotulo in caixas or CAIXAS],
        "total": len(itens),
        "numeros": [i["numero"] for i in itens],
        "itens": itens,
    }

    def gravar(caminho):
        with open(caminho, "w", encoding="utf-8") as fh:
            json.dump(dados, fh, ensure_ascii=False, indent=2)

    _salvar(arquivo, gravar)


def _salvar(arquivo, gravar):
    """Grava no caminho pedido; se falhar (ex.: Área de Trabalho no OneDrive),
    tenta na pasta de dados do programa."""
    try:
        pasta = os.path.dirname(os.path.abspath(arquivo))
        if pasta:
            os.makedirs(pasta, exist_ok=True)
        gravar(arquivo)
        print(f"✓ Salvo em: {os.path.abspath(arquivo)}")
        return os.path.abspath(arquivo)
    except Exception as e:
        fallback = os.path.join(caminhos.pasta_dados(), os.path.basename(arquivo))
        print(f"⚠ Falha ao salvar em '{arquivo}': {e}")
        gravar(fallback)
        print(f"✓ Salvo na pasta de dados do programa: {os.path.abspath(fallback)}")
        return os.path.abspath(fallback)


abrir_arquivo = caminhos.abrir_arquivo


# ============================================================
# MAIN
# ============================================================

def executar(alvo, cred, caixas=None, pasta_saida=None, abrir=None, hoje=None):
    """Roda a extração para o dia `alvo` (date) com as credenciais da sessão
    (sessao.obter_sessao). Devolve o caminho da planilha, ou None."""
    hoje = hoje or date.today()   # fixado uma vez: não muda se cruzar a meia-noite
    caixas = list(caixas or CAIXAS)
    cfg = config.carregar()
    pasta_saida = pasta_saida or config.pasta_saida(cfg)
    abrir = cfg["abrir_planilha"] if abrir is None else abrir
    if alvo > hoje:
        raise SystemExit(f"✗ {alvo:%d/%m/%Y} ainda não chegou.")
    if not caixas:
        raise SystemExit("✗ Marque ao menos uma caixa.")
    dia = alvo.strftime("%d/%m/%Y")
    exigir_24h = alvo == hoje
    for k in CONTAGEM:
        CONTAGEM[k] = 0

    print("=" * 60)
    print("EXTRATOR DE PROCESSOS DO DIA — 1Doc (por requisição)")
    print("-" * 60)
    print(f"Emitidos em {dia}  |  execução: {datetime.now().strftime('%d/%m/%Y %H:%M')}")
    print(f"Caixas: {', '.join(r for _, r in caixas)}")
    if not exigir_24h:
        print("Dia passado: processos com quadro entram com a etapa atual,")
        print("qualquer que seja o tempo nela (sem a janela de 24h).")
    print("=" * 60)

    t0 = time.time()
    sessao = sessao_http(cred.cookies, cred.ua)

    try:
        preparar_sessao(sessao)

        print(f"\n--- Listagem (filtro de emissão no servidor: {dia}) ---")
        todos, caixas_com_falha = coletar_do_dia(sessao, dia, caixas)
        print(f"✓ Processos únicos emitidos em {dia}: {len(todos)}")

        resultados = []
        if todos:
            print(f"\n--- Detalhe: etapa no kanban ou etiqueta, se não houver quadro "
                  f"({WORKERS} em paralelo) ---")
            with ThreadPoolExecutor(max_workers=WORKERS) as ex:
                for i, r in enumerate(ex.map(lambda p: processar_processo(sessao, p, exigir_24h), todos), 1):
                    status = "✗" if r["erro"] else ("✓" if r["incluir"] else "⏸")
                    # No terminal o tempo na etapa continua visível (é o que decide a janela de 24h)
                    etapa_tela = (f"{r['etapa_nome']} ({r['tempo_etapa']})" if r["tempo_etapa"]
                                  else r["etapa_nome"] or r["etiqueta"] or r["etapa"])
                    info = r["erro"] or (etapa_tela if r["incluir"] else r["motivo"]) or etapa_tela
                    print(f"  [{i}/{len(todos)}] [{status}] {r['numero']} | {info[:70]}")
                    resultados.append(r)
    except SessaoExpirada as e:
        sessao_1doc.esquecer()
        print(f"\n✗ Sessão HTTP recusada: {e}")
        print("  A sessão do 1Doc expirou ou caiu. Execute de novo: o navegador vai")
        print("  abrir para o login. Se persistir, o 1Doc mudou o login ou o endpoint.")
        return None
    except LayoutDesconhecido as e:
        print(f"\n✗ Interface não reconhecida: {e}")
        return None

    incluidos = [r for r in resultados if r["incluir"]]
    com_quadro = [r for r in incluidos if not r["sem_quadro"]]
    sem_quadro = [r for r in incluidos if r["sem_quadro"]]
    falhas = [r for r in resultados if r["erro"]]
    fora = [r for r in resultados if not r["incluir"] and not r["erro"]]
    abertura = [r for r in incluidos if r["abertura"]]
    abertura_sem_desc = [r for r in abertura if r["objeto_fonte"] != ID_DESCRICAO_OBJETO]

    print()
    caminho_xlsx = exportar_xlsx(incluidos, os.path.join(pasta_saida, NOME_PLANILHA.format(dia=alvo)))
    if NOME_JSON:
        exportar_json(resultados, dia, os.path.join(pasta_saida, NOME_JSON), caixas)

    print(f"\n{'=' * 60}")
    print("RESUMO")
    rotulo_quadro = "com quadro, etapa atual < 24h:" if exigir_24h else "com quadro, etapa atual:"
    print(f"  {f'Emitidos em {dia} (únicos):':<37}{len(resultados)}")
    print(f"  Exportados:                          {len(incluidos)}")
    print(f"    {rotulo_quadro:<35}{len(com_quadro)}")
    print(f"    sem quadro, fase de elaboração:    {len(sem_quadro)}")
    if OBJETO_DA_ABERTURA and abertura:
        print(f"    de abertura, objeto do campo:      "
              f"{len(abertura) - len(abertura_sem_desc)}/{len(abertura)}")
        if abertura_sem_desc:
            print(f"      ⚠ campo vazio ou ausente, ficou o título: "
                  f"{', '.join(r['numero'] for r in abertura_sem_desc)}")
    print(f"  Não exportados:                      {len(fora)}")
    # Agrupa pelo motivo sem o detalhe (o que vem depois de ":" ou "(")
    for motivo, qtd in Counter(re.split(r"[:(]", r["motivo"])[0].strip() for r in fora).most_common():
        print(f"    {motivo}: {qtd}")
    # Proc. Administrativo sem quadro que ficou de fora pela etiqueta: número e
    # etiqueta lida, para calibrar ETIQUETAS_ELABORACAO se for o caso
    for r in fora:
        if r["motivo"].startswith("sem quadro — etiqueta"):
            print(f"      · {r['numero']}  [{r['etiqueta'] or 'sem etiqueta'}]")
    sem_atual = [r["numero"] for r in fora if r["motivo"].startswith("quadro sem etapa atual")]
    if sem_atual:
        print(f"      quadro sem etapa atual: {', '.join(sem_atual)}")
    if falhas:
        print(f"  ✗ Falha ao ler o detalhe:            {len(falhas)} → "
              f"{', '.join(r['numero'] for r in falhas)}")
    if caixas_com_falha:
        print(f"  ⚠ Caixas NÃO lidas (resultado incompleto): {', '.join(caixas_com_falha)}")
    print(f"  Requisições: {CONTAGEM['listagem']} de listagem + "
          f"{CONTAGEM['detalhe']} de detalhe + {CONTAGEM['outras']} de preparo")
    print(f"  Tempo total: {time.time() - t0:.1f}s")
    print(f"{'=' * 60}")

    if abrir and caminho_xlsx:
        abrir_arquivo(caminho_xlsx)
    return caminho_xlsx


def main(argv=None):
    """Linha de comando: python -m ferramentas1doc.processos_do_dia [DIA]"""
    argv = sys.argv[1:] if argv is None else argv
    hoje = date.today()
    if argv:
        txt = argv[0].strip().lower()
        alvo = (hoje if txt in ("", "hoje") else hoje - timedelta(days=1) if txt == "ontem"
                else interpretar_data(txt, hoje))
        if alvo is None:
            raise SystemExit(f"✗ Data não reconhecida: {argv[0]!r}")
    else:
        alvo = pedir_data(hoje)
    cred = sessao_1doc.obter_sessao()
    return executar(alvo, cred, hoje=hoje)


if __name__ == "__main__":
    main()
