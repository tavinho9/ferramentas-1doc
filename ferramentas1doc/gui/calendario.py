"""Seletor de data: um botão com ícone de calendário ao lado do campo, que
abre o calendário do mês logo abaixo dele. Clicar num dia escreve a data no
campo (DD/MM/AAAA), formato que todas as abas aceitam; digitar continua
valendo. Feito só com Tk, sem biblioteca a mais no instalador."""

import calendar
import re
import tkinter as tk
from datetime import date, timedelta
from tkinter import ttk

MESES = ("janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro",
         "outubro", "novembro", "dezembro")
DIAS_SEMANA = ("D", "S", "T", "Q", "Q", "S", "S")        # a semana começa no domingo
FORMATO = "%d/%m/%Y"

BRANCO, TEXTO, APAGADO, BLOQUEADO = "#ffffff", "#222222", "#9a9a9a", "#cfcfcf"
AZUL, AZUL_CLARO, BORDA = "#1f6fc5", "#dce9f9", "#8a8a8a"


def ler_data_qualquer(txt, hoje):
    """A data que está no campo, em qualquer formato das abas (DD/MM/AAAA,
    DD/MM/AA, DD/MM, AAAA-MM-DD, hoje, ontem, -N), para o calendário abrir
    no mês dela. None se não for uma data."""
    t = re.sub(r"\s+", " ", (txt or "").strip().lower())
    if not t:
        return None
    if t == "hoje":
        return hoje
    if t == "ontem":
        return hoje - timedelta(days=1)
    try:
        m = re.fullmatch(r"- ?(\d{1,5})", t)
        if m:
            return hoje - timedelta(days=int(m[1]))
        m = re.fullmatch(r"(\d{4})-(\d{1,2})-(\d{1,2})", t)
        if m:
            return date(int(m[1]), int(m[2]), int(m[3]))
    except (ValueError, OverflowError):
        return None
    from ..processos_do_dia import interpretar_data
    return interpretar_data(t, hoje)


def semanas(ano, mes):
    """As seis semanas (domingo a sábado) que mostram o mês, com os dias dos
    meses vizinhos completando as pontas. Sempre seis, para o tamanho do
    calendário não mudar de um mês para outro."""
    dias = list(calendar.Calendar(firstweekday=6).itermonthdates(ano, mes))
    while len(dias) < 42:
        dias.append(dias[-1] + timedelta(days=1))
    return [dias[i:i + 7] for i in range(0, 42, 7)]


def _mes_ao_lado(ano, mes, passo):
    n = ano * 12 + (mes - 1) + passo
    return n // 12, n % 12 + 1


# ---------------------------------------------------------------- botão

_icones = {}


def icone(raiz):
    """Ícone 16×16 de calendário, desenhado aqui (um por interpretador Tk)."""
    chave = id(raiz.tk)
    if chave not in _icones:
        img = tk.PhotoImage(master=raiz, width=16, height=16)
        for cor, caixa in (("#555555", (1, 2, 15, 15)),      # moldura
                           ("#c0392b", (2, 3, 14, 6)),       # faixa do mês
                           ("#ffffff", (2, 6, 14, 14)),      # folha
                           ("#555555", (4, 0, 6, 4)), ("#555555", (10, 0, 12, 4))):   # argolas
            img.put(cor, to=caixa)
        for x in (4, 7, 10):                                # dias
            for y in (8, 11):
                img.put("#8a8a8a", to=(x, y, x + 2, y + 2))
        _icones[chave] = img
    return _icones[chave]


class BotaoCalendario(ttk.Button):
    """Botão ao lado de um campo de data. `maximo`: último dia que se pode
    escolher (date ou função que a devolve, avaliada ao abrir), ou None."""

    def __init__(self, pai, campo, var, maximo=None):
        super().__init__(pai, image=icone(pai.winfo_toplevel()), style="Calendario.TButton",
                         takefocus=False, command=self.abrir)
        ttk.Style(self).configure("Calendario.TButton", padding=2)
        self.campo, self.var, self.maximo = campo, var, maximo

    def abrir(self):
        maximo = self.maximo() if callable(self.maximo) else self.maximo
        return Calendario(self.campo, self.var, maximo)


# ---------------------------------------------------------------- calendário

class Calendario(tk.Toplevel):
    """O calendário aberto, logo abaixo do campo (ou acima, sem espaço na
    tela). Fecha ao escolher um dia, com Esc, com clique fora dele, quando o
    programa perde o foco ou quando a janela principal é movida ou minimizada.
    Teclado: setas mudam o dia, PageUp/PageDown o mês, Enter escolhe."""

    aberto = None       # um por vez

    @classmethod
    def fechar_aberto(cls):
        if cls.aberto is not None:
            cls.aberto.fechar()

    def __init__(self, campo, var, maximo=None):
        Calendario.fechar_aberto()
        super().__init__(campo)
        Calendario.aberto = self
        self.campo, self.var, self.maximo = campo, var, maximo
        self.hoje = date.today()
        self.escolhida = ler_data_qualquer(var.get(), self.hoje)
        self.foco = self.escolhida or (min(self.hoje, maximo) if maximo else self.hoje)
        self.ano, self.mes = self.foco.year, self.foco.month
        self.sob = None                 # dia sob o ponteiro
        self.withdraw()
        self.overrideredirect(True)
        self._montar()
        self._desenhar()
        self._posicionar()
        self.deiconify()
        self.lift()
        try:
            self.attributes("-topmost", True)
        except tk.TclError:
            pass
        self._ligar()
        self._prender()

    # ---- montagem
    def _montar(self):
        caixa = tk.Frame(self, background=BRANCO, highlightthickness=1, highlightbackground=BORDA)
        caixa.pack()
        topo = tk.Frame(caixa, background=BRANCO)
        topo.grid(row=0, column=0, columnspan=7, sticky="ew", padx=4, pady=(4, 2))
        self.btn_anterior = ttk.Button(topo, text="◀", width=3, takefocus=False,
                                       command=lambda: self._mudar_mes(-1))
        self.btn_anterior.pack(side="left")
        self.btn_seguinte = ttk.Button(topo, text="▶", width=3, takefocus=False,
                                       command=lambda: self._mudar_mes(1))
        self.btn_seguinte.pack(side="right")
        self.titulo = tk.Label(topo, background=BRANCO, foreground=TEXTO, font=("Segoe UI", 10, "bold"))
        self.titulo.pack(side="left", expand=True)
        for i, d in enumerate(DIAS_SEMANA):
            tk.Label(caixa, text=d, background=BRANCO, foreground=APAGADO, width=3,
                     font=("Segoe UI", 9, "bold")).grid(row=1, column=i, pady=(2, 0))
        self.celulas = []
        for i in range(42):
            c = tk.Label(caixa, width=3, background=BRANCO, foreground=TEXTO, font=("Segoe UI", 9),
                         highlightthickness=1, highlightbackground=BRANCO, cursor="hand2", pady=2)
            c.grid(row=2 + i // 7, column=i % 7, padx=1, pady=1)
            c.bind("<Button-1>", lambda _e, i=i: self._clicar(i))
            c.bind("<Enter>", lambda _e, i=i: self._passar(i))
            c.bind("<Leave>", lambda _e: self._passar(None))
            self.celulas.append(c)
        rodape = tk.Frame(caixa, background=BRANCO)
        rodape.grid(row=8, column=0, columnspan=7, sticky="ew", padx=4, pady=(2, 4))
        self.btn_hoje = ttk.Button(rodape, text="Hoje", takefocus=False, command=lambda: self.escolher(self.hoje))
        self.btn_hoje.pack(side="left")
        self.btn_limpar = ttk.Button(rodape, text="Limpar", takefocus=False, command=self._limpar)
        self.btn_limpar.pack(side="right")

    def _bloqueado(self, d):
        return self.maximo is not None and d > self.maximo

    def _desenhar(self):
        self.titulo.configure(text=f"{MESES[self.mes - 1]} de {self.ano}")
        self.dias = [d for semana in semanas(self.ano, self.mes) for d in semana]
        for c, d in zip(self.celulas, self.dias):
            fundo, cor, peso = BRANCO, TEXTO, "normal"
            if d.month != self.mes:
                cor = APAGADO
            if self._bloqueado(d):
                cor = BLOQUEADO
            elif d == self.escolhida:
                fundo, cor, peso = AZUL, BRANCO, "bold"
            elif d == self.foco or d == self.sob:
                fundo = AZUL_CLARO
            c.configure(text=str(d.day), background=fundo, foreground=cor, font=("Segoe UI", 9, peso),
                        highlightbackground=AZUL if d == self.hoje else fundo,
                        cursor="arrow" if self._bloqueado(d) else "hand2")
        self.btn_hoje.state(["disabled"] if self._bloqueado(self.hoje) else ["!disabled"])
        seguinte = date(*_mes_ao_lado(self.ano, self.mes, 1), 1)
        self.btn_seguinte.state(["disabled"] if self._bloqueado(seguinte) else ["!disabled"])

    def _posicionar(self):
        self.update_idletasks()
        w, h = self.winfo_reqwidth(), self.winfo_reqheight()
        x, y = self.campo.winfo_rootx(), self.campo.winfo_rooty() + self.campo.winfo_height()
        if y + h > self.winfo_screenheight():
            y = self.campo.winfo_rooty() - h
        x = max(0, min(x, self.winfo_screenwidth() - w))
        self.geometry(f"+{x}+{max(0, y)}")

    # ---- eventos
    def _ligar(self):
        self.bind("<ButtonPress>", self._clique_fora)
        for seq, passo in (("<MouseWheel>", None), ("<Button-4>", -1), ("<Button-5>", 1)):
            self.bind(seq, lambda ev, p=passo: self._mudar_mes(p or (-1 if ev.delta > 0 else 1)))
        teclas = {"<Left>": -1, "<Right>": 1, "<Up>": -7, "<Down>": 7}
        for seq, dias in teclas.items():
            self.bind(seq, lambda _e, n=dias: self._mover_foco(timedelta(days=n)))
        self.bind("<Prior>", lambda _e: self._mudar_mes(-1, foco=True))
        self.bind("<Next>", lambda _e: self._mudar_mes(1, foco=True))
        self.bind("<Home>", lambda _e: self._mover_foco(self.hoje - self.foco))
        for seq in ("<Return>", "<KP_Enter>"):
            self.bind(seq, lambda _e: self.escolher(self.foco))
        self.bind("<Escape>", lambda _e: self.fechar())
        self.bind("<FocusOut>", lambda _e: self.after(50, self._conferir_foco))
        principal = self.campo.winfo_toplevel()
        if not getattr(principal, "_fecha_calendario", False):
            principal._fecha_calendario = True
            principal.bind("<Unmap>", lambda ev: ev.widget is principal and Calendario.fechar_aberto(), add="+")
            principal.bind("<Configure>", lambda ev: ev.widget is principal and Calendario.fechar_aberto(),
                           add="+")

    def _prender(self, tentativas=25):
        """Prende os cliques do programa no calendário (como a lista suspensa
        do Tk faz), para o clique fora dele fechá-lo. Só com ele à mostra."""
        if not self.winfo_exists():
            return
        try:
            self.grab_set()
        except tk.TclError:
            if tentativas:
                self.after(20, lambda: self._prender(tentativas - 1))
            return
        self.focus_force()

    def _clique_fora(self, ev):
        x0, y0 = self.winfo_rootx(), self.winfo_rooty()
        if not (x0 <= ev.x_root < x0 + self.winfo_width() and y0 <= ev.y_root < y0 + self.winfo_height()):
            self.fechar()
            return "break"
        return None

    def _conferir_foco(self):
        if not self.winfo_exists():
            return
        try:
            foco = self.focus_get()
        except (KeyError, tk.TclError):
            foco = None
        if foco is None or foco.winfo_toplevel() is not self:
            self.fechar()

    def _passar(self, i):
        self.sob = None if i is None else self.dias[i]
        self._desenhar()

    def _clicar(self, i):
        d = self.dias[i]
        if not self._bloqueado(d):
            self.escolher(d)
        return "break"

    def _mudar_mes(self, passo, foco=False):
        ano, mes = _mes_ao_lado(self.ano, self.mes, passo)
        if not 1900 <= ano <= 2100 or (passo > 0 and self._bloqueado(date(ano, mes, 1))):
            return
        self.ano, self.mes = ano, mes
        if foco:
            dia = min(self.foco.day, calendar.monthrange(ano, mes)[1])
            self.foco = date(ano, mes, dia)
            if self._bloqueado(self.foco):
                self.foco = self.maximo
        self._desenhar()

    def _mover_foco(self, delta):
        novo = self.foco + delta
        if self._bloqueado(novo) or not 1900 <= novo.year <= 2100:
            return
        self.foco = novo
        self.ano, self.mes = novo.year, novo.month
        self._desenhar()

    # ---- resultado
    def escolher(self, d):
        if self._bloqueado(d):
            return
        self.var.set(d.strftime(FORMATO))
        self.fechar()

    def _limpar(self):
        self.var.set("")
        self.fechar()

    def fechar(self):
        if Calendario.aberto is self:
            Calendario.aberto = None
        if not self.winfo_exists():
            return
        try:
            self.grab_release()
        except tk.TclError:
            pass
        campo = self.campo
        self.destroy()
        if campo.winfo_exists():
            campo.focus_set()
            campo.icursor("end")
