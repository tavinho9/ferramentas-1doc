"""Funções puras: datas, critérios de etapa, formulário da busca, log."""

import json
from datetime import date, datetime

import pytest

from ferramentas1doc import busca_argv, faltantes, processos_do_dia as pdd, verificar_planilha
from ferramentas1doc.gui.log import TerminalEmMemoria

HOJE = date(2026, 1, 15)


@pytest.mark.parametrize("txt,esperado", [
    ("15/01/2026", date(2026, 1, 15)),
    ("15/01/26", date(2026, 1, 15)),
    ("15-01-2026", date(2026, 1, 15)),
    ("15012026", date(2026, 1, 15)),
    ("10/01", date(2026, 1, 10)),
    ("20/12", date(2025, 12, 20)),          # sem ano e no futuro: ano anterior
    ("31/02/2026", None),
    ("abc", None),
])
def test_interpretar_data(txt, esperado):
    assert pdd.interpretar_data(txt, HOJE) == esperado


@pytest.mark.parametrize("txt,dentro", [
    ("Esta aqui 1 hora", True), ("Aqui há alguns segundos", True), ("Está aqui 45 minutos", True),
    ("Está aqui 3 dias", False), ("Está aqui 1 semana", False), ("", False), ("???", False),
])
def test_etapa_dentro_24h(txt, dentro):
    assert pdd.etapa_dentro_24h(txt) is dentro


def test_kanban_ignora_copia_do_modelo_de_despacho():
    html = ('<div id="emissao_principal"><div class="kanban_estagios" data-current="true">'
            '<div class="kanban_titulo"><p>Validação de Requisição</p></div>'
            '<div class="kanban_itens"><p class="kanban_item">Esta aqui 3 dias</p></div></div></div>'
            '<div id="parent_wa"><div class="kanban_estagios" data-current="true">'
            '<div class="kanban_titulo"><p>Validação de Requisição</p></div>'
            '<div class="kanban_itens"><p class="kanban_item">Aqui há alguns segundos</p></div></div></div>')
    assert pdd.ler_etapa_atual(html) == ("Validação de Requisição", "Esta aqui 3 dias")


def test_rotulo_etapa():
    assert pdd.rotulo_etapa("Etapa Final", "Abertura de Procedimento Licitatório", False) == \
        "Etapa Final — Abertura"
    assert pdd.rotulo_etapa("", "Elaboração de Documentação Preparatória", True) == "Elaboração"
    assert pdd.rotulo_etapa("Etapa Final", "Outra fase", False) == "Elaboração"


def test_montar_argv_completo():
    form = {"caixas": ["entrada", "arquivo"], "numeros": "31.632/2026 5712/2025",
            "etapa": "Pesquisa de Mercado; Consulta Orçamentária", "fase": "Abertura",
            "antes_de": "Pesquisa de Mercado", "emissao_de": "01/10/2026", "dias_etapa_min": "15",
            "prioridade": ["Urgente", "Alta"], "objeto": "medicamento", "refazer": True,
            "extras": '--setor-etapa CPPFI --numeros-arquivo "C:\\Users\\joao\\Área de Trabalho\\x.xlsx"'}
    argv = busca_argv.montar_argv(form, pasta_saida="C:\\saida", agora=datetime(2026, 11, 6, 9, 5))
    assert argv[:4] == ["--caixas", "entrada,arquivo", "--numeros", "31.632/2026,5712/2025"]
    assert argv.count("--etapa") == 2
    assert ["--prioridade", "Urgente,Alta"] == argv[argv.index("--prioridade"):argv.index("--prioridade") + 2]
    assert argv[-2:] == ["--numeros-arquivo", "C:\\Users\\joao\\Área de Trabalho\\x.xlsx"]
    assert busca_argv.validar(argv) is None


def test_montar_argv_todas_as_caixas_e_todas_prioridades():
    argv = busca_argv.montar_argv({"caixas": list(busca_argv.CAIXAS), "prioridade": list(busca_argv.PRIORIDADES)})
    assert argv == ["--caixas", "todas"]


@pytest.mark.parametrize("exemplo", busca_argv.EXEMPLOS_DATA)
def test_busca_aceita_os_formatos_da_instrucao_de_datas(exemplo):
    assert busca_argv.validar(["--emissao-de", exemplo, "--mudanca-ate", exemplo]) is None


def test_instrucao_de_datas_da_busca():
    for formato in ("DD/MM/AAAA", "AAAA-MM-DD", "hoje", "ontem", "-N"):
        assert formato in busca_argv.DICA_DATAS
    # a instrução pede o ano completo: sem ele, a busca recusa
    assert "data inválida" in busca_argv.validar(["--emissao-de", "05/11"])


ELABORACAO = ["Elaboração do DFD", "Elaboração do ETP", "Etapa Final"]


def test_ordenar_etapas():
    base = list(busca_argv.ETAPAS_BASE)
    assert busca_argv.ordenar_etapas([]) == base
    abertura_lida = [n.upper() if n == "Aprovação do COMPAC" else n for n in base]
    abertura_lida[0] = "Validacao de  requisicao"              # grafia diferente, mesma etapa
    etapas = busca_argv.ordenar_etapas([ELABORACAO, abertura_lida])
    # o fluxo da Abertura primeiro, na ordem e com a grafia lida; depois as outras, sem repetir
    assert etapas == ["Validacao de  requisicao", *base[1:5], "APROVAÇÃO DO COMPAC", "Etapa Final",
                      "Elaboração do DFD", "Elaboração do ETP"]


def test_etapas_conhecidas_do_cache(tmp_path):
    arq = tmp_path / "1doc_cache.json"
    assert busca_argv.etapas_conhecidas(str(arq)) == list(busca_argv.ETAPAS_BASE)      # sem cache
    arq.write_text("{ quebrado", encoding="utf-8")
    assert busca_argv.etapas_conhecidas(str(arq)) == list(busca_argv.ETAPAS_BASE)
    det = {"3|1/2026": {"etapas_quadro": ELABORACAO}, "3|2/2026": {"etapas_quadro": ELABORACAO},
           "3|3/2026": {"etapas_quadro": []}, "3|4/2026": {"numero": "sem quadro"}}
    arq.write_text(json.dumps({"versao": "1.1", "det": det}), encoding="utf-8")
    assert busca_argv.etapas_conhecidas(str(arq))[-2:] == ["Elaboração do DFD", "Elaboração do ETP"]
    det["3|5/2026"] = {"etapas_quadro": ["Análise Jurídica", "Etapa Final"]}       # uma busca nova
    arq.write_text(json.dumps({"versao": "1.1", "det": det}), encoding="utf-8")
    assert busca_argv.etapas_conhecidas(str(arq))[-1] == "Análise Jurídica"


@pytest.mark.parametrize("antes,escolhida,varios,esperado", [
    ("", "Pesquisa de Mercado", True, "Pesquisa de Mercado"),
    ("Pesquisa", "Pesquisa de Mercado", True, "Pesquisa de Mercado"),               # substitui
    ("Pesquisa de Mercado;", "Consulta Orçamentária", True, "Pesquisa de Mercado; Consulta Orçamentária"),
    ("Pesquisa de Mercado; ", "pesquisa de mercado", True, "Pesquisa de Mercado"),  # não repete
    ("Pesquisa de Mercado;", "Consulta Orçamentária", False, "Consulta Orçamentária"),  # campo de uma só
])
def test_juntar_etapa(antes, escolhida, varios, esperado):
    assert busca_argv.juntar_etapa(antes, escolhida, varios) == esperado


def test_campos_de_etapa_viram_opcoes_da_busca():
    for campo in busca_argv.CAMPOS_ETAPA:
        assert campo in busca_argv.TEXTOS or campo in busca_argv.TEXTOS_MULTIPLOS
    argv = busca_argv.montar_argv({"caixas": ["entrada"], "etapa": "Pesquisa de Mercado; Consulta Orçamentária",
                                   "antes_de": "Pesquisa de Mercado"})
    assert argv[2:] == ["--antes-de", "Pesquisa de Mercado", "--etapa", "Pesquisa de Mercado",
                        "--etapa", "Consulta Orçamentária"]


def test_montar_argv_novo_processo():
    base = {"caixas": ["entrada"]}
    argv = busca_argv.montar_argv({**base, "novo_processo": "Sim", "novo_processo_de": "01/11/2026",
                                   "novo_processo_ate": "-1"})
    assert argv[2:] == ["--novo-processo-de", "01/11/2026", "--novo-processo-ate", "-1", "--novo-processo"]
    assert busca_argv.validar(argv) is None
    assert busca_argv.montar_argv({**base, "novo_processo": "Não"})[2:] == ["--sem-novo-processo"]
    assert busca_argv.montar_argv({**base, "novo_processo_de": "hoje"})[2:] == ["--novo-processo-de", "hoje"]
    with pytest.raises(busca_argv.FormularioInvalido, match="não combina"):
        busca_argv.montar_argv({**base, "novo_processo": "Não", "novo_processo_ate": "hoje"})
    # a mesma regra vale para quem digita as opções
    assert "não combina" in busca_argv.validar(["--sem-novo-processo", "--novo-processo"])
    assert "data inválida" in busca_argv.validar(["--novo-processo-ate", "31/02/2026"])


def test_novo_processo_so_conta_processo():
    from ferramentas1doc import busca
    det = {"emissoes": [{"numero": "2/2026", "tipo": "Memorando", "data": "2026-11-05T10:00:00"},
                        {"numero": "9/2026", "tipo": "Proc. Administrativo", "data": "2026-11-06T09:00:00"},
                        {"numero": "8/2026", "tipo": "Proc. Administrativo", "data": "2026-11-01T09:00:00"}],
           "derivados": ["2/2026", "9/2026", "8/2026"]}
    assert [n["numero"] for n in busca.novos_processos(det)] == ["8/2026", "9/2026"]   # do mais antigo
    # detalhe gravado antes desta leitura: só os números, sem data
    assert [(n["numero"], n["data"]) for n in busca.novos_processos({"derivados": ["2/2026"]})] == [("2/2026", None)]
    assert busca.novos_processos({}) == []


def test_citados_no_texto_e_nos_despachos():
    from bs4 import BeautifulSoup
    from ferramentas1doc import busca
    html = """<div class="emissao_conteudo"><div class="texto_original">Referente à Fase Preparatória
      nº 30.500/2026, conforme a Lei nº 14.133/2021 e a requisição 77/2026. Este é o processo 41.000/2026.</div></div>
      <table class="despacho"><tr><td><div class="texto_original fr-view">A demanda prossegue no
      <a href="./?pg=doc/ver&hash=65edaaaa&origem=emissao_anexada">Proc. Administrativo 39.144/2026</a>;
      veja o <a href="./?pg=doc/ver&hash=7777bbbb">Memorando 512/2026</a> e o
      <a href="./?pg=doc/ver&hash=8888cccc">link</a>. Processo 39.144/2026 de novo.</div></td></tr></table>
      <div class="outro">processo 99.999/2026 fora dos blocos de texto</div>"""
    assert busca._citados(BeautifulSoup(html, "html.parser"), "41.000/2026", "HASHPROPRIO") == [
        {"numero": "30.500/2026", "hash": "", "fonte": "texto"},
        {"numero": "39.144/2026", "hash": "65EDAAAA", "fonte": "link"},   # link e número: um registro
        {"numero": "", "hash": "8888CCCC", "fonte": "link"}]              # sem número: confere-se ao abrir


def test_citados_mencao_do_1doc():
    """A menção escolhida na lista do 1Doc (data-mention="2"), com a marcação real: no corpo do
    despacho e na lista de documentos mencionados, fora do bloco de texto. Vale sem contexto antes
    do número e vem primeiro, mesmo que o número também apareça digitado."""
    from bs4 import BeautifulSoup
    from ferramentas1doc import busca
    html = """<table class="despacho"><tr><td class="celula_despacho"><div class="texto_original fr-view">
      Segue em <a class="mention label mention_2" href="https://app.1doc.com.br/?pg=doc/ver&amp;hash=5476D715A594730A44C82DCD&amp;itd=3&amp;gr=10&amp;origem=mention&amp;s=montesclaros" target="_blank" data-mention="2" data-tipo="2" data-id="55747711" rel="noreferrer noopener">Proc. Administrativo 26.831/2026 - Licitação da Construção da Praça de Santa Rita Rural</a>,
      processo 26.831/2026.</div>
      <a class=" pre_visualizar" data-mention="2" href="./?pg=doc/ver&amp;hash=5476D715A594730A44C82DCD&amp;itd=3&amp;origem=emissao_anexada">
        <i class="icon-folder-open"></i> Proc. Administrativo 26.831/2026 - Abertura de Procedimento Licitatório </a>
      <a class=" pre_visualizar" data-mention="2" href="./?pg=doc/ver&amp;hash=AAAA0000BBBB1111CCCC2222">
        <i class="icon-folder-open"></i> Proc. Administrativo 27.000/2026 - Fase Preparatória </a>
      <a class="mention label mention_1" data-mention="1" href="./?pg=usuario&amp;hash=FFFF">Maria</a>
      </td></tr></table>"""
    assert busca._citados(BeautifulSoup(html, "html.parser"), "1/2026", "") == [
        {"numero": "26.831/2026", "hash": "5476D715A594730A44C82DCD", "fonte": "mencao"},
        {"numero": "27.000/2026", "hash": "AAAA0000BBBB1111CCCC2222", "fonte": "mencao"}]


def test_texto_mencoes():
    """Coluna Menções: todas as citações, com (pai)/(filho) quando se sabe e a fonte fraca marcada."""
    from ferramentas1doc import busca
    citados = [{"numero": "28.000/2026", "hash": "", "fonte": "texto"},
               {"numero": "39.144/2026", "hash": "AB", "fonte": "mencao"},
               {"numero": "", "hash": "5476D715A594730A44C8", "fonte": "link"},
               {"numero": "", "hash": "FFFF000011112222", "fonte": "mencao"}]
    achar = {"5476D715A594730A44C8": "26.831/2026"}.get
    assert busca.texto_mencoes(citados, "28.000/2026", ["39.144/2026"], lambda c: achar(c["hash"])) == (
        "28.000/2026 (pai; número no texto), 39.144/2026 (filho), 26.831/2026, link FFFF000011…")
    assert busca.texto_mencoes([]) == ""


def _det(numero, assunto, emissao, pai="", tipo_doc="Proc. Administrativo", citados=(), hash_=""):
    return {"numero": numero, "tipo_doc": tipo_doc, "quadro": assunto, "tipo": assunto, "pai": pai,
            "emissao": emissao, "citados": [{"numero": c, "hash": ""} for c in citados], "hash": hash_,
            "etapa": "Validação de Requisição", "url": f"u/{numero}"}


FASE, ABERTURA = "Fase Preparatória de Procedimento Licitatório", "Abertura de Procedimento Licitatório"
ELAB = "Elaboração de Documentação Preparatória"


def test_aceita_mencao():
    from ferramentas1doc import busca
    fase = _det("30.500/2026", FASE, "2026-11-01T09:00:00")
    assert busca.aceita_mencao(fase, _det("41.000/2026", ABERTURA, "2026-11-05T09:00:00"))
    assert busca.aceita_mencao(fase, _det("41.000/2026", ABERTURA, "2026-11-05T09:00:00", pai="30.500/2026"))
    assert busca.aceita_mencao(_det("1/2026", ELAB, "2026-10-01T09:00:00"), fase)
    assert busca.aceita_mencao(_det("1/2026", "Outro assunto", "2026-10-01T09:00:00"), fase)
    rejeitados = [
        _det("41.000/2026", ABERTURA, "2026-11-05T09:00:00", pai="28.000/2026"),     # emitida de outro processo
        _det("41.000/2026", ABERTURA, "2026-10-05T09:00:00"),                        # anterior à fase
        _det("41.000/2026", "Pagamento de Fornecedor", "2026-11-05T09:00:00"),       # fora da licitação
        _det("30.900/2026", FASE, "2026-11-05T09:00:00"),                            # outra fase: referência
        _det("41.000/2026", ABERTURA, "2026-11-05T09:00:00", tipo_doc="Memorando"),
        _det("30.500/2026", ABERTURA, "2026-11-05T09:00:00"),                        # ele mesmo
    ]
    assert [busca.aceita_mencao(fase, d) for d in rejeitados] == [False] * len(rejeitados)


def test_novos_processos_por_vinculo():
    from ferramentas1doc import busca
    fase = {**_det("30.500/2026", FASE, "2026-11-01T09:00:00", citados=["41.000/2026", "40.001/2026"]),
            "emissoes": [{"numero": "40.001/2026", "tipo": "Proc. Administrativo", "data": "2026-11-02T10:00:00"}]}
    fase["citados"][0]["fonte"] = "mencao"
    abertura = _det("41.000/2026", ABERTURA, "2026-11-05T09:00:00", citados=["30.500/2026"])
    abertura["citados"][0]["fonte"] = "texto"
    uni = busca.Universo([
        fase,
        _det("40.001/2026", ABERTURA, "2026-11-02T10:00:00", pai="30.500/2026", citados=["30.500/2026"]),
        abertura,
        _det("41.050/2026", ABERTURA, "2026-11-06T09:00:00", pai="30.500/2026"),
        _det("41.100/2026", ABERTURA, "2026-11-07T09:00:00", pai="28.000/2026", citados=["30.500/2026"]),
    ])
    novos = busca.novos_processos(fase, uni)
    # emissão normal basta (a menção e a origem do mesmo processo não se somam); menções se somam entre si
    assert [(n["numero"], n["vinculos"]) for n in novos] == [
        ("40.001/2026", ["emissão"]), ("41.000/2026", ["citado neste", "cita este"]), ("41.050/2026", ["origem"])]
    assert novos[1]["data"] == datetime(2026, 11, 5, 9, 0) and novos[1]["etapa"] == "Validação de Requisição"
    assert novos[1]["fontes"] == ["mencao", "texto"]          # a menção do 1Doc vem antes do número digitado
    assert busca.texto_novos(novos) == ("40.001/2026 (emissão), 41.000/2026 (citado neste + cita este), "
                                        "41.050/2026 (origem)")
    # só a emissão normal: fica a origem, saem as menções
    assert [n["numero"] for n in busca.novos_processos(fase, uni, mencoes=False)] == ["40.001/2026", "41.050/2026"]


def test_montar_argv_erros():
    with pytest.raises(busca_argv.FormularioInvalido, match="caixa"):
        busca_argv.montar_argv({"caixas": []})
    with pytest.raises(busca_argv.FormularioInvalido, match="inteiro"):
        busca_argv.montar_argv({"caixas": ["entrada"], "dias_etapa_min": "dez"})
    assert "data inválida" in busca_argv.validar(["--emissao-de", "31/31/2026"])
    assert "--nao-existe" in busca_argv.validar(["--nao-existe"])


@pytest.mark.parametrize("txt,esperado", [
    ("", None), ("  ", None), (None, None),
    ("hoje", HOJE), ("Ontem", date(2026, 1, 14)), ("-30", date(2025, 12, 16)), ("- 7", date(2026, 1, 8)),
    ("01/01/2025", date(2025, 1, 1)), ("1/1/25", date(2025, 1, 1)), ("10/01", date(2026, 1, 10)),
    ("20/12", date(2025, 12, 20)),          # sem ano e no futuro: ano anterior
])
def test_faltantes_ler_data(txt, esperado):
    assert faltantes.ler_data(txt, HOJE) == esperado


@pytest.mark.parametrize("txt", ["31/02/2026", "abc", "-99999999999", "2026"])
def test_faltantes_ler_data_invalida(txt):
    with pytest.raises(ValueError):
        faltantes.ler_data(txt, HOJE)


def test_faltantes_ler_periodo():
    assert faltantes.ler_periodo("", "", HOJE) == (None, None)
    assert faltantes.ler_periodo("01/12/2025", "ontem", HOJE) == (date(2025, 12, 1), date(2026, 1, 14))
    assert faltantes.ler_periodo("", "31/12/2020", HOJE) == (None, date(2020, 12, 31))
    with pytest.raises(ValueError, match="Data final não reconhecida"):
        faltantes.ler_periodo("", "32/01/2026", HOJE)
    with pytest.raises(ValueError, match="ainda não chegou"):
        faltantes.ler_periodo("16/01/2026", "", HOJE)
    with pytest.raises(ValueError, match="depois da final"):
        faltantes.ler_periodo("10/01/2026", "09/01/2026", HOJE)


def test_faltantes_resolver_periodo():
    itens = [{"ano": "2026"}, {"ano": "2024"}]
    assert faltantes.resolver_periodo(None, None, itens, HOJE) == (date(2024, 1, 1), HOJE)
    assert faltantes.resolver_periodo(None, None, [], HOJE) == (date(2026, 1, 1), HOJE)
    assert faltantes.resolver_periodo(date(2025, 6, 1), date(2027, 1, 1), itens, HOJE) == \
        (date(2025, 6, 1), HOJE)                                    # fim no futuro: hoje
    with pytest.raises(SystemExit, match="Período vazio"):
        faltantes.resolver_periodo(None, date(2023, 12, 31), itens, HOJE)


def test_faltantes_data_de_emissao_na_listagem():
    html = ('<table><tr><td><a class="link_emissao_a" href="./?pg=doc/ver&hash=AAAA1111BBBB2222CCCC">'
            'Proc. Administrativo 31.632/2026 <small class="data">05/11/2026 09:15</small></a></td></tr>'
            '<tr><td><a class="link_emissao_a" href="./?pg=doc/ver&hash=DDDD3333EEEE4444FFFF">'
            'Proc. Administrativo 31.700/2026</a></td></tr></table>')
    assert [ln["emissao"] for ln in faltantes.linhas_listagem(html)] == [date(2026, 11, 5), None]


# O despacho pode vir com outras classes junto de texto_original.
DESPACHOS_COM_VARIAS_CLASSES = (
    '<table class="relative despacho"><tr><td class="celula_despacho">'
    '<div class="texto_original">Segue para o processo nº 31.700/2026.</div></td></tr></table>'
    '<table class="relative despacho"><tr><td class="celula_despacho">'
    '<div class="texto_original fr-view">Emitido o Proc. Administrativo 31.800/2026 '
    '(<a href="./?pg=doc/ver&hash=AAAA1111BBBB2222CCCC">ver</a>).</div></td></tr></table>'
    '<table class="relative despacho"><tr><td class="celula_despacho">'
    '<div class="fr-view texto_original">Abertura: processo 31.900/2026.</div></td></tr></table>'
    '<p class="rodape">Processo 32.000/2026 fora dos despachos</p>')


@pytest.mark.parametrize("modulo", [faltantes, verificar_planilha])
def test_citacoes_le_despacho_com_varias_classes(modulo):
    proprio = {"numero": "31.632/2026", "ano": "2026"}
    assert modulo.citacoes(DESPACHOS_COM_VARIAS_CLASSES, proprio) == (
        ["AAAA1111BBBB2222CCCC"], ["31.700/2026", "31.800/2026", "31.900/2026"])


@pytest.mark.parametrize("modulo", [faltantes, verificar_planilha])
def test_citacoes_le_mencao_do_1doc_fora_do_texto(modulo):
    """A lista de documentos mencionados fica fora do .texto_original; a menção a usuário não conta."""
    html = ('<table class="relative despacho"><tr><td class="celula_despacho">'
            '<div class="texto_original fr-view">Segue.</div>'
            '<a class=" pre_visualizar" data-mention="2" href="./?pg=doc/ver&amp;hash=5476D715A594730A44C82DCD'
            '&amp;itd=3&amp;origem=emissao_anexada"><i class="icon-folder-open"></i> Proc. Administrativo '
            '26.831/2026 - Abertura de Procedimento Licitatório</a>'
            '<a class="mention label mention_1" data-mention="1" href="./?pg=x&amp;hash=FFFF0000FFFF0000FFFF">'
            'Maria</a></td></tr></table>')
    assert modulo.citacoes(html, {"numero": "1/2026", "ano": "2026"}) == (["5476D715A594730A44C82DCD"], [])


# Menção a uma pessoa no corpo do despacho, com a marcação real do 1Doc: o href é a página que quem
# escreveu tinha aberta (aqui, outro processo, porque o texto foi colado), não a pessoa.
MENCAO_A_PESSOA = (
    '<a class="mention label mention_1" style="color:#ffffff;font-size:12px;" '
    'href="https://montesclaros.1doc.com.br/?pg=doc%2Fver&amp;hash=3AD9FC350E70AF6872C1B7A2&amp;itd=3'
    '&amp;s=montesclaros&amp;origem=painel_setor&amp;caixa=entrada&amp;marcador=963214&amp;ordenar=2'
    '&amp;ordem=desc" data-mention="1" data-tipo="1" data-id="511" data-id-setor="351">'
    'Maria da Silva - SS-DAF</a>')
DESPACHO_COM_PESSOA = (
    '<table class="relative despacho"><tr><td class="celula_despacho"><div class="texto_original fr-view">'
    f'Prezada {MENCAO_A_PESSOA}, segue para ciência. Ver também '
    '<a class="mention label mention_2" data-mention="2" href="./?pg=doc/ver&amp;hash=5476D715A594730A44C82DCD">'
    'Proc. Administrativo 26.831/2026 - Abertura</a>.</div></td></tr></table>')


def test_citados_ignora_mencao_a_pessoa_no_despacho():
    from ferramentas1doc import busca
    assert busca.mencoes_da_pagina(DESPACHO_COM_PESSOA, "1/2026", "") == [
        {"numero": "26.831/2026", "hash": "5476D715A594730A44C82DCD", "fonte": "mencao"}]


@pytest.mark.parametrize("modulo", [faltantes, verificar_planilha])
def test_citacoes_ignora_mencao_a_pessoa_no_despacho(modulo):
    # o número do texto da menção ao documento também vale; o hash da página de quem escreveu, não
    assert modulo.citacoes(DESPACHO_COM_PESSOA, {"numero": "1/2026", "ano": "2026"}) == (
        ["5476D715A594730A44C82DCD"], ["26.831/2026"])


# O editor do 1Doc às vezes parte o número com a formatação. O navegador mostra o número inteiro;
# lido com espaço entre os pedaços, "40. 001/2026" virava 1/2026, ou a citação sumia.
NUMEROS_PARTIDOS = ["40.<span>001/2026</span>", "<span>40.</span><span>001/2026</span>",
                    "<b>40.</b>001/2026", "40<span>.001/2026</span>", "40.001<span>/2026</span>",
                    "40.<strong><em>001</em></strong>/2026"]


@pytest.mark.parametrize("numero", NUMEROS_PARTIDOS)
def test_citados_numero_partido_pela_formatacao(numero):
    from ferramentas1doc import busca
    html = f'<div class="texto_original fr-view">Segue o processo nº {numero} para análise.</div>'
    assert busca.mencoes_da_pagina(html, "1/2026", "") == [
        {"numero": "40.001/2026", "hash": "", "fonte": "texto"}]


@pytest.mark.parametrize("modulo", [faltantes, verificar_planilha])
@pytest.mark.parametrize("numero", NUMEROS_PARTIDOS)
def test_citacoes_numero_partido_pela_formatacao(modulo, numero):
    html = f'<div class="texto_original fr-view">Segue o processo nº {numero} para análise.</div>'
    assert modulo.citacoes(html, {"numero": "1/2026", "ano": "2026"}) == ([], ["40.001/2026"])


# Data no despacho, com contexto de processo antes: o 11/2026 de "12/11/2026" não é um processo.
DESPACHO_COM_DATAS = ('<div class="texto_original fr-view">Recebido no dia 12/11/2026 e autuado em '
                      '1/11/2026; segue o processo nº 40.001/2026 e o nº 3/2026.</div>')


@pytest.mark.parametrize("modulo", [faltantes, verificar_planilha])
def test_citacoes_data_nao_vira_processo(modulo):
    assert modulo.citacoes(DESPACHO_COM_DATAS, {"numero": "1/2026", "ano": "2026"}) == (
        [], ["40.001/2026", "3/2026"])


def test_texto_corrido():
    """Em linha, o texto continua; bloco, quebra de linha e célula separam. Comentário e script
    ficam de fora, como no get_text."""
    from bs4 import BeautifulSoup
    from ferramentas1doc import busca
    html = ('<div>Proc. n<sup>o</sup> 4<b>0</b>.001/2026<br>Lei 14.133/2021<p>processo 12</p>345/2026'
            '<!-- 99.999/2026 --><script>var x = "88.888/2026";</script><table><tr><td>1</td>'
            '<td>2</td></tr></table></div>')
    texto = " ".join(busca.texto_corrido(BeautifulSoup(html, "html.parser")).split())
    assert texto == "Proc. no 40.001/2026 Lei 14.133/2021 processo 12 345/2026 1 2"


@pytest.mark.parametrize("modulo", [faltantes, verificar_planilha])
def test_processo_novo_do_cache_precisa_continuar_citado(modulo, monkeypatch):
    """Processo novo guardado no cache só pela citação (sem origem no cabeçalho) volta só se a
    página ainda o citar: o vínculo achado antes pelo link de uma menção a pessoa não se repete."""
    import threading
    from types import SimpleNamespace
    import fake1doc
    h_novo = "3AD9FC350E70AF6872C1B7A2"
    monkeypatch.setitem(fake1doc.PROCESSOS, h_novo, {**fake1doc.PROCESSOS["AAAA1111BBBB2222CCCC"],
                                                     "numero": "39.144/2026"})
    pagina_novo = fake1doc.detalhe(h_novo)   # Abertura sem origem: só vale citada
    monkeypatch.setattr(modulo, "USAR_INDICE_ORIGEM", False)
    monkeypatch.setattr(modulo, "abrir", lambda ctx, item, link=None: (
        (h_novo, pagina_novo, "cache", "") if item["numero"] == "39.144/2026" else None))
    monkeypatch.setattr(modulo, "baixar", lambda sessao, url, **kw: pagina_novo if h_novo in url else "")
    elab = {"numero": "31.000/2026", "ano": "2026", "hash": "ELAB0000ELAB0000ELAB"}

    def novos(despacho):
        ctx = SimpleNamespace(cache={"novo:31.000/2026": ["39.144/2026"]}, lock_cache=threading.Lock(),
                              sessao=None)
        html = (f'<table class="relative despacho"><tr><td class="celula_despacho">'
                f'<div class="texto_original fr-view">{despacho}</div></td></tr></table>')
        return [(n["numero"], n["vinculo"]) for n in modulo.buscar_novos(ctx, elab, html)]

    assert novos(f"Prezada {MENCAO_A_PESSOA}, segue.") == []
    assert novos("A demanda segue no processo nº 39.144/2026.") == [("39.144/2026", "citado em despacho")]
    assert novos(f'Segue <a class="mention label mention_2" data-mention="2" href="./?pg=doc/ver&amp;hash={h_novo}">'
                 'Proc. Administrativo 39.144/2026 - Abertura</a>.') == [("39.144/2026", "citado em despacho")]


def test_terminal_reescreve_linha_com_retorno_de_carro():
    t = TerminalEmMemoria()
    for pedaco in ["Lendo caixas\n", "  Em aberto: página 1/3", "\r  Em aberto: página 2/3",
                   "\r", "  Em aberto: página 3/3, 25 processos", "\n", "fim\r\n"]:
        t.alimentar(pedaco)
    assert t.linhas == ["Lendo caixas", "  Em aberto: página 3/3, 25 processos", "fim", ""]
