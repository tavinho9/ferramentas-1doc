"""Login: o programa só espera; nunca digita nem clica."""

import threading

import pytest

from ferramentas1doc import navegador, sessao

HOST = "https://montesclaros.1doc.com.br/"


class NavegadorFalso:
    """Simula a pessoa: fica na tela de login por `login_apos` verificações e
    então "entra"; ou fecha a janela depois de `fechar_apos` consultas às abas.
    Guarda todo script executado nas páginas, para o teste conferir que o
    programa só lê."""

    def __init__(self, login_apos=3, fechar_apos=None, url_forcada=None):
        self.login_apos, self.fechar_apos = login_apos, fechar_apos
        self.verificacoes = self.acessos = 0
        self.urls, self.scripts = [], []
        self.fechado = self.encerrado = False
        self.url_forcada = url_forcada

    def ir(self, url, alvo=None):
        self.urls.append(url)

    @property
    def logado(self):
        return self.login_apos is not None and self.verificacoes >= self.login_apos

    def paginas(self):
        self.acessos += 1
        if self.fechar_apos is not None and self.acessos >= self.fechar_apos:
            self.fechado = True
        if self.fechado:
            raise navegador.ErroNavegador("conexão com o navegador perdida")
        return [{"id": "aba1",
                 "url": self.url_forcada or (HOST + "?pg=painel/listar" if self.logado else HOST + "login")}]

    def viva(self):
        try:
            return bool(self.paginas())
        except navegador.ErroNavegador:
            return False

    def avaliar(self, alvo, js):
        self.scripts.append(js)
        self.verificacoes += 1
        if self.logado:
            return {"email": "teste@pmc", "abas": True, "senha": False, "ua": "Mozilla/5.0 (teste)"}
        return {"email": "", "abas": True, "senha": True, "ua": "Mozilla/5.0 (teste)"}

    def cookies(self, host=None):
        return [{"name": "PHPSESSID", "value": "abc", "domain": "montesclaros.1doc.com.br", "path": "/"}]

    def fechar(self):
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
    assert set(nav.scripts) == {sessao.JS_ESTADO}   # só leu: nada digitado nem clicado
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
    assert nav.verificacoes == 0              # nem leu a página fora do domínio do 1Doc


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


def test_sem_conexao_nao_descarta_a_sessao_salva(dados, cred, monkeypatch):
    import requests

    def cai(self, *a, **k):
        raise requests.ConnectionError("rede fora")
    sessao.salvar(cred)
    monkeypatch.setattr(requests.Session, "request", cai)
    monkeypatch.setattr(sessao, "aguardar_login", lambda *a, **k: pytest.fail("abriu o navegador"))
    with pytest.raises(sessao.LoginCancelado, match="não respondeu"):
        sessao.obter_sessao(avisar=lambda *_: None)
    assert sessao.carregar_salva() is not None          # continua guardada


def test_sem_conexao_durante_a_espera_continua_esperando(dados, monkeypatch):
    monkeypatch.setattr(sessao, "esperar", lambda s: sessao.checar_cancelamento())
    respostas = iter([sessao.SemConexao("x"), ("SETOR", "x@y")])

    def validar(c, u):
        r = next(respostas)
        if isinstance(r, Exception):
            raise r
        return r
    assert _aguardar(NavegadorFalso(login_apos=1), validar=validar).setor == "SETOR"


class _WsFalso:
    """WebSocket do navegador: responde cada pedido com o resultado do método."""

    def __init__(self, resultados):
        self.resultados, self.pedidos = resultados, []

    def settimeout(self, t):
        pass

    def send(self, texto):
        import json
        self.pedidos.append(json.loads(texto))

    def recv(self):
        import json
        p = self.pedidos[-1]
        return json.dumps({"id": p["id"], "result": self.resultados[p["method"]]})

    def close(self):
        pass


def _navegador_falso(resultados):
    class Processo:
        def poll(self):
            return None
    return navegador.Navegador(Processo(), _WsFalso(resultados), "chrome")


def test_cookies_do_navegador_so_os_do_1doc_no_formato_da_sessao_salva():
    nav = _navegador_falso({"Storage.getCookies": {"cookies": [
        {"name": "PHPSESSID", "value": "abc", "domain": "montesclaros.1doc.com.br", "path": "/",
         "expires": -1, "session": True, "httpOnly": True, "secure": True, "sameSite": "Lax"},
        {"name": "lembrar", "value": "1", "domain": ".1doc.com.br", "path": "/", "expires": 1893456000.5,
         "session": False},
        {"name": "NID", "value": "x", "domain": ".google.com", "path": "/", "expires": 1893456000},
        {"name": "outra", "value": "y", "domain": "outra.1doc.com.br", "path": "/"}]}})
    assert nav.cookies("montesclaros.1doc.com.br") == [
        {"name": "PHPSESSID", "value": "abc", "domain": "montesclaros.1doc.com.br", "path": "/", "secure": True,
         "httpOnly": True, "sameSite": "Lax"},
        {"name": "lembrar", "value": "1", "domain": ".1doc.com.br", "path": "/", "secure": False,
         "httpOnly": False, "expiry": 1893456000}]


def test_cookies_salvos_voltam_ao_navegador_com_o_mesmo_alcance():
    """Sessões salvas pela versão com Selenium continuam valendo no navegador invisível."""
    so_host = navegador._cookie_cdp({"name": "PHPSESSID", "value": "abc", "domain": "montesclaros.1doc.com.br",
                                     "path": "/", "secure": True, "httpOnly": True, "expiry": 1893456000,
                                     "sameSite": "Lax"})
    assert so_host == {"name": "PHPSESSID", "value": "abc", "path": "/", "url": "https://montesclaros.1doc.com.br/",
                       "secure": True, "httpOnly": True, "sameSite": "Lax", "expires": 1893456000.0}
    dominio = navegador._cookie_cdp({"name": "x", "value": "1", "domain": ".1doc.com.br", "path": "/"})
    assert dominio["domain"] == ".1doc.com.br" and dominio["url"] == "http://1doc.com.br/"


def test_automatico_usa_o_edge_quando_nao_ha_chrome(dados, monkeypatch):
    tentativas = []

    def criar(qual, perfil=None, headless=False):
        tentativas.append((qual, perfil is not None))
        if qual == "chrome":
            raise navegador.ErroNavegador("Google Chrome não encontrado neste computador")
        return "navegador-edge"
    monkeypatch.setattr(sessao, "_criar_driver", criar)
    monkeypatch.setattr(navegador, "localizar_executavel", lambda q: None if q == "chrome" else "msedge.exe")
    assert sessao.abrir_navegador("auto") == ("navegador-edge", "edge")
    assert tentativas == [("chrome", True), ("edge", True)]   # sem Chrome, nem tenta o perfil temporário


def test_sem_navegador_explica_cada_tentativa(dados, monkeypatch):
    def criar(qual, perfil=None, headless=False):
        raise navegador.ErroNavegador(f"{navegador.NOMES[qual]} não encontrado neste computador")
    monkeypatch.setattr(sessao, "_criar_driver", criar)
    monkeypatch.setattr(navegador, "localizar_executavel", lambda q: None)
    with pytest.raises(sessao.LoginCancelado, match="Google Chrome não encontrado(.|\n)*Microsoft Edge não"):
        sessao.abrir_navegador("auto")
