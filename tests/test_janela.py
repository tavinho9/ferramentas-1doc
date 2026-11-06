"""A janela de verdade, rodando uma ferramenta contra o 1Doc de mentira."""

import os
import sys
import time
from datetime import date, timedelta

import pytest

tk = pytest.importorskip("tkinter")

import fake1doc  # noqa: E402


@pytest.fixture(scope="module")
def _janela_unica():
    """Uma janela só para todos os testes. No Windows, criar um segundo Tk no
    mesmo processo depois de fechar o primeiro às vezes quebra o Tcl
    ("invalid command name tcl_findLibrary"). O programa nunca faz isso: ele
    abre uma única janela por execução."""
    if os.name != "nt" and not os.environ.get("DISPLAY"):
        pytest.skip("sem tela para o Tk (no Linux, rode com xvfb-run)")
    from ferramentas1doc.gui import janela as mod
    from ferramentas1doc.gui.log import SaidaFila
    j = mod.Janela(SaidaFila())
    yield j
    j.destroy()


@pytest.fixture
def janela(_janela_unica, dados, cred, monkeypatch):
    from ferramentas1doc import processos_do_dia, sessao
    from ferramentas1doc.gui import janela as mod

    def obter_sessao(ao_esperar=None, **kw):
        ao_esperar(True)                    # como se o navegador tivesse aberto
        ao_esperar(False)
        sessao.salvar(cred)                 # como o obter_sessao real faz
        return cred
    monkeypatch.setattr(sessao, "obter_sessao", obter_sessao)
    monkeypatch.setattr(processos_do_dia, "PAUSA_REQ", 0)
    fake1doc.instalar(monkeypatch)
    avisos = []
    monkeypatch.setattr(mod.messagebox, "showwarning", lambda *a, **k: avisos.append(a))
    j = _janela_unica
    j.avisos = avisos
    j.ultimo_arquivo = None
    return j


def _esperar_fim(j, limite=30):
    fim = time.monotonic() + limite
    while j.executando is not None and time.monotonic() < fim:
        j.update()
        time.sleep(0.05)
    j.update()
    for _ in range(3):                      # deixa a fila de mensagens esvaziar
        time.sleep(0.12)
        j.update()


def test_executar_processos_do_dia_pela_janela(janela, dados, monkeypatch):
    # no programa, app.py faz isto; aqui, na fase de execução do teste (o
    # pytest reinstala a própria captura de saída entre a preparação e o teste)
    monkeypatch.setattr(sys, "stdout", janela.saida)
    aba = janela.abas[0]
    aba.dia.set("hoje")
    janela._executar(aba)
    assert str(aba.btn_executar["state"]) == "disabled"      # bloqueado enquanto roda
    _esperar_fim(janela)
    texto = janela.texto.get("1.0", "end")
    assert janela.lbl_estado.cget("text") == "Concluído.", texto
    assert "Processos exportados: 1" in texto
    assert janela.ultimo_arquivo == os.path.join(str(dados["saida"]),
                                                 f"processos novos {date.today():%d.%m}.xlsx")
    assert str(janela.btn_abrir["state"]) == "normal"
    assert str(aba.btn_executar["state"]) == "normal"
    assert "Conectado" in janela.lbl_sessao.cget("text")
    logs = os.listdir(os.path.join(str(dados["dados"]), "logs"))
    assert len(logs) == 1 and "Processos exportados: 1" in open(
        os.path.join(str(dados["dados"]), "logs", logs[0]), encoding="utf-8").read()


def test_data_invalida_avisa_sem_executar(janela):
    aba = janela.abas[0]
    aba.dia.set("31/02/2026")
    janela._executar(aba)
    assert janela.executando is None
    assert janela.avisos and "Data não reconhecida" in janela.avisos[0][1]


def test_busca_mostra_instrucao_de_datas(janela):
    from ferramentas1doc import busca_argv
    aba = janela.abas[1]
    assert aba.dica_datas.cget("text") == busca_argv.DICA_DATAS
    assert aba.dica_datas.winfo_manager() == "grid"
    assert aba.dica_datas.master.cget("text") == "Datas e mudanças de etapa"     # junto dos campos


def test_busca_campos_de_etapa_com_lista(janela, dados):
    import json
    from tkinter import ttk
    from ferramentas1doc import busca_argv
    with open(dados["dados"] / "1doc_cache.json", "w", encoding="utf-8") as f:
        json.dump({"versao": "1.1", "det": {"3|1/2026": {"etapas_quadro": ["Elaboração do ETP", "Etapa Final"]}}},
                  f)
    aba = janela.abas[1]
    assert set(aba.combos_etapa) == set(busca_argv.CAMPOS_ETAPA)
    for chave, cb in aba.combos_etapa.items():
        assert isinstance(cb, ttk.Combobox) and str(cb["state"]) != "readonly"   # aceita digitar
        assert str(cb.cget("textvariable")) == str(aba.v[chave])

    def escolher(cb, nome):                 # como o Tk faz: abre a lista, troca o valor, avisa
        cb.tk.call(cb.cget("postcommand"))
        assert nome in cb["values"]
        cb.set(nome)
        cb.event_generate("<<ComboboxSelected>>")

    cb = aba.combos_etapa["etapa"]
    escolher(cb, "Pesquisa de Mercado")
    assert aba.v["etapa"].get() == "Pesquisa de Mercado"
    cb.set("Pesquisa de Mercado; ")
    escolher(cb, "Elaboração do ETP")        # etapa vinda do cache
    assert aba.v["etapa"].get() == "Pesquisa de Mercado; Elaboração do ETP"
    escolher(aba.combos_etapa["antes_de"], "Pesquisa de Mercado")
    form = aba.formulario()
    assert form["etapa"] == "Pesquisa de Mercado; Elaboração do ETP"
    assert form["antes_de"] == "Pesquisa de Mercado"
    for c in aba.combos_etapa.values():
        c.set("")


def test_busca_com_formulario_invalido(janela):
    aba = janela.abas[1]
    for v in aba.caixas.values():
        v.set(False)
    janela._executar(aba)
    assert janela.executando is None and "caixa" in janela.avisos[0][1]


def test_faltantes_periodo_invalido_avisa_sem_executar(janela):
    aba = janela.abas[2]
    aba.de.set("31/02/2026")
    assert "não reconhecida" in aba.leitura.cget("text")          # aviso ao digitar
    janela._executar(aba)
    assert janela.executando is None
    assert janela.avisos and "Data inicial não reconhecida" in janela.avisos[0][1]


def test_faltantes_periodo_vai_para_a_execucao_e_fica_salvo(janela, dados, cred, monkeypatch):
    from openpyxl import Workbook
    from ferramentas1doc import config, faltantes
    planilha = str(dados["dados"] / "acomp.xlsx")
    Workbook().save(planilha)
    chamadas = []
    monkeypatch.setattr(faltantes, "executar", lambda *a, **k: chamadas.append((a, k)))
    aba = janela.abas[2]
    aba.planilha.set(planilha)
    aba.de.set("-30")
    aba.ate.set("ontem")
    hoje = date.today()
    assert aba.leitura.cget("text") == (f"→ de {hoje - timedelta(days=30):%d/%m/%Y} "
                                        f"até {hoje - timedelta(days=1):%d/%m/%Y}")
    _titulo, funcao, precisa_sessao = aba.tarefa()
    funcao(cred)
    assert precisa_sessao and chamadas[0][1] == {"de": hoje - timedelta(days=30), "ate": hoje - timedelta(days=1)}
    cfg = config.carregar()
    assert (cfg["faltantes_de"], cfg["faltantes_ate"]) == ("-30", "ontem")      # como digitado

    aba.de.set("")                                                  # vazio = padrão
    aba.ate.set("")
    assert aba.leitura.cget("text").startswith("→ de 1º/jan do ano mais antigo da planilha até hoje")
    aba.tarefa()[1](cred)
    assert chamadas[1][1] == {"de": None, "ate": None}


def _processar(j, vezes=6):
    for _ in range(vezes):
        j.update()
        time.sleep(0.05)


def _dialogo_configuracoes(j):
    from ferramentas1doc.gui import janela as mod
    abertos = [w for w in j.winfo_children() if isinstance(w, mod.DialogoConfiguracoes)]
    assert len(abertos) == 1, abertos
    return abertos[0]


def _principal_livre(j):
    """Nem grab do Tk nem janela desabilitada: os dois fazem o Windows ignorar
    minimizar e restaurar a principal (ver gui/secundaria.py)."""
    return j.grab_current() is None and not (os.name == "nt" and int(j.attributes("-disabled")))


def test_configuracoes_uma_so_presa_a_principal(janela):
    j = janela
    j._configuracoes()
    _processar(j)
    d = _dialogo_configuracoes(j)
    j._configuracoes()                      # clicar de novo traz a mesma, não abre outra
    _processar(j)
    assert _dialogo_configuracoes(j) is d and j.dialogo_configuracoes is d
    assert d.winfo_viewable()
    assert str(d.transient()) == str(j)     # presa à principal: minimiza e volta junto
    assert _principal_livre(j)
    d.destroy()                             # Cancelar, Salvar, Esc e o X
    _processar(j)
    j._configuracoes()                      # fechada, abre de novo
    _processar(j)
    d2 = _dialogo_configuracoes(j)
    assert d2 is not d and d2.winfo_viewable()
    d2.destroy()
    _processar(j)


def test_opcoes_da_busca_uma_so_presa_a_principal(janela):
    aba = janela.abas[1]
    aba._ajuda()
    _processar(janela)
    o = aba.janela_ajuda
    aba._ajuda()
    _processar(janela)
    assert aba.janela_ajuda is o
    assert [w for w in aba.winfo_children() if isinstance(w, tk.Toplevel)] == [o]
    assert str(o.transient()) == str(janela)
    assert _principal_livre(janela)
    o.destroy()
    _processar(janela)
    aba._ajuda()
    _processar(janela)
    assert aba.janela_ajuda is not o and aba.janela_ajuda.winfo_exists()
    aba.janela_ajuda.destroy()
    _processar(janela)


@pytest.mark.skipif(os.name != "nt", reason="minimizar e restaurar como o Windows faz (só no Windows)")
@pytest.mark.parametrize("qual", ["Configurações", "Opções da busca"])
def test_com_janela_secundaria_aberta_o_programa_minimiza_e_volta(janela, qual):
    """Com a janela de Configurações aberta, o programa minimizado tem de voltar
    pelo botão da barra de tarefas. Com um grab do Tk (e também com a principal
    desabilitada), o Windows ignora os comandos de sistema da janela principal
    (WM_SYSCOMMAND), inclusive o "restaurar": minimizada pelo Win+D, ela não
    voltava mais."""
    import ctypes
    from ctypes import wintypes
    user32 = ctypes.WinDLL("user32")
    user32.SendMessageW.argtypes = (wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM)
    user32.SendMessageW.restype = wintypes.LPARAM
    user32.ShowWindow.argtypes = (wintypes.HWND, ctypes.c_int)
    user32.IsIconic.argtypes = user32.IsWindowVisible.argtypes = (wintypes.HWND,)
    user32.GetWindow.argtypes = (wintypes.HWND, wintypes.UINT)
    user32.GetWindow.restype = wintypes.HWND
    WM_SYSCOMMAND, SC_MINIMIZE, SC_RESTORE = 0x0112, 0xF020, 0xF120
    SW_MINIMIZE, SW_RESTORE, GW_OWNER = 6, 9, 4

    j = janela
    principal = int(j.wm_frame(), 16)
    if qual == "Configurações":
        j._configuracoes()
        _processar(j)
        d = _dialogo_configuracoes(j)
    else:
        j.abas[1]._ajuda()
        _processar(j)
        d = j.abas[1].janela_ajuda

    def moldura():
        # o Tk recria a moldura (wrapper) da secundária quando a principal volta
        return int(d.wm_frame(), 16)

    def visivel_na_frente_da_principal():
        return (d.winfo_viewable() and user32.IsWindowVisible(moldura())
                and user32.GetWindow(moldura(), GW_OWNER) == principal)

    try:
        assert visivel_na_frente_da_principal()
        # "Mostrar área de trabalho" (Win+D): minimiza sem passar por WM_SYSCOMMAND
        user32.ShowWindow(principal, SW_MINIMIZE)
        _processar(j)
        assert user32.IsIconic(principal)
        assert not user32.IsWindowVisible(moldura())        # some junto com a principal
        # clique no botão do programa na barra de tarefas
        user32.SendMessageW(principal, WM_SYSCOMMAND, SC_RESTORE, 0)
        _processar(j)
        assert not user32.IsIconic(principal), "o programa ficou preso minimizado"
        assert j.state() == "normal"
        assert visivel_na_frente_da_principal(), "a janela secundária não voltou"

        # minimizar pela barra de tarefas e restaurar de novo
        user32.SendMessageW(principal, WM_SYSCOMMAND, SC_MINIMIZE, 0)
        _processar(j)
        assert user32.IsIconic(principal), "o comando de minimizar foi ignorado"
        assert not user32.IsWindowVisible(moldura())
        user32.SendMessageW(principal, WM_SYSCOMMAND, SC_RESTORE, 0)
        _processar(j)
        assert not user32.IsIconic(principal), "o programa ficou preso minimizado"
        assert visivel_na_frente_da_principal(), "a janela secundária não voltou"
    finally:
        user32.ShowWindow(principal, SW_RESTORE)
        d.destroy()
        _processar(j)

