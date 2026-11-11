"""As quatro ferramentas de ponta a ponta contra o 1Doc de mentira (fake1doc).

Confere a ligação nova (sessão por cookies, executar(), pasta de saída,
cache na pasta de dados) e que as regras de antes continuam dando o mesmo
resultado nos dois processos de exemplo.
"""

import json
import os
from datetime import date, datetime, time, timedelta

import pytest
from openpyxl import Workbook, load_workbook

import fake1doc


def _planilha(caminho, numeros):
    wb = Workbook()
    ws = wb.active
    ws.title = "Acompanhamento"
    ws.append(["Nº Processo", "Prioridade", "Objeto", "Etapa", "Secretaria"])
    for n in numeros:
        ws.append([n, "Alta", "objeto", "etapa", "Secretaria de Saúde"])
    wb.save(caminho)
    return str(caminho)


def _valores(caminho, aba=None):
    wb = load_workbook(caminho)
    ws = wb[aba] if aba else wb.active
    return [[c.value for c in linha] for linha in ws.iter_rows()]


def test_processos_do_dia(dados, cred, monkeypatch):
    from ferramentas1doc import processos_do_dia
    monkeypatch.setattr(processos_do_dia, "PAUSA_REQ", 0)
    fake = fake1doc.instalar(monkeypatch)
    caminho = processos_do_dia.executar(date.today(), cred)
    assert caminho == os.path.join(str(dados["saida"]), f"processos novos {date.today():%d.%m}.xlsx")
    linhas = _valores(caminho)
    assert linhas[0] == ["Nº Processo", "Prioridade", "Objeto", "Etapa", "Secretaria"]
    # hoje: só entra quem está na etapa atual há menos de 24h (P1, "Esta aqui 2 horas")
    assert [l[0] for l in linhas[1:]] == ["31.632/2026"]
    assert linhas[1][2] == "Aquisição de medicamentos para a rede municipal de saúde"   # campo da abertura
    assert linhas[1][3] == "Pesquisa de Mercado — Abertura"
    assert linhas[1][4] == "Secretaria de Saúde"
    js = json.load(open(os.path.join(str(dados["saida"]), "processos_do_dia.json"), encoding="utf-8"))
    assert js["total"] == 2 and js["dia"] == f"{date.today():%d/%m/%Y}"
    p2 = [i for i in js["itens"] if i["numero"] == "31.700/2026"][0]
    assert p2["incluir"] is False and p2["motivo"] == "etapa atual há 1 dia ou mais"
    assert ("GET", "caixa") in fake.pedidos


def test_processos_do_dia_rejeita_futuro(dados, cred):
    from ferramentas1doc import processos_do_dia
    with pytest.raises(SystemExit, match="ainda não chegou"):
        processos_do_dia.executar(date.today() + timedelta(days=1), cred)


def test_processos_do_dia_sessao_caida_apaga_a_salva(dados, cred, monkeypatch):
    from ferramentas1doc import processos_do_dia, sessao
    sessao.salvar(cred)
    fake1doc.instalar(monkeypatch, logado=False)
    assert processos_do_dia.executar(date.today(), cred) is None
    assert sessao.carregar_salva() is None


def test_cancelar_interrompe(dados, cred, monkeypatch):
    from ferramentas1doc import processos_do_dia, sessao
    fake1doc.instalar(monkeypatch)
    sessao.CANCELAR.set()
    with pytest.raises(sessao.Cancelado):
        processos_do_dia.executar(date.today(), cred)


def test_busca(dados, cred, monkeypatch):
    from ferramentas1doc import busca, busca_argv
    fake1doc.instalar(monkeypatch)
    argv = busca_argv.montar_argv({"caixas": ["entrada"], "etapa": "Pesquisa de Mercado", "csv": True},
                                  pasta_saida=str(dados["saida"]))
    argv += ["--pausa", "0"]
    res = busca.main(argv, cred=cred)
    assert [r["numero"] for r in res["regs"]] == ["31.632/2026"]
    assert os.path.dirname(res["arquivo"]) == str(dados["saida"])
    assert [l[0] for l in _valores(res["arquivo"], "Resultado")] == ["Nº Processo", "31.632/2026"]
    assert any(f.endswith(".csv") for f in os.listdir(str(dados["saida"])))
    assert os.path.isfile(os.path.join(str(dados["dados"]), "1doc_cache.json"))     # cache na pasta de dados

    # a mesma consulta sem acessar o 1Doc, só com o cache
    fake1doc.instalar(monkeypatch, logado=False)
    res2 = busca.main(["--offline", "--antes-de", "Pesquisa de Mercado", "--sem-xlsx"])
    assert [r["numero"] for r in res2["regs"]] == ["31.700/2026"]


def test_busca_novo_processo(dados, cred, monkeypatch):
    from ferramentas1doc import busca, busca_argv
    tres_dias = datetime.combine(date.today() - timedelta(days=3), time(14, 30))
    monkeypatch.setitem(fake1doc.PROCESSOS["AAAA1111BBBB2222CCCC"], "emissoes", [
        ("Proc. Administrativo", "40.001/2026", "9999AAAA0000BBBB1111", tres_dias),
        ("Memorando", "512/2026", "8888CCCC9999DDDD0000", datetime.combine(date.today(), time(8, 0)))])
    fake = fake1doc.instalar(monkeypatch)
    argv = busca_argv.montar_argv({"caixas": ["entrada"], "novo_processo": "Sim"}, pasta_saida=str(dados["saida"]))
    res = busca.main(argv + ["--pausa", "0"], cred=cred)
    assert [r["numero"] for r in res["regs"]] == ["31.632/2026"]
    assert res["regs"][0]["novo_processo_em"] == tres_dias
    assert res["regs"][0]["derivados"] == "40.001/2026, 512/2026"   # a coluna de antes segue com tudo
    linhas = _valores(res["arquivo"], "Resultado")
    assert linhas[1][linhas[0].index("Novo processo emitido em")] == tres_dias
    # só o processo: o memorando emitido a partir dele fica de fora
    assert _valores(res["arquivo"], "Novos processos") == [
        ["Nº Processo", "Objeto", "Novo processo", "Vínculo", "Emitido em", "Autor", "Setor",
         "Etapa atual do processo", "Etapa do novo processo"],
        ["31.632/2026", "Aquisição de medicamentos", "40.001/2026", "emissão", tres_dias, "Maria da Silva", "SS-DAF",
         "Pesquisa de Mercado", None]]
    assert linhas[1][linhas[0].index("Novos processos (vínculo)")] == "40.001/2026 (emissão)"
    link = load_workbook(res["arquivo"])["Novos processos"]["C2"].hyperlink.target
    assert "hash=9999AAAA0000BBBB1111" in link
    assert "emitiu novo processo a partir dele" in dict(res["params"])["Filtros"]

    fake1doc.instalar(monkeypatch, logado=False)

    def offline(*opcoes):
        return busca.main(["--offline", "--sem-xlsx", *opcoes])

    assert [r["numero"] for r in offline("--novo-processo-de", "-5", "--novo-processo-ate", "ontem")["regs"]] \
        == ["31.632/2026"]
    assert offline("--novo-processo-de", "hoje")["regs"] == []       # o de hoje é memorando, não processo
    assert [r["numero"] for r in offline("--sem-novo-processo")["regs"]] == ["31.700/2026"]
    # com período, a aba Novos processos diz quais caíram nele
    saida = os.path.join(str(dados["saida"]), "periodo.xlsx")
    busca.main(["--offline", "--saida", saida, "--novo-processo-de", "-5"])
    assert [l[5] for l in _valores(saida, "Novos processos")] == ["No período filtrado?", "sim"]

    # Cache gravado antes de a busca ler as emissões (só com os números em "derivados")
    arq = os.path.join(str(dados["dados"]), "1doc_cache.json")
    with open(arq, encoding="utf-8") as f:
        cache = json.load(f)
    for det in cache["det"].values():
        del det["emissoes"]
    with open(arq, "w", encoding="utf-8") as f:
        json.dump(cache, f)
    # sem acessar o 1Doc, o período não tem como ser conferido: lacuna declarada, não chute
    res = offline("--novo-processo-de", "-5")
    assert res["regs"] == []
    assert [(f["numero"], f["motivo"]) for f in res["falhas"]] == [
        ("31.632/2026", "cache sem a data do novo processo (modo offline)")]
    assert [r["numero"] for r in offline("--novo-processo")["regs"]] == ["31.632/2026"]
    # com acesso, só o processo que tem derivado é reaberto; o outro vem do cache
    fake = fake1doc.instalar(monkeypatch)
    res = busca.main(["--novo-processo-de", "-5", "--sem-xlsx", "--pausa", "0"], cred=cred)
    assert [r["numero"] for r in res["regs"]] == ["31.632/2026"] and not res["falhas"]
    assert fake.pedidos.count(("GET", "doc/ver")) == 1


FASE_PREP = "Fase Preparatória de Procedimento Licitatório"
ETAPAS_FASE = [("Elaboração de Documentos", "Ficou aqui 2 dias", False), ("Etapa Final", "Esta aqui 5 dias", True)]


def _mencoes(monkeypatch):
    """Fases preparatórias na caixa Em aberto e as aberturas que saíram delas sem a emissão normal
    ('a partir deste documento'): a abertura cita a fase, ou a fase cita a abertura."""
    from ferramentas1doc import busca
    hoje = datetime.combine(busca.agora_br().date(), time(9, 15))   # o "-1" da busca conta do dia em Brasília
    proc = {
        # a Abertura 41.000 (Arquivados) cita esta: "cita este"
        "FP01AAAA0000000000AA": {"numero": "30.500/2026", "objeto": "Fase de medicamentos", "assunto": FASE_PREP,
                                 "etapas": ETAPAS_FASE, "prioridade": "Alta", "emissao_dt": hoje - timedelta(days=10),
                                 "despacho": "Concluída a fase preparatória."},
        # esta cita a Abertura 39.144, que não está em caixa nenhuma do setor: "citado neste"
        "FP02AAAA0000000000BB": {"numero": "30.600/2026", "objeto": "Fase de merenda", "assunto": FASE_PREP,
                                 "etapas": ETAPAS_FASE, "prioridade": "Alta", "emissao_dt": hoje - timedelta(days=10),
                                 "despacho": 'A demanda prossegue no <a href="./?pg=doc/ver&hash=AB02AAAA0000000000BB'
                                             '&itd=3&origem=emissao_anexada">Proc. Administrativo 39.144/2026</a>.'},
        # citada só por quem não conta: abertura com outra origem e processo de pagamento
        "FP03AAAA0000000000CC": {"numero": "30.700/2026", "objeto": "Fase de obras", "assunto": FASE_PREP,
                                 "etapas": ETAPAS_FASE, "prioridade": "Alta", "emissao_dt": hoje - timedelta(days=10),
                                 "despacho": "Aguardando os ajustes da secretaria."},
        "AB01AAAA0000000000AA": {"numero": "41.000/2026", "objeto": "Medicamentos", "caixa": "arquivo",
                                 "etapas": fake1doc.PROCESSOS["DDDD3333EEEE4444FFFF"]["etapas"], "prioridade": "Alta",
                                 "emissao_dt": hoje - timedelta(days=2),
                                 "despacho": "Abertura referente à Fase Preparatória nº 30.500/2026."},
        "AB02AAAA0000000000BB": {"numero": "39.144/2026", "objeto": "Merenda", "caixa": None,
                                 "etapas": fake1doc.PROCESSOS["DDDD3333EEEE4444FFFF"]["etapas"], "prioridade": "Alta",
                                 "emissao_dt": hoje - timedelta(days=1), "despacho": "Para validação."},
        "AB03AAAA0000000000CC": {"numero": "41.100/2026", "objeto": "Obras", "caixa": "arquivo",
                                 "etapas": fake1doc.PROCESSOS["DDDD3333EEEE4444FFFF"]["etapas"], "prioridade": "Alta",
                                 "emissao_dt": hoje - timedelta(days=1), "pai": ("28.000/2026", "ZZZZ0000ZZZZ0000ZZZZ"),
                                 "despacho": "Ver também o processo 30.700/2026."},
        "PG01AAAA0000000000CC": {"numero": "41.200/2026", "objeto": "Pagamento", "caixa": "saida",
                                 "assunto": "Pagamento de Fornecedor", "etapas": ETAPAS_FASE, "prioridade": "Baixa",
                                 "emissao_dt": hoje, "despacho": "Pagamento do processo nº 30.700/2026."},
    }
    for h, p in proc.items():
        monkeypatch.setitem(fake1doc.PROCESSOS, h, p)
        monkeypatch.setitem(fake1doc.POR_NUMERO, p["numero"].replace(".", ""), h)
    return hoje


def test_busca_novo_processo_por_mencao(dados, cred, monkeypatch):
    from ferramentas1doc import busca
    hoje = _mencoes(monkeypatch)
    fake = fake1doc.instalar(monkeypatch)

    def buscar(*opcoes, offline=False):
        base = ["--offline"] if offline else ["--pausa", "0"]
        return busca.main([*base, "--fase", "Elaboração", "--sem-xlsx", *opcoes], cred=None if offline else cred)

    saida = os.path.join(str(dados["saida"]), "mencoes.xlsx")
    res = busca.main(["--pausa", "0", "--fase", "Elaboração", "--novo-processo", "--saida", saida], cred=cred)
    assert [r["numero"] for r in res["regs"]] == ["30.500/2026", "30.600/2026"]
    assert [r["novos_texto"] for r in res["regs"]] == ["41.000/2026 (cita este)", "39.144/2026 (citado neste)"]
    # a data é a de emissão do processo novo
    assert [r["novo_processo_em"].date() for r in res["regs"]] == [(hoje - timedelta(days=2)).date(),
                                                                  (hoje - timedelta(days=1)).date()]
    aba = _valores(saida, "Novos processos")
    assert [(l[0], l[2], l[3], l[8]) for l in aba[1:]] == [
        ("30.500/2026", "41.000/2026", "cita este", "Validação de Requisição"),
        ("30.600/2026", "39.144/2026", "citado neste", "Validação de Requisição")]
    assert "hash=AB02AAAA0000000000BB" in load_workbook(saida)["Novos processos"]["C3"].hyperlink.target
    params = dict(res["params"])
    assert "(emissão normal ou menção)" in params["Filtros"]
    # 5 da família em Em aberto (já abertos pela busca) e 2 aberturas nos Arquivados; o pagamento fica de fora
    assert "índice das caixas: 7 processo(s) da família da licitação" in params["Menções"]
    assert "(2 aberto(s) agora); citados nos despachos abertos agora: 1" in params["Menções"]
    # o índice leu as quatro caixas; a abertura fora delas foi aberta pelo link do despacho
    assert len([q for q in fake.intervalos if q]) == 4
    assert ("GET", "doc/ver") in fake.pedidos

    assert [r["numero"] for r in buscar("--sem-novo-processo")["regs"]] == ["30.700/2026"]
    assert [r["numero"] for r in buscar("--novo-processo-de", "-1")["regs"]] == ["30.600/2026"]
    # só a emissão normal: nenhuma das três tem o evento, e o índice não é montado
    fake.pedidos.clear()
    assert buscar("--sem-mencoes", "--novo-processo")["regs"] == []
    assert fake.pedidos.count(("GET", "caixa")) == 1                 # só a caixa Em aberto
    assert "--sem-mencoes" in dict(buscar("--sem-mencoes", "--novo-processo")["params"])["Menções"]

    # de novo, com o cache: nada é reaberto (a última movimentação não mudou)
    fake.pedidos.clear()
    assert [r["numero"] for r in buscar("--novo-processo")["regs"]] == ["30.500/2026", "30.600/2026"]
    assert ("GET", "doc/ver") not in fake.pedidos
    # sem acessar o 1Doc, com o que ficou no cache
    fake1doc.instalar(monkeypatch, logado=False)
    assert [r["numero"] for r in buscar("--novo-processo", offline=True)["regs"]] == ["30.500/2026", "30.600/2026"]
    assert [r["numero"] for r in buscar("--sem-novo-processo", offline=True)["regs"]] == ["30.700/2026"]

    # Cache gravado antes de a busca ler as menções: no --offline, lacuna declarada
    arq = os.path.join(str(dados["dados"]), "1doc_cache.json")
    with open(arq, encoding="utf-8") as f:
        cache = json.load(f)
    for det in cache["det"].values():
        del det["citados"]
    with open(arq, "w", encoding="utf-8") as f:
        json.dump(cache, f)
    res = buscar("--sem-novo-processo", offline=True)
    assert res["regs"] == [] and {f["motivo"] for f in res["falhas"]} == {
        "cache sem as menções a outros processos (modo offline)"}
    # sem o filtro de novo processo, o cache antigo serve como antes
    assert len(buscar(offline=True)["regs"]) == 3
    # com acesso, reabre o que não tem as menções e volta ao mesmo resultado
    fake1doc.instalar(monkeypatch)
    assert [r["numero"] for r in buscar("--sem-novo-processo")["regs"]] == ["30.700/2026"]


def _preparar_faltantes(dados, monkeypatch):
    from ferramentas1doc import faltantes
    monkeypatch.setattr(faltantes, "PAUSA_REQ", 0)
    monkeypatch.setattr(faltantes, "ARQ_CACHE", os.path.join(str(dados["dados"]), "hash_cache_1doc.json"))
    return faltantes, fake1doc.instalar(monkeypatch)


def test_faltantes(dados, cred, monkeypatch):
    faltantes, fake = _preparar_faltantes(dados, monkeypatch)
    planilha = _planilha(dados["dados"] / "acomp.xlsx", ["31.632/2026"])
    destino = faltantes.executar(planilha, None, cred)
    assert destino and os.path.dirname(destino) == str(dados["saida"])
    linhas = _valores(destino, "Faltantes")
    # P2 está antes da Pesquisa de Mercado e fora da planilha; P1 está na planilha
    assert [l[0] for l in linhas[1:]] == ["31.700/2026"]
    assert linhas[1][2] == "Reforma da Escola Municipal Exemplo"
    assert os.path.isfile(faltantes.ARQ_CACHE)
    # período padrão: de 1º/jan do ano mais antigo da planilha até hoje
    assert set(fake.intervalos) == {f"01/01/2026 - {date.today():%d/%m/%Y}"}


def test_faltantes_periodo_escolhido(dados, cred, monkeypatch):
    faltantes, fake = _preparar_faltantes(dados, monkeypatch)
    planilha = _planilha(dados["dados"] / "acomp.xlsx", ["31.632/2026"])
    hoje = date.today()
    destino = faltantes.executar(planilha, None, cred, de=hoje - timedelta(days=10),
                                 ate=hoje + timedelta(days=5))       # fim no futuro vira hoje
    assert [l[0] for l in _valores(destino, "Faltantes")[1:]] == ["31.700/2026"]
    assert set(fake.intervalos) == {f"{hoje - timedelta(days=10):%d/%m/%Y} - {hoje:%d/%m/%Y}"}


def test_faltantes_periodo_sem_processos(dados, cred, monkeypatch, capsys):
    """Os dois processos foram emitidos hoje. Com o período terminando ontem
    nenhum entra, mesmo com o servidor (de mentira) ignorando o intervalo: a
    data de emissão lida na listagem confere de novo. Não é erro."""
    faltantes, fake = _preparar_faltantes(dados, monkeypatch)
    planilha = _planilha(dados["dados"] / "acomp.xlsx", ["31.632/2026"])
    ontem = date.today() - timedelta(days=1)
    assert faltantes.executar(planilha, None, cred, de=ontem - timedelta(days=30), ate=ontem) is None
    assert set(fake.intervalos) == {f"{ontem - timedelta(days=30):%d/%m/%Y} - {ontem:%d/%m/%Y}"}
    assert ("GET", "doc/ver") not in fake.pedidos               # nenhum processo aberto
    assert "Nenhum Proc. Administrativo emitido de" in capsys.readouterr().out
    assert os.listdir(str(dados["saida"])) == []


def test_faltantes_todos_ja_na_planilha(dados, cred, monkeypatch, capsys):
    faltantes, _fake = _preparar_faltantes(dados, monkeypatch)
    planilha = _planilha(dados["dados"] / "acomp.xlsx", ["31.632/2026", "31.700/2026"])
    assert faltantes.executar(planilha, None, cred) is None
    assert "já estão na planilha" in capsys.readouterr().out


def test_faltantes_periodo_vazio(dados, cred, monkeypatch):
    """Só a data final, antes do início padrão (1º/jan de 2026, pela planilha)."""
    faltantes, _fake = _preparar_faltantes(dados, monkeypatch)
    planilha = _planilha(dados["dados"] / "acomp.xlsx", ["31.632/2026"])
    with pytest.raises(SystemExit, match="Período vazio"):
        faltantes.executar(planilha, None, cred, ate=date(2025, 12, 31))


def test_verificar_planilha(dados, cred, monkeypatch):
    from ferramentas1doc import verificar_planilha
    monkeypatch.setattr(verificar_planilha, "PAUSA_REQ", 0)
    monkeypatch.setattr(verificar_planilha, "ARQ_CACHE",
                        os.path.join(str(dados["dados"]), "hash_cache_1doc.json"))
    fake1doc.instalar(monkeypatch)
    planilha = _planilha(dados["dados"] / "acomp.xlsx", ["31.632/2026", "31.700/2026"])
    destino = verificar_planilha.executar(planilha, "Acompanhamento", cred)
    linhas = _valores(destino)
    # só P1 chegou à Pesquisa de Mercado; colunas novas no fim
    assert [l[0] for l in linhas[1:]] == ["31.632/2026"]
    assert linhas[0][5:] == ["Etapa atual (1Doc)", "Processo pai", "Processo filho", "Objeto (abertura)",
                             "Observação"]
    assert linhas[1][5] == "Pesquisa de Mercado"
    assert linhas[1][8] == "Aquisição de medicamentos para a rede municipal de saúde"
    cache = json.load(open(verificar_planilha.ARQ_CACHE, encoding="utf-8"))
    assert cache["31.632/2026"] == "AAAA1111BBBB2222CCCC"       # localizado por HTTP e guardado


def test_planilha_inexistente(dados, cred):
    from ferramentas1doc import faltantes
    with pytest.raises(SystemExit, match="não encontrado"):
        faltantes.executar(os.path.join(str(dados["dados"]), "nao_existe.xlsx"), None, cred)
