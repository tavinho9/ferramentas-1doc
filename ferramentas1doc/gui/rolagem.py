"""Área com rolagem, para o formulário de uma aba que pode não caber na
janela (tela pequena, janela no tamanho mínimo, escala de 125–150% no Windows
ou a área de Mensagens aumentada pelo divisor)."""

import tkinter as tk
from tkinter import ttk


class AreaRolavel(ttk.Frame):
    """Rolagem para um conteúdo criado dentro de `area.canvas`:

        area = AreaRolavel(pai)
        conteudo = ttk.Frame(area.canvas)      # ou uma aba
        area.colocar(conteudo)
        ...                                     # montagem pronta
        area.update_idletasks(); area.medir()

    Enquanto o conteúdo couber, ele ocupa a área toda (como num pack com
    expand) e não há barras. Quando a área fica mais baixa ou mais estreita
    que ele, aparece a barra daquela direção; a roda do mouse rola na
    vertical e, com Shift, na horizontal. Dentro da área, a roda nunca troca
    o valor de uma lista suspensa (o que acontecia sem querer)."""

    PASSO = 20          # pixels por unidade de rolagem
    UNIDADES_RODA = 3   # unidades por "clique" da roda

    def __init__(self, pai):
        super().__init__(pai)
        self.canvas = tk.Canvas(self, highlightthickness=0, borderwidth=0,
                                xscrollincrement=self.PASSO, yscrollincrement=self.PASSO)
        fundo = ttk.Style(self).lookup("TFrame", "background")
        if fundo:
            self.canvas.configure(background=fundo)
        self.barra = ttk.Scrollbar(self, orient="vertical", command=self.canvas.yview)
        self.barra_h = ttk.Scrollbar(self, orient="horizontal", command=self.canvas.xview)
        self.canvas.configure(yscrollcommand=self.barra.set, xscrollcommand=self.barra_h.set)
        self.canvas.grid(row=0, column=0, sticky="nsew")
        self.rowconfigure(0, weight=1)
        self.columnconfigure(0, weight=1)
        self.conteudo = self._item = None
        self.canvas.bind("<Configure>", lambda _e: self._ajustar())

    def colocar(self, conteudo):
        self.conteudo = conteudo
        self._item = self.canvas.create_window(0, 0, window=conteudo, anchor="nw")
        self._ligar_roda(conteudo)

    def medir(self):
        """Pede para a área o tamanho do conteúdo, para ela ocupar na janela o
        mesmo espaço que ele ocuparia sem rolagem. Chamar com a montagem
        pronta e os tamanhos calculados (update_idletasks)."""
        self.canvas.configure(width=self.conteudo.winfo_reqwidth(), height=self.conteudo.winfo_reqheight())

    def rolando(self):
        """True quando o conteúdo não cabe na altura e a barra vertical está à mostra."""
        return self.barra.winfo_manager() != ""

    def rolando_h(self):
        """True quando o conteúdo não cabe na largura e a barra horizontal está à mostra."""
        return self.barra_h.winfo_manager() != ""

    def _ajustar(self):
        """Conteúdo do tamanho da área, ou do que ele pede se for maior (aí com
        a barra daquela direção). Mostrar uma barra só encolhe a área, o que
        só pode fazer aparecer a outra: não há vaivém. Quando uma barra
        aparece ou some, avisa com <<RolagemMudou>> (a barra horizontal muda a
        altura que a área precisa na janela)."""
        if self.conteudo is None:
            return
        largura, altura = self.canvas.winfo_width(), self.canvas.winfo_height()
        total_l = max(largura, self.conteudo.winfo_reqwidth())
        total_a = max(altura, self.conteudo.winfo_reqheight())
        self.canvas.itemconfigure(self._item, width=total_l, height=total_a)
        self.canvas.configure(scrollregion=(0, 0, total_l, total_a))
        mudou = self._barra(self.barra, total_a > altura, self.canvas.yview_moveto,
                            row=0, column=1, sticky="ns")
        mudou |= self._barra(self.barra_h, total_l > largura, self.canvas.xview_moveto,
                             row=1, column=0, sticky="ew")
        if mudou:
            self.event_generate("<<RolagemMudou>>")

    @staticmethod
    def _barra(barra, precisa, ao_inicio, **grade):
        """Mostra ou esconde a barra; devolve True se mudou."""
        if precisa and not barra.winfo_manager():
            barra.grid(**grade)
            return True
        if not precisa and barra.winfo_manager():
            barra.grid_remove()
            ao_inicio(0)
            return True
        return False

    def _ligar_roda(self, raiz):
        """A roda em qualquer ponto do conteúdo rola a área. Uma marca própria
        à frente das de cada widget pega o evento antes da classe (a lista
        suspensa, que trocaria de valor)."""
        marca = f"Rolagem{self}"
        for seq in ("<MouseWheel>", "<Button-4>", "<Button-5>"):
            self.bind_class(marca, seq, lambda ev: self._roda(ev, self.rolando, self.canvas.yview_scroll))
        for seq in ("<Shift-MouseWheel>", "<Shift-Button-4>", "<Shift-Button-5>"):
            self.bind_class(marca, seq, lambda ev: self._roda(ev, self.rolando_h, self.canvas.xview_scroll))
        pendentes = [raiz, self.canvas]
        while pendentes:
            w = pendentes.pop()
            if isinstance(w, tk.Toplevel):          # janela secundária: rolagem própria
                continue
            if marca not in w.bindtags():
                w.bindtags((marca,) + w.bindtags())
            pendentes.extend(w.winfo_children())

    def _roda(self, ev, pode, rolar):
        if pode():
            if ev.num == 4 or (ev.num != 5 and ev.delta > 0):
                rolar(-self.UNIDADES_RODA, "units")
            elif ev.num == 5 or ev.delta < 0:
                rolar(self.UNIDADES_RODA, "units")
        return "break"
