"""Janelas secundárias do programa (Configurações, Opções da busca)."""

import tkinter as tk


class JanelaSecundaria(tk.Toplevel):
    """Janela presa à principal (transient): fica sempre na frente dela e
    some e volta junto quando o programa é minimizado e restaurado (a volta
    depende de acompanhar(principal), ver abaixo).

    De propósito, não é modal. Com um grab_set do Tk, o Tk do Windows descarta
    os comandos de sistema da janela principal (WM_SYSCOMMAND), inclusive
    minimizar e restaurar: com a janela de Configurações aberta, o programa
    minimizado pelo Win+D não voltava mais pela barra de tarefas e ficava
    inacessível. Desabilitar a principal (wm attributes -disabled) não resolve:
    o Windows também ignora esses comandos numa janela desabilitada."""

    abertas = set()

    def __init__(self, pai, titulo):
        super().__init__(pai)
        self.withdraw()                     # monta escondida e aparece já no lugar (mostrar)
        self.principal = pai.winfo_toplevel()
        self.mostrada = False
        self.title(titulo)
        self.transient(self.principal)
        self.bind("<Escape>", lambda _e: self.destroy())
        self.bind("<Destroy>", self._destruida, add="+")
        JanelaSecundaria.abertas.add(self)

    def _destruida(self, evento):
        if evento.widget is self:           # <Destroy> também chega pelos filhos
            JanelaSecundaria.abertas.discard(self)

    def mostrar(self, largura=None, altura=None):
        """Centraliza sobre a principal e mostra. Chamar no fim da montagem."""
        self.update_idletasks()
        p = self.principal
        w, h = largura or self.winfo_reqwidth(), altura or self.winfo_reqheight()
        x = p.winfo_rootx() + max(0, (p.winfo_width() - w) // 2)
        y = p.winfo_rooty() + max(0, (p.winfo_height() - h) // 3)
        self.geometry((f"{w}x{h}" if largura and altura else "") + f"+{x}+{y}")
        self.mostrada = True
        self.trazer()

    def trazer(self):
        self.deiconify()
        self.lift()
        self.focus_force()


def acompanhar(principal):
    """Quando a principal volta do minimizado, as secundárias abertas voltam
    com ela. O Windows esconde as secundárias junto com a principal, mas nem
    sempre as mostra de novo, e aí elas ficavam abertas e invisíveis."""
    def voltou(evento):
        if evento.widget is not principal:  # <Map> também chega pelos filhos
            return
        for janela in list(JanelaSecundaria.abertas):
            if janela.principal is principal and janela.mostrada and janela.winfo_exists():
                janela.deiconify()
                janela.lift()
    principal.bind("<Map>", voltou, add="+")


def abrir_unica(atual, criar):
    """Traz para a frente a janela `atual` se ela ainda está aberta; senão,
    cria outra com `criar()`. Devolve a janela aberta."""
    if atual is not None and atual.winfo_exists():
        atual.trazer()
        return atual
    return criar()
