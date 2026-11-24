"""Casos tirados das páginas reais do 1Doc salvas em sourcecode/ (view-source do painel e de
processos de Elaboração, Fase Preparatória, Abertura e Pregão). Os trechos repetem a marcação
real; nomes, números e hashes são fictícios."""

from datetime import datetime

import pytest

from ferramentas1doc import busca, faltantes, verificar_planilha

# ---------------------------------------------------------------------------------------------
# Rótulo de etapa do despacho cortado pelo 1Doc
# ---------------------------------------------------------------------------------------------
# O 1Doc corta o rótulo "via etapa" do despacho em bytes, não em letras: no Pregão real, o
# despacho para "Termo de Autuação" chega como "Termo de Autua\xc3..." (meio "ç"), que vira
# "Termo de Autua�..." ao decodificar.
TITULOS_PREGAO = ["Abertura do Processo", "Termo de Abertura", "Termo de Autuação",
                  "Intenção de Registro de Preço", "Assinatura do Edital", "Parecer Jurídico",
                  "Correção de Edital", "Parecer Jurídico Final", "Etapa Final"]


def test_resolver_etapa_com_letra_cortada_ao_meio():
    assert busca.resolver_etapa("Termo de Autua�...", TITULOS_PREGAO) == "Termo de Autuação"


def _despacho(data, n, rotulo, para="SPOT-DCL-GPAL (Em aberto)"):
    return {"data": data, "n": n, "situacao": "Encaminhado", "rotulo_etapa": rotulo, "autor": "Ana Souza",
            "setor_de": "SPOT-DCL-GPAL", "para": para, "texto": "", "bruto": ""}


def _evento(data, de, para):
    return {"data": data, "de": de, "para": para, "autor": "Ana Souza", "setor": "SPOT-DCL-GPAL"}


def test_transicao_por_evento_acha_o_despacho_com_letra_cortada():
    desp = [_despacho("2026-10-16T11:08:53", "3", "Termo de Autua�...")]
    eventos = [_evento(datetime(2026, 10, 16, 11, 8, 53), "Termo de Abertura", "Termo de Autuação")]
    t = busca._transicoes(desp, TITULOS_PREGAO, eventos, None)
    assert [(x["para"], x["despacho"]) for x in t] == [("Termo de Autuação", "3")]


def test_transicao_por_evento_acha_o_despacho_com_rotulo_ambiguo():
    """'Parecer Jurídi...' começa tanto "Parecer Jurídico" quanto "Parecer Jurídico Final": sozinho
    não se resolve, mas o evento do mesmo instante diz o destino, e o rótulo é compatível com ele."""
    desp = [_despacho("2026-10-16T16:47:25", "8", "Parecer Jurídi...", "PGM-PACJ (Sem pendência)")]
    eventos = [_evento(datetime(2026, 10, 16, 16, 47, 26), "Assinatura do Edital", "Parecer Jurídico")]
    t = busca._transicoes(desp, TITULOS_PREGAO, eventos, None)
    assert [(x["para"], x["despacho"], x["destinatario"]) for x in t] == [
        ("Parecer Jurídico", "8", "PGM-PACJ (Sem pendência)")]


@pytest.mark.parametrize("rotulo,destino", [
    ("Correção de E...", "Parecer Jurídico"),          # cortado, mas de outra etapa
    ("Parecer Jurídico", "Parecer Jurídico Final"),    # inteiro: só vale a etapa com esse nome
])
def test_transicao_por_evento_nao_pega_despacho_de_outra_etapa(rotulo, destino):
    """O rótulo tem de ser compatível com o destino do evento, mesmo no mesmo instante."""
    desp = [_despacho("2026-10-16T16:47:25", "8", rotulo)]
    eventos = [_evento(datetime(2026, 10, 16, 16, 47, 26), "Assinatura do Edital", destino)]
    t = busca._transicoes(desp, TITULOS_PREGAO, eventos, None)
    assert [(x["para"], x["despacho"]) for x in t] == [(destino, "")]


# ---------------------------------------------------------------------------------------------
# Despacho cancelado (Faltantes: cancelamento no último despacho)
# ---------------------------------------------------------------------------------------------
# Marcação real: o despacho cancelado continua na linha do tempo, recolhido num
# div.canceled-accordion-group (a classe da tabela vem colada: "table-borderedtable_emission_canceled"),
# e o 1Doc publica um despacho de aviso, "<b>Proc. Administrativo 7-41.039/2026</b> cancelado por
# <b>Fulano</b><span class="motivo-cancelamento">, com a seguinte justificativa: …</span>". O aviso
# fala do despacho, não do processo.
def _tabela(n, texto, cancelado=False):
    cls = "table table-borderedtable_emission_canceled" if cancelado else "table table-bordered"
    cab = "rf_header rf_header_canceled_emission" if cancelado else "rf_header"
    return f"""<table class="relative despacho despacho_ABCDEF{n:02d} {cls}"><tbody><tr>
<th class="th_td"><div class="row-fluid {cab}"><div class="span12">
<strong class="" data-im="{n}">Despacho {n}- 41.039/2026</strong>
<abbr class="tt hidden-print despacho_data muted menor" title="Em 16/10/2026 16:{40 + n}:00">
<small><span class="label label-inverse">Encaminhado</span> 16/10/2026 16:{40 + n}</small></abbr>
</div></div></th>
<td class="celula_despacho"><div class="emissao_conteudo"><span class="texto_original">{texto}</span></div></td>
</tr></tbody></table>"""


def _timeline(conteudo, n):
    return (f'<div class="timeline_conteudo tc_{n} desp_despacho" data-datamk="17921800{n:02d}">'
            f'<div class="timeline_point tp_{n} canceled_emission_timeline tt desp_despacho"></div>'
            f'<a name="i{n}"></a>{conteudo}</div>')


def _recolhido(tabela):
    return ('<div class="relative accordion-group canceled-accordion-group"><div class="accordion-heading">'
            '<div class="canceled_emission_header"><p>O despacho foi cancelado em 16/10/2026 16:48:18 por '
            'Ana Souza.</p></div><div class="canceled_emission_flag_container"><span class="canceled_emission_flag">'
            'Situação: Cancelado</span></div></div>'
            f'<div class="accordion-body canceled_emission_body collapse">{tabela}</div></div>')


def _aviso(n, de_n):
    return _tabela(n, f'<b>Proc. Administrativo {de_n}-41.039/2026</b> cancelado por <b>Ana Souza</b>'
                      '<span class="motivo-cancelamento">, com a seguinte justificativa: <br/><br/> '
                      '<p>Retificação de documentos. </p></span>')


PAGINA_COM_DESPACHO_CANCELADO = (
    _timeline(_tabela(6, "<p>Segue o edital para assinatura.</p>"), 30)
    + _timeline(_recolhido(_tabela(7, "<p>Solicito cancelar a licitação.</p>", cancelado=True)), 35)
    + _timeline(_aviso(9, 7), 45))


def test_aviso_de_despacho_cancelado_nao_e_cancelamento_do_processo():
    assert faltantes.cancelamento_em(faltantes.ler_despachos(PAGINA_COM_DESPACHO_CANCELADO)) is None


def test_despacho_cancelado_e_aviso_nao_sao_o_ultimo_despacho():
    assert [d["rotulo"] for d in faltantes.ler_despachos(PAGINA_COM_DESPACHO_CANCELADO)] == [
        "Despacho 6- 41.039/2026"]


def test_cancelamento_em_despacho_valido_continua_contando():
    html = PAGINA_COM_DESPACHO_CANCELADO + _timeline(_tabela(10, "<p>A demanda foi cancelada pela "
                                                                   "secretaria.</p>"), 50)
    d = faltantes.cancelamento_em(faltantes.ler_despachos(html))
    assert d is not None and d["rotulo"] == "Despacho 10- 41.039/2026"


# ---------------------------------------------------------------------------------------------
# Número do processo licitatório não é número de processo do 1Doc
# ---------------------------------------------------------------------------------------------
# Despacho real de formalização do Pregão: o "processo licitatório" e o "pregão eletrônico" têm
# numeração própria da licitação; o processo do 1Doc é o "processo administrativo digital".
FORMALIZACAO = ('<table class="relative despacho"><tr><td class="celula_despacho"><span class="texto_original">'
                '<p>Bom dia, Formalizado o PROCESSO ADMINISTRATIVO DIGITAL 40.001/2026, PROCESSO LICITATÓRIO '
                'número <strong>449/2026</strong>, PREGÃO ELETRÔNICO número <strong>164/2026</strong>, '
                'oriundo da Abertura de Procedimento Licitatório nº 38.364/2026.</p></span></td></tr></table>')


def test_citados_ignora_numero_do_processo_licitatorio():
    assert busca.mencoes_da_pagina(FORMALIZACAO, "41.039/2026", "") == [
        {"numero": "40.001/2026", "hash": "", "fonte": "texto"},
        {"numero": "38.364/2026", "hash": "", "fonte": "texto"}]


@pytest.mark.parametrize("modulo", [faltantes, verificar_planilha])
def test_citacoes_ignora_numero_do_processo_licitatorio(modulo):
    assert modulo.citacoes(FORMALIZACAO, {"numero": "41.039/2026", "ano": "2026"}) == (
        [], ["40.001/2026", "38.364/2026"])
