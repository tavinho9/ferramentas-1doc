"""Seletor de data: um botão com ícone de calendário ao lado do campo, que
abre o calendário do mês logo abaixo dele. Clicar num dia escreve a data no
campo (DD/MM/AAAA), formato que todas as abas aceitam; digitar continua
valendo. Para ir longe sem passar mês a mês, o título do calendário (mês e
ano) abre a grade dos meses do ano, e o título desta, a dos anos da década.
Feito só com Tk, sem biblioteca a mais no instalador."""

import calendar
import re
import tkinter as tk
from datetime import date, timedelta
from tkinter import font as tkfont
from tkinter import ttk

MESES = ("janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro",
         "outubro", "novembro", "dezembro")
MESES_CURTOS = tuple(m[:3] for m in MESES)
DIAS_SEMANA = ("D", "S", "T", "Q", "Q", "S", "S")        # a semana começa no domingo
FORMATO = "%d/%m/%Y"
ANO_MIN, ANO_MAX = 1900, 2100                            # até onde o calendário vai
COLUNAS = 4                                              # grades de meses e de anos: 4 × 3 casas
PAGINA = {"dias": 1, "meses": 12, "anos": 120}           # meses de uma grade inteira (PageUp/PageDown)
CASA = {"meses": 1, "anos": 12}                          # meses de uma casa (setas)

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


def anos_da_decada(ano):
    """Os anos da grade de anos: a década de `ano` (2020–2029), com o último
    ano da anterior e o primeiro da seguinte nas pontas, como os dias dos
    meses vizinhos na grade dos dias. 12 casas."""
    inicio = ano // 10 * 10
    return list(range(inicio - 1, inicio + 11))


def _mes_ao_lado(ano, mes, passo):
    n = ano * 12 + (mes - 1) + passo
    return n // 12, n % 12 + 1


def _no_mes(ano, mes, dia):
    """O dia `dia` do mês, ou o último dia dele (31 em fevereiro → 28 ou 29)."""
    return date(ano, mes, min(dia, calendar.monthrange(ano, mes)[1]))


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
    tela). Fecha ao escolher um dia, com Esc, com clique fora dele ou quando
    perde o foco (outro programa, a janela principal minimizada ou arrastada
    pela barra de título). Preso à principal (transient), como a lista
    suspensa do Tk: some junto se ela for minimizada.

    Três grades, no mesmo lugar e do mesmo tamanho: os dias do mês, os meses
    do ano e os anos da década. O título (com ▾) sobe de uma para a outra;
    clicar num ano abre os meses dele, e num mês, os dias. ◀ ▶ e a roda do
    mouse passam o mês, o ano ou a década, conforme a grade.
    Teclado: setas mudam o dia (nas outras grades, o mês ou o ano),
    PageUp/PageDown o mês (o ano, a década), Shift+PageUp/PageDown o ano,
    Home vai a hoje e Enter escolhe o dia (ou abre o mês ou o ano).

    Nada fica ligado à janela principal, e o que é agendado depois (conferir
    o foco, prender os cliques) fica no campo, que continua existindo: um
    after() no próprio calendário, disparado depois de ele fechar, chamaria
    um comando já apagado e daria erro no Tcl."""

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
        self.foco = (self.escolhida if self.escolhida and ANO_MIN <= self.escolhida.year <= ANO_MAX
                     else min(self.hoje, maximo) if maximo else self.hoje)
        self.ano, self.mes = self.foco.year, self.foco.month
        self.vista = "dias"             # a grade à mostra: "dias", "meses" ou "anos"
        self.sob = None                 # casa sob o ponteiro (a chave dela, ver _chave)
        self._sob_titulo = False
        self._fechando = False
        self.withdraw()
        self.overrideredirect(True)
        self.transient(campo.winfo_toplevel())
        self._montar()
        self._desenhar()
        self._fixar_tamanho()
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
        self.caixa = caixa = tk.Frame(self, background=BRANCO, highlightthickness=1, highlightbackground=BORDA)
        caixa.pack()
        topo = tk.Frame(caixa, background=BRANCO)
        topo.grid(row=0, column=0, sticky="ew", padx=4, pady=(4, 2))
        self.btn_anterior = ttk.Button(topo, text="◀", width=3, takefocus=False,
                                       command=lambda: self._avancar(-1))
        self.btn_anterior.pack(side="left")
        self.btn_seguinte = ttk.Button(topo, text="▶", width=3, takefocus=False,
                                       command=lambda: self._avancar(1))
        self.btn_seguinte.pack(side="right")
        self.titulo = tk.Label(topo, background=BRANCO, foreground=TEXTO, font=("Segoe UI", 10, "bold"),
                               padx=6)
        self.titulo.pack(side="left", expand=True)
        self.titulo.bind("<Button-1>", lambda _e: self._subir())
        self.titulo.bind("<Enter>", lambda _e: self._passar_titulo(True))
        self.titulo.bind("<Leave>", lambda _e: self._passar_titulo(False))

        self.grade_dias = tk.Frame(caixa, background=BRANCO)
        self.grade_dias.grid(row=1, column=0, sticky="nsew")
        for i, d in enumerate(DIAS_SEMANA):
            tk.Label(self.grade_dias, text=d, background=BRANCO, foreground=APAGADO, width=3,
                     font=("Segoe UI", 9, "bold")).grid(row=0, column=i, pady=(2, 0))
        self.celulas = []
        for i in range(42):
            c = tk.Label(self.grade_dias, width=3, background=BRANCO, foreground=TEXTO, font=("Segoe UI", 9),
                         highlightthickness=1, highlightbackground=BRANCO, cursor="hand2", pady=2)
            c.grid(row=1 + i // 7, column=i % 7, padx=1, pady=1)
            c.bind("<Button-1>", lambda _e, i=i: self._clicar(i))
            c.bind("<Enter>", lambda _e, i=i: self._passar(self.dias[i]))
            c.bind("<Leave>", lambda _e: self._passar(None))
            self.celulas.append(c)

        # meses do ano ou anos da década, no lugar da grade dos dias
        self.grade_lista = tk.Frame(caixa, background=BRANCO)
        self.grade_lista.grid(row=1, column=0, sticky="nsew", padx=2, pady=2)
        self.celulas_lista = []
        for i in range(12):
            c = tk.Label(self.grade_lista, background=BRANCO, foreground=TEXTO, font=("Segoe UI", 9),
                         highlightthickness=1, highlightbackground=BRANCO, cursor="hand2")
            c.grid(row=i // COLUNAS, column=i % COLUNAS, padx=2, pady=2, sticky="nsew")
            c.bind("<Button-1>", lambda _e, i=i: self._clicar_lista(i))
            c.bind("<Enter>", lambda _e, i=i: self._passar(self.itens[i]))
            c.bind("<Leave>", lambda _e: self._passar(None))
            self.celulas_lista.append(c)
        for col in range(COLUNAS):
            self.grade_lista.grid_columnconfigure(col, weight=1, uniform="coluna")
        for lin in range(12 // COLUNAS):
            self.grade_lista.grid_rowconfigure(lin, weight=1, uniform="linha")
        self.grade_lista.grid_remove()
        self.itens = []

        rodape = tk.Frame(caixa, background=BRANCO)
        rodape.grid(row=2, column=0, sticky="ew", padx=4, pady=(2, 4))
        self.btn_hoje = ttk.Button(rodape, text="Hoje", takefocus=False, command=lambda: self.escolher(self.hoje))
        self.btn_hoje.pack(side="left")
        self.btn_limpar = ttk.Button(rodape, text="Limpar", takefocus=False, command=self._limpar)
        self.btn_limpar.pack(side="right")

    def _fixar_tamanho(self):
        """As três grades ocupam o mesmo lugar, do mesmo tamanho: trocar de
        grade (ou de mês) não muda o tamanho do calendário. A largura também
        cabe o título mais comprido, para ◀ e ▶ não saírem do lugar."""
        self.update_idletasks()
        largura = max(self.grade_dias.winfo_reqwidth(), self.grade_lista.winfo_reqwidth())
        altura = max(self.grade_dias.winfo_reqheight(), self.grade_lista.winfo_reqheight())
        fonte = tkfont.Font(font=self.titulo.cget("font"))
        titulo = max(fonte.measure(f"{m} de 2000 ▾") for m in MESES) + 2 * int(self.titulo.cget("padx"))
        botoes = self.btn_anterior.winfo_reqwidth() + self.btn_seguinte.winfo_reqwidth()
        self.caixa.grid_columnconfigure(0, minsize=max(largura, titulo + botoes + 8))
        self.caixa.grid_rowconfigure(1, minsize=altura)

    def _posicionar(self):
        self.update_idletasks()
        w, h = self.winfo_reqwidth(), self.winfo_reqheight()
        x, y = self.campo.winfo_rootx(), self.campo.winfo_rooty() + self.campo.winfo_height()
        if y + h > self.winfo_screenheight():
            y = self.campo.winfo_rooty() - h
        x = max(0, min(x, self.winfo_screenwidth() - w))
        self.geometry(f"+{x}+{max(0, y)}")

    # ---- as grades
    def _bloqueado(self, d):
        return self.maximo is not None and d > self.maximo

    def _chave(self, d):
        """O que identifica a casa de um dia na grade à mostra: o dia, o mês
        (ano, mês) ou o ano. None fica None."""
        if d is None or self.vista == "dias":
            return d
        return (d.year, d.month) if self.vista == "meses" else d.year

    def _fora(self, chave):
        """A casa não se escolhe: depois do máximo, ou ano fora de 1900–2100."""
        if self.vista == "dias":
            return self._bloqueado(chave)
        if self.vista == "meses":
            return self._bloqueado(date(*chave, 1))
        return not ANO_MIN <= chave <= ANO_MAX or self._bloqueado(date(chave, 1, 1))

    def _pintar(self, celula, chave, texto, apagada):
        fundo, cor, peso = BRANCO, APAGADO if apagada else TEXTO, "normal"
        fora = self._fora(chave)
        if fora:
            cor = BLOQUEADO
        elif chave == self._chave(self.escolhida):
            fundo, cor, peso = AZUL, BRANCO, "bold"
        elif chave == self._chave(self.foco) or chave == self.sob:
            fundo = AZUL_CLARO
        celula.configure(text=texto, background=fundo, foreground=cor, font=("Segoe UI", 9, peso),
                         highlightbackground=AZUL if chave == self._chave(self.hoje) else fundo,
                         cursor="arrow" if fora else "hand2")

    def _desenhar(self):
        if self.vista == "dias":
            titulo = f"{MESES[self.mes - 1]} de {self.ano}"
            self.dias = [d for semana in semanas(self.ano, self.mes) for d in semana]
            for c, d in zip(self.celulas, self.dias):
                self._pintar(c, d, str(d.day), d.month != self.mes)
        elif self.vista == "meses":
            titulo = str(self.ano)
            self.itens = [(self.ano, m) for m in range(1, 13)]
            for c, chave in zip(self.celulas_lista, self.itens):
                self._pintar(c, chave, MESES_CURTOS[chave[1] - 1], False)
        else:
            self.itens = anos_da_decada(self.ano)
            titulo = f"{self.itens[1]}–{self.itens[10]}"
            for c, a in zip(self.celulas_lista, self.itens):
                self._pintar(c, a, str(a) if ANO_MIN <= a <= ANO_MAX else "", a in self.itens[::11])
        mostrar, esconder = ((self.grade_dias, self.grade_lista) if self.vista == "dias"
                             else (self.grade_lista, self.grade_dias))
        if not mostrar.winfo_manager():         # trocou de grade
            esconder.grid_remove()
            mostrar.grid()
        self.titulo.configure(text=titulo if self.vista == "anos" else f"{titulo} ▾")
        self._pintar_titulo()
        self.btn_hoje.state(["disabled"] if self._bloqueado(self.hoje) else ["!disabled"])
        for botao, passo in ((self.btn_anterior, -1), (self.btn_seguinte, 1)):
            botao.state(["!disabled"] if self._ao_lado(passo) else ["disabled"])

    def _pintar_titulo(self):
        """O título sobe de grade (dias → meses → anos): realçado sob o ponteiro, menos na de anos."""
        sobe = self.vista != "anos"
        self.titulo.configure(background=AZUL_CLARO if sobe and self._sob_titulo else BRANCO,
                              cursor="hand2" if sobe else "arrow")

    def _ao_lado(self, passo):
        """Para onde ◀ (passo -1), ▶ (1) e a roda do mouse levam: (ano, mês)
        do mês, do ano ou da década ao lado da grade à mostra. None se não há
        para onde ir: antes de 1900, depois de 2100 ou, para a frente, só
        dias depois do máximo."""
        mes = self.mes
        if self.vista == "dias":
            ano, mes = _mes_ao_lado(self.ano, self.mes, passo)
            primeiro, ultimo, inicio = ano, ano, date(ano, mes, 1)
        elif self.vista == "meses":
            ano = primeiro = ultimo = self.ano + passo
            inicio = date(ano, 1, 1)
        else:
            ano = primeiro = self.ano // 10 * 10 + 10 * passo
            ultimo, inicio = primeiro + 9, date(primeiro, 1, 1)
        if ultimo < ANO_MIN or primeiro > ANO_MAX or (passo > 0 and self._bloqueado(inicio)):
            return None
        return ano, mes

    # ---- eventos
    def _ligar(self):
        self.bind("<ButtonPress>", self._clique_fora)
        for seq, passo in (("<MouseWheel>", None), ("<Button-4>", -1), ("<Button-5>", 1)):
            self.bind(seq, lambda ev, p=passo: self._avancar(p or (-1 if ev.delta > 0 else 1)))
        setas = {"<Left>": -1, "<Right>": 1, "<Up>": -7, "<Down>": 7}      # na grade dos dias; nas outras,
        for seq, dias in setas.items():                                     # de uma linha (7) a outra (4)
            self.bind(seq, lambda _e, n=dias: self._seta(n))
        for seq, passo in (("<Prior>", -1), ("<Next>", 1)):
            self.bind(seq, lambda _e, p=passo: self._mover_meses(p * PAGINA[self.vista]))
        for seq, passo in (("<Shift-Prior>", -1), ("<Shift-Next>", 1)):
            self.bind(seq, lambda _e, p=passo: self._mover_meses(p * max(12, PAGINA[self.vista])))
        self.bind("<Home>", lambda _e: self._mover_foco(self.hoje - self.foco))
        for seq in ("<Return>", "<KP_Enter>"):
            self.bind(seq, lambda _e: self._confirmar())
        self.bind("<Escape>", lambda _e: self.fechar())
        self.bind("<FocusOut>", lambda _e: self._fechando or self.campo.after(50, self._conferir_foco))

    def _prender(self, tentativas=25):
        """Prende os cliques do programa no calendário (como a lista suspensa
        do Tk faz), para o clique fora dele fechá-lo. Só com ele à mostra."""
        if self._fechando or not self.winfo_exists():
            return
        try:
            self.grab_set()
        except tk.TclError:
            if tentativas:
                self.campo.after(20, lambda: self._prender(tentativas - 1))
            return
        self.focus_force()

    def _clique_fora(self, ev):
        x0, y0 = self.winfo_rootx(), self.winfo_rooty()
        if not (x0 <= ev.x_root < x0 + self.winfo_width() and y0 <= ev.y_root < y0 + self.winfo_height()):
            self.fechar()
            return "break"
        return None

    def _conferir_foco(self):
        if self._fechando or not self.winfo_exists():
            return
        try:
            foco = self.focus_get()
        except (KeyError, tk.TclError):
            foco = None
        if foco is None or foco.winfo_toplevel() is not self:
            self.fechar()

    def _passar(self, chave):
        self.sob = chave
        self._desenhar()

    def _passar_titulo(self, sob):
        self._sob_titulo = sob
        self._pintar_titulo()

    def _clicar(self, i):
        d = self.dias[i]
        if not self._bloqueado(d):
            self.escolher(d)
        return "break"

    def _clicar_lista(self, i):
        if i < len(self.itens):
            self._entrar(self.itens[i])
        return "break"

    def _subir(self):
        """Título: da grade dos dias à dos meses do ano, e desta à dos anos da década."""
        if self.vista != "anos":
            self.vista = "meses" if self.vista == "dias" else "anos"
            self.sob = None
            self._desenhar()
        return "break"

    def _entrar(self, chave):
        """Abre a casa: o mês (ano, mês) na grade dos dias, ou o ano na dos
        meses. O foco vai junto, para o mesmo dia (sem passar do máximo)."""
        if self.vista == "dias" or self._fora(chave):
            return
        if self.vista == "meses":
            (ano, mes), vista = chave, "dias"
        else:
            ano, mes, vista = chave, self.foco.month, "meses"
        novo = _no_mes(ano, mes, self.foco.day)
        self.vista, self.sob = vista, None
        self._focar(self.maximo if self._bloqueado(novo) else novo)

    def _avancar(self, passo):
        """◀ ▶ e a roda do mouse: o mês, o ano ou a década ao lado. O foco do teclado fica onde está."""
        destino = self._ao_lado(passo)
        if destino:
            self.ano, self.mes = destino
            self._desenhar()

    def _seta(self, n):
        if self.vista == "dias":
            self._mover_foco(timedelta(days=n))
        else:
            casas = n if abs(n) == 1 else COLUNAS * (1 if n > 0 else -1)
            self._mover_meses(casas * CASA[self.vista])

    def _mover_foco(self, delta):
        novo = self.foco + delta
        if self._bloqueado(novo) or not ANO_MIN <= novo.year <= ANO_MAX:
            return
        self._focar(novo)

    def _mover_meses(self, n):
        """PageUp/PageDown (e as setas nas grades de meses e anos): o foco vai
        n meses para trás ou para a frente, no mesmo dia, e a grade vai
        junto. Para a frente, para no máximo."""
        ano, mes = _mes_ao_lado(self.foco.year, self.foco.month, n)
        if not ANO_MIN <= ano <= ANO_MAX:
            return
        novo = _no_mes(ano, mes, self.foco.day)
        if self._bloqueado(novo):
            novo = self.maximo
            if n > 0 and novo <= self.foco:
                return
        self._focar(novo)

    def _focar(self, d):
        self.foco = d
        self.ano, self.mes = d.year, d.month
        self._desenhar()

    def _confirmar(self):
        """Enter: escolhe o dia do foco ou, nas grades de meses e anos, abre o mês ou o ano dele."""
        if self.vista == "dias":
            self.escolher(self.foco)
        else:
            self._entrar(self._chave(self.foco))

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
        if self._fechando or not self.winfo_exists():
            return
        self._fechando = True           # o <FocusOut> da destruição não agenda mais nada
        try:
            self.grab_release()
        except tk.TclError:
            pass
        campo = self.campo
        self.destroy()
        if campo.winfo_exists():
            campo.focus_set()
            campo.icursor("end")
