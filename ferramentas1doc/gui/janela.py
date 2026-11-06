"""Janela principal: barra da sessão, abas das ferramentas e mensagens."""

import os
import queue
import sys
import threading
import traceback
import tkinter as tk
from datetime import datetime
from tkinter import filedialog, messagebox, ttk

from .. import NOME_APP, VERSAO, caminhos, config, sessao
from .abas import ABAS, FormularioInvalido
from .log import SaidaFila, Terminal
from .rolagem import AreaRolavel
from .secundaria import JanelaSecundaria, abrir_unica

COR_OK, COR_AVISO, COR_ERRO, COR_NEUTRA = "#1b7f3b", "#9a6700", "#b00020", "#555555"


def caminho_recurso(nome):
    """Arquivo que acompanha o programa (ícone), empacotado ou não."""
    base = getattr(sys, "_MEIPASS", os.path.dirname(os.path.abspath(__file__)))
    for c in (os.path.join(base, nome), os.path.join(os.path.dirname(os.path.abspath(__file__)), nome)):
        if os.path.exists(c):
            return c
    return None


class Janela(tk.Tk):
    def __init__(self, saida: SaidaFila):
        super().__init__()
        self.saida = saida
        self.eventos = queue.Queue()      # avisos das threads para a janela
        self.executando = None            # thread da tarefa em curso
        self.ultimo_arquivo = None
        self.dialogo_configuracoes = None
        self._acomodar_pendente = None
        self.title(f"{NOME_APP} — Prefeitura de Montes Claros  (v{VERSAO})")
        self.geometry("1100x920")
        self.minsize(960, 700)
        icone = caminho_recurso("icone.ico")
        if icone and os.name == "nt":
            try:
                self.iconbitmap(default=icone)
            except tk.TclError:
                pass
        estilo = ttk.Style(self)
        if "vista" in estilo.theme_names():
            estilo.theme_use("vista")
        estilo.configure("Executar.TButton", font=("Segoe UI", 10, "bold"), padding=(14, 6))
        estilo.configure("Aviso.TLabel", background="#fff4ce", foreground="#5c4400", padding=8,
                         font=("Segoe UI", 10, "bold"))

        self._montar_topo()
        self._montar_base()                 # antes das abas: a barra de baixo nunca some
        self.painel = ttk.Panedwindow(self, orient="vertical")
        self.painel.pack(fill="both", expand=True, padx=12, pady=6)
        self._montar_abas()
        self._montar_mensagens()
        self.update_idletasks()             # tamanhos calculados: cada aba pede o espaço do formulário
        for area in self.areas:
            area.medir()
        self.update_idletasks()             # o pedido novo das abas chega até o painel
        self._acomodar_abas()
        self.painel.bind("<Configure>", lambda _e: self._agendar_acomodar())
        self.protocol("WM_DELETE_WINDOW", self._fechar)

        self.terminal = Terminal(self._apagar_linha, self._inserir)
        self.after(100, self._consumir)
        self._mostrar_sessao_salva()
        print(f"{NOME_APP} {VERSAO}. Pasta de dados: {caminhos.pasta_dados()}")
        print("O login no 1Doc é sempre feito por você, na janela do navegador: o programa só espera.\n")

    # ---------------------------------------------------------- montagem
    def _montar_topo(self):
        topo = ttk.Frame(self, padding=(12, 10, 12, 4))
        topo.pack(fill="x")
        self.lbl_sessao = ttk.Label(topo, text="", font=("Segoe UI", 10))
        self.lbl_sessao.pack(side="left")
        ttk.Button(topo, text="Configurações…", command=self._configuracoes).pack(side="right")
        self.btn_sair = ttk.Button(topo, text="Sair do 1Doc", command=self._sair)
        self.btn_sair.pack(side="right", padx=6)
        self.btn_entrar = ttk.Button(topo, text="Entrar no 1Doc", command=self._entrar)
        self.btn_entrar.pack(side="right")
        self.aviso_login = ttk.Label(self, style="Aviso.TLabel",
                                     text="➜ Faça o login no 1Doc na janela do navegador que abriu. "
                                          "O programa continua sozinho assim que você entrar.")

    def _montar_abas(self):
        """Cada aba: o formulário numa área com rolagem e, fora dela, o
        rodapé com o Executar, que assim nunca fica escondido."""
        self.notebook = ttk.Notebook(self.painel)
        self.painel.add(self.notebook, weight=0)
        self.abas, self.areas = [], []
        for cls in ABAS:
            pagina = ttk.Frame(self.notebook)
            rodape = ttk.Frame(pagina)
            rodape.pack(side="bottom", fill="x", padx=12, pady=10)
            area = AreaRolavel(pagina)
            area.pack(fill="both", expand=True)
            area.bind("<<RolagemMudou>>", lambda _e: self._agendar_acomodar())
            aba = cls(area.canvas)
            area.colocar(aba)
            btn = ttk.Button(rodape, text="Executar", style="Executar.TButton",
                             command=lambda a=aba: self._executar(a))
            btn.pack(side="right")
            aba.btn_executar = btn
            self.notebook.add(pagina, text=f"  {cls.titulo}  ")
            self.abas.append(aba)
            self.areas.append(area)

    def _agendar_acomodar(self):
        """Junta numa só acomodação os avisos de uma rajada (redimensionar a
        janela gera vários) e a faz depois de o Tk propagar os tamanhos pedidos
        (o painel não move o divisor quando o pedido da aba muda), sem forçar
        um update dentro do tratamento de um evento."""
        if self._acomodar_pendente is not None:
            self.after_cancel(self._acomodar_pendente)
        self._acomodar_pendente = self.after(30, self._acomodar_abas)

    def _acomodar_abas(self):
        """Divide a altura entre as abas e as Mensagens: as abas ficam com o
        que o formulário pede, desde que sobre para as Mensagens uma altura
        mínima; se não sobrar, o formulário rola. (Sem isto, as Mensagens
        sumiam e a aba era cortada embaixo, levando o Executar.)"""
        self._acomodar_pendente = None
        if not self.painel.winfo_exists() or len(self.painel.panes()) < 2:
            return
        minimo_mensagens = self.winfo_pixels("1.2i")
        cabe = self.painel.winfo_height() - minimo_mensagens
        self.painel.sashpos(0, max(0, min(self.notebook.winfo_reqheight(), cabe)))

    def _montar_mensagens(self):
        quadro = ttk.LabelFrame(self.painel, text="Mensagens", padding=6)
        self.painel.add(quadro, weight=1)
        self.texto = tk.Text(quadro, height=10, wrap="word", font=("Consolas", 9), state="disabled",
                             background="#fbfbfb")
        sb = ttk.Scrollbar(quadro, command=self.texto.yview)
        self.texto.configure(yscrollcommand=sb.set)
        sb.pack(side="right", fill="y")
        self.texto.pack(fill="both", expand=True)

    def _montar_base(self):
        base = ttk.Frame(self, padding=(12, 0, 12, 10))
        base.pack(side="bottom", fill="x")
        self.progresso = ttk.Progressbar(base, mode="determinate", length=180)
        self.progresso.pack(side="left")
        self.lbl_estado = ttk.Label(base, text="Pronto.", foreground=COR_NEUTRA)
        self.lbl_estado.pack(side="left", padx=10)
        self.btn_cancelar = ttk.Button(base, text="Cancelar", command=self._cancelar, state="disabled")
        self.btn_cancelar.pack(side="right")
        ttk.Button(base, text="Limpar mensagens", command=self._limpar).pack(side="right", padx=6)
        ttk.Button(base, text="Abrir pasta de saída",
                   command=lambda: caminhos.abrir_arquivo(config.pasta_saida())).pack(side="right")
        self.btn_abrir = ttk.Button(base, text="Abrir última planilha", state="disabled",
                                    command=lambda: self.ultimo_arquivo and caminhos.abrir_arquivo(
                                        self.ultimo_arquivo))
        self.btn_abrir.pack(side="right", padx=6)

    # ---------------------------------------------------------- mensagens
    def _inserir(self, t):
        self.texto.insert("end", t)

    def _apagar_linha(self):
        self.texto.delete("end-1c linestart", "end-1c")

    def _consumir(self):
        pedacos = []
        try:
            while len(pedacos) < 500:
                pedacos.append(self.saida.fila.get_nowait())
        except queue.Empty:
            pass
        if pedacos:
            no_fim = self.texto.yview()[1] >= 0.999
            self.texto.configure(state="normal")
            for p in pedacos:
                self.terminal.alimentar(p)
            linhas = int(self.texto.index("end-1c").split(".")[0])
            if linhas > 20000:                        # não deixa a caixa crescer sem limite
                self.texto.delete("1.0", f"{linhas - 15000}.0")
            self.texto.configure(state="disabled")
            if no_fim:
                self.texto.see("end")
        try:
            while True:
                tipo, valor = self.eventos.get_nowait()
                self._evento(tipo, valor)
        except queue.Empty:
            pass
        self.after(100, self._consumir)

    def _limpar(self):
        self.texto.configure(state="normal")
        self.texto.delete("1.0", "end")
        self.texto.configure(state="disabled")

    def _evento(self, tipo, valor):
        if tipo == "aguardando_login":
            if valor:
                self.aviso_login.pack(fill="x", padx=12, pady=(4, 0), after=self.lbl_sessao.master)
                self.lbl_estado.config(text="Esperando o login no navegador…", foreground=COR_AVISO)
            else:
                self.aviso_login.pack_forget()
        elif tipo == "sessao":
            self._mostrar_sessao(valor)
        elif tipo == "fim":
            self._terminou(*valor)

    def _mostrar_sessao(self, cred, validada=True):
        if cred is None:
            self.lbl_sessao.config(text="○ Não conectado ao 1Doc", foreground=COR_NEUTRA)
        elif validada:
            self.lbl_sessao.config(text=f"● Conectado: {cred.email} — setor {cred.setor}", foreground=COR_OK)
        else:
            self.lbl_sessao.config(text=f"◐ Sessão salva de {cred.email or 'usuário'} "
                                        "(conferida ao executar)", foreground=COR_AVISO)

    def _mostrar_sessao_salva(self):
        salva = sessao.carregar_salva()
        self._mostrar_sessao(salva, validada=False)

    # ---------------------------------------------------------- execução
    def _ocupado(self, sim):
        for aba in self.abas:
            aba.btn_executar.configure(state="disabled" if sim else "normal")
        self.btn_entrar.configure(state="disabled" if sim else "normal")
        self.btn_sair.configure(state="disabled" if sim else "normal")
        self.btn_cancelar.configure(state="normal" if sim else "disabled")
        if sim:
            self.progresso.configure(mode="indeterminate")
            self.progresso.start(12)
        else:
            self.progresso.stop()
            self.progresso.configure(mode="determinate", value=0)

    def _executar(self, aba):
        if self.executando:
            return
        try:
            titulo, funcao, precisa_sessao = aba.tarefa()
        except FormularioInvalido as e:
            messagebox.showwarning(NOME_APP, str(e), parent=self)
            return
        self._iniciar(titulo, funcao, precisa_sessao)

    def _entrar(self):
        if not self.executando:
            self._iniciar("Entrar no 1Doc", None, True)

    def _iniciar(self, titulo, funcao, precisa_sessao):
        sessao.CANCELAR.clear()
        self._ocupado(True)
        self.lbl_estado.config(text=f"{titulo}: executando…", foreground=COR_NEUTRA)
        nome_log = f"{datetime.now():%Y-%m-%d_%H%M%S} {titulo.split(' ')[0].lower()}.log"
        try:
            self.saida.gravar_em(os.path.join(caminhos.pasta_logs(), nome_log))
        except OSError:
            self.saida.gravar_em(None)
        self.executando = threading.Thread(target=self._trabalho, args=(titulo, funcao, precisa_sessao),
                                           daemon=True)
        self.executando.start()

    def _trabalho(self, titulo, funcao, precisa_sessao):
        """Roda na thread. Só fala com a janela pela fila de eventos."""
        resultado, estado, cor = None, "Concluído.", COR_OK
        print(f"\n{'─' * 70}\n▶ {titulo} — {datetime.now():%d/%m/%Y %H:%M}\n{'─' * 70}")
        try:
            cred = None
            if precisa_sessao:
                cred = sessao.obter_sessao(
                    ao_esperar=lambda sim: self.eventos.put(("aguardando_login", sim)))
                self.eventos.put(("sessao", cred))
            if funcao is not None:
                resultado = funcao(cred)
        except sessao.Cancelado:
            print("\n⏹ Cancelado.")
            estado, cor = "Cancelado.", COR_AVISO
        except sessao.LoginCancelado as e:
            print(f"\n✗ Login não concluído: {e}")
            estado, cor = "Login não concluído.", COR_ERRO
        except SystemExit as e:
            msg = e.code if isinstance(e.code, str) else f"encerrado (código {e.code})"
            print(f"\n{msg}")
            estado, cor = "Terminou com aviso — veja as mensagens.", COR_ERRO
        except Exception as e:
            self.saida.gravar_no_log(traceback.format_exc())
            print(f"\n✗ Erro inesperado: {type(e).__name__}: {e}")
            print("  Detalhes técnicos no arquivo de log (Configurações → Abrir pasta de dados → logs).")
            estado, cor = "Erro — veja as mensagens.", COR_ERRO
        finally:
            if not sessao.carregar_salva():
                self.eventos.put(("sessao", None))
            self.eventos.put(("fim", (estado, cor, resultado)))

    def _terminou(self, estado, cor, resultado):
        self.executando = None
        self.saida.gravar_em(None)
        self._ocupado(False)
        self.aviso_login.pack_forget()
        self.lbl_estado.config(text=estado, foreground=cor)
        if isinstance(resultado, str) and os.path.isfile(resultado):
            self.ultimo_arquivo = resultado
            self.btn_abrir.configure(state="normal")

    def _cancelar(self):
        if self.executando:
            sessao.CANCELAR.set()
            self.lbl_estado.config(text="Cancelando… (termina a requisição em curso)", foreground=COR_AVISO)

    def _sair(self):
        if self.executando:
            return
        if not messagebox.askyesno(NOME_APP, "Sair do 1Doc neste computador?\n\nA sessão salva e o "
                                             "perfil do navegador usado no login serão apagados; na próxima "
                                             "execução o navegador abre para você entrar de novo.",
                                   parent=self):
            return
        for aviso in sessao.sair():
            print(f"⚠ {aviso}")
        print("Sessão apagada. Na próxima execução o navegador abre para o login.")
        self._mostrar_sessao(None)

    def _fechar(self):
        if self.executando:
            if not messagebox.askyesno(NOME_APP, "Há uma execução em andamento. Cancelar e fechar?",
                                       parent=self):
                return
            sessao.CANCELAR.set()
        self.destroy()

    # ---------------------------------------------------------- configurações
    def _configuracoes(self):
        self.dialogo_configuracoes = abrir_unica(self.dialogo_configuracoes,
                                                 lambda: DialogoConfiguracoes(self))


class DialogoConfiguracoes(JanelaSecundaria):
    def __init__(self, pai):
        super().__init__(pai, "Configurações")
        self.resizable(False, False)
        self.cfg = config.carregar()
        f = ttk.Frame(self, padding=14)
        f.pack(fill="both", expand=True)

        ttk.Label(f, text="Navegador do login:").grid(row=0, column=0, sticky="w", pady=4)
        self.nav = tk.StringVar(value=config.NAVEGADORES[self.cfg["navegador"]])
        ttk.Combobox(f, textvariable=self.nav, values=list(config.NAVEGADORES.values()), state="readonly",
                     width=40).grid(row=0, column=1, columnspan=2, sticky="w")

        ttk.Label(f, text="Esperar o login por até:").grid(row=1, column=0, sticky="w", pady=4)
        self.minutos = tk.StringVar(value=str(self.cfg["minutos_login"]))
        g = ttk.Frame(f)
        g.grid(row=1, column=1, sticky="w")
        ttk.Spinbox(g, from_=1, to=60, textvariable=self.minutos, width=5).pack(side="left")
        ttk.Label(g, text="minutos").pack(side="left", padx=4)

        ttk.Label(f, text="Pasta das planilhas:").grid(row=2, column=0, sticky="w", pady=4)
        self.pasta = tk.StringVar(value=self.cfg["pasta_saida"])
        ttk.Entry(f, textvariable=self.pasta, width=44).grid(row=2, column=1, sticky="w")
        ttk.Button(f, text="Escolher…", command=self._escolher).grid(row=2, column=2, padx=4)
        ttk.Label(f, text=f"vazio = Área de Trabalho ({caminhos.area_de_trabalho()})",
                  foreground="#777").grid(row=3, column=1, columnspan=2, sticky="w")

        self.abrir = tk.BooleanVar(value=self.cfg["abrir_planilha"])
        ttk.Checkbutton(f, text="Abrir a planilha no Excel ao terminar", variable=self.abrir).grid(
            row=4, column=1, columnspan=2, sticky="w", pady=8)

        ttk.Separator(f).grid(row=5, column=0, columnspan=3, sticky="ew", pady=8)
        ttk.Label(f, text=f"Sessão, caches e logs ficam em:\n{caminhos.pasta_dados()}",
                  foreground="#555").grid(row=6, column=0, columnspan=2, sticky="w")
        ttk.Button(f, text="Abrir pasta de dados",
                   command=lambda: caminhos.abrir_arquivo(caminhos.pasta_dados())).grid(row=6, column=2)

        b = ttk.Frame(f)
        b.grid(row=7, column=0, columnspan=3, sticky="e", pady=(14, 0))
        ttk.Button(b, text="Cancelar", command=self.destroy).pack(side="right")
        ttk.Button(b, text="Salvar", command=self._salvar).pack(side="right", padx=6)
        self.mostrar()

    def _escolher(self):
        p = filedialog.askdirectory(title="Pasta das planilhas", parent=self,
                                    initialdir=self.pasta.get() or caminhos.area_de_trabalho())
        if p:
            self.pasta.set(os.path.normpath(p))

    def _salvar(self):
        nav = next((k for k, v in config.NAVEGADORES.items() if v == self.nav.get()), "auto")
        try:
            minutos = max(1, int(self.minutos.get()))
        except ValueError:
            messagebox.showwarning("Configurações", "Minutos: use um número inteiro.", parent=self)
            return
        pasta = self.pasta.get().strip()
        if pasta and not os.path.isdir(pasta):
            messagebox.showwarning("Configurações", f"A pasta não existe:\n{pasta}", parent=self)
            return
        cfg = config.carregar()
        cfg.update(navegador=nav, minutos_login=minutos, pasta_saida=pasta, abrir_planilha=self.abrir.get())
        config.salvar(cfg)
        self.destroy()
