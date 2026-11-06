"""Funções puras: datas, critérios de etapa, formulário da busca, log."""

from datetime import date, datetime

import pytest

from ferramentas1doc import busca_argv, faltantes, processos_do_dia as pdd
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


def test_terminal_reescreve_linha_com_retorno_de_carro():
    t = TerminalEmMemoria()
    for pedaco in ["Lendo caixas\n", "  Em aberto: página 1/3", "\r  Em aberto: página 2/3",
                   "\r", "  Em aberto: página 3/3, 25 processos", "\n", "fim\r\n"]:
        t.alimentar(pedaco)
    assert t.linhas == ["Lendo caixas", "  Em aberto: página 3/3, 25 processos", "fim", ""]
