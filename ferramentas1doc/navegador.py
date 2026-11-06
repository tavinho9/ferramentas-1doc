"""Chrome ou Edge já instalados, controlados pelo protocolo do navegador (CDP).

Substitui o Selenium. O navegador é aberto com --remote-debugging-port=0 num
perfil próprio; ele escolhe uma porta livre em 127.0.0.1 e a grava no arquivo
DevToolsActivePort do perfil, e o programa conversa com ele por WebSocket.
Não há driver (chromedriver/msedgedriver) e nada é baixado da internet.

O perfil próprio é obrigatório: a partir do Chrome 136 a porta de depuração é
ignorada no perfil padrão da pessoa. Sem perfil informado, usa-se uma pasta
temporária, apagada ao fechar.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time

import websocket

NOMES = {"chrome": "Google Chrome", "edge": "Microsoft Edge"}
_EXE_WINDOWS = {"chrome": "chrome.exe", "edge": "msedge.exe"}
_PASTA_WINDOWS = {"chrome": ("Google", "Chrome"), "edge": ("Microsoft", "Edge")}
_EXE_UNIX = {"chrome": ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser"),
             "edge": ("microsoft-edge", "microsoft-edge-stable")}
_EXE_MAC = {"chrome": "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "edge": "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"}
_LOCAL = ["127.0.0.1", "localhost"]


class ErroNavegador(Exception):
    """O navegador não abriu, fechou ou não respondeu."""


class _SemResposta(ErroNavegador):
    """O navegador não respondeu a tempo, mas a conexão continua de pé."""


# ============================================================
# Onde está o navegador
# ============================================================

def _app_paths(exe):
    """Caminho registrado pelo instalador em App Paths (usuário, depois máquina)."""
    try:
        import winreg
    except ImportError:
        return []
    achados = []
    for raiz in (winreg.HKEY_CURRENT_USER, winreg.HKEY_LOCAL_MACHINE):
        try:
            with winreg.OpenKey(raiz, rf"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}") as k:
                valor, _ = winreg.QueryValueEx(k, "")
        except OSError:
            continue
        if valor:
            achados.append(os.path.expandvars(valor.strip('"')))
    return achados


def candidatos(qual):
    """Caminhos possíveis do executável, na ordem em que são tentados."""
    if qual == "chrome" and os.environ.get("FERRAMENTAS1DOC_CHROME"):
        return [os.environ["FERRAMENTAS1DOC_CHROME"]]       # só para desenvolvimento e testes
    if os.name == "nt":
        exe = _EXE_WINDOWS[qual]
        lista = _app_paths(exe)
        for var in ("ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"):
            base = os.environ.get(var)
            if base:
                lista.append(os.path.join(base, *_PASTA_WINDOWS[qual], "Application", exe))
        return lista
    if sys.platform == "darwin":
        return [_EXE_MAC[qual]]
    return [c for c in (shutil.which(n) for n in _EXE_UNIX[qual]) if c]


def localizar_executavel(qual):
    """Caminho do chrome.exe/msedge.exe instalado, ou None."""
    for c in candidatos(qual):
        if os.path.isfile(c):
            return c
    return None


# ============================================================
# Navegador
# ============================================================

class Navegador:
    """Um navegador aberto pelo programa, falando CDP por um único WebSocket
    (o do próprio navegador). As páginas são acessadas por sessões
    "flatten" desse WebSocket."""

    def __init__(self, processo, ws, nome, perfil_temporario=None):
        self.processo = processo
        self.nome = nome
        self._ws = ws
        self._perfil_temporario = perfil_temporario
        self._trava = threading.Lock()
        self._id = 0
        self._sessoes = {}          # targetId -> sessionId
        self._pagina = None
        self._caiu = False
        self._fechado = False

    # ---------- abrir ----------

    @classmethod
    def abrir(cls, qual, perfil=None, headless=False, url="about:blank", espera=30):
        exe = localizar_executavel(qual)
        if not exe:
            raise ErroNavegador(f"{NOMES.get(qual, qual)} não encontrado neste computador")
        temporario = None
        if perfil is None:
            perfil = temporario = tempfile.mkdtemp(prefix="ferramentas1doc-navegador-")
        else:
            os.makedirs(perfil, exist_ok=True)
        porta_arq = os.path.join(perfil, "DevToolsActivePort")
        try:
            os.remove(porta_arq)            # de uma execução anterior: não pode ser confundido com o novo
        except FileNotFoundError:
            pass
        except OSError as e:
            raise ErroNavegador(f"o perfil do navegador está em uso ({e})") from None

        args = [exe, "--remote-debugging-port=0", f"--user-data-dir={perfil}",
                "--no-first-run", "--no-default-browser-check"]
        if headless:
            args += ["--headless=new", "--window-size=1920,1080"]
        if qual == "chrome" and os.environ.get("FERRAMENTAS1DOC_CHROME"):
            args.append("--no-sandbox")
        args.append(url)
        try:
            proc = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                    stderr=subprocess.DEVNULL)
        except OSError as e:
            _apagar(temporario)
            raise ErroNavegador(f"não consegui iniciar {exe} ({e})") from None

        try:
            ws = _conectar(proc, porta_arq, espera)
        except BaseException:
            _encerrar(proc, espera=0)
            _apagar(temporario)
            raise
        return cls(proc, ws, qual, temporario)

    # ---------- protocolo ----------

    def _enviar(self, metodo, params=None, sessao=None, timeout=15):
        with self._trava:
            if self._caiu or self._fechado:
                raise ErroNavegador("o navegador foi fechado")
            self._id += 1
            pedido = {"id": self._id, "method": metodo, "params": params or {}}
            if sessao:
                pedido["sessionId"] = sessao
            try:
                self._ws.settimeout(timeout)
                self._ws.send(json.dumps(pedido))
                while True:
                    resposta = json.loads(self._ws.recv())
                    if resposta.get("id") == self._id:
                        break               # o resto são eventos ou respostas atrasadas: ignorados
            except websocket.WebSocketTimeoutException:
                raise _SemResposta(f"o navegador não respondeu a {metodo}") from None
            except (websocket.WebSocketException, OSError, ValueError) as e:
                self._caiu = True
                raise ErroNavegador(f"conexão com o navegador perdida ({type(e).__name__})") from None
        if "error" in resposta:
            raise ErroNavegador(f"{metodo}: {resposta['error'].get('message', resposta['error'])}")
        return resposta.get("result") or {}

    def _sessao(self, alvo):
        s = self._sessoes.get(alvo)
        if s is None:
            s = self._enviar("Target.attachToTarget", {"targetId": alvo, "flatten": True})["sessionId"]
            self._sessoes[alvo] = s
        return s

    def _na_pagina(self, alvo, metodo, params=None, timeout=15):
        try:
            return self._enviar(metodo, params, sessao=self._sessao(alvo), timeout=timeout)
        except ErroNavegador:
            self._sessoes.pop(alvo, None)   # a página pode ter fechado; na próxima, conecta de novo
            raise

    # ---------- páginas ----------

    def paginas(self):
        """[{id, url}] das abas abertas."""
        r = self._enviar("Target.getTargets")
        return [{"id": t["targetId"], "url": t.get("url", "")}
                for t in r.get("targetInfos", []) if t.get("type") == "page"]

    def pagina(self):
        """A aba de trabalho: a mesma enquanto existir; senão a primeira; senão uma nova."""
        ids = [p["id"] for p in self.paginas()]
        if self._pagina not in ids:
            self._pagina = ids[0] if ids else self._enviar("Target.createTarget", {"url": "about:blank"})["targetId"]
        return self._pagina

    def ir(self, url, alvo=None):
        """Abre `url` na aba (não espera carregar)."""
        r = self._na_pagina(alvo or self.pagina(), "Page.navigate", {"url": url})
        if r.get("errorText"):
            raise ErroNavegador(f"não abriu {url} ({r['errorText']})")

    def url(self, alvo=None):
        alvo = alvo or self.pagina()
        return self._enviar("Target.getTargetInfo", {"targetId": alvo})["targetInfo"].get("url", "")

    def avaliar(self, alvo, expressao, timeout=15):
        """Valor (JSON) de uma expressão JavaScript na aba."""
        r = self._na_pagina(alvo, "Runtime.evaluate",
                            {"expression": expressao, "returnByValue": True, "awaitPromise": True},
                            timeout=timeout)
        if "exceptionDetails" in r:
            d = r["exceptionDetails"]
            raise ErroNavegador("erro no script: " + str((d.get("exception") or {}).get("description")
                                                         or d.get("text", ""))[:160])
        return (r.get("result") or {}).get("value")

    def versao(self):
        r = self._enviar("Browser.getVersion")
        return r.get("product", "")

    def viva(self):
        """False se a pessoa fechou todas as janelas (ou o navegador)."""
        if self._caiu or self._fechado or self.processo.poll() is not None:
            return False
        try:
            return bool(self.paginas())
        except _SemResposta:
            return True                     # lento, mas aberto
        except ErroNavegador:
            return False

    # ---------- cookies ----------

    def cookies(self, host=None):
        """Cookies do navegador (inclusive HttpOnly), no formato do sessao.json.
        Com `host`, só os que o navegador mandaria para ele."""
        lista = []
        for c in self._enviar("Storage.getCookies").get("cookies", []):
            dominio = c.get("domain", "")
            if host and not dominio_vale(dominio, host):
                continue
            d = {"name": c.get("name", ""), "value": c.get("value", ""), "domain": dominio,
                 "path": c.get("path") or "/", "secure": bool(c.get("secure")),
                 "httpOnly": bool(c.get("httpOnly"))}
            if c.get("sameSite"):
                d["sameSite"] = c["sameSite"]
            expira = c.get("expires")
            if not c.get("session") and isinstance(expira, (int, float)) and expira > 0:
                d["expiry"] = int(expira)
            lista.append(d)
        return lista

    def definir_cookies(self, cookies):
        """Grava os cookies no navegador. Um a um: um cookie recusado não
        impede os outros. Devolve quantos foram aceitos."""
        aceitos = 0
        for c in cookies:
            try:
                self._enviar("Storage.setCookies", {"cookies": [_cookie_cdp(c)]})
                aceitos += 1
            except (ErroNavegador, KeyError):
                pass
        return aceitos

    # ---------- fechar ----------

    def fechar(self):
        if self._fechado:
            return
        try:
            if not self._caiu:
                self._enviar("Browser.close", timeout=5)
        except ErroNavegador:
            pass                            # normal: o navegador fecha a conexão ao sair
        self._fechado = True
        try:
            self._ws.close()
        except Exception:
            pass
        _encerrar(self.processo)
        _apagar(self._perfil_temporario)


def dominio_vale(dominio, host):
    """True se um cookie do `dominio` é enviado ao `host`."""
    d = (dominio or "").lstrip(".").lower()
    host = (host or "").lower()
    return bool(d) and (host == d or host.endswith("." + d))


def _cookie_cdp(c):
    """Cookie do sessao.json → parâmetro do Storage.setCookies. Cookie de
    domínio (".x.com") continua de domínio; os demais ficam só para o host."""
    dominio = c.get("domain") or ""
    host = dominio.lstrip(".")
    caminho = c.get("path") or "/"
    p = {"name": c["name"], "value": c["value"], "path": caminho,
         "url": f"{'https' if c.get('secure') else 'http'}://{host}{caminho}"}
    if dominio.startswith("."):
        p["domain"] = dominio
    if c.get("secure"):
        p["secure"] = True
    if c.get("httpOnly"):
        p["httpOnly"] = True
    if c.get("sameSite") in ("Strict", "Lax", "None"):
        p["sameSite"] = c["sameSite"]
    if c.get("expiry"):
        p["expires"] = float(c["expiry"])
    return p


def _conectar(proc, porta_arq, espera):
    """Espera o navegador gravar DevToolsActivePort e abre o WebSocket."""
    fim = time.monotonic() + espera
    while True:
        try:
            with open(porta_arq, encoding="utf-8") as f:
                linhas = f.read().split()
        except OSError:
            linhas = []
        if len(linhas) >= 2 and linhas[0].isdigit():
            break
        if proc.poll() is not None:
            # Sai na hora quando o perfil já está aberto noutra janela (a
            # instância antiga recebe o pedido e esta encerra).
            raise ErroNavegador(f"o navegador fechou ao abrir (código {proc.returncode}); "
                                "o perfil pode estar em uso por outra janela")
        if time.monotonic() > fim:
            raise ErroNavegador(f"o navegador não ativou a porta de controle em {espera} s")
        time.sleep(0.1)
    url = f"ws://127.0.0.1:{linhas[0]}{linhas[1]}"
    try:
        # sem Origin (o navegador recusa origens desconhecidas) e sem proxy para 127.0.0.1
        return websocket.create_connection(url, timeout=15, suppress_origin=True, http_no_proxy=_LOCAL)
    except (websocket.WebSocketException, OSError) as e:
        raise ErroNavegador(f"não consegui falar com o navegador ({type(e).__name__}: {str(e)[:120]})") from None


def _encerrar(proc, espera=10):
    """Espera o processo sair sozinho (depois do Browser.close); senão, encerra."""
    try:
        proc.wait(timeout=espera)
        return
    except subprocess.TimeoutExpired:
        pass
    for passo in (proc.terminate, proc.kill):
        try:
            passo()
            proc.wait(timeout=5)
            return
        except (subprocess.TimeoutExpired, OSError):
            continue


def _apagar(pasta):
    """Apaga o perfil temporário. No Windows, os arquivos podem ficar presos
    por alguns instantes depois que o navegador sai."""
    if not pasta:
        return
    for _ in range(10):
        shutil.rmtree(pasta, ignore_errors=True)
        if not os.path.exists(pasta):
            return
        time.sleep(0.3)


# ============================================================
# "Localizar" do painel (último recurso dos verificadores de planilha)
# ============================================================

_JS_PAINEL_PRONTO = ("!window.__ferramentas1doc_antiga && document.readyState === 'complete'"
                     " && !!document.getElementById('num_codigo')")

_JS_PREENCHER = """(function (idDocumento, numero, ano) {
  // O <select> é escondido pelo select2: o valor vai no próprio elemento
  // nativo, que é o que o formulário envia.
  var s = document.getElementById('id_documento_rapido');
  if (!s) return 'sem o campo de tipo de documento';
  s.value = idDocumento;
  s.dispatchEvent(new Event('change', {bubbles: true}));
  function digitar(id, valor) {
    var e = document.getElementById(id);
    if (!e) return false;
    e.focus();
    e.value = valor;
    ['input', 'keyup', 'change'].forEach(function (t) { e.dispatchEvent(new Event(t, {bubbles: true})); });
    return true;
  }
  if (!digitar('num_codigo', numero)) return 'sem o campo do número';
  if (!digitar('localiza_ano', ano)) return 'sem o campo do ano';
  var b = document.querySelector("form.form_localiza button[type='submit']");
  if (!b) return 'sem o botão de localizar';
  window.__ferramentas1doc_antiga = true;
  b.click();
  return '';
})(%s, %s, %s)"""


def _aguardar(condicao, timeout, intervalo=0.2):
    fim = time.monotonic() + timeout
    while True:
        try:
            r = condicao()
            if r:
                return r
        except ErroNavegador:
            pass                            # página trocando: tenta de novo
        if time.monotonic() > fim:
            return None
        time.sleep(intervalo)


def localizar_no_painel(nav, url_painel, id_documento, digitos, ano, timeout, achou):
    """Preenche o "Localizar" do painel e devolve a URL em que a aba parou
    quando `achou(url, url_antes)` for verdadeiro; None se não chegou lá."""
    alvo = nav.pagina()
    try:
        # Marca o documento atual: o painel só conta como carregado num documento novo.
        nav.avaliar(alvo, "window.__ferramentas1doc_antiga = true")
    except ErroNavegador:
        pass
    nav.ir(url_painel, alvo)
    if not _aguardar(lambda: nav.avaliar(alvo, _JS_PAINEL_PRONTO), timeout):
        return None
    url_antes = nav.url(alvo)
    falta = nav.avaliar(alvo, _JS_PREENCHER % (json.dumps(str(id_documento)), json.dumps(str(digitos)),
                                               json.dumps(str(ano))))
    if falta:
        return None
    return _aguardar(lambda: (lambda u: u if achou(u, url_antes) else None)(nav.url(alvo)), timeout)
