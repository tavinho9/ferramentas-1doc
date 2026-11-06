"""Um 1Doc de mentira, em memória, para os testes de ponta a ponta.

Substitui requests.Session.request: toda requisição de qualquer módulo cai no
roteador abaixo, que responde com HTML no formato da interface clássica do
1Doc (os mesmos seletores que os scripts leem). Dois processos:

  P1 31.632/2026 — Abertura, etapa atual "Pesquisa de Mercado" (há 2 horas)
  P2 31.700/2026 — Abertura, etapa atual "Validação de Requisição" (há 3 dias)
"""

import time
from datetime import date, datetime
from urllib.parse import parse_qs, urlparse

import requests

HOJE = date.today()
EMISSAO = f"{HOJE:%d/%m/%Y} 09:15"

PROCESSOS = {
    "AAAA1111BBBB2222CCCC": {
        "numero": "31.632/2026", "objeto": "Aquisição de medicamentos",
        "descricao": "Aquisição de medicamentos para a rede municipal de saúde",
        "etapas": [("Validação de Requisição", "Ficou aqui 1 dia", False),
                   ("Correção de Documentação", "", False),
                   ("Pesquisa de Mercado", "Esta aqui 2 horas", True),
                   ("Etapa Final", "", False)],
        "prioridade": "Alta",
    },
    "DDDD3333EEEE4444FFFF": {
        "numero": "31.700/2026", "objeto": "Reforma da escola municipal",
        "descricao": "Reforma da Escola Municipal Exemplo",
        "etapas": [("Validação de Requisição", "Esta aqui 3 dias", True),
                   ("Correção de Documentação", "", False),
                   ("Pesquisa de Mercado", "", False),
                   ("Etapa Final", "", False)],
        "prioridade": "Média",
    },
}
POR_NUMERO = {p["numero"].replace(".", ""): h for h, p in PROCESSOS.items()}

LOGADO = """<input type="hidden" id="sis_usr_email" value="teste@montesclaros.mg.gov.br">
<input type="hidden" id="s_s" value="SPOT-DCL-GP-CPPFI"><input type="hidden" id="s_is" value="285">"""


def painel():
    return f"""<html><body>{LOGADO}
<ul><li class="li_setor active">Inbox de SPOT-DCL-GP-CPPFI</li></ul>
<div class="tab-content"><input type="hidden" id="caixa_parametro_ordem" value="desc"></div>
</body></html>"""


def linha(h, p):
    return f"""<tr>
<td><a class="link_emissao_a" href="./?pg=doc/ver&hash={h}&itd=3&origem=painel_setor">Proc.
   Administrativo {p['numero']} <small class="data">{EMISSAO}</small></a></td>
<td><div class="td_remetente"><span class="badge badge_env tt">SS-DAF</span>
  <span class="pp" data-content="Maria da Silva">Maria S.</span></div></td>
<td class="link_emissao td ignora normal celula_para"><span class="badge">SPOT-DCL-GP-CPPFI</span></td>
<td class="link_emissao td ignora normal">{p['objeto']}
  <span class="badge" title="Abertura de Procedimento Licitatório">Abertura de Proc...</span>
  <div class="resumo">resumo do processo</div>
  <span class="label" title="Prioridade">{p['prioridade']}</span></td>
<td><small class="tt data" title="Última movimentação em {HOJE:%d/%m/%Y} 10:00:00">2 horas</small></td>
</tr>"""


def listagem(caixa, pagina):
    linhas = "".join(linha(h, p) for h, p in PROCESSOS.items()) if caixa == "entrada" and pagina == 1 else ""
    return f"""<div class="navega_caixa">Página 1/1</div>
<table class="table">{linhas}</table>
<script>$('#num_tab_{caixa}').html('({len(PROCESSOS) if caixa == 'entrada' else 0})')</script>"""


def kanban(p, modelo=False):
    blocos = []
    for nome, marca, atual in p["etapas"]:
        if modelo:
            marca = "Aqui há alguns segundos" if atual else ""
        cls = "kanban_estagios estagio_final" if nome == "Etapa Final" else "kanban_estagios"
        if atual:
            item = f'<p class="kanban_item">{marca}</p>'
        elif marca:
            item = f'<div class="kanban-time">{marca}</div>'
        else:
            item = ""
        blocos.append(f'<div class="{cls}" data-current="{"true" if atual else "false"}">'
                      f'<div class="kanban_titulo"><p>{nome}</p></div>'
                      f'<div class="kanban_itens">{item}</div>'
                      f'<div class="kanban_footer"><span class="badge_env">SPOT-DCL-GP-CPPFI</span></div></div>')
    return '<div class="workflow_v2_kanban">' + "".join(blocos) + "</div>"


def detalhe(h):
    p = PROCESSOS[h]
    mk = int(time.mktime(datetime.combine(HOJE, datetime.min.time()).replace(hour=9, minute=15).timetuple()))
    return f"""<html><body>{LOGADO}
<input type="hidden" id="nome_em" value="Proc. Administrativo {p['numero']}">
<input type="hidden" id="hash_emissao" value="{h}">
<input type="hidden" id="id_setor_origem" value="12">
<div id="emissao_principal" data-datamk="{mk}">
  <h2 class="tt pull-left emission_infos_item_atual"><span class="nd">Proc. Administrativo</span>
    <small class="nd_num">{p['numero']}</small> <span class="label" title="Prioridade">{p['prioridade']}</span></h2>
  <h4 class="tit tit_assunto" data-id_assunto="961721">{p['objeto']}
    <span class="badge" title="Assunto: Abertura de Procedimento Licitatório">Abertura de Proc...</span></h4>
  <a class="ir_quadro">Ir ao quadro: Abertura de Procedimento Licitatório</a>
  {kanban(p)}
  <div id="campo_adicional_descricao_objeto"><div class="cp">
    <span data-valor="{p['descricao']}">Objeto<span>*</span>:</span><small>{p['descricao']}</small></div></div>
</div>
<span class="badge" data-id_setor="12" title="Secretaria de Saúde » SS-DAF">SS-DAF</span>
<div class="timeline_conteudo" data-datamk="{mk}">
<table class="relative despacho"><tr><th>
  <abbr class="despacho_data" title="Em {HOJE:%d/%m/%Y} 10:00">hoje</abbr>
  <div class="rf_header"><strong>Despacho 1</strong></div>
  <div class="rf_de"><span class="badge_env">SS-DAF</span></div>
</th></tr><tr><td class="celula_despacho"><div class="texto_original">Encaminho para análise.</div></td></tr>
</table></div>
<div class="tm_conteudo"><div id="parent_wa" data-current_idph="1">{kanban(p, modelo=True)}</div></div>
</body></html>"""


class Resposta:
    def __init__(self, texto, url, status=200):
        self.text = texto
        self.content = texto.encode("utf-8")
        self.url = url
        self.status_code = status
        self.headers = {"Content-Type": "text/html; charset=utf-8"}
        self.history = []
        self.encoding = "utf-8"

    def raise_for_status(self):
        if self.status_code >= 400:
            raise requests.HTTPError(f"{self.status_code}")


class Fake1Doc:
    """Roteador. `pedidos` guarda (método, pg/chamada) de cada requisição."""

    def __init__(self, logado=True):
        self.logado = logado
        self.pedidos = []

    def __call__(self, sessao, metodo, url, params=None, data=None, **kw):
        q = {k: v[0] for k, v in parse_qs(urlparse(url).query, keep_blank_values=True).items()}
        q.update({k: str(v) for k, v in (params or {}).items()})
        self.pedidos.append((metodo, q.get("chamada") or q.get("pg") or urlparse(url).path))
        if not self.logado:
            return Resposta('<form><input name="identifier"><input type="password" name="senha"></form>', url)
        if "formAcessoRapido" in url:
            h = POR_NUMERO.get(f"{data['num_codigo']}/{data['ano']}".replace(".", "")
                               if data else "")
            destino = f"https://montesclaros.1doc.com.br/?pg=doc/ver&hash={h}&itd=3" if h else url
            return Resposta("<html></html>", destino)
        if q.get("chamada") == "caixa":
            return Resposta(listagem(q.get("caixa"), int(q.get("num_pagina", 1))), url)
        if q.get("pg") == "doc/ver":
            h = (q.get("hash") or "").upper()
            if h in PROCESSOS:
                return Resposta(detalhe(h), url)
            return Resposta("não encontrado", url, 404)
        return Resposta(painel(), url)


def instalar(monkeypatch, logado=True):
    fake = Fake1Doc(logado)
    monkeypatch.setattr(requests.Session, "request",
                        lambda self, metodo, url, **kw: fake(self, metodo, url, **kw))
    return fake
