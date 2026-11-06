"""Saída dos scripts (print) → caixa de mensagens da janela e arquivo de log.

Os scripts escrevem no terminal com print, inclusive com '\\r' para atualizar
uma linha de progresso ("página 3/12…"). No programa, sys.stdout e sys.stderr
viram uma SaidaFila: cada write vai para uma fila que a janela consome (só a
thread principal mexe no Tk) e, durante uma execução, para o .log dela.
"""

import queue
import re
import threading

_RE_QUEBRA = re.compile(r"(\r\n|\r|\n)")


class SaidaFila:
    encoding = "utf-8"
    errors = "replace"

    def __init__(self):
        self.fila = queue.Queue()
        self._arquivo = None
        self._lock = threading.Lock()

    def write(self, texto):
        if not texto:
            return 0
        if not isinstance(texto, str):
            texto = str(texto)
        self.fila.put(texto)
        with self._lock:
            if self._arquivo is not None:
                try:
                    # no arquivo, cada atualização de progresso vira uma linha
                    self._arquivo.write(texto.replace("\r\n", "\n").replace("\r", "\n"))
                    self._arquivo.flush()
                except (OSError, ValueError):
                    pass
        return len(texto)

    def flush(self):
        pass

    def isatty(self):
        return False

    def gravar_em(self, caminho):
        """Começa a espelhar a saída no arquivo `caminho` (None = parar)."""
        with self._lock:
            if self._arquivo is not None:
                try:
                    self._arquivo.close()
                except OSError:
                    pass
                self._arquivo = None
            if caminho:
                self._arquivo = open(caminho, "a", encoding="utf-8")

    def gravar_no_log(self, texto):
        """Só no arquivo (ex.: traceback completo), não na janela."""
        with self._lock:
            if self._arquivo is not None:
                try:
                    self._arquivo.write(texto)
                    self._arquivo.flush()
                except (OSError, ValueError):
                    pass


class Terminal:
    """Aplica pedaços de texto com '\\r' e '\\n' como um terminal: '\\r' sem
    '\\n' faz o próximo texto substituir a linha atual. As operações reais
    (apagar a linha corrente, inserir texto) são passadas pela janela; assim a
    regra é testável sem Tk."""

    def __init__(self, apagar_linha, inserir):
        self.apagar_linha = apagar_linha
        self.inserir = inserir
        self._voltar = False

    def alimentar(self, texto):
        for parte in _RE_QUEBRA.split(texto):
            if not parte:
                continue
            if parte in ("\n", "\r\n"):
                self.inserir("\n")
                self._voltar = False
            elif parte == "\r":
                self._voltar = True
            else:
                if self._voltar:
                    self.apagar_linha()
                    self._voltar = False
                self.inserir(parte)


class TerminalEmMemoria(Terminal):
    """Para testes: guarda o resultado numa lista de linhas."""

    def __init__(self):
        self.linhas = [""]
        super().__init__(self._apagar, self._inserir)

    def _apagar(self):
        self.linhas[-1] = ""

    def _inserir(self, t):
        partes = t.split("\n")
        self.linhas[-1] += partes[0]
        self.linhas.extend(partes[1:])
