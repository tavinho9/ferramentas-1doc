"""Janelas secundárias do programa (Configurações, Opções da busca)."""

import tkinter as tk


class JanelaSecundaria(tk.Toplevel):
    """Janela presa à principal (transient): fica sempre na frente dela e
    some e volta junto quando o programa é minimizado e restaurado.

    De propósito, não é modal. Com um grab_set do Tk, o Tk do Windows descarta
    os comandos de sistema da janela principal (WM_SYSCOMMAND), inclusive
    minimizar e restaurar: com a janela de Configurações aberta, o programa
    minimizado pelo Win+D não voltava mais pela barra de tarefas e ficava
    inacessível. Desabilitar a principal (wm attributes -disabled) não resolve:
    o Windows também ignora esses comandos numa janela desabilitada."""

    def __init__(self, pai, titulo):
        super().__init__(pai)
        self.withdraw()                     # monta escondida e aparece já no lugar (mostrar)
        self.principal = pai.winfo_toplevel()
        self.title(titulo)
        self.transient(self.principal)
        self.bind("<Escape>", lambda _e: self.destroy())

    def mostrar(self, largura=None, altura=None):
        """Centraliza sobre a principal e mostra. Chamar no fim da montagem."""
        self.update_idletasks()
        p = self.principal
        w, h = largura or self.winfo_reqwidth(), altura or self.winfo_reqheight()
        x = p.winfo_rootx() + max(0, (p.winfo_width() - w) // 2)
        y = p.winfo_rooty() + max(0, (p.winfo_height() - h) // 3)
        self.geometry((f"{w}x{h}" if largura and altura else "") + f"+{x}+{y}")
        self.trazer()

    def trazer(self):
        self.deiconify()
        self.lift()
        self.focus_force()


def abrir_unica(atual, criar):
    """Traz para a frente a janela `atual` se ela ainda está aberta; senão,
    cria outra com `criar()`. Devolve a janela aberta."""
    if atual is not None and atual.winfo_exists():
        atual.trazer()
        return atual
    return criar()
