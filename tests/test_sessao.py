"""Login: o programa só espera; nunca digita nem clica."""

import threading

import pytest
from selenium.common.exceptions import NoSuchWindowException

from ferramentas1doc import sessao

HOST = "https://montesclaros.1doc.com.br/"


class Elemento:
    def __init__(self, valor="", registro=None):
        self.valor = valor
        self.registro = registro

    def get_attribute(self, nome):
        return self.valor if nome == "value" else None

    def send_keys(self, *a):            # se o programa digitar algo, o teste acusa
        self.registro.append(("send_keys", a))

    def click(self):
        self.registro.append(("click",))

    def clear(self):
        self.registro.append(("clear",))


class NavegadorFalso:
    """Simula a pessoa: fica na tela de login por `login_apos` verificações e
    então "entra"; ou fecha a janela depois de `fechar_apos`."""

    def __init__(self, login_apos=3, fechar_apos=None, url_forcada=None):
        self.login_apos, self.fechar_apos = login_apos, fechar_apos
        self.verificacoes = self.acessos = 0
        self.urls, self.acoes = [], []
        self.fechado = self.encerrado = False
        self.url_forcada = url_forcada

    def get(self, url):
        self.urls.append(url)

    @property
    def logado(self):
        return self.login_apos is not None and self.verificacoes >= self.login_apos

    @property
    def current_url(self):
        self.acessos += 1
        if self.fechar_apos is not None and self.acessos >= self.fechar_apos:
            self.fechado = True
        if self.fechado:
            raise NoSuchWindowException("fechada")
        return self.url_forcada or (HOST + "?pg=painel/listar" if self.logado else HOST + "login")

    @property
    def window_handles(self):
        return [] if self.fechado else ["w1"]

    def find_elements(self, por, valor):
        if valor == "sis_usr_email":
            self.verificacoes += 1
            return [Elemento("teste@pmc", self.acoes)] if self.logado else []
        if valor == "input[type=password]":
            return [] if self.logado else [Elemento("", self.acoes)]
        return []

    def get_cookies(self):
        return [{"name": "PHPSESSID", "value": "abc", "domain": "montesclaros.1doc.com.br", "path": "/"}]

    def execute_script(self, js):
        return "Mozilla/5.0 (teste)"

    def quit(self):
        self.encerrado = True


def _aguardar(nav, validar=lambda c, u: ("SPOT-DCL-GP-CPPFI", "teste@pmc"), minutos=1):
    return sessao.aguardar_login(minutos=minutos, avisar=lambda *_: None, fabrica=lambda: (nav, "chrome"),
                                 validar=validar, intervalo=0.01, intervalo_validacao=0)


def test_espera_o_login_sem_digitar_nada(dados, monkeypatch):
    monkeypatch.setattr(sessao, "esperar", lambda s: sessao.checar_cancelamento())
    nav = NavegadorFalso(login_apos=5)
    cred = _aguardar(nav)
    assert cred.setor == "SPOT-DCL-GP-CPPFI" and cred.email == "teste@pmc"
    assert cred.cookies[0]["name"] == "PHPSESSID" and cred.ua == "Mozilla/5.0 (teste)"
    assert nav.verificacoes >= 5             # esperou: não tomou a tela de login por logada
    assert nav.urls == [sessao.URL_LOGIN]     # só abriu a página
    assert nav.acoes == []                    # nenhum send_keys, clear ou click
    assert nav.encerrado                      # fechou o navegador


def test_cookies_que_nao_valem_continuam_esperando(dados, monkeypatch):
    monkeypatch.setattr(sessao, "esperar", lambda s: sessao.checar_cancelamento())
    respostas = iter([None, None, ("SETOR", "x@y")])
    cred = _aguardar(NavegadorFalso(login_apos=1), validar=lambda c, u: next(respostas))
    assert cred.setor == "SETOR"


def test_pagina_de_login_em_outro_dominio_nao_conta(dados, monkeypatch):
    """SSO num domínio diferente: só a página do 1Doc da prefeitura vale."""
    monkeypatch.setattr(sessao, "esperar", lambda s: sessao.checar_cancelamento())
    nav = NavegadorFalso(login_apos=0, fechar_apos=12, url_forcada="https://login.exemplo.com/?sis_usr_email")
    with pytest.raises(sessao.LoginCancelado, match="fechada"):
        _aguardar(nav)
    assert nav.verificacoes == 0              # nem procurou o campo fora do domínio do 1Doc


def test_janela_fechada_cancela_o_login(dados, monkeypatch):
    monkeypatch.setattr(sessao, "esperar", lambda s: sessao.checar_cancelamento())
    nav = NavegadorFalso(login_apos=None, fechar_apos=7)
    with pytest.raises(sessao.LoginCancelado, match="fechada"):
        _aguardar(nav)
    assert nav.encerrado


def test_tempo_esgotado(dados):
    nav = NavegadorFalso(login_apos=None)
    with pytest.raises(sessao.LoginCancelado, match="tempo esgotado"):
        _aguardar(nav, minutos=0.002)


def test_botao_cancelar(dados):
    nav = NavegadorFalso(login_apos=None)
    threading.Timer(0.2, sessao.CANCELAR.set).start()
    with pytest.raises(sessao.Cancelado):
        _aguardar(nav)
    assert nav.encerrado


def test_sessao_salva_valida_nao_abre_navegador(dados, cred, monkeypatch):
    sessao.salvar(cred)
    monkeypatch.setattr(sessao, "verificar_cookies", lambda c, u: ("SETOR X", "a@b"))
    monkeypatch.setattr(sessao, "aguardar_login", lambda *a, **k: pytest.fail("abriu o navegador"))
    obtida = sessao.obter_sessao(avisar=lambda *_: None)
    assert obtida.cookies == cred.cookies and obtida.setor == "SETOR X"


def test_sessao_expirada_abre_o_navegador_e_regrava(dados, cred, monkeypatch):
    sessao.salvar(cred)
    monkeypatch.setattr(sessao, "verificar_cookies", lambda c, u: None)
    nova = sessao.Credenciais(cookies=[{"name": "novo", "value": "1"}], ua="UA", setor="S", email="e")
    estados = []
    monkeypatch.setattr(sessao, "aguardar_login", lambda *a, **k: nova)
    obtida = sessao.obter_sessao(avisar=lambda *_: None, ao_esperar=estados.append)
    assert obtida is nova and estados == [True, False]
    assert sessao.carregar_salva().cookies == [{"name": "novo", "value": "1"}]


def test_sair_apaga_sessao_e_perfil(dados, cred):
    import os
    from ferramentas1doc import caminhos
    sessao.salvar(cred)
    os.makedirs(os.path.join(caminhos.pasta_perfil_navegador(), "chrome"))
    assert sessao.sair() == []
    assert sessao.carregar_salva() is None
    assert not os.path.exists(caminhos.pasta_perfil_navegador())


def test_verificar_cookies_troca_inbox_pessoal_para_o_do_setor(monkeypatch):
    import requests
    pedidos = []

    class R:
        status_code = 200

        def __init__(self, html):
            self.content = html.encode()

        def raise_for_status(self):
            pass

    def falso(self, metodo, url, params=None, **kw):
        pedidos.append(params)
        pessoal = len(pedidos) == 1
        return R('<input id="sis_usr_email" value="a@b"><input id="s_s" value="SPOT">'
                 + ('<li class="li_pessoal active">x</li>' if pessoal else ""))
    monkeypatch.setattr(requests.Session, "request", falso)
    assert sessao.verificar_cookies([], "UA") == ("SPOT", "a@b")
    assert pedidos[1] == {"pg": "painel/listar", "meu": "0", "trocar": "1"}


def test_verificar_cookies_tela_de_login(monkeypatch):
    import fake1doc
    fake1doc.instalar(monkeypatch, logado=False)
    assert sessao.verificar_cookies([], "UA") is None
