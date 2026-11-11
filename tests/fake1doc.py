"""Um 1Doc de mentira, em memória, para os testes de ponta a ponta.

Substitui requests.Session.request: toda requisição de qualquer módulo cai no
roteador abaixo, que responde com HTML no formato da interface clássica do
1Doc (os mesmos seletores que os scripts leem). Dois processos:

  P1 31.632/2026 — Abertura, etapa atual "Pesquisa de Mercado" (há 2 horas)
  P2 31.700/2026 — Abertura, etapa atual "Validação de Requisição" (há 3 dias)

Um processo pode ganhar "emissoes": [(tipo, número, hash, datetime)], que viram
eventos "emitiu o <tipo> <número> a partir deste documento" na linha do tempo.
Para as menções entre processos, também: "assunto" (padrão: Abertura de
Procedimento Licitatório), "emissao_dt" (datetime; padrão: hoje 09:15),
"caixa" (padrão: entrada; None = fora das caixas, só abre pelo link ou pelo
número), "pai" ((número, hash) do processo de origem, no cabeçalho),
"despacho" (HTML do texto do despacho) e "anexos" (HTML que fica no despacho
fora do bloco de texto, como a lista de documentos mencionados).
"""

from datetime import date, datetime, timedelta, timezone
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
ASSUNTO = "Abertura de Procedimento Licitatório"
ID_ASSUNTO = {"Abertura de Procedimento Licitatório": "961721",
              "Fase Preparatória de Procedimento Licitatório": "962809"}


def emissao_dt(p):
    return p.get("emissao_dt") or datetime.combine(HOJE, datetime.min.time()).replace(hour=9, minute=15)

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
   Administrativo {p['numero']} <small class="data">{emissao_dt(p):%d/%m/%Y %H:%M}</small></a></td>
<td><div class="td_remetente"><span class="badge badge_env tt">SS-DAF</span>
  <span class="pp" data-content="Maria da Silva">Maria S.</span></div></td>
<td class="link_emissao td ignora normal celula_para"><span class="badge">SPOT-DCL-GP-CPPFI</span></td>
<td class="link_emissao td ignora normal">{p['objeto']}
  <span class="badge" title="{p.get('assunto', ASSUNTO)}">{p.get('assunto', ASSUNTO)[:12]}...</span>
  <div class="resumo">resumo do processo</div>
  <span class="label" title="Prioridade">{p['prioridade']}</span></td>
<td><small class="tt data" title="Última movimentação em {HOJE:%d/%m/%Y} 10:00:00">2 horas</small></td>
</tr>"""


def listagem(caixa, pagina):
    da_caixa = [(h, p) for h, p in PROCESSOS.items() if p.get("caixa", "entrada") == caixa]
    linhas = "".join(linha(h, p) for h, p in da_caixa) if pagina == 1 else ""
    return f"""{'<div class="navega_caixa">Página 1/1</div>' if da_caixa else ''}
<table class="table">{linhas}</table>
<script>$('#num_tab_{caixa}').html('({len(da_caixa)})')</script>"""


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


def evento_emissao(tipo, numero, hash_, quando):
    """Evento da linha do tempo de um documento emitido a partir do processo. O
    carimbo é o instante de `quando` no horário de Brasília, como o do 1Doc."""
    mk = int(quando.replace(tzinfo=timezone(timedelta(hours=-3))).timestamp())
    return f"""<div class="timeline_conteudo" data-datamk="{mk}">
<div class="desp_notificacao"><div class="marcou marcou_file">
  <span class="marcou_nome">Maria da Silva</span> <span class="badge_env">SS-DAF</span>
  <small class="marcou_quem">emitiu o <a href="./?pg=doc/ver&hash={hash_}&origem=emissao_evento_base">{tipo}
    {numero}</a> a partir deste documento.</small>
</div></div></div>"""


def detalhe(h):
    p = PROCESSOS[h]
    mk = int(emissao_dt(p).replace(tzinfo=timezone(timedelta(hours=-3))).timestamp())   # horário de Brasília
    eventos = "".join(evento_emissao(*e) for e in p.get("emissoes", []))
    assunto = p.get("assunto", ASSUNTO)
    pai = ""
    if p.get("pai"):
        pai = (f'<h2 class="tt pull-left emission_infos_item_from" title="Emitido a partir de">'
               f'<a class="nd" href="./?pg=doc/ver&itd=3&hash={p["pai"][1]}&origem=base">Proc. Administrativo '
               f'<small>{p["pai"][0]}</small></a></h2>')
    return f"""<html><body>{LOGADO}
<input type="hidden" id="nome_em" value="Proc. Administrativo {p['numero']}">
<input type="hidden" id="hash_emissao" value="{h}">
<input type="hidden" id="id_setor_origem" value="12">
<div id="emissao_principal" data-datamk="{mk}">
  {pai}
  <h2 class="tt pull-left emission_infos_item_atual"><span class="nd">Proc. Administrativo</span>
    <small class="nd_num">{p['numero']}</small> <span class="label" title="Prioridade">{p['prioridade']}</span></h2>
  <h4 class="tit tit_assunto" data-id_assunto="{ID_ASSUNTO.get(assunto, '1')}">{p['objeto']}
    <span class="badge" title="Assunto: {assunto}">{assunto[:12]}...</span></h4>
  <a class="ir_quadro">Ir ao quadro: {assunto}</a>
  {kanban(p)}
  <div id="campo_adicional_descricao_objeto"><div class="cp">
    <span data-valor="{p.get('descricao', p['objeto'])}">Objeto<span>*</span>:</span><small>{p.get('descricao', p['objeto'])}</small></div></div>
</div>
<span class="badge" data-id_setor="12" title="Secretaria de Saúde » SS-DAF">SS-DAF</span>
<div class="timeline_conteudo" data-datamk="{mk}">
<table class="relative despacho"><tr><th>
  <abbr class="despacho_data" title="Em {HOJE:%d/%m/%Y} 10:00">hoje</abbr>
  <div class="rf_header"><strong>Despacho 1</strong></div>
  <div class="rf_de"><span class="badge_env">SS-DAF</span></div>
</th></tr><tr><td class="celula_despacho"><div class="texto_original fr-view">{p.get('despacho', 'Encaminho para análise.')}</div>{p.get('anexos', '')}</td></tr>
</table></div>
{eventos}
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
    """Roteador. `pedidos` guarda (método, pg/chamada) de cada requisição e
    `intervalos`, o parâmetro intervalo de cada listagem (que o roteador,
    como o servidor poderia fazer, ignora: a listagem traz sempre tudo)."""

    def __init__(self, logado=True):
        self.logado = logado
        self.pedidos = []
        self.intervalos = []

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
            self.intervalos.append(q.get("intervalo"))
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
