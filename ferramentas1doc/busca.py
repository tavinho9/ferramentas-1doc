#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
busca.py (antigo busca_1doc.py) — busca de processos no 1Doc (Prefeitura de Montes Claros) por
etapa, fase, datas e momento de mudança de etapa.

SOMENTE LEITURA. O script só consulta: não despacha, não encaminha, não arquiva,
não marca, não cria prazo. Reproduz as mesmas requisições de consulta da
interface (listagem das caixas, abertura do processo, localizar por número).
Única alteração de estado possível: se a sessão estiver no "Inbox pessoal", troca
para o "Inbox de <setor>" (o mesmo link do menu), porque a caixa segue o inbox ativo.

Como funciona
  1. Sessão — reaproveita os cookies salvos da última execução. Se expiraram,
     abre o navegador (Chrome/Edge) e espera você fazer o login nele; então
     captura os cookies. Usuário e senha nunca passam pelo programa
     (ferramentas1doc.sessao).
  2. Listagem — lê as caixas do seu setor (Em aberto, Saída, Arquivados,
     Favoritos) via link.php?chamada=caixa, ~10 processos por requisição.
  3. Detalhe — abre cada processo que passou nos filtros baratos e lê o quadro
     de etapas (kanban) e a linha do tempo de despachos.
  4. Mudança de etapa — fonte primária: os eventos "Mudança de etapa" da linha do
     tempo (.marcou_fast-forward: "mudou da etapa X" + "para Y", com segundos,
     autor e setor, nome da etapa sem truncamento). Sem esses eventos, recorre ao
     rótulo dos despachos "via etapa" (.despacho_tit, ícone ⏩, truncado). A
     entrada na etapa atual é conferida contra o "Está aqui N …" do quadro; se
     divergir além da granularidade do rótulo, prevalece o quadro.
  5. Novo processo — os eventos "emitiu o Proc. Administrativo N/AAAA a partir deste
     documento" da linha do tempo dizem que processos saíram deste e quando. Como a emissão
     nem sempre é a normal, contam também as menções entre os processos (este cita o novo, ou
     o novo cita este) e a origem no cabeçalho do processo novo; com o filtro de novo
     processo, a busca lê os processos da licitação de todas as caixas para achá-las.
  6. Filtros e saída — .xlsx com abas Resultado, Transições, Novos processos e
     Parâmetros (CSV opcional) e um resumo no terminal.

Dependências:  pip install websocket-client requests beautifulsoup4 openpyxl
               (lxml é opcional e acelera a leitura das páginas)

Exemplos: python -m ferramentas1doc.busca --help
(no programa, a aba "Busca" monta estes mesmos argumentos)
"""
from __future__ import annotations

import argparse
import copy
import csv
import json
import os
import re
import sys
import threading
import time
import unicodedata
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import date, datetime, timedelta, timezone
from pathlib import Path
from urllib.parse import parse_qs, urlparse

try:
    import requests
    from requests.adapters import HTTPAdapter
    from urllib3.util.retry import Retry
    from bs4 import BeautifulSoup, Comment, NavigableString
except ImportError as _e:  # pragma: no cover
    sys.exit(f"Dependência ausente ({_e.name}). Instale: pip install websocket-client requests beautifulsoup4 openpyxl")

import importlib.util

PARSER = "lxml" if importlib.util.find_spec("lxml") else "html.parser"

from . import caminhos, config, sessao as sessao_1doc

VERSAO = "1.1"  # mudou o formato do detalhe: cache de versão anterior é descartado
BASE_PADRAO = "https://montesclaros.1doc.com.br/"
TZ_BR = timezone(timedelta(hours=-3))  # Brasília, sem horário de verão desde 2019
CAIXAS = {"entrada": "Em aberto", "saida": "Caixa de saída", "arquivo": "Arquivados", "favoritos": "Favoritos"}
PRIORIDADES = ("Urgente", "Alta", "Média", "Media", "Baixa")
ORDEM_PRIORIDADE = {"urgente": 0, "alta": 1, "media": 2, "baixa": 3}
SEM_QUADRO = " (sem quadro de etapas padrão)"
GUARDAR_DESPACHOS = 10          # quantos dos últimos despachos ficam no cache (para --cita-setor)
RE_LOGIN = re.compile(r"""name=["']?senha|formLogin|login\.php""", re.I)
RE_LINK = re.compile(r"^(.*?)\s*([\d.]+/\d{4})\s+(\d{2}/\d{2}/\d{4})\s+(\d{2}:\d{2})")
RE_DH = re.compile(r"(\d{2})/(\d{2})/(\d{4})(?:\s+(\d{2}):(\d{2})(?::(\d{2}))?)?")
RE_NUM_LIVRE = re.compile(r"(?<![\d/.])(\d{1,3}(?:\.\d{3})+|\d{1,6})\s*/\s*(20\d{2})(?!\d)")
RE_ETIQUETA = re.compile(r"Procedimento Licitat|Fase Preparat[óo]ria|Elabora[çc][ãa]o|Abertura|Aditivo|"
                         r"Documenta[çc][ãa]o Preparat", re.I)
RE_PAGINAS = re.compile(r"P[áa]gina\s+(\d+)\s*/\s*(\d+)")
RE_TOTAL_CAIXA = re.compile(r"""num_tab_(\w+)['"]\)\.html\(['"]\((\d[\d.]*)\)""")
RE_DERIVADO = re.compile(r"emitiu\s+o\s+(.+?)\s+([\d.]+/\d{4})\s+a partir deste documento", re.I)
# Menção a outro processo no texto (abertura e despachos): número com contexto de processo logo
# antes e sem contexto de lei, requisição, contrato etc. (a mesma regra do Faltantes/Verificar).
RE_CTX_PROCESSO = re.compile(r"(proc|abertura|emitid|gerad|autuad|instaurad|\bn[o]?\b|numero)")
RE_CTX_OUTRO_DOC = re.compile(r"(lei|decreto|portaria|requisi|\brc\b|contrato|\bata\b|edital|pregao|"
                              r"oficio|memorando|resolucao|instrucao|acordao|cnpj|cpf)")
# Link de despacho para documento que não é processo (o texto do link diz o tipo)
RE_LINK_OUTRO_DOC = re.compile(r"^(memorando|oficio|parecer|circular|despacho|decreto|portaria|anexo)")
MAX_CITADOS = 40
# De onde veio a citação, da mais confiável para a menos (ver _citados), e como aparece na planilha
FONTES_CITACAO = ("mencao", "link", "texto")
ROTULO_FONTE = {"mencao": "menção do 1Doc", "link": "link no texto", "texto": "número no texto"}
# Família da licitação, na ordem do fluxo: Elaboração de Documentação Preparatória → Fase
# Preparatória → Abertura de Procedimento Licitatório. Um processo citado só conta como novo se
# estiver adiante na família (ver aceita_mencao).
FAMILIA = ((2, re.compile(r"abertura de procedimento")), (1, re.compile(r"fase preparatori")),
           (0, re.compile(r"elaboracao de document\w* preparatori")))
# Como o novo processo foi achado (coluna "Novos processos (vínculo)" e aba "Novos processos")
V_EMISSAO, V_ORIGEM, V_CITADO, V_CITA = "emissão", "origem", "citado neste", "cita este"
VINCULOS_NORMAIS = (V_EMISSAO, V_ORIGEM)


class SessaoExpirada(Exception):
    pass


# =============================================================================
# Utilidades
# =============================================================================
def sem_acento(s: str) -> str:
    return "".join(c for c in unicodedata.normalize("NFD", s or "") if unicodedata.category(c) != "Mn")


def normtxt(s: str) -> str:
    """Comparação de textos: sem acento, sem caixa, espaços colapsados."""
    return re.sub(r"\s+", " ", sem_acento(s).casefold()).strip()


def norm_contexto(s: str) -> str:
    """Como o _norm do Faltantes: só letras e dígitos, minúsculos e sem acento, separados por um
    espaço ('nº' vira 'n', 'Proc.' vira 'proc'). Para o contexto antes de um número citado."""
    return " ".join(re.sub(r"[^a-z0-9]+", " ", sem_acento(s or "").lower()).split())


def normcod(s: str) -> str:
    """Comparação de códigos de setor e nomes: só alfanumérico maiúsculo.
    SPOT-DCL-GP-CPPFI, SPOT/DCL/GP/CPPFI e 'spot dcl gp cppfi' ficam iguais."""
    return re.sub(r"[^A-Z0-9]", "", sem_acento(s).upper())


def casa_setor(codigo: str, pedido: str) -> bool:
    """Código inteiro ('SPOT-DCL-GP-CPPFI') ou só o último nível ('CPPFI').
    Antes era substring: 'SPOT-DCL' casava também SPOT-DCL-GP, SPOT-DCL-GP-CPPFI, SPOT-DCL-GPAL…"""
    c, p = normcod(codigo), normcod(pedido)
    return bool(p) and (c == p or normcod(codigo.rsplit("-", 1)[-1]) == p)


def contem(valor: str, padroes) -> bool:
    v = normtxt(valor)
    return any(normtxt(p) in v for p in padroes)


def igual(valor: str, padroes) -> bool:
    v = normtxt(valor)
    return any(normtxt(p) == v for p in padroes)


def txt(el) -> str:
    return re.sub(r"\s+", " ", el.get_text(" ")).strip() if el is not None else ""


def primeiro_texto(el) -> str:
    """Primeiro nó de texto direto (o objeto vem antes do badge do assunto)."""
    if el is None:
        return ""
    for n in el.children:
        if isinstance(n, NavigableString) and not isinstance(n, Comment) and n.strip():
            return re.sub(r"\s+", " ", str(n)).strip()
    return ""


def texto_sem(el, seletores: str) -> str:
    """Primeira linha de texto de `el` depois de remover os elementos `seletores`."""
    if el is None:
        return ""
    c = copy.copy(el)  # cópia da subárvore (antes: re-parse do HTML inteiro do elemento a cada chamada)
    for x in c.select(seletores):
        x.decompose()
    linhas = [s.strip() for s in c.get_text("\n").split("\n") if s.strip()]
    return linhas[0][:60] if linhas else ""


def nome_pessoa(el) -> str:
    """Nome completo (span.pp[data-content]); o texto visível é abreviado ('Maria R.')."""
    if el is None:
        return ""
    pp = el.select_one("span.pp[data-content]")
    if pp and pp.get("data-content", "").strip():
        return re.sub(r"\s+", " ", pp["data-content"]).strip()
    return texto_sem(el, ".badge, i, .label")


def _ultimo_segmento(caminho: str) -> str:
    """'SPOT &raquo; SPOT-DCL &raquo; SPOT-DCL-GP-CPPFI' -> 'SPOT-DCL-GP-CPPFI'."""
    return re.split(r"\s*(?:»|&raquo;)\s*", caminho.strip())[-1].strip()


def codigos_badges(el) -> list[str]:
    """Códigos de setor dos badges, sem perder os escondidos.
    Na caixa, '+4' guarda os demais no title ('A » B / C » D'); 'SPOT-DCL-GPAL-CP...' guarda o
    caminho completo no title. No despacho, o texto é 'COD - Nome trunc...' e o title, o nome."""
    out: list[str] = []
    for b in el.select("span.badge") if el is not None else []:
        out += [c for c in codigos_badge(b) if c and c not in out]
    return out


def codigos_badge(b) -> list[str]:
    t = re.sub(r"\s+", " ", b.get_text(" ")).strip()
    tit = (b.get("data-original-title") or b.get("title") or "").strip()
    if re.fullmatch(r"\+\d+", t) and tit:
        return [_ultimo_segmento(p) for p in re.split(r"\s+/\s+", tit) if p.strip()]
    if re.search(r"(\.\.\.|…)$", t) and ("»" in tit or "&raquo;" in tit):
        return [_ultimo_segmento(tit)]
    return [re.split(r"\s+-\s+", t, maxsplit=1)[0].strip()]


def texto_e_atributos(el) -> str:
    """Texto + nomes completos (span.pp[data-content]) + caminho completo dos badges (title).
    Fica de fora a lista de CC (label 'CC' com data-content), para não ampliar o que conta como
    'citar o setor' sem decisão explícita."""
    if el is None:
        return ""
    extra = [p.get("data-content", "") for p in el.select("span.pp[data-content]")]
    extra += [b.get("data-original-title") or b.get("title") or "" for b in el.select(".badge")]
    return txt(el) + " " + " ".join(x for x in extra if x)


def dt_br(s) -> datetime | None:
    m = RE_DH.search(str(s or ""))
    if not m:
        return None
    d, mo, a, h, mi, se = m.groups()
    try:
        return datetime(int(a), int(mo), int(d), int(h or 0), int(mi or 0), int(se or 0))
    except ValueError:
        return None


def iso(dt: datetime | None) -> str:
    return dt.isoformat(timespec="seconds") if dt else ""


def de_iso(s) -> datetime | None:
    if not s:
        return None
    if isinstance(s, datetime):
        return s
    try:
        return datetime.fromisoformat(s)
    except ValueError:
        return dt_br(s)


def agora_br() -> datetime:
    return datetime.now(TZ_BR).replace(tzinfo=None, microsecond=0)


def data_cli(s: str, hoje: date, fim: bool = False) -> datetime:
    """DD/MM/AAAA, AAAA-MM-DD, 'hoje', 'ontem' ou -N (N dias atrás).
    `fim=True` devolve 23:59:59 do dia (limite superior inclusivo)."""
    t = s.strip().lower()
    if t == "hoje":
        d = hoje
    elif t == "ontem":
        d = hoje - timedelta(days=1)
    elif re.fullmatch(r"-\d+", t):
        d = hoje - timedelta(days=int(t[1:]))
    else:
        try:
            if re.fullmatch(r"\d{4}-\d{2}-\d{2}", t):
                d = date.fromisoformat(t)
            else:
                m = re.fullmatch(r"(\d{1,2})/(\d{1,2})/(\d{4})", t)
                if not m:
                    raise ValueError
                d = date(int(m[3]), int(m[2]), int(m[1]))
        except ValueError:      # formato errado ou dia/mês inexistente (31/02)
            raise argparse.ArgumentTypeError(f"data inválida: {s!r} (use DD/MM/AAAA, hoje, ontem ou -N)") from None
    return datetime.combine(d, datetime.max.time().replace(microsecond=0) if fim else datetime.min.time())


def norm_numero(s: str) -> str | None:
    m = re.fullmatch(r"\s*(\d{1,3}(?:\.\d{3})+|\d{1,6})\s*/\s*(\d{4})\s*", s or "")
    if not m:
        return None
    return f"{int(m[1].replace('.', '')):,}".replace(",", ".") + "/" + m[2]


def numeros_de_texto(texto: str) -> list[str]:
    vistos, out = set(), []
    for m in RE_NUM_LIVRE.finditer(texto or ""):
        n = norm_numero(f"{m[1]}/{m[2]}")
        if n and n not in vistos:
            vistos.add(n)
            out.append(n)
    return out


def numeros_de_arquivo(caminho: str) -> list[str]:
    p = Path(caminho)
    if p.suffix.lower() in (".xlsx", ".xlsm"):
        from openpyxl import load_workbook
        wb = load_workbook(p, read_only=True, data_only=True)
        partes = []
        for ws in wb.worksheets:
            for row in ws.iter_rows(values_only=True):
                partes.extend(str(v) for v in row if v is not None)
        return numeros_de_texto("\n".join(partes))
    return numeros_de_texto(p.read_text(encoding="utf-8", errors="replace"))


def permanencia(t: str) -> dict | None:
    """'Esta aqui 3 dias' -> {delta, tol (dias), texto}. Tolerância = granularidade do rótulo."""
    t = t or ""
    m = re.search(r"(\d+)\s*(minuto|hora|dia|semana|m[eê]s|meses|ano)", t, re.I)
    if not m:
        if re.search(r"segundo|instante|agora", t, re.I):
            return {"delta": timedelta(0), "tol": 1, "texto": t}
        return None
    n, u = int(m[1]), sem_acento(m[2].lower())
    if u.startswith("minuto"):
        return {"delta": timedelta(minutes=n), "tol": 1, "texto": t}
    if u.startswith("hora"):
        return {"delta": timedelta(hours=n), "tol": 1, "texto": t}
    if u.startswith("semana"):
        return {"delta": timedelta(weeks=n), "tol": 7, "texto": t}
    if u.startswith("mes"):
        return {"delta": timedelta(days=30 * n), "tol": 31, "texto": t}
    if u.startswith("ano"):
        return {"delta": timedelta(days=365 * n), "tol": 366, "texto": t}
    return {"delta": timedelta(days=n), "tol": 1, "texto": t}


# =============================================================================
# Sessão (login no navegador -> cookies -> requests)
# =============================================================================
class Sessao1Doc:
    """Sessão HTTP autenticada. Uma requests.Session por thread, com repetição automática."""

    def __init__(self, base: str, cookies: list[dict], user_agent: str, pausa: float = 0.15, timeout: int = 40):
        self.base = base if base.endswith("/") else base + "/"
        self.cookies, self.ua, self.pausa, self.timeout = cookies, user_agent, pausa, timeout
        self.id_setor = ""
        self._local = threading.local()

    def _s(self) -> requests.Session:
        s = getattr(self._local, "s", None)
        if s is None:
            s = requests.Session()
            s.headers.update({"User-Agent": self.ua or "Mozilla/5.0", "Accept-Language": "pt-BR,pt;q=0.9"})
            for c in self.cookies:
                s.cookies.set(c["name"], c["value"], domain=c.get("domain"), path=c.get("path", "/"))
            retry = Retry(total=3, backoff_factor=0.8, status_forcelist=(429, 500, 502, 503, 504),
                          allowed_methods=frozenset({"GET", "POST"}))
            s.mount("https://", HTTPAdapter(max_retries=retry))
            s.mount("http://", HTTPAdapter(max_retries=retry))
            self._local.s = s
        return s

    def _texto(self, r: requests.Response) -> str:
        r.raise_for_status()
        time.sleep(self.pausa)
        return r.content.decode("utf-8", errors="replace")

    def get(self, caminho: str, params: dict | None = None) -> str:
        sessao_1doc.checar_cancelamento()     # botão Cancelar da interface
        return self._texto(self._s().get(self.base + caminho, params=params, timeout=self.timeout))

    def post(self, caminho: str, dados: dict) -> str:
        sessao_1doc.checar_cancelamento()
        return self._texto(self._s().post(self.base + caminho, data=dados, timeout=self.timeout))


def verificar_sessao(sess: Sessao1Doc, _tentou_inbox: bool = False) -> str | None:
    """Devolve o nome do setor ativo se a sessão vale; None se caiu no login.
    Grava em `sess.id_setor` o id do setor (input#s_is), usado para saber se a etapa é do setor."""
    try:
        html = sess.get("", {"pg": "painel/listar"})
    except requests.RequestException:
        return None
    s = BeautifulSoup(html, PARSER)
    email = s.select_one("input#sis_usr_email")
    if not email or not (email.get("value") or "").strip():
        return None
    # A caixa servida por link.php segue o inbox ativo na sessão (setor ou pessoal). Se o último
    # uso foi o pessoal, a varredura leria só os processos da pessoa. A troca é a mesma do menu
    # "Inbox de <setor>": muda a visão, não mexe em processo nenhum.
    if s.select_one("li.li_pessoal.active") and not _tentou_inbox:
        print("  Inbox pessoal estava ativo; trocando para o inbox do setor.")
        try:
            sess.get("", {"pg": "painel/listar", "meu": "0", "trocar": "1"})
        except requests.RequestException:
            return None
        return verificar_sessao(sess, _tentou_inbox=True)
    if s.select_one("li.li_pessoal.active"):
        print("  Aviso: não foi possível ativar o inbox do setor; a varredura pode ser só do inbox pessoal.")
    fl = s.select_one("input#enableFacelift")
    if fl is not None and fl.has_attr("checked"):
        print("  Aviso: o 'novo visual da 1Doc' está ativo nesta conta. Os seletores foram calibrados no "
              "visual clássico; desative-o se a coleta vier vazia ou com lacunas.")
    is_ = s.select_one("input#s_is")
    sess.id_setor = (is_.get("value") or "").strip() if is_ else ""
    setor = s.select_one("input#s_s")
    return (setor.get("value") if setor else "") or "(setor não identificado)"


def abrir_sessao(args, cred=None) -> tuple[Sessao1Doc, str]:
    """Sessão salva, se ainda vale; senão o navegador abre e o programa espera o
    login (ferramentas1doc.sessao). `cred` = credenciais já obtidas pela interface."""
    if cred is None:
        minutos = max(1, round(args.timeout_login / 60)) if args.timeout_login else None
        cred = sessao_1doc.obter_sessao(forcar_login=args.relogar, navegador=args.navegador, minutos=minutos)
    sess = Sessao1Doc(sessao_1doc.BASE, cred.cookies, cred.ua, args.pausa)
    setor = verificar_sessao(sess)
    if not setor:
        sessao_1doc.esquecer()
        sys.exit("O 1Doc recusou a sessão nas requisições. Execute de novo: o navegador vai abrir para o login.")
    return sess, setor


# =============================================================================
# Cliente 1Doc (somente leitura)
# =============================================================================
class Cliente1Doc:
    def __init__(self, sess: Sessao1Doc):
        self.sess = sess
        self.base = sess.base

    def pagina_caixa(self, caixa: str, num: int, de: datetime | None = None, ate: datetime | None = None) -> str:
        p = {"chamada": "caixa", "pg": "painel/listar", "caixa": caixa, "num_pagina": str(num),
             "ordenar": "1", "ordem": "desc", "modo_visualizacao": "1", "emissoes_arquivado_prazos": "0"}
        if de:  # só "até": sem intervalo no servidor; o corte é feito linha a linha
            p["intervalo"] = f"{de:%d/%m/%Y} - {(ate or agora_br()):%d/%m/%Y}"
        return self.sess.get("link.php", p)

    def processo_por_hash(self, hash_: str, itd: str) -> str:
        return self.sess.get("", {"pg": "doc/ver", "itd": itd, "hash": hash_})

    def processo_por_numero(self, numero: str, itd: str) -> str:
        num, ano = numero.split("/")
        return self.sess.post("formEmissao.php?formAcessoRapido",
                              {"id_documento": itd, "id_assunto": "", "num_codigo": num, "ano": ano})


# =============================================================================
# Parser: listagem das caixas
# =============================================================================
def meta_listagem(soup, html: str, caixa: str) -> dict:
    """Total de páginas ('Página 1/21') e total de itens da caixa ("num_tab_entrada').html('(210)')").
    Servem para declarar varredura incompleta em vez de entregá-la calada."""
    m = RE_PAGINAS.search(soup.get_text(" "))
    total = None
    for c, n in RE_TOTAL_CAIXA.findall(html):
        if c == caixa:
            total = int(n.replace(".", ""))
    return {"pagina": int(m[1]) if m else None, "paginas": int(m[2]) if m else None, "total": total}


def parse_listagem(html: str, rotulo_caixa: str, itds: set[str] | None, base: str, soup=None) -> list[dict]:
    soup = soup if soup is not None else BeautifulSoup(html, PARSER)
    out = []
    for a in soup.select("a.link_emissao_a"):
        href = a.get("href", "")
        q = parse_qs(urlparse(href).query)
        itd = (q.get("itd") or [""])[0]
        hash_ = (q.get("hash") or [""])[0]
        if itds and itd not in itds:
            continue
        m = RE_LINK.search(txt(a))
        if not m:
            continue
        tr = a.find_parent("tr")
        if tr is None:
            continue
        tds = tr.find_all("td", recursive=False)
        td_para = tr.select_one("td.celula_para") or (tds[3] if len(tds) > 3 else None)
        resumo = tr.select_one(".resumo")
        td_ass = resumo.find_parent("td") if resumo else (tds[4] if len(tds) > 4 else None)
        tipo_el = td_ass.select_one('span[title]:not([title="Prioridade"]), span.badge') if td_ass else None
        rem = tr.select_one("div.td_remetente")
        setor_de = rem.select_one("span.badge") if rem else None
        prio = ""
        if td_ass:
            for s in td_ass.select("span.label"):
                if txt(s) in PRIORIDADES:
                    prio = txt(s)
                    break
        ult = tr.select_one('small[title^="Última movimentação"], small[title^="Ultima movimentação"]')
        out.append({
            "numero": m[2], "tipo_doc": m[1].strip(), "itd": itd, "hash": hash_,
            "emissao": iso(dt_br(f"{m[3]} {m[4]}")),
            "assunto": primeiro_texto(td_ass),
            "tipo": (tipo_el.get("data-original-title") or tipo_el.get("title") or txt(tipo_el)) if tipo_el else "",
            "prioridade": prio,
            "de": nome_pessoa(rem) if rem else "",
            "setor_de": txt(setor_de),
            # Coluna "Para" da caixa = todos os setores envolvidos (o '+N' esconde o resto no title),
            # não o setor que está com o processo agora.
            "para": " | ".join(codigos_badges(td_para)) if td_para else "",
            "ult_mov": iso(dt_br(ult.get("title"))) if ult else "",
            "resumo": txt(resumo)[:160],
            "caixa": rotulo_caixa,
            "url": f"{base}?pg=doc/ver&hash={hash_}&itd={itd}" if hash_ else "",
        })
    return out


# =============================================================================
# Parser: página do processo
# =============================================================================
def _ids_setor(v) -> list[str]:
    """data-restrito_para_id_setor=["285","286","287"] (atributo sem aspas externas)."""
    return re.findall(r"\d+", v if isinstance(v, str) else " ".join(v or []))


def _kanban(soup) -> list[dict]:
    blocos = [{
        "titulo": txt(b.select_one(".kanban_titulo")),
        "itens": txt(b.select_one(".kanban_itens")),
        "atual": b.get("data-current") == "true",
        "setores": [txt(s) for s in b.select(".kanban_footer .badge_env")],
        "ids_setor": _ids_setor(b.get("data-restrito_para_id_setor")),
    } for b in soup.select(".kanban_estagios")]
    # O 1Doc renderiza o quadro duas vezes (a 2ª cópia no formulário de despacho).
    # Desduplica pelos TÍTULOS; a 1ª cópia traz a permanência ("Está aqui N dias").
    t = [b["titulo"] for b in blocos]
    meia = len(t) // 2
    if len(t) >= 2 and len(t) % 2 == 0 and t[:meia] == t[meia:]:
        blocos = [{"titulo": a["titulo"], "itens": a["itens"] or o["itens"], "atual": a["atual"] or o["atual"],
                   "setores": a["setores"] or o["setores"], "ids_setor": a["ids_setor"] or o["ids_setor"]}
                  for a, o in zip(blocos[:meia], blocos[meia:])]
    return blocos


def _data_tc(el) -> datetime | None:
    """Carimbo unix do contêiner .timeline_conteudo (precisão de segundos)."""
    tc = el if el is not None and "timeline_conteudo" in (el.get("class") or []) else \
        (el.find_parent(class_="timeline_conteudo") if el is not None else None)
    v = (tc.get("data-datamk") or "") if tc is not None else ""
    return datetime.fromtimestamp(int(v), TZ_BR).replace(tzinfo=None) if v.isdigit() else None


def _eventos(soup) -> tuple[list[dict], list[str], list[dict]]:
    """Linha do tempo, notificações de sistema (.desp_notificacao .marcou):
      - 'Mudança de etapa' (.marcou_fast-forward): dois itens com o mesmo carimbo,
        'mudou da etapa <strong>X</strong>.' e 'para <strong>Y</strong>.'
      - 'emitiu o Proc. Administrativo N/AAAA a partir deste documento' (processo derivado).
    Devolve (mudanças [{data, de, para, autor, setor}], derivados ['N/AAAA', ...],
    emissões [{numero, tipo, data, autor, setor, hash}] dos derivados, na ordem da página)."""
    brutos, derivados, emissoes = [], [], []
    for i, m in enumerate(soup.select(".desp_notificacao .marcou")):
        quem = txt(m.select_one(".marcou_quem"))
        md = RE_DERIVADO.search(quem)
        if md:
            n = norm_numero(md[2])
            if n and n not in derivados:
                derivados.append(n)
                a = m.select_one(".marcou_quem a[href]")
                h = (parse_qs(urlparse(a["href"]).query).get("hash") or [""])[0] if a else ""
                emissoes.append({"numero": n, "tipo": md[1].strip(), "data": iso(_data_hora_evento(m)),
                                 "autor": txt(m.select_one(".marcou_nome")),
                                 "setor": txt(m.select_one(".badge_env")), "hash": h.upper()})
        if "marcou_fast-forward" not in (m.get("class") or []):
            continue
        forte = txt(m.select_one(".marcou_quem strong")) or re.sub(r"^(mudou da etapa|para)\s*|\s*\.$", "", quem)
        dt = _data_hora_evento(m)
        bd = m.select_one(".badge_env")
        brutos.append({"_i": i, "data": dt, "tipo": "de" if re.match(r"mudou da etapa", quem, re.I) else
                       "para" if re.match(r"para\b", quem, re.I) else "?", "etapa": forte,
                       "autor": txt(m.select_one(".marcou_nome")), "setor": txt(bd)})
    brutos.sort(key=lambda e: (e["data"] or datetime.min, e["_i"]))
    mud, pend = [], None
    for e in brutos:
        if e["tipo"] == "de":
            if pend:  # 'de' sem 'para' correspondente: registra com destino vazio
                mud.append({**pend, "para": ""})
            pend = {"data": e["data"], "de": e["etapa"], "autor": e["autor"], "setor": e["setor"]}
        elif e["tipo"] == "para":
            if pend and pend["data"] == e["data"]:
                mud.append({**pend, "para": e["etapa"]})
            else:
                if pend:
                    mud.append({**pend, "para": ""})
                mud.append({"data": e["data"], "de": "", "para": e["etapa"], "autor": e["autor"], "setor": e["setor"]})
            pend = None
    if pend:
        mud.append({**pend, "para": ""})
    return mud, derivados, emissoes


def _data_hora_evento(m) -> datetime | None:
    """Carimbo do contêiner da linha do tempo; sem ele, a data escrita no evento."""
    return _data_tc(m) or dt_br(txt(m.select_one(".despacho_data")))


def _despachos(soup) -> list[dict]:
    out = []
    for i, tb in enumerate(soup.select("table.despacho")):
        th = tb.find("th") or tb
        d_el = th.select_one('.despacho_data[title^="Em "]') or th.select_one(".despacho_data")
        dt = dt_br(d_el.get("title") or txt(d_el)) if d_el else None
        if dt is None:  # recurso: carimbo unix do contêiner da linha do tempo
            dt = _data_tc(tb)
        tit = th.select_one(".despacho_tit")
        rotulo = ""
        if tit:
            ff = tit.select_one(".icon-fast-forward")
            rotulo = txt(ff.parent) if ff else txt(tit)
        forte = th.find("strong")
        mnum = re.search(r"Despacho\s+(\d+)", txt(forte))
        rf_de, rf_para = tb.select_one(".rf_de"), tb.select_one(".rf_para")
        setor_de = rf_de.select_one(".badge_env") if rf_de else None
        # Destinatário: badge 'SPOT-DP-GOC - Ge...' + ícone de status (title 'Em aberto' etc.).
        # Resposta sem destinatário próprio mostra só 'Envolvidos internos acompanhando'.
        paras = []
        for b in rf_para.select(".badge_env") if rf_para else []:
            cod = (codigos_badge(b) or [txt(b)])[0]
            st = b.select_one("i[title]")
            paras.append(cod + (f" ({st['title']})" if st and st.get("title") else ""))
        corpo = tb.select_one(".texto_original") or tb.select_one("td.celula_despacho")
        lab = th.select_one(".despacho_data .label")
        out.append({
            "_ordem": i, "data": iso(dt), "n": mnum[1] if mnum else "",
            "situacao": txt(lab), "rotulo_etapa": rotulo,
            "autor": nome_pessoa(rf_de) if rf_de else "",
            "setor_de": txt(setor_de),
            "para": " | ".join(paras) if paras else ("(envolvidos internos)" if rf_para else ""),
            "texto": txt(corpo)[:400],
            # Inclui title/data-content: nome completo do autor e caminho completo de badge truncado
            "bruto": normcod(texto_e_atributos(tb))[:6000],
        })
    out.sort(key=lambda d: (d["data"] or "", d["_ordem"]))
    for d in out:
        d.pop("_ordem", None)
    return out


def _citados(soup, proprio_numero: str | None, proprio_hash: str) -> list[dict]:
    """Outros processos mencionados neste, de três fontes, da mais confiável para a menos:
      - "mencao": a menção do 1Doc, escolhida na lista ao escrever o despacho. Vem como
        <a data-mention="2"> com o hash no href e "Proc. Administrativo N/AAAA - …" no texto,
        tanto no corpo do despacho (class "mention label mention_2") quanto na lista de
        documentos mencionados (class "pre_visualizar"). Lida em qualquer lugar da página;
      - "link": outro link para documento dentro do texto de abertura e dos despachos
        (blocos .texto_original), com o número quando o texto do link o traz;
      - "texto": número NN.NNN/AAAA digitado nesses blocos, com contexto de processo antes
        ("processo", "abertura", "nº"…) e sem contexto de lei, requisição, contrato etc.
    Não filtra por ano: o mesmo registro serve para os dois sentidos (este cita o novo; o novo
    cita este), e a data é conferida no uso (aceita_mencao).
    Devolve [{numero, hash, fonte}], um por processo (com a fonte mais confiável), no máximo
    MAX_CITADOS."""
    out = []

    def add(numero, h, fonte):
        if (numero and numero == proprio_numero) or (h and h == proprio_hash):
            return
        for c in out:  # link e número do mesmo processo viram um registro só
            if (numero and c["numero"] == numero) or (h and c["hash"] == h):
                c["numero"], c["hash"] = c["numero"] or numero, c["hash"] or h
                if forca_fonte(fonte) < forca_fonte(c["fonte"]):
                    c["fonte"] = fonte
                return
        if len(out) < MAX_CITADOS:
            out.append({"numero": numero, "hash": h, "fonte": fonte})

    def de_link(a, fonte):
        h = ((parse_qs(urlparse(a.get("href", "")).query).get("hash") or [""])[0]).upper()
        nums = numeros_de_texto(txt(a))
        if (h or nums) and not RE_LINK_OUTRO_DOC.match(normtxt(txt(a))):
            add(nums[0] if nums else "", h, fonte)

    for a in soup.select('[data-mention="2"], .mention_2'):
        de_link(a, "mencao")
    for bloco in soup.find_all(class_=lambda c: bool(c) and "texto_original" in str(c).split()):
        for a in bloco.select("a[href]"):
            if (parse_qs(urlparse(a["href"]).query).get("hash") or [""])[0]:
                de_link(a, "link")
        texto = bloco.get_text(" ")
        for m in RE_NUM_LIVRE.finditer(texto):
            n = norm_numero(f"{m[1]}/{m[2]}")
            antes = norm_contexto(texto[max(0, m.start() - 45):m.start()])
            if n and RE_CTX_PROCESSO.search(antes) and not RE_CTX_OUTRO_DOC.search(antes[-30:]):
                add(n, "", "texto")
    return out


def forca_fonte(fonte: str) -> int:
    """Posição em FONTES_CITACAO (menor = mais confiável); fonte desconhecida fica por último."""
    return FONTES_CITACAO.index(fonte) if fonte in FONTES_CITACAO else len(FONTES_CITACAO)


def resolver_etapa(rotulo: str, titulos: list[str]) -> str:
    """O rótulo do despacho vem truncado ('Validação de ...'): casa por prefixo com o quadro."""
    r = (rotulo or "").strip()
    base = normtxt(re.sub(r"(\.\.\.|…)\s*$", "", r)).strip()
    if not base:
        return r
    for t in titulos:
        if normtxt(t) == base:
            return t
    cands = [t for t in titulos if normtxt(t).startswith(base)]
    return cands[0] if len(cands) == 1 else r


def _transicoes(desp: list[dict], titulos: list[str], eventos: list[dict] | None = None,
                emissao: datetime | None = None) -> list[dict]:
    """Histórico de etapas. Com eventos 'Mudança de etapa', eles são a fonte (nomes inteiros,
    segundos); o despacho 'via etapa' do mesmo instante só complementa nº e destinatário.
    Sem eventos (processos antigos, outro quadro), vale o rótulo dos despachos."""
    if eventos:
        return _transicoes_eventos(desp, titulos, eventos, emissao)
    return _transicoes_rotulo(desp, titulos)


def _transicoes_eventos(desp, titulos, eventos, emissao) -> list[dict]:
    out = []
    primeiro = eventos[0]
    if primeiro["de"] and emissao:  # etapa inicial: entrou na emissão (não há evento para isso)
        out.append({"data": iso(emissao), "de": "", "para": resolver_etapa(primeiro["de"], titulos), "rotulo": "",
                    "despacho": "", "autor": "", "setor_de": "", "destinatario": "", "mesma_etapa": False,
                    "estimada": False, "fonte": "emissão (etapa inicial)"})
    for e in eventos:
        para = resolver_etapa(e["para"], titulos) if e["para"] else ""
        de = resolver_etapa(e["de"], titulos) if e["de"] else ""
        # despacho 'via etapa' gravado no mesmo instante (até 2 min) e com rótulo da etapa de destino
        par = None
        for d in desp:
            dd = de_iso(d["data"])
            if dd and e["data"] and abs((dd - e["data"]).total_seconds()) <= 120 and d["rotulo_etapa"] \
                    and normtxt(resolver_etapa(d["rotulo_etapa"], titulos)) == normtxt(para):
                par = d
                break
        out.append({"data": iso(e["data"]), "de": de, "para": para, "rotulo": par["rotulo_etapa"] if par else "",
                    "despacho": par["n"] if par else "", "autor": e["autor"] or (par or {}).get("autor", ""),
                    "setor_de": e["setor"] or (par or {}).get("setor_de", ""),
                    "destinatario": (par or {}).get("para", ""),
                    "mesma_etapa": bool(de) and normtxt(de) == normtxt(para), "estimada": False,
                    "fonte": "evento"})
    return out


def _transicoes_rotulo(desp: list[dict], titulos: list[str]) -> list[dict]:
    out, anterior = [], ""
    for d in desp:
        if not d["rotulo_etapa"]:
            continue
        destino = resolver_etapa(d["rotulo_etapa"], titulos)
        out.append({"data": d["data"], "de": anterior, "para": destino, "rotulo": d["rotulo_etapa"],
                    "despacho": d["n"], "autor": d["autor"], "setor_de": d["setor_de"], "destinatario": d["para"],
                    "mesma_etapa": bool(anterior) and normtxt(anterior) == normtxt(destino), "estimada": False,
                    "fonte": "despacho"})
        anterior = destino
    return out


def _entrada_etapa(etapa, atipico, titulos, trans, perm, emissao, agora):
    """Data de entrada na etapa atual -> (datetime|None, fonte, observação).
    No atípico 'Saiu – Próxima' (só 'Ficou aqui' no quadro, típico da Etapa Final, que não mostra
    'Está aqui'), a etapa nomeada continua a da regra calibrada; a data é a da entrada na 'Próxima'."""
    saiu = (atipico or "").startswith("sem etapa atual") and " – " in (etapa or "")
    alvo = normtxt(etapa.split(" – ", 1)[1] if saiu else (etapa or ""))
    est = (agora - perm["delta"]) if perm else None

    def coerente(dt):
        if not perm or dt is None:
            return True
        return abs((agora.date() - dt.date()).days - (agora.date() - est.date()).days) <= perm["tol"]

    cand, fonte_cand = None, ""
    if etapa and (not atipico or saiu) and trans and normtxt(trans[-1]["para"]) == alvo:
        i = len(trans) - 1
        while i > 0 and normtxt(trans[i - 1]["para"]) == alvo:  # reenvio para a mesma etapa: vale o 1º
            i -= 1
        cand, fonte_cand = de_iso(trans[i]["data"]), trans[i].get("fonte", "despacho")
    if cand:
        if coerente(cand):
            return cand, fonte_cand, ""
        return est, "quadro (estimada)", (f"último registro de entrada na etapa ({cand:%d/%m/%Y %H:%M}, "
                                          f"{fonte_cand}) diverge do quadro ('{perm['texto']}'); prevaleceu o quadro")
    if (etapa and not atipico and not trans and titulos and normtxt(titulos[0]) == alvo
            and emissao and coerente(emissao)):
        return emissao, "emissão (etapa inicial)", ""
    if est:
        tol = f" (±{perm['tol']} dia{'s' if perm['tol'] > 1 else ''})"
        return est, "quadro (estimada)", f"sem despacho com rótulo desta etapa; estimada por '{perm['texto']}'{tol}"
    return None, "", ""


def sessao_caiu(html: str, soup) -> bool:
    email = soup.select_one("input#sis_usr_email")
    if email is not None and (email.get("value") or "").strip():
        return False
    return bool(RE_LOGIN.search(html[:20000]))


def parse_detalhe(html: str, agora: datetime) -> dict:
    soup = BeautifulSoup(html, PARSER)
    if sessao_caiu(html, soup):
        raise SessaoExpirada()
    numero = txt(soup.select_one(".emission_infos_item_atual .nd_num")) or None
    tipo_doc = txt(soup.select_one(".emission_infos_item_atual .nd"))
    pai = soup.select_one(".emission_infos_item_from small")
    tit = soup.select_one("h4.tit_assunto")
    tipo_el = tit.select_one("span[title]") if tit else None
    sit = soup.select("small.situacao_geral")
    prio_el = soup.select_one('.emission_infos_item_atual [title="Prioridade"]')

    iq = soup.select_one("a.ir_quadro")
    quadro = re.sub(r"^\s*Ir ao quadro:?\s*", "", txt(iq), flags=re.I) if iq else ""
    if not quadro:
        mq = re.search(r"Ir ao quadro:\s*([^\n]{5,90})", soup.get_text("\n"))
        quadro = mq[1].strip() if mq else ""
    etiqueta = next((txt(b) for b in soup.select(".badge") if RE_ETIQUETA.search(txt(b))), "")
    tipo = re.sub(r"^Assunto:\s*", "", tipo_el.get("title", ""), flags=re.I) if tipo_el else etiqueta

    # ---- ETAPA (regras calibradas da skill etapas-processos-1doc; não altere sem recalibrar) ----
    bl = _kanban(soup)
    titulos = [b["titulo"] for b in bl]
    etapa, desde, setor_etapa, atipico = None, "", "", ""
    for b in bl:
        if re.match(r"^Est[áa] aqui", b["itens"], re.I):
            etapa, desde, setor_etapa = b["titulo"], b["itens"], " | ".join(b["setores"])
            break
    if not etapa and bl:
        i_f = max((i for i, b in enumerate(bl) if re.match(r"^Ficou aqui", b["itens"], re.I)), default=-1)
        if i_f >= 0:
            prox = bl[i_f + 1] if i_f + 1 < len(bl) else None
            etapa = f"{bl[i_f]['titulo']} – {prox['titulo']}" if prox else bl[i_f]["titulo"]
            atipico = f"sem etapa atual; saiu de {bl[i_f]['titulo']}"
        else:
            for b in bl:
                if b["atual"]:
                    etapa, setor_etapa, atipico = b["titulo"], " | ".join(b["setores"]), "etapa só por data-current"
                    break
    if not etapa:
        etapa = etiqueta + SEM_QUADRO if etiqueta else None
        atipico = "sem etapa identificável" if bl else "sem quadro de etapas"

    # ---- FASE: discriminada pelo QUADRO (os dois quadros terminam em "Etapa Final") ----
    fonte = " | ".join(x for x in (quadro, etiqueta, tipo) if x)
    nomes = " | ".join(titulos)
    if re.search(r"Abertura de Procedimento Licitat", fonte, re.I):
        fase = "Abertura"
    elif re.search(r"Fase Preparat[óo]ria", fonte, re.I):
        fase = "Elaboração"
    elif re.search(r"Valida[çc][ãa]o de Requisi[çc][ãa]o|Consulta Or[çc]ament[áa]ria|Controle Interno|COMPAC",
                   nomes, re.I):
        fase = "Abertura"
    else:
        fase = "Elaboração"

    h_el = (soup.select_one("input#hash_emissao") or soup.select_one("input#movimenta_hash_pai")
            or soup.select_one("input#hash"))
    hash_ = (h_el.get("value") or "").strip() if h_el else ""

    # Emissão: o carimbo unix de #emissao_principal é o que existe no visual atual (div.horario sumiu)
    emissao = None
    ep = soup.select_one("#emissao_principal[data-datamk]")
    if ep and ep["data-datamk"].isdigit():
        emissao = datetime.fromtimestamp(int(ep["data-datamk"]), TZ_BR).replace(tzinfo=None)
    if emissao is None:
        horario = soup.select_one("div.horario span")
        emissao = dt_br(txt(horario)) if horario else None

    # Setores da etapa atual por id (data-restrito_para_id_setor), para cruzar com o setor logado
    bloco_atual = next((b for b in bl if normtxt(b["titulo"]) == normtxt((etapa or "").split(" – ")[-1])), None)
    ids_etapa = bloco_atual["ids_setor"] if bloco_atual else []
    if bloco_atual and not setor_etapa:
        setor_etapa = " | ".join(bloco_atual["setores"])

    # Situação do processo para o setor logado (rótulos ao lado do número: Recebido, Arquivado…)
    status_setor = " | ".join(dict.fromkeys(
        txt(x) for x in soup.select(".emission_infos_item_atual span.label")
        if x.get("title") != "Prioridade" and txt(x)))

    desp = _despachos(soup)
    eventos, derivados, emissoes = _eventos(soup)
    trans = _transicoes(desp, titulos, eventos, emissao)
    perm = permanencia(desde) if desde else None
    entrada, entrada_fonte, entrada_obs = _entrada_etapa(etapa, atipico, titulos, trans, perm, emissao, agora)
    # Transição sintética quando a entrada na etapa atual não veio de evento nem de despacho rotulado
    if entrada and entrada_fonte not in ("despacho", "evento") and etapa and not atipico:
        ultimo = trans[-1]["para"] if trans else ""
        if normtxt(ultimo) != normtxt(etapa) or entrada_obs.startswith("último registro"):
            trans.append({"data": iso(entrada), "de": "" if normtxt(ultimo) == normtxt(etapa) else ultimo,
                          "para": etapa, "rotulo": "", "despacho": "", "autor": "", "setor_de": "",
                          "destinatario": "", "mesma_etapa": False, "estimada": entrada_fonte != "emissão (etapa inicial)",
                          "fonte": entrada_fonte})

    return {
        "numero": numero, "hash": hash_, "tipo_doc": tipo_doc, "pai": txt(pai), "emissao": iso(emissao),
        "assunto": primeiro_texto(tit), "tipo": tipo,
        "situacao": txt(sit[1]) if len(sit) > 1 else txt(sit[0]) if sit else "",
        "prioridade": txt(prio_el), "quadro": quadro, "etapas_quadro": titulos,
        "etapa": etapa, "fase": fase, "setor_etapa": setor_etapa, "setor_etapa_ids": ids_etapa,
        "permanencia": re.sub(r"^Est[áa] aqui\s*", "", desde, flags=re.I), "atipico": atipico,
        "entrada": iso(entrada), "entrada_fonte": entrada_fonte, "entrada_obs": entrada_obs,
        "status_setor": status_setor, "derivados": derivados, "emissoes": emissoes,
        "citados": _citados(soup, norm_numero(numero or ""), hash_.upper()),
        "n_despachos": len(desp), "despachos": desp[-GUARDAR_DESPACHOS:], "transicoes": trans,
        "coletado_em": iso(agora), "versao": VERSAO,
    }


# =============================================================================
# Coleta (com cache)
# =============================================================================
def chave(numero: str, itd: str) -> str:
    return f"{itd}|{numero}"


def carregar_cache(caminho: str | None) -> dict:
    vazio = {"versao": VERSAO, "lista": {}, "det": {}, "posicao_lista": ""}
    if not caminho or not Path(caminho).exists():
        return vazio
    try:
        c = json.loads(Path(caminho).read_text(encoding="utf-8"))
    except ValueError:
        print(f"Aviso: cache {caminho} ilegível; ignorado.")
        return vazio
    if c.get("versao") != VERSAO:
        # Detalhe de outra versão do parser não tem os campos novos (eventos, derivados…):
        # reaproveitá-lo misturaria critérios. A listagem (só para hash e --offline) também cai.
        print(f"Aviso: cache de versão {c.get('versao') or '?'} (atual {VERSAO}); será refeito.")
        return vazio
    return {**vazio, **c}


def salvar_cache(caminho: str | None, cache: dict):
    if caminho:
        Path(caminho).write_text(json.dumps(cache, ensure_ascii=False), encoding="utf-8")


def listar(cli: Cliente1Doc, caixas: list[str], itds: set[str], de, ate, max_paginas: int
           ) -> tuple[dict, dict, list[str]]:
    """Devolve (lista, contagem por caixa, avisos de completude)."""
    lista, por_caixa, avisos = {}, {}, []
    for cx in caixas:
        rot, n, primeira, tamanho = CAIXAS.get(cx, cx), 0, None, None
        meta, lidos, ultima = {}, 0, 0
        for num in range(1, max_paginas + 1):
            html = cli.pagina_caixa(cx, num, de, ate)
            soup = BeautifulSoup(html, PARSER)  # um parse por página (antes eram dois)
            itens = parse_listagem(html, rot, itds, cli.base, soup)
            if num == 1 and not itens and RE_LOGIN.search(html[:20000]):
                raise SessaoExpirada()
            brutos = len(soup.select("a.link_emissao_a"))
            if num == 1:
                meta = meta_listagem(soup, html, cx)
                if brutos == 0 and (meta.get("total") or 0) > 0:
                    avisos.append(f"{rot}: a caixa informa {meta['total']} itens, mas nenhuma linha foi reconhecida "
                                  f"(layout mudou ou o 'novo visual' está ativo?)")
            if brutos == 0:
                break
            lidos, ultima = lidos + brutos, num
            if itens and itens[0]["numero"] == primeira:  # servidor repetiu a última página
                break
            primeira = itens[0]["numero"] if itens else primeira
            tamanho = tamanho or brutos
            for it in itens:
                em = de_iso(it["emissao"])
                if (de and em and em < de) or (ate and em and em > ate):
                    continue
                n += 1
                k = chave(it["numero"], it["itd"])
                if k in lista:
                    if rot not in lista[k]["caixa"]:
                        lista[k]["caixa"] += " | " + rot
                else:
                    lista[k] = it
            tot_pg = meta.get("paginas")
            print(f"\r  {rot}: página {num}{f'/{tot_pg}' if tot_pg else ''}, {n} processos", end="", flush=True)
            if (tot_pg and num >= tot_pg) or (not tot_pg and brutos < tamanho):
                break
        print()
        por_caixa[rot] = n
        # Completude: varredura que parou antes do fim é declarada, não entregue como se fosse o todo
        tot_pg, total = meta.get("paginas"), meta.get("total")
        if tot_pg and ultima < tot_pg:
            avisos.append(f"{rot}: lidas {ultima} de {tot_pg} páginas"
                          + (" (limite --max-paginas)" if ultima >= max_paginas else ""))
        elif total is not None and not de and lidos != total:
            avisos.append(f"{rot}: a caixa informa {total} itens e foram lidas {lidos} linhas "
                          f"(processos movimentados durante a leitura mudam de página)")
    return lista, por_caixa, avisos


def detalhar_um(cli: Cliente1Doc, numero: str | None, itd: str, item: dict | None, agora: datetime):
    """`numero=None`: processo citado só por link (hash); vale o número que a página mostrar."""
    h = (item or {}).get("hash")
    det = parse_detalhe(cli.processo_por_hash(h, itd) if h else cli.processo_por_numero(numero, itd), agora)
    if numero is None:
        numero = det["numero"]
        if not numero:
            return None, {"numero": f"link {h[:10]}…", "motivo": "página sem número de processo", "obtido": ""}
    if det["numero"] != numero and h:  # hash velho: tenta pelo número
        det = parse_detalhe(cli.processo_por_numero(numero, itd), agora)
    # Cinto de segurança: o número devolvido TEM de ser o pedido (barra processo filho
    # e busca que cai numa lista). Lacuna declarada é melhor que valor errado.
    if det["numero"] != numero:
        return None, {"numero": numero, "motivo": "número divergente", "obtido": det["numero"] or ""}
    if not det["etapa"]:
        return None, {"numero": numero, "motivo": "sem etapa nem rótulo utilizável", "obtido": ""}
    det["itd"] = itd
    det["url"] = f"{cli.base}?pg=doc/ver&hash={det['hash'] or h}&itd={itd}" if (det["hash"] or h) else ""
    for e in det["emissoes"]:
        e["url"] = f"{cli.base}?pg=doc/ver&hash={e['hash']}&itd={itd}" if e["hash"] else ""
    det["_ult_mov_lista"] = (item or {}).get("ult_mov", "")
    return det, None


def detalhar(cli, pedidos: list[tuple[str, str, dict | None]], agora, paralelo: int,
             dets: dict) -> list:
    """Preenche `dets` à medida que avança (se a sessão cair, o parcial não se perde)."""
    falhas, feitos = [], 0
    total = len(pedidos)

    def tarefa(p):
        numero, itd, item = p
        try:
            return p, *detalhar_um(cli, numero, itd, item, agora)
        except SessaoExpirada:
            raise
        except Exception as e:  # rede, HTML inesperado
            return p, None, {"numero": numero or f"link {(item or {}).get('hash', '')[:10]}…", "motivo": "exceção",
                             "obtido": str(e)[:120]}

    for rodada in (1, 2):  # falha é "refazer", não "não existe": uma segunda tentativa
        if not pedidos:
            break
        falhas_rodada = []
        with ThreadPoolExecutor(max_workers=max(1, paralelo)) as ex:
            futs = [ex.submit(tarefa, p) for p in pedidos]
            for f in as_completed(futs):
                try:
                    p, det, falha = f.result()
                except SessaoExpirada:
                    # sem isto, o 'with' esperaria todas as tarefas pendentes baterem no login
                    ex.shutdown(wait=False, cancel_futures=True)
                    raise
                if det:
                    dets[chave(det["numero"], p[1])] = det
                    feitos += 1
                else:
                    falhas_rodada.append((p, falha))
                print(f"\r  detalhando {feitos}/{total}" + (" (2ª tentativa)" if rodada == 2 else ""),
                      end="", flush=True)
        pedidos = [p for p, _ in falhas_rodada]
        falhas = [fl for _, fl in falhas_rodada]
    if total:
        print()
    return falhas


# =============================================================================
# Registro consolidado (listagem + detalhe) e filtros
# =============================================================================
def cita_alvo(despachos: list[dict], alvo: str, pessoas: list[str]) -> bool:
    a = normcod(alvo or "")
    for d in despachos:
        b = d.get("bruto", "")
        if a and a in b:
            return True
        if a:  # badge truncado ("SPOT-DCL-GP-CPPF..."): prefixo longo do alvo também vale
            for t in re.findall(r"[A-Z0-9\-/]{8,}(?=\.\.\.|…)", f"{d.get('setor_de', '')} {d.get('para', '')}", re.I):
                if a.startswith(normcod(t)):
                    return True
        if any(normcod(p) and normcod(p) in b for p in pessoas or []):
            return True
    return False


def posicao_etapa(etapa: str, titulos: list[str]):
    ts = [normtxt(t) for t in titulos]
    n = normtxt(etapa or "")
    if n in ts:
        return ts.index(n)
    partes = re.split(r"\s+[–-]\s+", etapa or "", maxsplit=1)  # atípico "Saiu – Próxima"
    if len(partes) == 2 and normtxt(partes[0]) in ts:
        return ts.index(normtxt(partes[0])) + 0.5
    return None


def indice_alvo(alvo: str, titulos: list[str]):
    for i, t in enumerate(titulos):
        if normtxt(alvo) == normtxt(t):
            return i
    for i, t in enumerate(titulos):
        if normtxt(alvo) in normtxt(t):
            return i
    return None


def eh_processo(tipo: str) -> bool:
    """Tipo de documento de uma emissão: 'Proc. Administrativo' conta como processo novo;
    memorando, ofício etc. emitidos a partir do processo, não."""
    return normtxt(tipo).startswith("proc")


def familia(texto: str) -> int | None:
    """Posição na família da licitação (0 Elaboração de Documentação Preparatória, 1 Fase
    Preparatória, 2 Abertura de Procedimento Licitatório) pelo quadro/assunto; None fora dela."""
    t = normtxt(texto)
    return next((n for n, rx in FAMILIA if rx.search(t)), None)


def familia_det(det: dict) -> int | None:
    return familia(f"{det.get('quadro', '')} | {det.get('tipo', '')}")


class Universo:
    """Processos já abertos (nesta busca, no índice das caixas ou no cache), para cruzar as
    menções nos dois sentidos e a origem registrada no cabeçalho do processo novo."""

    def __init__(self, dets):
        self.por_numero, self.por_hash = {}, {}
        self.citam, self.filhos = {}, {}
        for d in dets:
            n = norm_numero(d.get("numero") or "")
            if not n:
                continue
            velho = self.por_numero.get(n)
            if velho is not None and (velho.get("coletado_em") or "") > (d.get("coletado_em") or ""):
                continue
            self.por_numero[n] = d
        for n, d in self.por_numero.items():
            if d.get("hash"):
                self.por_hash[d["hash"].upper()] = n
            pai = norm_numero(d.get("pai") or "")
            if pai:
                self.filhos.setdefault(pai, []).append(n)
            for c in d.get("citados") or []:
                for k in {c.get("numero"), (c.get("hash") or "").upper()} - {"", None}:
                    quem = self.citam.setdefault(k, {})  # número de quem cita → fonte da citação
                    if n not in quem or forca_fonte(c.get("fonte", "")) < forca_fonte(quem[n]):
                        quem[n] = c.get("fonte", "")

    def achar(self, citado: dict) -> dict | None:
        """Pelo número; sem ele (ou sem processo aberto com ele), pelo hash do link."""
        d = self.por_numero.get(citado.get("numero") or "")
        if d is None and citado.get("hash"):
            d = self.por_numero.get(self.por_hash.get(citado["hash"].upper(), ""))
        return d


def aceita_mencao(det: dict, outro: dict) -> bool:
    """Um processo que cita este, ou que este cita, conta como o novo processo dele quando a
    emissão não foi a normal ('a partir deste documento'): é Proc. Administrativo, não tem outra
    origem registrada, foi emitido depois deste e está adiante na família da licitação
    (Elaboração → Fase Preparatória → Abertura). Assim ficam de fora o pagamento ou o contrato que
    citam o número da licitação, o processo semelhante citado como referência e o processo antigo."""
    n, n_outro = norm_numero(det.get("numero") or ""), norm_numero(outro.get("numero") or "")
    if not n_outro or n_outro == n or not eh_processo(outro.get("tipo_doc", "")):
        return False
    pai = norm_numero(outro.get("pai") or "")
    if pai and pai != n:
        return False
    em, em_outro = de_iso(det.get("emissao")), de_iso(outro.get("emissao"))
    if em and em_outro and em_outro < em:
        return False
    f, f_outro = familia_det(det), familia_det(outro)
    return f_outro is not None and (f is None or f_outro > f)


def novos_processos(det: dict, uni: Universo | None = None, mencoes: bool = True) -> list[dict]:
    """Novos processos deste, do mais antigo ao mais recente, cada um com os vínculos que o acharam:
      - emissão: evento 'emitiu o Proc. Administrativo N/AAAA a partir deste documento' (data do evento);
      - origem: o processo novo traz este no cabeçalho de origem, sem o evento aqui;
      - citado neste / cita este: emissão fora do normal, achada pela menção num sentido ou no
        outro (ver aceita_mencao); a data é a de emissão do processo novo.
    Origem e menções precisam do `uni`; `mencoes=False` fica só com emissão e origem.
    Detalhe gravado no cache antes da leitura das emissões só tem os números ('derivados'):
    entram sem data nem tipo."""
    if "emissoes" in det:
        novos = [{**e, "data": de_iso(e.get("data")), "vinculos": [V_EMISSAO]} for e in det["emissoes"] or []
                 if eh_processo(e.get("tipo", ""))]
    else:
        novos = [{"numero": n, "tipo": "", "data": None, "autor": "", "setor": "", "hash": "",
                  "vinculos": [V_EMISSAO]} for n in det.get("derivados") or []]
    por_num = {n["numero"]: n for n in novos}

    def add(outro, vinculo, fonte=""):
        n = norm_numero(outro.get("numero") or "")
        if n in por_num:
            # vínculo normal (emissão/origem) já basta; menções só se somam entre si
            x = por_num[n]
            if not any(v in VINCULOS_NORMAIS for v in x["vinculos"]):
                if vinculo not in x["vinculos"]:
                    x["vinculos"].append(vinculo)
                if fonte and fonte not in x["fontes"]:
                    x["fontes"] = sorted([*x["fontes"], fonte], key=forca_fonte)
            return
        por_num[n] = {"numero": n, "tipo": outro.get("tipo_doc", ""), "data": de_iso(outro.get("emissao")),
                      "autor": "", "setor": "", "hash": (outro.get("hash") or "").upper(),
                      "url": outro.get("url", ""), "vinculos": [vinculo], "fontes": [fonte] if fonte else []}
        novos.append(por_num[n])

    proprio = norm_numero(det.get("numero") or "")
    if uni is not None and proprio:
        for n in uni.filhos.get(proprio, []):
            outro = uni.por_numero[n]
            if n != proprio and eh_processo(outro.get("tipo_doc", "")):
                add(outro, V_ORIGEM)
        if mencoes:
            for c in det.get("citados") or []:
                outro = uni.achar(c)
                if outro is not None and aceita_mencao(det, outro):
                    add(outro, V_CITADO, c.get("fonte", ""))
            chaves = [proprio] + ([det["hash"].upper()] if det.get("hash") else [])
            for k in chaves:
                for n, fonte in uni.citam.get(k, {}).items():
                    outro = uni.por_numero[n]
                    if aceita_mencao(det, outro):
                        add(outro, V_CITA, fonte)
        for n in novos:  # etapa do novo processo, quando ele foi aberto
            outro = uni.por_numero.get(n["numero"])
            if outro is not None:
                n["etapa"] = outro.get("etapa", "")
                n["url"] = n.get("url") or outro.get("url", "")
    return sorted(novos, key=lambda n: n["data"] or datetime.min)


def texto_novos(novos: list[dict]) -> str:
    """'40.001/2026 (emissão), 39.144/2026 (cita este)' para a coluna do Resultado."""
    return ", ".join(f"{n['numero']} ({' + '.join(n['vinculos'])})" for n in novos)


def consolidar(item: dict | None, det: dict | None, agora: datetime, op, uni: Universo | None = None) -> dict:
    item, det = item or {}, det or {}
    desp = det.get("despachos") or []
    ult = desp[-1] if desp else {}
    trans = [{**t, "data": de_iso(t["data"])} for t in det.get("transicoes", [])]
    novos = novos_processos(det, uni, mencoes=not getattr(op, "sem_mencoes", False))
    r = {
        "numero": det.get("numero") or item.get("numero"),
        "itd": det.get("itd") or item.get("itd", ""),
        "tipo_doc": det.get("tipo_doc") or item.get("tipo_doc", ""),
        "url": item.get("url") or det.get("url", ""),
        "emissao": de_iso(item.get("emissao")) or de_iso(det.get("emissao")),
        # campos que o pré-filtro julga pela listagem: a listagem prevalece (mesmo critério nos dois estágios)
        "prioridade": item.get("prioridade") if item.get("numero") else det.get("prioridade", ""),
        "assunto": det.get("assunto") or item.get("assunto", ""),
        "tipo": det.get("tipo") or item.get("tipo", ""),
        "quadro": det.get("quadro", ""),
        "fase": det.get("fase", ""),
        "etapa": det.get("etapa", ""),
        "etapas_quadro": det.get("etapas_quadro", []),
        "setor_etapa": det.get("setor_etapa", ""),
        "permanencia": det.get("permanencia", ""),
        "entrada": de_iso(det.get("entrada")),
        "entrada_fonte": det.get("entrada_fonte", ""),
        "para": item.get("para") or ult.get("para", ""),
        "ult_mov": de_iso(item.get("ult_mov")) or de_iso(ult.get("data")),
        "ult_data": de_iso(ult.get("data")),
        "ult_de": " — ".join(x for x in (ult.get("autor", ""), ult.get("setor_de", "")) if x),
        "ult_para": ult.get("para", ""),
        "ult_texto": ult.get("texto", "")[:200],
        "situacao": det.get("situacao", ""),
        "status_setor": det.get("status_setor", ""),
        "pai": det.get("pai", ""),
        "derivados": ", ".join(det.get("derivados") or []),
        "novos": novos,
        "novos_texto": texto_novos(novos),
        # o mais recente; com o filtro de período, o mais recente dentro dele (ver montar_filtros)
        "novo_processo_em": max((n["data"] for n in novos if n["data"]), default=None),
        "caixa": item.get("caixa", ""),
        "transicoes": trans,
        "despachos": desp,
        "tem_detalhe": bool(det),
        "_obs": [x for x in (det.get("atipico"), det.get("entrada_obs")) if x],
    }
    # Etapa reservada ao setor logado? (data-restrito_para_id_setor contém o id do setor da sessão)
    id_setor = getattr(op, "_id_setor", "") or ""
    ids = det.get("setor_etapa_ids") or []
    r["etapa_meu_setor"] = (id_setor in ids) if (id_setor and det.get("etapas_quadro")) else None
    r["dias_etapa"] = (agora.date() - r["entrada"].date()).days if r["entrada"] else None
    r["dias_sem_mov"] = (agora.date() - r["ult_mov"].date()).days if r["ult_mov"] else None
    mud = [t for t in trans if not t["mesma_etapa"]]
    r["n_mudancas"] = len(mud)
    r["historico"] = " → ".join(
        f"{t['data']:%d/%m/%y %H:%M} {t['para']}{' (estim.)' if t.get('estimada') else ''}"
        for t in mud if t["data"])
    r["cita_alvo"] = None
    alvo = getattr(op, "cita_setor", None) or getattr(op, "excecao_cita_setor", None)
    pessoas = getattr(op, "pessoa", None) or []
    if alvo or pessoas:
        n = max(1, min(getattr(op, "ultimos_despachos", 1) or 1, GUARDAR_DESPACHOS))
        r["cita_alvo"] = cita_alvo(desp[-n:], alvo, pessoas) if desp else False
    return r


class Filtro:
    def __init__(self, descricao: str, grupo: str, fonte: str, pred, novo: bool = False):
        self.descricao, self.grupo, self.fonte, self.pred = descricao, grupo, fonte, pred
        self.novo = novo  # filtro de novo processo (a busca completa as menções antes de aplicá-lo)
        # grupo:  "posicao" (afastável pela --excecao-cita-setor) | "geral"
        # fonte:  "lista" (avaliável só com a listagem) | "detalhe"


def _lista(v):
    out = []
    for x in v or []:
        out.extend(p.strip() for p in str(x).split(",") if p.strip())
    return out


def montar_filtros(a) -> list[Filtro]:
    F = []
    exata = a.etapa_exata
    casa = igual if exata else contem
    if a.etapa:
        F.append(Filtro(f"etapa {'=' if exata else '∋'} {' OU '.join(a.etapa)}", "posicao", "detalhe",
                        lambda r: casa(r["etapa"], a.etapa)))
    if a.excluir_etapa:
        F.append(Filtro(f"etapa ∌ {' / '.join(a.excluir_etapa)}", "posicao", "detalhe",
                        lambda r: not casa(r["etapa"], a.excluir_etapa)))
    if a.fase:
        F.append(Filtro(f"fase = {a.fase}", "posicao", "detalhe", lambda r: igual(r["fase"], [a.fase])))
    if a.quadro:
        F.append(Filtro(f"quadro/assunto ∋ {' OU '.join(a.quadro)}", "posicao", "detalhe",
                        lambda r: contem(f"{r['quadro']} | {r['tipo']}", a.quadro)))
    if a.antes_de:
        def antes(r, alvo=a.antes_de):
            ia = indice_alvo(alvo, r["etapas_quadro"])
            if ia is None:  # quadro sem essa etapa: a fase preparatória precede a de abertura
                return r["fase"] == "Elaboração"
            p = posicao_etapa(r["etapa"], r["etapas_quadro"])
            return p is not None and p < ia
        F.append(Filtro(f"ainda não chegou a '{a.antes_de}' (quadros sem essa etapa: só fase Elaboração)",
                        "posicao", "detalhe", antes))
    if a.a_partir_de:
        def apartir(r, alvo=a.a_partir_de):
            ia = indice_alvo(alvo, r["etapas_quadro"])
            p = posicao_etapa(r["etapa"], r["etapas_quadro"])
            return ia is not None and p is not None and p >= ia
        F.append(Filtro(f"já chegou a '{a.a_partir_de}' (etapa atual igual ou posterior no quadro)",
                        "posicao", "detalhe", apartir))
    if a.passou_por:
        F.append(Filtro(f"passou por {' OU '.join(a.passou_por)}", "posicao", "detalhe",
                        lambda r: contem(r["etapa"], a.passou_por)
                        or any(contem(t["para"], a.passou_por) for t in r["transicoes"])))
    if a.nao_passou_por:
        F.append(Filtro(f"nunca passou por {' / '.join(a.nao_passou_por)}", "posicao", "detalhe",
                        lambda r: not contem(r["etapa"], a.nao_passou_por)
                        and not any(contem(t["para"], a.nao_passou_por) for t in r["transicoes"])))
    if a.setor_etapa:
        F.append(Filtro(f"setor da etapa ∋ {' OU '.join(a.setor_etapa)}", "geral", "detalhe",
                        lambda r: any(casa_setor(p, s) for s in a.setor_etapa for p in r["setor_etapa"].split(" | "))))
    if a.dias_etapa_min is not None:
        F.append(Filtro(f"dias na etapa ≥ {a.dias_etapa_min}", "geral", "detalhe",
                        lambda r: r["dias_etapa"] is not None and r["dias_etapa"] >= a.dias_etapa_min))
    if a.dias_etapa_max is not None:
        F.append(Filtro(f"dias na etapa ≤ {a.dias_etapa_max}", "geral", "detalhe",
                        lambda r: r["dias_etapa"] is not None and r["dias_etapa"] <= a.dias_etapa_max))
    if a.entrada_de or a.entrada_ate:
        de, ate = a.entrada_de, a.entrada_ate
        F.append(Filtro(f"entrou na etapa atual {_periodo(de, ate)}", "geral", "detalhe",
                        lambda r: r["entrada"] is not None and (not de or r["entrada"] >= de)
                        and (not ate or r["entrada"] <= ate)))
    if a.mudanca_de or a.mudanca_ate or a.mudanca_para or a.mudanca_origem:
        de, ate, para, orig = a.mudanca_de, a.mudanca_ate, a.mudanca_para, a.mudanca_origem
        desc = "mudou de etapa" + (f" {_periodo(de, ate)}" if de or ate else "")
        desc += f" para {' OU '.join(para)}" if para else ""
        desc += f" saindo de {' OU '.join(orig)}" if orig else ""

        def mudou(r):
            r["_trans_periodo"] = [t for t in r["transicoes"] if not t["mesma_etapa"] and t["data"]
                                   and (not de or t["data"] >= de) and (not ate or t["data"] <= ate)
                                   and (not para or contem(t["para"], para))
                                   and (not orig or contem(t["de"], orig))]
            return bool(r["_trans_periodo"])
        F.append(Filtro(desc, "geral", "detalhe", mudou))
    if a.novo_processo or a.novo_processo_de or a.novo_processo_ate:
        de, ate = a.novo_processo_de, a.novo_processo_ate
        periodo = bool(de or ate)

        def emitiu(r):
            r["_novos_periodo"] = [n for n in r["novos"] if not periodo or (
                n["data"] is not None and (not de or n["data"] >= de) and (not ate or n["data"] <= ate))]
            if periodo and r["_novos_periodo"]:
                r["novo_processo_em"] = max(n["data"] for n in r["_novos_periodo"])
            return bool(r["_novos_periodo"])
        F.append(Filtro("emitiu novo processo a partir dele" + (f" {_periodo(de, ate)}" if periodo else "")
                        + ("" if a.sem_mencoes else " (emissão normal ou menção)"), "geral", "detalhe", emitiu,
                        novo=True))
    if a.sem_novo_processo:
        F.append(Filtro("não emitiu novo processo a partir dele"
                        + ("" if a.sem_mencoes else " (nem por emissão normal nem por menção)"), "geral", "detalhe",
                        lambda r: not r["novos"], novo=True))
    if a.emissao_de or a.emissao_ate:
        de, ate = a.emissao_de, a.emissao_ate
        F.append(Filtro(f"emissão {_periodo(de, ate)}", "geral", "lista",
                        lambda r: r["emissao"] is not None and (not de or r["emissao"] >= de)
                        and (not ate or r["emissao"] <= ate)))
    if a.ult_mov_de or a.ult_mov_ate:
        de, ate = a.ult_mov_de, a.ult_mov_ate
        F.append(Filtro(f"última movimentação {_periodo(de, ate)}", "geral", "lista",
                        lambda r: r["ult_mov"] is not None and (not de or r["ult_mov"] >= de)
                        and (not ate or r["ult_mov"] <= ate)))
    if a.sem_mov_min is not None:
        F.append(Filtro(f"dias sem movimentação ≥ {a.sem_mov_min}", "geral", "lista",
                        lambda r: r["dias_sem_mov"] is not None and r["dias_sem_mov"] >= a.sem_mov_min))
    if a.sem_mov_max is not None:
        F.append(Filtro(f"dias sem movimentação ≤ {a.sem_mov_max}", "geral", "lista",
                        lambda r: r["dias_sem_mov"] is not None and r["dias_sem_mov"] <= a.sem_mov_max))
    if a.prioridade:
        F.append(Filtro(f"prioridade ∈ {', '.join(a.prioridade)}", "geral", "lista",
                        lambda r: igual(r["prioridade"], a.prioridade)))
    if a.com:
        # Coluna 'Para' da caixa: setores envolvidos (inclusive os escondidos no '+N'), não o atual
        F.append(Filtro(f"setores envolvidos (caixa) ∋ {' OU '.join(a.com)}", "geral", "lista",
                        lambda r: any(casa_setor(p, s) for s in a.com for p in r["para"].split(" | "))))
    if a.etapa_do_meu_setor:
        F.append(Filtro("etapa atual reservada ao setor logado (id em data-restrito_para_id_setor)", "posicao",
                        "detalhe", lambda r: bool(r["etapa_meu_setor"])))
    if a.objeto:
        F.append(Filtro(f"objeto ∋ {' OU '.join(a.objeto)}", "geral", "detalhe",
                        lambda r: contem(r["assunto"], a.objeto)))
    if a.situacao:
        F.append(Filtro(f"situação ∋ {' OU '.join(a.situacao)}", "geral", "detalhe",
                        lambda r: contem(r["situacao"], a.situacao)))
    if a.cita_setor or (a.pessoa and not a.excecao_cita_setor):
        quem = " / ".join(x for x in [a.cita_setor or "", *(a.pessoa or [])] if x)
        F.append(Filtro(f"últimos {a.ultimos_despachos} despacho(s) citam {quem}", "geral", "detalhe",
                        lambda r: bool(r["cita_alvo"])))
    return F


def _periodo(de, ate) -> str:
    if de and ate:
        return f"entre {de:%d/%m/%Y} e {ate:%d/%m/%Y}"
    return f"desde {de:%d/%m/%Y}" if de else f"até {ate:%d/%m/%Y}"


def avaliar(r: dict, filtros: list[Filtro], excecao: bool, so_lista: bool = False) -> bool:
    reprovou_posicao = False
    for f in filtros:
        if so_lista and f.fonte != "lista":
            continue
        if not f.pred(r):
            if f.grupo == "posicao" and excecao:
                reprovou_posicao = True
            else:
                return False
    if reprovou_posicao:
        if r.get("cita_alvo"):
            r["_obs"].append("incluído pela exceção: últimos despachos citam o setor-alvo")
            return True
        return False
    return True


def ordenar(regs: list[dict], spec: str) -> list[dict]:
    def chave_campo(r, campo):
        if campo == "etapa":
            p = posicao_etapa(r["etapa"], r["etapas_quadro"])
            return (0 if r["fase"] == "Elaboração" else 1, p if p is not None else 99, normtxt(r["etapa"]))
        if campo == "prioridade":
            return ORDEM_PRIORIDADE.get(normtxt(r["prioridade"]), 9)
        if campo == "numero":
            n, a = (r["numero"] or "0/0").split("/")
            return (int(a), int(n.replace(".", "")))
        v = r.get(campo)
        return normtxt(v) if isinstance(v, str) else v

    out = list(regs)
    for parte in reversed([p.strip() for p in spec.split(",") if p.strip()]):
        campo, _, sentido = parte.partition(":")
        desc = sentido.lower() == "desc"
        com = [r for r in out if chave_campo(r, campo) not in (None, "")]
        sem = [r for r in out if chave_campo(r, campo) in (None, "")]
        com.sort(key=lambda r: chave_campo(r, campo), reverse=desc)
        out = com + sem  # vazios sempre por último
    return out


# =============================================================================
# Saída
# =============================================================================
COLUNAS = [
    ("Nº Processo", "numero", 13), ("Emissão", "emissao", 16), ("Prioridade", "prioridade", 10),
    ("Objeto", "assunto", 55), ("Assunto (1Doc)", "tipo", 30), ("Fase", "fase", 11),
    ("Etapa atual", "etapa", 28), ("Entrou na etapa em", "entrada", 17), ("Fonte da data", "entrada_fonte", 20),
    ("Dias na etapa", "dias_etapa", 9), ("Setor da etapa", "setor_etapa", 18),
    ("Etapa do meu setor", "etapa_meu_setor", 10),
    ("Últ. despacho — para", "ult_para", 26), ("Últ. despacho — de", "ult_de", 30),
    ("Última movimentação", "ult_mov", 17), ("Dias sem mov.", "dias_sem_mov", 9),
    ("Histórico de etapas", "historico", 60), ("Nº mudanças", "n_mudancas", 9),
    ("Cita setor-alvo", "cita_alvo", 10), ("Observação", "obs", 45), ("Situação", "situacao", 12),
    ("Status no meu setor", "status_setor", 16), ("Processo de origem", "pai", 13),
    ("Processos derivados", "derivados", 16), ("Novo processo emitido em", "novo_processo_em", 17),
    ("Novos processos (vínculo)", "novos_texto", 30),
    ("Setores envolvidos (caixa)", "para", 34), ("Caixa", "caixa", 14),
]
CAMPOS_SIM_NAO = ("cita_alvo", "etapa_meu_setor")


def _valor(r, campo):
    if campo == "obs":
        return "; ".join(r["_obs"])
    v = r.get(campo)
    if campo in CAMPOS_SIM_NAO:
        return "" if v is None else ("sim" if v else "não")
    return v


def escrever_xlsx(caminho: str, regs: list[dict], params: list[tuple[str, str]], falhas: list[dict],
                  mostrar_cita: bool, marcar_periodo: bool, marcar_novo: bool = False):
    from openpyxl import Workbook
    from openpyxl.cell.cell import ILLEGAL_CHARACTERS_RE
    from openpyxl.styles import Alignment, Font, PatternFill
    from openpyxl.utils import get_column_letter

    def limpo(v):
        # Caractere de controle (comum em texto colado do Word) faz o openpyxl abortar o save
        # inteiro no fim da coleta; troca por espaço em vez de perder a execução.
        return ILLEGAL_CHARACTERS_RE.sub(" ", v) if isinstance(v, str) else v

    cab_font, cab_fill = Font(bold=True, color="FFFFFF"), PatternFill("solid", fgColor="1F3A5F")
    fmt_dh = "dd/mm/yyyy hh:mm"

    def cabecalho(ws, cols):
        for j, (rot, _, larg) in enumerate(cols, 1):
            c = ws.cell(row=1, column=j, value=rot)
            c.font, c.fill = cab_font, cab_fill
            c.alignment = Alignment(vertical="center", wrap_text=True)
            ws.column_dimensions[get_column_letter(j)].width = larg
        ws.freeze_panes = "B2"
        ws.row_dimensions[1].height = 30

    wb = Workbook()
    ws = wb.active
    ws.title = "Resultado"
    cols = [c for c in COLUNAS if mostrar_cita or c[1] != "cita_alvo"]
    cabecalho(ws, cols)
    for i, r in enumerate(regs, 2):
        for j, (_, campo, _) in enumerate(cols, 1):
            v = _valor(r, campo)
            c = ws.cell(row=i, column=j, value=limpo(v))
            if isinstance(v, datetime):
                c.number_format = fmt_dh
            if campo in ("assunto", "historico", "obs", "para"):
                c.alignment = Alignment(wrap_text=True, vertical="top")
            if campo == "numero" and r.get("url"):
                c.hyperlink, c.font = r["url"], Font(color="0563C1", underline="single")
    ws.auto_filter.ref = f"A1:{get_column_letter(len(cols))}{max(1, len(regs) + 1)}"

    wt = wb.create_sheet("Transições")
    tcols = [("Nº Processo", "numero", 13), ("Objeto", "assunto", 45), ("Data/hora", "data", 17),
             ("Etapa de origem", "de", 26), ("Etapa de destino", "para", 26), ("Estimada?", "estimada", 10),
             ("Fonte", "fonte", 18), ("Despacho nº", "despacho", 10), ("Autor", "autor", 24),
             ("Setor de origem", "setor_de", 18), ("Destinatário", "destinatario", 22),
             ("Etapa atual do processo", "etapa_atual", 26)]
    if marcar_periodo:
        tcols.insert(6, ("No período filtrado?", "no_periodo", 11))
    cabecalho(wt, tcols)
    lin = 2
    for r in regs:
        no_per = {id(t) for t in r.get("_trans_periodo", [])}
        for t in r["transicoes"]:
            if t["mesma_etapa"]:
                continue
            linha = {**t, "numero": r["numero"], "assunto": r["assunto"], "etapa_atual": r["etapa"],
                     "de": t["de"] or "(início)", "estimada": "sim" if t.get("estimada") else "não",
                     "no_periodo": "sim" if id(t) in no_per else "não"}
            for j, (_, campo, _) in enumerate(tcols, 1):
                c = wt.cell(row=lin, column=j, value=limpo(linha.get(campo)))
                if isinstance(linha.get(campo), datetime):
                    c.number_format = fmt_dh
            lin += 1
    wt.auto_filter.ref = f"A1:{get_column_letter(len(tcols))}{max(1, lin - 1)}"

    # Um processo novo por linha (o processo da aba Resultado é o de origem)
    wn = wb.create_sheet("Novos processos")
    ncols = [("Nº Processo", "numero", 13), ("Objeto", "assunto", 45), ("Novo processo", "novo", 13),
             ("Vínculo", "vinculo", 16), ("Citação", "citacao", 16), ("Emitido em", "data", 17), ("Autor", "autor", 24), ("Setor", "setor", 18),
             ("Etapa atual do processo", "etapa_atual", 26), ("Etapa do novo processo", "etapa", 26)]
    if marcar_novo:
        ncols.insert(6, ("No período filtrado?", "no_periodo", 11))
    cabecalho(wn, ncols)
    lin = 2
    for r in regs:
        no_per = {id(n) for n in r.get("_novos_periodo", [])}
        for n in r["novos"]:
            linha = {**n, "numero": r["numero"], "assunto": r["assunto"], "novo": n["numero"],
                     "vinculo": " + ".join(n.get("vinculos") or []),
                     "citacao": " / ".join(ROTULO_FONTE.get(f, f) for f in n.get("fontes") or []),
                     "etapa_atual": r["etapa"], "no_periodo": "sim" if id(n) in no_per else "não"}
            for j, (_, campo, _) in enumerate(ncols, 1):
                c = wn.cell(row=lin, column=j, value=limpo(linha.get(campo)))
                if isinstance(linha.get(campo), datetime):
                    c.number_format = fmt_dh
                if campo == "novo" and n.get("url"):
                    c.hyperlink, c.font = n["url"], Font(color="0563C1", underline="single")
            lin += 1
    wn.auto_filter.ref = f"A1:{get_column_letter(len(ncols))}{max(1, lin - 1)}"

    wp = wb.create_sheet("Parâmetros")
    wp.column_dimensions["A"].width, wp.column_dimensions["B"].width = 30, 110
    for i, (k, v) in enumerate(params, 1):
        wp.cell(row=i, column=1, value=k).font = Font(bold=True)
        wp.cell(row=i, column=2, value=limpo(v)).alignment = Alignment(wrap_text=True, vertical="top")
    if falhas:
        base = len(params) + 2
        wp.cell(row=base, column=1, value="Falhas (lacunas declaradas)").font = Font(bold=True, color="C00000")
        for i, f in enumerate(falhas, base + 1):
            wp.cell(row=i, column=1, value=f["numero"])
            wp.cell(row=i, column=2, value=limpo(f"{f['motivo']}" + (f" — obtido: {f['obtido']}" if f.get("obtido") else "")))
    wb.save(caminho)


def escrever_csv(caminho: str, regs: list[dict], mostrar_cita: bool):
    cols = [c for c in COLUNAS if mostrar_cita or c[1] != "cita_alvo"]
    with open(caminho, "w", newline="", encoding="utf-8-sig") as fh:
        w = csv.writer(fh, delimiter=";")
        w.writerow([c[0] for c in cols])
        for r in regs:
            linha = []
            for _, campo, _ in cols:
                v = _valor(r, campo)
                linha.append(v.strftime("%d/%m/%Y %H:%M") if isinstance(v, datetime) else ("" if v is None else v))
            w.writerow(linha)


def imprimir_resumo(regs: list[dict], mostrar: int):
    if not regs:
        print("\nNenhum processo atende aos filtros.")
        return
    dist = Counter(r["etapa"] or "(sem etapa)" for r in regs)
    print(f"\nResultado: {len(regs)} processo(s)")
    print("  Por etapa: " + " · ".join(f"{k} {v}" for k, v in dist.most_common()))
    print(f"\n{'Nº':<13}{'Etapa atual':<32}{'Desde':<13}{'Dias':>5}  {'Últ. despacho p/':<22}Objeto")
    for r in regs[:mostrar]:
        desde = r["entrada"].strftime("%d/%m %H:%M") if r["entrada"] else "—"
        if r["entrada"] and "estimada" in r["entrada_fonte"]:
            desde = r["entrada"].strftime("%d/%m") + " ~"
        dias = "" if r["dias_etapa"] is None else str(r["dias_etapa"])
        ult_p = re.sub(r"\s*\([^)]*\)", "", r.get("ult_para") or "")
        print(f"{r['numero']:<13}{(r['etapa'] or '')[:31]:<32}{desde:<13}{dias:>5}  {ult_p[:21]:<22}"
              f"{(r['assunto'] or '')[:60]}")
    if len(regs) > mostrar:
        print(f"… e mais {len(regs) - mostrar} (na planilha).")


# =============================================================================
# Orquestração
# =============================================================================
def _ano(numero: str) -> int:
    n = norm_numero(numero or "")
    return int(n.split("/")[1]) if n else 0


def completar_mencoes(a, cli: Cliente1Doc, cache: dict, dets: dict, lista: dict, lista_completa: bool,
                      candidatos: list[str], filtros: list[Filtro], agora: datetime, itds_ordem: list[str]) -> str:
    """Antes do filtro de novo processo, abre o que falta para cruzar as menções (a emissão nem
    sempre é a normal, 'a partir deste documento'):
      1. índice das caixas do setor (Em aberto, Saída, Arquivados, Favoritos): os processos da
         família da licitação emitidos desde o mais antigo dos processos de origem. É deles que
         vem o "cita este" (o processo novo cita o de origem) e a origem no cabeçalho;
      2. os processos que os de origem citam e que ainda não foram abertos ("citado neste").
    Processo de origem = detalhado que passa nos demais filtros. O que for aberto fica no cache
    (detalhe completo), e só é reaberto quando a última movimentação muda.
    Devolve o resumo para a aba Parâmetros."""
    outros = [f for f in filtros if not f.novo]
    excecao = bool(a.excecao_cita_setor)
    origens = [dets[k] for k in candidatos if k in dets
               and avaliar(consolidar(lista.get(k), dets[k], agora, a), outros, excecao)]
    if not origens:
        return "nenhum processo de origem a conferir"
    desde = min((de_iso(d.get("emissao")) or datetime(_ano(d["numero"]) or agora.year, 1, 1)) for d in origens)
    desde = datetime.combine(desde.date(), datetime.min.time())

    # 1. índice das caixas
    if lista_completa:  # a listagem desta busca já leu todas as caixas, sem corte de emissão
        indice, avisos = {k: v for k, v in lista.items() if v and (de_iso(v.get("emissao")) or desde) >= desde}, []
    else:
        print(f"Índice de menções: todas as caixas, processos emitidos desde {desde:%d/%m/%Y}")
        indice, _, avisos = listar(cli, list(CAIXAS), set(itds_ordem), desde, None, a.max_paginas)
        for av in avisos:
            print(f"  Aviso: {av}")
    da_familia = {k: it for k, it in indice.items() if familia(it.get("tipo", "")) is not None}
    pedidos, do_cache = [], 0
    for k, it in da_familia.items():
        c = cache["det"].get(k)
        if k in dets or (c and not a.refazer and "citados" in c and it.get("ult_mov")
                         and c.get("_ult_mov_lista") == it.get("ult_mov")):
            do_cache += 1
            continue
        itd, num = k.split("|")
        pedidos.append((num, itd, it))
    abertos, falhas_idx = {}, []
    if pedidos:
        print(f"Abrindo {len(pedidos)} processo(s) do índice de menções; {do_cache} já aberto(s) ou no cache.")
        try:
            falhas_idx = detalhar(cli, pedidos, agora, a.paralelo, abertos)
        finally:
            cache["det"].update(abertos)
            if not a.sem_cache:
                salvar_cache(a.cache, cache)

    # 2. citados pelos processos de origem que ainda não foram abertos
    uni = Universo([*cache["det"].values(), *dets.values()])
    descartados = set(cache.setdefault("citados_descartados", []))
    abrir = {}
    for d in origens:
        for c in d.get("citados") or []:
            k = c["numero"] or c["hash"]
            if uni.achar(c) is not None or k in descartados or k in abrir:
                continue
            if c["numero"] and _ano(c["numero"]) < _ano(d["numero"]):
                continue  # o processo novo não é de ano anterior ao de origem
            abrir[k] = (c["numero"] or None, itds_ordem[0], {"hash": c["hash"]} if c["hash"] else None)
    citados, falhas_cit = {}, []
    if abrir:
        print(f"Abrindo {len(abrir)} processo(s) citado(s) nos despachos.")
        try:
            falhas_cit = detalhar(cli, list(abrir.values()), agora, a.paralelo, citados)
        finally:
            cache["det"].update(citados)
            # número que não abre, página sem quadro nem rótulo: não adianta reabrir na próxima busca
            # (falha de rede, sim)
            ruins = {f["numero"] for f in falhas_cit if f["motivo"] != "exceção"}
            cache["citados_descartados"] = sorted(descartados | {k for k, p in abrir.items()
                                                                if (p[0] or f"link {k[:10]}…") in ruins})
            if not a.sem_cache:
                salvar_cache(a.cache, cache)
    falhou = f", {len(falhas_idx)} falha(s): {', '.join(f['numero'] for f in falhas_idx[:10])}" if falhas_idx else ""
    resumo = (f"índice das caixas: {len(da_familia)} processo(s) da família da licitação emitidos desde "
              f"{desde:%d/%m/%Y} ({len(abertos)} aberto(s) agora{falhou}); "
              f"citados nos despachos abertos agora: {len(citados)}")
    if avisos:
        resumo += "\nAVISOS do índice: " + "; ".join(avisos)
    if falhas_idx:
        print(f"  Aviso: {len(falhas_idx)} processo(s) do índice de menções não abriram; "
              "a menção deles fica de fora.")
    return resumo


def executar(a, cli: Cliente1Doc | None, setor: str, agora: datetime) -> dict:
    itds_ordem = _lista([a.itd]) or ["3"]
    itds = set(itds_ordem)
    cache = {"versao": VERSAO, "lista": {}, "det": {}, "posicao_lista": ""} if a.sem_cache else carregar_cache(a.cache)
    if a.offline and not cache["det"]:
        sys.exit("--offline: cache vazio ou de versão anterior; rode uma vez com acesso ao 1Doc.")
    filtros = montar_filtros(a)
    caixas = list(CAIXAS) if a.caixas == "todas" else _lista([a.caixas])
    for cx in caixas:
        if cx not in CAIXAS:
            sys.exit(f"Caixa desconhecida: {cx} (use {', '.join(CAIXAS)} ou todas)")

    # ---- universo ----
    numeros = [n for n in (norm_numero(x) for x in _lista(a.numeros)) if n]
    for arq in a.numeros_arquivo or []:
        numeros += [n for n in numeros_de_arquivo(arq) if n not in numeros]
    por_caixa, lista_fresca = {}, False
    if numeros:
        # Da listagem antiga só aproveita o hash (atalho para abrir a página); setor atual e
        # última movimentação velhos não entram no resultado.
        itd0 = itds_ordem[0]
        lista = {}
        for n in numeros:
            velho = cache["lista"].get(chave(n, itd0)) or {}
            lista[chave(n, itd0)] = {"hash": velho["hash"]} if velho.get("hash") else None
        universo = f"{len(numeros)} número(s) informado(s)"
    elif a.offline:
        lista = {k: v for k, v in cache["lista"].items() if not itds or v.get("itd") in itds}
        universo = f"listagem do cache ({cache.get('posicao_lista') or 'sem data'})"
    else:
        print(f"Lendo caixas: {', '.join(CAIXAS[c] for c in caixas)}")
        lista, por_caixa, avisos = listar(cli, caixas, itds, a.emissao_de, a.emissao_ate, a.max_paginas)
        for av in avisos:
            print(f"  Aviso: {av}")
        cache["lista"], cache["posicao_lista"] = lista, iso(agora)
        universo = "caixas " + ", ".join(f"{k} ({v})" for k, v in por_caixa.items())
        if avisos:
            universo += "\nAVISOS: " + "; ".join(avisos)
        lista_fresca = True

    # id do setor logado: da sessão; no --offline, o da última coleta
    if cli is not None and getattr(cli.sess, "id_setor", ""):
        cache["id_setor"] = cli.sess.id_setor
    a._id_setor = cache.get("id_setor", "")
    if a.etapa_do_meu_setor and not a._id_setor:
        sys.exit("--etapa-do-meu-setor: id do setor não identificado na sessão (input#s_is).")

    # ---- pré-filtro com o que a listagem já traz (barato) ----
    candidatos = []
    for k, item in lista.items():
        if item and item.get("numero"):
            r0 = consolidar(item, None, agora, a)
            if not avaliar(r0, filtros, excecao=False, so_lista=True):
                continue
        candidatos.append(k)

    # ---- detalhe (com reaproveitamento do cache) ----
    precisa = [f for f in filtros if f.fonte == "detalhe"]
    if a.sem_detalhe and precisa:
        sys.exit("Os filtros pedidos exigem abrir os processos; retire --sem-detalhe.")
    dets, falhas, do_cache = {}, [], 0
    periodo_novo = bool(a.novo_processo_de or a.novo_processo_ate)
    # Com o filtro de novo processo, as menções entre processos contam (a emissão nem sempre é a normal)
    mencoes = any(f.novo for f in filtros) and not a.sem_mencoes
    if not a.sem_detalhe:
        pedidos = []
        for k in candidatos:
            item = lista.get(k)
            c = cache["det"].get(k)
            # Detalhe gravado antes de a busca ler as emissões, com processo derivado: falta a
            # data do processo novo. Reabre (são poucos); no --offline, só pesa no filtro de período.
            sem_data_novo = bool(c) and "emissoes" not in c and bool(c.get("derivados"))
            # Detalhe gravado antes de a busca ler as menções: reabre se o filtro precisa delas.
            sem_citados = mencoes and bool(c) and "citados" not in c
            # Reaproveita só se a listagem é de agora e a última movimentação não mudou
            # (mudança de etapa sempre gera movimentação).
            reaproveita = c and not a.refazer and (a.offline or (
                not sem_data_novo and not sem_citados and lista_fresca and item and item.get("ult_mov")
                and c.get("_ult_mov_lista") == item.get("ult_mov")))
            if reaproveita and sem_data_novo and periodo_novo:
                falhas.append({"numero": k.split("|")[1], "obtido": "",
                               "motivo": "cache sem a data do novo processo (modo offline)"})
            elif reaproveita and sem_citados:
                falhas.append({"numero": k.split("|")[1], "obtido": "",
                               "motivo": "cache sem as menções a outros processos (modo offline)"})
            elif reaproveita:
                dets[k] = c
                do_cache += 1
            elif a.offline:
                falhas.append({"numero": k.split("|")[1], "motivo": "sem detalhe no cache (modo offline)", "obtido": ""})
            else:
                itd, num = k.split("|")
                pedidos.append((num, itd, item))
        if pedidos:
            est = len(pedidos) * 0.8 / max(1, a.paralelo)
            print(f"Abrindo {len(pedidos)} processo(s) (~{est:.0f} s); {do_cache} reaproveitado(s) do cache.")
            novos = {}
            try:
                falhas += detalhar(cli, pedidos, agora, a.paralelo, novos)
            finally:  # salva o parcial mesmo se a sessão cair no meio
                dets.update(novos)
                cache["det"].update(novos)
                if not a.sem_cache:
                    salvar_cache(a.cache, cache)

    # ---- novo processo: menções nos dois sentidos e origem no cabeçalho do processo novo ----
    if not mencoes:
        resumo_mencoes = ("não consideradas (--sem-mencoes)" if a.sem_mencoes else
                          "sem o filtro de novo processo, só entre os processos já abertos (nesta busca ou no cache)")
    elif a.offline:
        resumo_mencoes = "modo offline: só entre os processos que estão no cache"
    else:
        lista_completa = (lista_fresca and set(caixas) == set(CAIXAS) and not a.emissao_de and not a.emissao_ate)
        resumo_mencoes = completar_mencoes(a, cli, cache, dets, lista, lista_completa, candidatos, filtros,
                                           agora, itds_ordem)
    uni = Universo([*cache["det"].values(), *dets.values()])

    # ---- filtros finais ----
    excecao = bool(a.excecao_cita_setor)
    regs = []
    for k in candidatos:
        if not a.sem_detalhe and k not in dets:
            continue  # falha: fica declarada, não vira linha com chute
        r = consolidar(lista.get(k), dets.get(k), agora, a, uni)
        if r["numero"] and avaliar(r, filtros, excecao):
            regs.append(r)
    regs = ordenar(regs, a.ordenar)

    if not a.sem_cache:
        salvar_cache(a.cache, cache)

    params = [
        ("Posição", f"{agora:%d/%m/%Y %H:%M}"),
        ("Sessão / setor", setor or "(offline)"),
        ("Escopo", "Somente as caixas do setor do usuário logado — não é a Prefeitura inteira."),
        ("Universo", universo),
        ("Filtros", "\n".join(f"• {f.descricao}" for f in filtros) or "(nenhum)"),
        ("Exceção", f"processos reprovados só nos filtros de posição entram se os últimos {a.ultimos_despachos} "
                    f"despacho(s) citarem {a.excecao_cita_setor}" + (f" ou {', '.join(a.pessoa)}" if a.pessoa else "")
         if excecao else "(nenhuma)"),
        ("Contagens", f"listados {len(lista)} · após pré-filtro {len(candidatos)} · detalhados {len(dets)} "
                      f"({do_cache} do cache) · falhas {len(falhas)} · resultado {len(regs)}"),
        ("Etapa", "Texto exato do quadro: etapa com 'Está aqui'; só 'Ficou aqui' → 'Saiu – Próxima'; "
                  "sem quadro → rótulo do documento + '(sem quadro de etapas padrão)'."),
        ("Entrada na etapa", "Evento 'Mudança de etapa' da linha do tempo (data/hora com segundos); sem eventos, "
                             "despacho 'via etapa' mais recente para a etapa atual. Conferido contra o 'Está aqui N …' "
                             "do quadro; se divergir além da granularidade do rótulo, vale o quadro (data estimada, "
                             "marcada '~'). Sem registro: emissão (se for a 1ª etapa) ou estimativa pelo quadro. "
                             "Em 'Saiu – Próxima' (Etapa Final não exibe 'Está aqui'), a data é a da entrada na Próxima."),
        ("Setores envolvidos", "Coluna 'Para' da caixa, com os setores escondidos no '+N' e nos badges truncados. "
                               "Não é o setor que está com o processo: para isso, veja 'Últ. despacho — para'."),
        ("Novo processo", "Vínculos (coluna 'Novos processos (vínculo)' e aba 'Novos processos'): "
                          "• emissão — evento 'emitiu o Proc. Administrativo N/AAAA a partir deste documento' "
                          "da linha do tempo, com a data/hora dele; • origem — o processo novo traz este no "
                          "cabeçalho de origem, sem o evento aqui; • citado neste — este processo cita o novo; "
                          "• cita este — o processo novo cita este. A coluna 'Citação' diz como: menção do 1Doc "
                          "(o processo escolhido na lista ao escrever o despacho, data-mention), link no texto "
                          "ou número no texto (com 'processo', 'nº'… antes). Os dois últimos cobrem a emissão "
                          "que não foi a normal e só valem para Proc. Administrativo sem outra origem, emitido "
                          "depois deste e "
                          "adiante na licitação (Elaboração → Fase Preparatória → Abertura); a data é a de emissão "
                          "do processo novo. Só conta processo: memorando, ofício etc. ficam de fora. 'Novo processo "
                          "emitido em' é o mais recente (com o filtro de período, o mais recente dentro dele)."),
        ("Menções", resumo_mencoes),
        ("Dias", "Dias corridos de calendário até a data de posição."),
    ]
    return {"regs": regs, "falhas": falhas, "params": params, "filtros": filtros}


def montar_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="busca", fromfile_prefix_chars="@",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        description="Busca processos no 1Doc (PMC) por etapa, fase, datas e mudanças de etapa. Somente leitura.",
        epilog="""Exemplos (no programa, escreva só as opções no campo "Outras opções")
  Em Pesquisa de Mercado agora (caixa Em aberto):
    --etapa "Pesquisa de Mercado"
  O que mudou de etapa em outubro, em todas as caixas:
    --caixas todas --mudanca-de 01/10/2026 --mudanca-ate 31/10/2026
  Quem foi para Correção de Documentação nos últimos 7 dias:
    --mudanca-para "Correção de Documentação" --mudanca-de -7
  Entraram na etapa atual hoje:
    --entrada-de hoje
  Fase de Abertura há 15+ dias na mesma etapa, urgentes primeiro:
    --fase Abertura --dias-etapa-min 15 --ordenar prioridade,dias_etapa:desc
  Regra da CPPFI (antes da Pesquisa de Mercado, ou depois mas com o último despacho citando a CPPFI):
    --antes-de "Pesquisa de Mercado" --excecao-cita-setor SPOT-DCL-GP-CPPFI --pessoa "João Otávio"
  Emitiram novo processo (ex.: a Abertura saída da Fase Preparatória) em novembro, em todas as caixas:
    --caixas todas --novo-processo-de 01/11/2026 --novo-processo-ate 30/11/2026
  Na Etapa Final da Elaboração e ainda sem novo processo emitido (nem citado em outro processo):
    --fase Elaboração --etapa "Etapa Final" --sem-novo-processo
  O mesmo, contando só a emissão normal ("a partir deste documento"):
    --fase Elaboração --etapa "Etapa Final" --sem-novo-processo --sem-mencoes
  Processos de uma planilha/lista, sem varrer as caixas:
    --numeros-arquivo acompanhamento.xlsx
  Refiltrar sem acessar o 1Doc (usa o cache da última coleta):
    --offline --etapa "Validação de Requisição"
  Parâmetros num arquivo (um por linha):  @consulta_cppfi.txt

Datas: DD/MM/AAAA, AAAA-MM-DD, hoje, ontem ou -N (N dias atrás). Limites inclusivos.
Textos: sem distinção de maiúsculas e acentos; "contém". Valores repetidos de um mesmo filtro = OU;
filtros diferentes = E.""")

    g = p.add_argument_group("Sessão (o login é sempre feito por você, na janela do navegador)")
    g.add_argument("--relogar", action="store_true", help="ignora a sessão salva e abre o navegador para novo login")
    g.add_argument("--navegador", choices=("auto", "chrome", "edge"), default=None,
                   help="padrão: o das Configurações do programa")
    g.add_argument("--timeout-login", type=int, default=None, metavar="S",
                   help="segundos esperando o login (padrão: o das Configurações)")
    # Aceitos por compatibilidade com consultas antigas (@arquivo), sem efeito:
    # a sessão fica na pasta de dados do programa e o endereço é fixo.
    g.add_argument("--base", default=BASE_PADRAO, help=argparse.SUPPRESS)
    g.add_argument("--cookies", help=argparse.SUPPRESS)
    g.add_argument("--perfil", help=argparse.SUPPRESS)

    g = p.add_argument_group("Universo (o que coletar)")
    g.add_argument("--caixas", default="entrada", help="entrada, saida, arquivo, favoritos (vírgula) ou todas")
    g.add_argument("--numeros", action="append", metavar="N", help="processos específicos: 31.632/2026,5712/2025")
    g.add_argument("--numeros-arquivo", action="append", metavar="ARQ",
                   help=".txt/.csv/.xlsx de onde extrair os números NN.NNN/AAAA")
    g.add_argument("--itd", default="3", help="tipo de documento (3 = Proc. Administrativo; ex.: 3,24)")
    g.add_argument("--max-paginas", type=int, default=200)

    g = p.add_argument_group("Filtros de etapa e posição")
    g.add_argument("--etapa", action="append", metavar="TXT", help="etapa atual contém TXT")
    g.add_argument("--etapa-exata", action="store_true", help="--etapa/--excluir-etapa exigem igualdade")
    g.add_argument("--excluir-etapa", action="append", metavar="TXT")
    g.add_argument("--fase", type=tipo_fase, metavar="FASE", help="Elaboração ou Abertura")
    g.add_argument("--quadro", action="append", metavar="TXT", help="quadro/assunto do 1Doc contém TXT")
    g.add_argument("--antes-de", metavar="ETAPA", help="etapa atual anterior a ETAPA no quadro")
    g.add_argument("--a-partir-de", metavar="ETAPA", help="etapa atual igual ou posterior a ETAPA")
    g.add_argument("--passou-por", action="append", metavar="ETAPA", help="já esteve em ETAPA (histórico)")
    g.add_argument("--nao-passou-por", action="append", metavar="ETAPA")
    g.add_argument("--setor-etapa", action="append", metavar="COD", help="setor responsável pela etapa atual")
    g.add_argument("--etapa-do-meu-setor", action="store_true",
                   help="etapa atual reservada ao setor logado (pelo id do setor no quadro, não pelo texto)")
    g.add_argument("--dias-etapa-min", type=int, metavar="N")
    g.add_argument("--dias-etapa-max", type=int, metavar="N")

    g = p.add_argument_group("Datas e momento de mudança de etapa")
    g.add_argument("--entrada-de", metavar="DATA", help="entrou na etapa ATUAL a partir de DATA")
    g.add_argument("--entrada-ate", metavar="DATA")
    g.add_argument("--mudanca-de", metavar="DATA", help="teve mudança de etapa a partir de DATA")
    g.add_argument("--mudanca-ate", metavar="DATA")
    g.add_argument("--mudanca-para", action="append", metavar="ETAPA", help="…com destino a ETAPA")
    g.add_argument("--mudanca-origem", action="append", metavar="ETAPA", help="…saindo de ETAPA")
    g.add_argument("--emissao-de", metavar="DATA", help="criado a partir de DATA (filtra no servidor)")
    g.add_argument("--emissao-ate", metavar="DATA")
    g.add_argument("--ult-mov-de", metavar="DATA")
    g.add_argument("--ult-mov-ate", metavar="DATA")
    g.add_argument("--sem-mov-min", type=int, metavar="N", help="dias sem movimentação ≥ N")
    g.add_argument("--sem-mov-max", type=int, metavar="N")

    g = p.add_argument_group("Novo processo emitido a partir dele (evento 'emitiu o Proc. Administrativo …' "
                             "ou menção entre os processos)")
    g.add_argument("--novo-processo", action="store_true", help="emitiu ao menos um novo processo")
    g.add_argument("--novo-processo-de", metavar="DATA", help="…emitido a partir de DATA")
    g.add_argument("--novo-processo-ate", metavar="DATA", help="…emitido até DATA")
    g.add_argument("--sem-novo-processo", action="store_true", help="não emitiu nenhum novo processo")
    g.add_argument("--sem-mencoes", action="store_true",
                   help="só a emissão normal: ignora o processo novo que cita este ou é citado nele "
                        "(e não monta o índice das caixas)")

    g = p.add_argument_group("Outros filtros")
    g.add_argument("--prioridade", action="append", metavar="P", help="Urgente, Alta, Média, Baixa (vírgula)")
    g.add_argument("--com", action="append", metavar="COD",
                   help="setor entre os envolvidos da coluna 'Para' da caixa (código inteiro ou último nível)")
    g.add_argument("--objeto", action="append", metavar="TXT", help="objeto contém TXT")
    g.add_argument("--situacao", action="append", metavar="TXT")
    g.add_argument("--cita-setor", metavar="COD", help="últimos N despachos citam o setor (ex.: SPOT-DCL-GP-CPPFI)")
    g.add_argument("--pessoa", action="append", metavar="NOME", help="variantes de nome que contam como o setor")
    g.add_argument("--ultimos-despachos", type=int, default=1, metavar="N", help="padrão: %(default)s")
    g.add_argument("--excecao-cita-setor", metavar="COD",
                   help="inclui quem falhou só nos filtros de posição se os últimos N despachos citarem COD")

    g = p.add_argument_group("Saída e desempenho")
    g.add_argument("--saida", metavar="ARQ.xlsx",
                   help="padrão: 'Busca 1Doc DD-MM-AAAA HHhMM.xlsx' na pasta de saída das Configurações")
    g.add_argument("--abrir", action="store_true", help="abre a planilha ao terminar")
    g.add_argument("--csv", metavar="ARQ.csv", help="também grava CSV (separador ;)")
    g.add_argument("--sem-xlsx", action="store_true")
    g.add_argument("--ordenar", default="fase,etapa,dias_etapa:desc",
                   help="campos: numero, emissao, prioridade, fase, etapa, entrada, dias_etapa, ult_mov, "
                        "dias_sem_mov, novo_processo_em (sufixo :desc). Padrão: %(default)s")
    g.add_argument("--mostrar", type=int, default=30, metavar="N", help="linhas no terminal")
    g.add_argument("--cache", default=None, help="cache de detalhes (padrão: 1doc_cache.json na pasta de dados)")
    g.add_argument("--sem-cache", action="store_true")
    g.add_argument("--offline", action="store_true", help="não acessa o 1Doc: filtra o cache")
    g.add_argument("--refazer", action="store_true", help="reabre todos os processos, ignorando o cache")
    g.add_argument("--sem-detalhe", action="store_true", help="só a listagem (rápido; sem etapa)")
    g.add_argument("--paralelo", type=int, default=3, metavar="N", help="processos abertos em paralelo")
    g.add_argument("--pausa", type=float, default=0.15, metavar="S", help="pausa entre requisições por conexão")
    return p


def tipo_fase(s: str) -> str:
    n = normtxt(s)
    if n.startswith("elab") or n.startswith("prep"):
        return "Elaboração"
    if n.startswith("abert"):
        return "Abertura"
    raise argparse.ArgumentTypeError("fase: Elaboração ou Abertura")


DATAS_INICIO = ("entrada_de", "mudanca_de", "emissao_de", "ult_mov_de", "novo_processo_de")
DATAS_FIM = ("entrada_ate", "mudanca_ate", "emissao_ate", "ult_mov_ate", "novo_processo_ate")


def normalizar_args(a, agora: datetime | None = None):
    """Resolve as datas contra a data de posição da execução (hoje, ontem, -N são relativos a ela)."""
    hoje = (agora or agora_br()).date()
    for nome in DATAS_INICIO + DATAS_FIM:
        v = getattr(a, nome)
        if v:
            try:
                setattr(a, nome, data_cli(v, hoje, fim=nome in DATAS_FIM))
            except argparse.ArgumentTypeError as e:
                sys.exit(f"--{nome.replace('_', '-')}: {e}")
    a.prioridade = _lista(a.prioridade)
    for nome in ("etapa", "excluir_etapa", "quadro", "passou_por", "nao_passou_por", "setor_etapa",
                 "mudanca_para", "mudanca_origem", "com", "objeto", "situacao", "pessoa"):
        setattr(a, nome, [x.strip() for x in (getattr(a, nome) or []) if x and x.strip()])
    if a.offline and a.sem_cache:
        sys.exit("--offline precisa do cache; retire --sem-cache.")
    if a.sem_novo_processo and (a.novo_processo or a.novo_processo_de or a.novo_processo_ate):
        sys.exit("--sem-novo-processo não combina com --novo-processo, --novo-processo-de ou --novo-processo-ate.")
    if not a.cache:
        a.cache = caminhos.arquivo_dados("1doc_cache.json")
    return a


def main(argv=None, cred=None):
    """`cred`: credenciais já obtidas pela interface (sessao.obter_sessao)."""
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):
        pass
    agora = agora_br()
    a = normalizar_args(montar_parser().parse_args(argv), agora)
    cli, setor = None, ""
    if not a.offline:
        sess, setor = abrir_sessao(a, cred)
        print(f"Sessão ativa — setor: {setor}")
        cli = Cliente1Doc(sess)
    try:
        res = executar(a, cli, setor, agora)
    except SessaoExpirada:
        sessao_1doc.esquecer()
        sys.exit("A sessão do 1Doc caiu durante a coleta. Execute de novo (o navegador vai abrir para o "
                 "login); o que já foi detalhado está no cache.")
    imprimir_resumo(res["regs"], a.mostrar)
    if res["falhas"]:
        print(f"\nFalhas ({len(res['falhas'])}) — ficam como lacuna, sem chute: "
              + ", ".join(f"{f['numero']} ({f['motivo']})" for f in res["falhas"][:15]))
    mostrar_cita = bool(a.cita_setor or a.excecao_cita_setor or a.pessoa)
    marcar = bool(a.mudanca_de or a.mudanca_ate or a.mudanca_para or a.mudanca_origem)
    marcar_novo = bool(a.novo_processo_de or a.novo_processo_ate)
    res["arquivo"] = None
    if not a.sem_xlsx:
        saida = a.saida or os.path.join(config.pasta_saida(), f"Busca 1Doc {agora:%d-%m-%Y %Hh%M}.xlsx")
        escrever_xlsx(saida, res["regs"], res["params"], res["falhas"], mostrar_cita, marcar, marcar_novo)
        res["arquivo"] = os.path.abspath(saida)
        print(f"\nPlanilha: {res['arquivo']}")
        if a.abrir:
            caminhos.abrir_arquivo(res["arquivo"])
    if a.csv:
        escrever_csv(a.csv, res["regs"], mostrar_cita)
        print(f"CSV: {a.csv}")
    return res


if __name__ == "__main__":
    main()
