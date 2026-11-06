"""Sessão do 1Doc: login feito pela pessoa no navegador, sessão lembrada.

O programa NUNCA pede nem digita usuário e senha. Quando não há sessão válida,
ele abre o Chrome (ou o Edge) na página do 1Doc e espera: a pessoa faz o login
na janela, do jeito de sempre (inclusive captcha ou segundo fator, se houver).
Detectado o login, os cookies e o User-Agent do navegador são copiados, a
janela é fechada e todo o resto sai por HTTP, como nos scripts originais.

A sessão fica em sessao.json, na pasta de dados do usuário, e é reaproveitada
nas execuções seguintes enquanto o 1Doc a aceitar. "Sair" apaga o arquivo e o
perfil do navegador usado no login.

Base: login_navegador / verificar_sessao / abrir_sessao do busca_1doc.py, sem
o preenchimento automático por variáveis de ambiente.
"""

import json
import os
import shutil
import threading
import time
from dataclasses import dataclass, field
from datetime import datetime
from urllib.parse import urlparse

import requests

from . import caminhos

URL_1DOC = "https://montesclaros.1doc.com.br"
BASE = URL_1DOC + "/"
URL_LOGIN = BASE + "?pg=painel/listar"     # sem sessão, o 1Doc redireciona para o login
ARQUIVO_SESSAO = "sessao.json"

from importlib.util import find_spec

PARSER = "lxml" if find_spec("lxml") else "html.parser"


# ============================================================
# Cancelamento (botão Cancelar da interface)
# ============================================================

class Cancelado(BaseException):
    """A pessoa pediu para parar. É BaseException de propósito: os scripts têm
    `except Exception` que transformariam o cancelamento em "falha de leitura"
    de um processo e seguiriam para o próximo."""


class LoginCancelado(Exception):
    """O login não aconteceu: janela fechada, tempo esgotado ou navegador indisponível."""


CANCELAR = threading.Event()


def checar_cancelamento():
    if CANCELAR.is_set():
        raise Cancelado()


def esperar(segundos):
    """time.sleep que acorda se a pessoa cancelar."""
    if CANCELAR.wait(segundos):
        raise Cancelado()


# ============================================================
# Sessão salva
# ============================================================

@dataclass
class Credenciais:
    cookies: list
    ua: str
    setor: str = ""
    email: str = ""
    salvo_em: str = ""
    extras: dict = field(default_factory=dict)


def _caminho_sessao():
    return caminhos.arquivo_dados(ARQUIVO_SESSAO)


def carregar_salva():
    try:
        with open(_caminho_sessao(), encoding="utf-8") as f:
            dados = json.load(f)
    except (OSError, ValueError):
        return None
    if dados.get("base") != BASE or not isinstance(dados.get("cookies"), list):
        return None
    return Credenciais(cookies=dados["cookies"], ua=dados.get("ua", ""), setor=dados.get("setor", ""),
                       email=dados.get("email", ""), salvo_em=dados.get("salvo_em", ""))


def salvar(cred):
    caminho = _caminho_sessao()
    dados = {"base": BASE, "ua": cred.ua, "cookies": cred.cookies, "setor": cred.setor,
             "email": cred.email, "salvo_em": cred.salvo_em or datetime.now().isoformat(timespec="seconds")}
    with open(caminho, "w", encoding="utf-8") as f:
        json.dump(dados, f, ensure_ascii=False)
    try:
        os.chmod(caminho, 0o600)
    except OSError:
        pass


def esquecer():
    """Apaga só a sessão salva (ex.: o 1Doc a recusou no meio da execução)."""
    try:
        os.remove(_caminho_sessao())
    except OSError:
        pass


def sair():
    """Apaga a sessão salva e o perfil do navegador do login. Devolve a lista
    de avisos (pastas que não puderam ser apagadas, p.ex. navegador aberto)."""
    esquecer()
    avisos = []
    perfil = caminhos.pasta_perfil_navegador()
    if os.path.isdir(perfil):
        shutil.rmtree(perfil, ignore_errors=True)
        if os.path.isdir(perfil):
            avisos.append(f"não consegui apagar {perfil} (o navegador do login está aberto?)")
    return avisos


# ============================================================
# Validação por HTTP
# ============================================================

def _sessao_requests(cookies, ua):
    s = requests.Session()
    s.headers.update({"User-Agent": ua or "Mozilla/5.0", "Accept-Language": "pt-BR,pt;q=0.9"})
    for c in cookies:
        s.cookies.set(c["name"], c["value"], domain=c.get("domain"), path=c.get("path") or "/")
    return s


def verificar_cookies(cookies, ua, timeout=30):
    """(setor, e-mail) se os cookies valem para requisições; None se o 1Doc
    devolveu a tela de login. Se o "Inbox pessoal" estiver ativo, troca para o
    inbox do setor (o mesmo link do menu), porque as caixas seguem o inbox ativo."""
    from bs4 import BeautifulSoup

    s = _sessao_requests(cookies, ua)
    for tentativa in (1, 2):
        try:
            r = s.get(URL_LOGIN, timeout=timeout)
            r.raise_for_status()
        except requests.RequestException:
            return None
        soup = BeautifulSoup(r.content.decode("utf-8", errors="replace"), PARSER)
        email = soup.select_one("input#sis_usr_email")
        email = (email.get("value") or "").strip() if email is not None else ""
        if not email:
            return None
        if soup.select_one("li.li_pessoal.active") and tentativa == 1:
            try:
                s.get(BASE, params={"pg": "painel/listar", "meu": "0", "trocar": "1"}, timeout=timeout)
            except requests.RequestException:
                return None
            continue
        setor = soup.select_one("input#s_s")
        return ((setor.get("value") if setor is not None else "") or "(setor não identificado)", email)
    return None


# ============================================================
# Navegador
# ============================================================

def _criar_driver(qual, perfil=None, headless=False):
    """Chrome ou Edge pelo Selenium Manager (baixa o driver sozinho, como antes)."""
    from selenium import webdriver

    if qual == "edge":
        opts, cls, svc_cls = webdriver.EdgeOptions(), webdriver.Edge, webdriver.EdgeService
    else:
        opts, cls, svc_cls = webdriver.ChromeOptions(), webdriver.Chrome, webdriver.ChromeService
    if headless:
        opts.add_argument("--headless=new")
        opts.add_argument("--window-size=1920,1080")
    if perfil:
        os.makedirs(perfil, exist_ok=True)
        opts.add_argument(f"--user-data-dir={perfil}")
    opts.add_argument("--no-first-run")
    opts.add_argument("--no-default-browser-check")
    # Só para desenvolvimento/testes: binário e driver fora do lugar padrão
    if qual == "chrome" and os.environ.get("FERRAMENTAS1DOC_CHROME"):
        opts.binary_location = os.environ["FERRAMENTAS1DOC_CHROME"]
        opts.add_argument("--no-sandbox")
    driver_path = os.environ.get("FERRAMENTAS1DOC_CHROMEDRIVER") if qual == "chrome" else None
    servico = svc_cls(executable_path=driver_path) if driver_path else svc_cls()
    if os.name == "nt":
        import subprocess
        servico.creation_flags = subprocess.CREATE_NO_WINDOW   # sem janela preta do driver
    drv = cls(options=opts, service=servico)
    drv.implicitly_wait(0)
    return drv


def abrir_navegador(navegador="auto", headless=False, usar_perfil=True, avisar=print):
    """Devolve (driver, nome). Em "auto": Chrome, depois Edge. Se o perfil
    próprio estiver travado (outra janela do login aberta), tenta sem ele."""
    ordem = ["chrome", "edge"] if navegador == "auto" else [navegador]
    erros = []
    for qual in ordem:
        perfis = [os.path.join(caminhos.pasta_perfil_navegador(), qual), None] if usar_perfil else [None]
        for perfil in perfis:
            try:
                return _criar_driver(qual, perfil, headless), qual
            except Exception as e:      # WebDriverException, SessionNotCreated, OSError...
                erros.append(f"{qual}{' (perfil próprio)' if perfil else ''}: {str(e).splitlines()[0][:160]}")
    raise LoginCancelado("não consegui abrir o navegador.\n  " + "\n  ".join(erros))


NOMES = {"chrome": "Google Chrome", "edge": "Microsoft Edge"}


def _pagina_logada(drv, host):
    """True se a aba atual é uma página do 1Doc com usuário logado. O sinal é o
    campo #sis_usr_email preenchido, que só existe em páginas logadas (o mesmo
    teste do busca_1doc); a área de abas (.tab-content), que era o sinal dos
    outros scripts, vale como reserva desde que não haja campo de senha."""
    from selenium.webdriver.common.by import By

    if (urlparse(drv.current_url).hostname or "") != host:
        return False
    el = drv.find_elements(By.ID, "sis_usr_email")
    if el and (el[0].get_attribute("value") or "").strip():
        return True
    return bool(drv.find_elements(By.CSS_SELECTOR, ".tab-content")) and \
        not drv.find_elements(By.CSS_SELECTOR, "input[type=password]")


def _janela_viva(drv):
    """Garante que o driver aponta para uma janela aberta. False se a pessoa
    fechou todas (ou o navegador)."""
    from selenium.common.exceptions import WebDriverException

    try:
        drv.current_url
        return True
    except WebDriverException:
        pass
    try:
        abas = drv.window_handles
    except WebDriverException:
        return False
    if not abas:
        return False
    try:
        drv.switch_to.window(abas[-1])
        return True
    except WebDriverException:
        return False


def aguardar_login(navegador="auto", minutos=10, avisar=print, fabrica=None, validar=None,
                   intervalo=1.0, intervalo_validacao=4.0):
    """Abre o navegador no 1Doc e espera a pessoa fazer o login. Não preenche
    nem clica em nada. Devolve Credenciais já validadas por HTTP.

    fabrica() -> (driver, nome) e validar(cookies, ua) -> (setor, email)|None
    existem para os testes; por padrão, abrir_navegador e verificar_cookies."""
    from selenium.common.exceptions import WebDriverException

    fabrica = fabrica or (lambda: abrir_navegador(navegador, avisar=avisar))
    validar = validar or verificar_cookies
    host = urlparse(BASE).hostname

    drv, nome = fabrica()
    try:
        try:
            drv.get(URL_LOGIN)
        except WebDriverException as e:
            raise LoginCancelado(f"o navegador não abriu o 1Doc ({str(e).splitlines()[0][:120]})")
        avisar(f"→ Faça o login no 1Doc na janela do {NOMES.get(nome, nome)} que abriu.")
        avisar(f"  O programa está esperando (até {minutos} min). Ele não digita nada: "
               "usuário e senha são só com você.")
        fim = time.monotonic() + minutos * 60
        ultima_validacao = 0.0
        while True:
            checar_cancelamento()
            if time.monotonic() > fim:
                raise LoginCancelado(f"tempo esgotado ({minutos} min) esperando o login no navegador")
            if not _janela_viva(drv):
                raise LoginCancelado("a janela do navegador foi fechada antes do login")
            try:
                logado = _pagina_logada(drv, host)
            except WebDriverException:
                logado = False          # página carregando / trocando
            if logado and time.monotonic() - ultima_validacao >= intervalo_validacao:
                ultima_validacao = time.monotonic()
                esperar(1.0)            # deixa o 1Doc terminar de gravar os cookies
                try:
                    cookies = drv.get_cookies()
                    ua = drv.execute_script("return navigator.userAgent;")
                except WebDriverException:
                    continue
                checar_cancelamento()
                ok = validar(cookies, ua)
                if ok:
                    setor, email = ok
                    avisar(f"✓ Login detectado — {email}, setor {setor}")
                    return Credenciais(cookies=cookies, ua=ua, setor=setor, email=email,
                                       salvo_em=datetime.now().isoformat(timespec="seconds"))
                # Página parece logada, mas os cookies ainda não valem fora do
                # navegador: continua esperando (o 1Doc pode estar redirecionando).
            esperar(intervalo)
    finally:
        try:
            drv.quit()
        except Exception:
            pass


def obter_sessao(forcar_login=False, navegador=None, minutos=None, avisar=print, ao_esperar=None):
    """Credenciais válidas: as salvas, se o 1Doc ainda as aceita; senão abre o
    navegador e espera o login. `ao_esperar(True/False)` avisa a interface de
    que a janela de login está aberta."""
    from . import config

    cfg = config.carregar()
    navegador = navegador or cfg["navegador"]
    minutos = minutos or cfg["minutos_login"]

    if not forcar_login:
        salva = carregar_salva()
        if salva is not None:
            checar_cancelamento()
            ok = verificar_cookies(salva.cookies, salva.ua)
            if ok:
                salva.setor, salva.email = ok
                avisar(f"✓ Sessão salva reaproveitada — {salva.email}, setor {salva.setor}")
                return salva
            avisar("A sessão salva expirou; é preciso entrar de novo.")
            esquecer()

    avisar("Abrindo o navegador para o login no 1Doc…")
    if ao_esperar:
        ao_esperar(True)
    try:
        cred = aguardar_login(navegador, minutos, avisar)
    finally:
        if ao_esperar:
            ao_esperar(False)
    salvar(cred)
    return cred


# ============================================================
# Navegador invisível sob demanda (fallback do "Localizar")
# ============================================================

class NavegadorSobDemanda:
    """Os verificadores de planilha usam o navegador como último recurso para
    achar o hash de um processo pelo formulário "Localizar". Antes o Chrome
    ficava aberto a execução inteira; agora um navegador invisível é aberto só
    na primeira vez que precisar, com os cookies da sessão, e fechado no fim."""

    def __init__(self, cred, navegador=None):
        self.cred = cred
        self.navegador = navegador
        self._drv = None
        self._falhou = False

    def obter(self):
        if self._drv is not None or self._falhou:
            return self._drv
        from . import config
        try:
            drv, _ = abrir_navegador(self.navegador or config.carregar()["navegador"],
                                     headless=True, usar_perfil=False)
            drv.get(BASE + "robots.txt")      # precisa estar no domínio para aceitar os cookies
            for c in self.cred.cookies:
                c = {k: v for k, v in c.items() if k in ("name", "value", "path", "domain", "secure",
                                                          "httpOnly", "expiry")}
                try:
                    drv.add_cookie(c)
                except Exception:
                    pass
            self._drv = drv
        except Exception as e:
            self._falhou = True
            print(f"  ⚠ navegador invisível indisponível para o 'Localizar' ({str(e)[:80]})")
        return self._drv

    def fechar(self):
        if self._drv is not None:
            try:
                self._drv.quit()
            except Exception:
                pass
            self._drv = None
