"""As quatro abas do programa, uma por script.

Cada aba só monta os parâmetros e devolve uma tarefa:
    tarefa() -> (título, função(cred) -> caminho da planilha | None, precisa_de_sessão)
Quem executa (thread, login, mensagens) é a janela principal.
"""

import os
import tkinter as tk
from datetime import date, datetime, timedelta
from tkinter import filedialog, ttk

from .. import busca_argv, caminhos, config
from .secundaria import JanelaSecundaria, abrir_unica

DIAS_SEMANA = ("segunda-feira", "terça-feira", "quarta-feira", "quinta-feira", "sexta-feira",
               "sábado", "domingo")


class FormularioInvalido(ValueError):
    pass


def _descricao(pai, texto):
    lbl = ttk.Label(pai, text=texto, wraplength=860, justify="left", foreground="#444")
    lbl.pack(fill="x", padx=12, pady=(10, 6))
    return lbl


def _caixas(pai, rotulos, marcadas=None):
    """Linha de caixas de seleção; devolve {código: BooleanVar}."""
    quadro = ttk.Frame(pai)
    vars_ = {}
    for cod, rot in rotulos:
        v = tk.BooleanVar(value=(marcadas is None or cod in marcadas))
        ttk.Checkbutton(quadro, text=rot, variable=v).pack(side="left", padx=(0, 14))
        vars_[cod] = v
    return quadro, vars_


# ============================================================
# 1. Processos do dia
# ============================================================

class AbaProcessosDoDia(ttk.Frame):
    titulo = "Processos do dia"

    def __init__(self, pai):
        super().__init__(pai)
        from .. import processos_do_dia as mod
        self.mod = mod
        _descricao(self, "Processos emitidos num dia, nas caixas do setor, com a etapa atual do quadro. "
                         "Gera a planilha \"processos novos DD.MM.xlsx\" e o processos_do_dia.json. "
                         "Para hoje, entram só os processos cuja etapa atual tem menos de 24h; "
                         "para um dia passado, entram com a etapa atual, qualquer que seja o tempo nela.")
        f = ttk.Frame(self)
        f.pack(fill="x", padx=12, pady=6)
        ttk.Label(f, text="Dia de emissão:").grid(row=0, column=0, sticky="w")
        self.dia = tk.StringVar(value=date.today().strftime("%d/%m/%Y"))
        e = ttk.Entry(f, textvariable=self.dia, width=14)
        e.grid(row=0, column=1, padx=6)
        ttk.Button(f, text="Hoje", width=7,
                   command=lambda: self.dia.set(date.today().strftime("%d/%m/%Y"))).grid(row=0, column=2)
        ttk.Button(f, text="Ontem", width=7,
                   command=lambda: self.dia.set((date.today() - timedelta(days=1)).strftime("%d/%m/%Y"))
                   ).grid(row=0, column=3, padx=4)
        self.leitura = ttk.Label(f, foreground="#555")
        self.leitura.grid(row=0, column=4, padx=10, sticky="w")
        ttk.Label(f, text="DD/MM/AAAA, DD/MM/AA, DD/MM ou \"ontem\"; vazio = hoje",
                  foreground="#777").grid(row=1, column=1, columnspan=4, sticky="w", padx=6)
        self.dia.trace_add("write", lambda *_: self._mostrar_leitura())
        self._mostrar_leitura()

        g = ttk.Frame(self)
        g.pack(fill="x", padx=12, pady=(10, 4))
        ttk.Label(g, text="Caixas:").pack(side="left", padx=(0, 10))
        q, self.caixas = _caixas(g, mod.CAIXAS)
        q.pack(side="left")

    def _interpretar(self):
        hoje = date.today()
        txt = self.dia.get().strip().lower()
        if txt in ("", "hoje"):
            return hoje
        if txt == "ontem":
            return hoje - timedelta(days=1)
        return self.mod.interpretar_data(txt, hoje)

    def _mostrar_leitura(self):
        alvo = self._interpretar()
        if alvo is None:
            self.leitura.config(text="data não reconhecida", foreground="#b00020")
        elif alvo > date.today():
            self.leitura.config(text=f"{alvo:%d/%m/%Y} ainda não chegou", foreground="#b00020")
        else:
            self.leitura.config(text=f"→ {DIAS_SEMANA[alvo.weekday()]}, {alvo:%d/%m/%Y}", foreground="#555")

    def tarefa(self):
        alvo = self._interpretar()
        if alvo is None:
            raise FormularioInvalido("Data não reconhecida. Use DD/MM/AAAA, DD/MM/AA, DD/MM ou 'ontem'.")
        if alvo > date.today():
            raise FormularioInvalido(f"{alvo:%d/%m/%Y} ainda não chegou.")
        caixas = [(c, r) for c, r in self.mod.CAIXAS if self.caixas[c].get()]
        if not caixas:
            raise FormularioInvalido("Marque ao menos uma caixa.")
        return (f"Processos do dia {alvo:%d/%m/%Y}",
                lambda cred: self.mod.executar(alvo, cred, caixas=caixas), True)


# ============================================================
# 2. Busca
# ============================================================

class AbaBusca(ttk.Frame):
    titulo = "Busca"

    def __init__(self, pai):
        super().__init__(pai)
        _descricao(self, "Busca nas caixas do setor por etapa, fase, datas e mudanças de etapa. Gera uma "
                         "planilha com as abas Resultado, Transições e Parâmetros. Textos: sem diferença de "
                         "maiúsculas e acentos, \"contém\"; vários valores num campo, separados por ';', "
                         "valem como OU. Datas: DD/MM/AAAA, hoje, ontem ou -N (N dias atrás). "
                         "Ex.: \"Ainda não chegou a\" = Pesquisa de Mercado.")
        self.v = {}
        self.janela_ajuda = None
        corpo = ttk.Frame(self)
        corpo.pack(fill="both", expand=True, padx=12)

        # Universo
        u = ttk.LabelFrame(corpo, text="O que ler")
        u.grid(row=0, column=0, columnspan=2, sticky="ew", pady=4)
        q, self.caixas = _caixas(u, [("entrada", "Em aberto"), ("saida", "Caixa de saída"),
                                     ("arquivo", "Arquivados"), ("favoritos", "Favoritos")],
                                 marcadas={"entrada"})
        q.grid(row=0, column=0, columnspan=4, sticky="w", padx=8, pady=4)
        self._campo(u, 1, 0, "numeros", "Só estes números:", 30,
                    dica="ex.: 31.632/2026, 5712/2025 (no lugar das caixas)")
        ttk.Label(u, text="Ou números de um arquivo:").grid(row=2, column=0, sticky="w", padx=8, pady=2)
        self.v["numeros_arquivo"] = tk.StringVar()
        ttk.Entry(u, textvariable=self.v["numeros_arquivo"], width=44).grid(row=2, column=1, sticky="w")
        ttk.Button(u, text="Escolher…", command=self._escolher_numeros).grid(row=2, column=2, padx=4,
                                                                             sticky="w")

        # Etapa e posição
        e = ttk.LabelFrame(corpo, text="Etapa e posição")
        e.grid(row=1, column=0, sticky="nsew", pady=4, padx=(0, 6))
        self._campo(e, 0, 0, "etapa", "Etapa atual contém:", 24)
        self.v["etapa_exata"] = tk.BooleanVar()
        ttk.Checkbutton(e, text="nome exato", variable=self.v["etapa_exata"]).grid(row=0, column=2, sticky="w")
        self._campo(e, 1, 0, "excluir_etapa", "Exceto a etapa:", 24)
        ttk.Label(e, text="Fase:").grid(row=2, column=0, sticky="w", padx=8, pady=2)
        self.v["fase"] = tk.StringVar()
        ttk.Combobox(e, textvariable=self.v["fase"], values=("", "Elaboração", "Abertura"), width=14,
                     state="readonly").grid(row=2, column=1, sticky="w")
        self._campo(e, 3, 0, "antes_de", "Ainda não chegou a:", 24)
        self._campo(e, 4, 0, "a_partir_de", "Já chegou a:", 24)
        self._campo(e, 5, 0, "passou_por", "Já passou por:", 24)
        self._campo(e, 6, 0, "dias_etapa_min", "Dias na etapa, mín.:", 6)
        self._campo(e, 7, 0, "dias_etapa_max", "Dias na etapa, máx.:", 6)

        # Datas
        d = ttk.LabelFrame(corpo, text="Datas e mudanças de etapa")
        d.grid(row=1, column=1, sticky="nsew", pady=4)
        self._campo(d, 0, 0, "emissao_de", "Emitido de:", 11)
        self._campo(d, 0, 2, "emissao_ate", "até:", 11)
        self._campo(d, 1, 0, "mudanca_de", "Mudou de etapa de:", 11)
        self._campo(d, 1, 2, "mudanca_ate", "até:", 11)
        self._campo(d, 2, 0, "mudanca_para", "…para a etapa:", 24, span=3)
        self._campo(d, 3, 0, "mudanca_origem", "…saindo da etapa:", 24, span=3)
        self._campo(d, 4, 0, "entrada_de", "Entrou na etapa atual de:", 11)
        self._campo(d, 4, 2, "entrada_ate", "até:", 11)
        self._campo(d, 5, 0, "sem_mov_min", "Parado há (dias), mín.:", 6)

        # Outros
        o = ttk.LabelFrame(corpo, text="Outros")
        o.grid(row=2, column=0, columnspan=2, sticky="ew", pady=4)
        ttk.Label(o, text="Prioridade:").grid(row=0, column=0, sticky="w", padx=8)
        q, self.prioridades = _caixas(o, [(p, p) for p in busca_argv.PRIORIDADES])
        q.grid(row=0, column=1, columnspan=3, sticky="w")
        self._campo(o, 1, 0, "objeto", "Objeto contém:", 40, span=3)
        f = ttk.Frame(o)
        f.grid(row=2, column=0, columnspan=4, sticky="w", padx=8, pady=2)
        for chave, rot in (("csv", "Gerar também CSV"),
                           ("refazer", "Reabrir todos os processos (ignorar o cache)"),
                           ("offline", "Sem acessar o 1Doc (só o cache da última busca)")):
            self.v[chave] = tk.BooleanVar()
            ttk.Checkbutton(f, text=rot, variable=self.v[chave]).pack(side="left", padx=(0, 14))
        ttk.Label(o, text="Outras opções:").grid(row=3, column=0, sticky="w", padx=8, pady=(4, 6))
        self.v["extras"] = tk.StringVar()
        ttk.Entry(o, textvariable=self.v["extras"], width=70).grid(row=3, column=1, columnspan=2, sticky="w",
                                                                  pady=(4, 6))
        ttk.Button(o, text="Ver todas as opções…", command=self._ajuda).grid(row=3, column=3, padx=6,
                                                                           pady=(4, 6))
        corpo.columnconfigure(0, weight=1)
        corpo.columnconfigure(1, weight=1)

    def _campo(self, pai, linha, coluna, chave, rotulo, largura, dica=None, span=1):
        ttk.Label(pai, text=rotulo).grid(row=linha, column=coluna, sticky="w", padx=(8, 4), pady=2)
        self.v[chave] = tk.StringVar()
        ttk.Entry(pai, textvariable=self.v[chave], width=largura).grid(row=linha, column=coluna + 1,
                                                                      columnspan=span, sticky="w", pady=2)
        if dica:
            ttk.Label(pai, text=dica, foreground="#777").grid(row=linha, column=coluna + 1 + span,
                                                             sticky="w", padx=6)

    def _escolher_numeros(self):
        c = filedialog.askopenfilename(title="Arquivo com números de processo",
                                       filetypes=[("Planilha ou texto", "*.xlsx *.xlsm *.txt *.csv"),
                                                  ("Todos", "*.*")])
        if c:
            self.v["numeros_arquivo"].set(os.path.normpath(c))

    def _ajuda(self):
        self.janela_ajuda = abrir_unica(self.janela_ajuda, self._criar_ajuda)

    def _criar_ajuda(self):
        from .. import busca
        janela = JanelaSecundaria(self, "Opções da busca")
        t = tk.Text(janela, wrap="none", font=("Consolas", 9))
        sb = ttk.Scrollbar(janela, command=t.yview)
        t.configure(yscrollcommand=sb.set)
        sb.pack(side="right", fill="y")
        t.pack(fill="both", expand=True)
        t.insert("1.0", "No campo \"Outras opções\", escreva as opções como abaixo, por exemplo:\n"
                        "   --setor-etapa CPPFI --ordenar prioridade,dias_etapa:desc\n"
                        "Textos com espaço vão entre aspas: --cita-setor \"SPOT-DCL-GP-CPPFI\"\n\n"
                 + busca.montar_parser().format_help())
        t.configure(state="disabled")
        janela.mostrar(900, 620)
        return janela

    def formulario(self):
        dados = {k: v.get() for k, v in self.v.items()}
        dados["caixas"] = [c for c, v in self.caixas.items() if v.get()]
        dados["prioridade"] = [p for p, v in self.prioridades.items() if v.get()]
        return dados

    def tarefa(self):
        from .. import busca
        cfg = config.carregar()
        try:
            argv = busca_argv.montar_argv(self.formulario(), pasta_saida=config.pasta_saida(cfg),
                                          agora=datetime.now(), abrir=cfg["abrir_planilha"])
        except busca_argv.FormularioInvalido as e:
            raise FormularioInvalido(str(e)) from None
        erro = busca_argv.validar(argv)
        if erro:
            raise FormularioInvalido(f"Opções inválidas: {erro}")
        offline = "--offline" in argv

        def rodar(cred):
            print("Opções: " + " ".join(f'"{a}"' if " " in a else a for a in argv))
            res = busca.main(argv, cred=cred)
            return (res or {}).get("arquivo")
        return "Busca", rodar, not offline


# ============================================================
# 3 e 4. Planilha de acompanhamento
# ============================================================

class _AbaPlanilha(ttk.Frame):
    chave_config = ""

    def __init__(self, pai, descricao):
        super().__init__(pai)
        _descricao(self, descricao)
        self.campos = f = ttk.Frame(self)
        f.pack(fill="x", padx=12, pady=6)
        ttk.Label(f, text="Planilha:").grid(row=0, column=0, sticky="w")
        self.planilha = tk.StringVar(value=config.carregar().get(self.chave_config, ""))
        ttk.Entry(f, textvariable=self.planilha, width=80).grid(row=0, column=1, padx=6, sticky="we")
        ttk.Button(f, text="Escolher…", command=self._escolher).grid(row=0, column=2)
        ttk.Label(f, text="Aba:").grid(row=1, column=0, sticky="w", pady=6)
        self.aba = tk.StringVar()
        self.combo_aba = ttk.Combobox(f, textvariable=self.aba, width=30, state="readonly")
        self.combo_aba.grid(row=1, column=1, padx=6, sticky="w", pady=6)
        f.columnconfigure(1, weight=1)
        self.extra = ttk.Frame(self)
        self.extra.pack(fill="x", padx=12)
        self.planilha.trace_add("write", lambda *_: self._ler_abas())
        self._ler_abas()

    def _escolher(self):
        inicial = os.path.dirname(self.planilha.get()) if self.planilha.get() else caminhos.area_de_trabalho()
        c = filedialog.askopenfilename(title="Planilha de processos", initialdir=inicial,
                                       filetypes=[("Excel", "*.xlsx *.xlsm")])
        if c:
            self.planilha.set(os.path.normpath(c))

    def _ler_abas(self):
        caminho = self.planilha.get().strip().strip('"')
        nomes, ativa = [], ""
        if caminho and os.path.isfile(caminho):
            try:
                from openpyxl import load_workbook
                wb = load_workbook(caminho, read_only=True)
                nomes, ativa = wb.sheetnames, wb.active.title
                wb.close()
            except Exception:
                nomes = []
        self.combo_aba["values"] = nomes
        if self.aba.get() not in nomes:
            self.aba.set(ativa if nomes else "")

    def _validar(self):
        caminho = self.planilha.get().strip().strip('"')
        if not caminho:
            raise FormularioInvalido("Escolha a planilha.")
        if not os.path.isfile(caminho):
            raise FormularioInvalido(f"Arquivo não encontrado:\n{caminho}")
        cfg = config.carregar()
        cfg[self.chave_config] = caminho
        config.salvar(cfg)
        return caminho, (self.aba.get() or None)


class AbaFaltantes(_AbaPlanilha):
    titulo = "Faltantes na planilha"
    chave_config = "planilha_faltantes"

    def __init__(self, pai):
        super().__init__(pai, "Procura, nas caixas do setor, os Proc. Administrativos de abertura, fase "
                              "preparatória ou elaboração emitidos no período que estão ANTES da Pesquisa de "
                              "Mercado e não estão na planilha de acompanhamento. Gera "
                              "<planilha>_faltantes_….xlsx (a original não é alterada), com a aba "
                              "\"Cancelados (conferir)\" quando houver. A 1ª execução lê os arquivados e "
                              "demora; as seguintes usam o cache.")
        from .. import faltantes
        self.mod = faltantes
        cfg = config.carregar()
        ttk.Label(self.campos, text="Emitidos de:").grid(row=2, column=0, sticky="w")
        p = ttk.Frame(self.campos)
        p.grid(row=2, column=1, columnspan=2, padx=6, sticky="w")
        self.de = tk.StringVar(value=cfg["faltantes_de"])
        ttk.Entry(p, textvariable=self.de, width=12).pack(side="left")
        ttk.Label(p, text="até:").pack(side="left", padx=(10, 6))
        self.ate = tk.StringVar(value=cfg["faltantes_ate"])
        ttk.Entry(p, textvariable=self.ate, width=12).pack(side="left")
        ttk.Button(p, text="Padrão", width=8,
                   command=lambda: (self.de.set(""), self.ate.set(""))).pack(side="left", padx=(10, 0))
        self.leitura = ttk.Label(p, foreground="#555")
        self.leitura.pack(side="left", padx=10)
        ttk.Label(self.campos, text=f"{faltantes.FORMATOS_DATA}; vazio = padrão",
                  foreground="#777").grid(row=3, column=1, columnspan=2, sticky="w", padx=6)
        for v in (self.de, self.ate):
            v.trace_add("write", lambda *_: self._mostrar_periodo())
        self._mostrar_periodo()

    def _mostrar_periodo(self):
        hoje = date.today()
        try:
            de, ate = self.mod.ler_periodo(self.de.get(), self.ate.get(), hoje)
        except ValueError as e:
            self.leitura.config(text=str(e), foreground="#b00020")
            return
        inicio = f"{de:%d/%m/%Y}" if de else "1º/jan do ano mais antigo da planilha"
        fim = f"{ate:%d/%m/%Y}" if ate and ate < hoje else f"hoje ({hoje:%d/%m/%Y})"
        self.leitura.config(text=f"→ de {inicio} até {fim}", foreground="#555")

    def tarefa(self):
        try:
            de, ate = self.mod.ler_periodo(self.de.get(), self.ate.get())
        except ValueError as e:
            raise FormularioInvalido(str(e)) from None
        caminho, aba = self._validar()
        cfg = config.carregar()
        cfg["faltantes_de"], cfg["faltantes_ate"] = self.de.get().strip(), self.ate.get().strip()
        config.salvar(cfg)
        return ("Faltantes na planilha",
                lambda cred: self.mod.executar(caminho, aba, cred, de=de, ate=ate), True)


class AbaVerificar(_AbaPlanilha):
    titulo = "Verificar planilha"
    chave_config = "planilha_verificar"

    def __init__(self, pai):
        super().__init__(pai, "Confere, para cada processo da planilha, se ele já chegou à Pesquisa de "
                              "Mercado (ou a etapa posterior). Processos em elaboração são avaliados pelo "
                              "processo novo emitido a partir deles. Gera <planilha>_verificada_….xlsx só "
                              "com os que chegaram, mais as colunas Etapa atual, Processo pai, Processo "
                              "filho, Objeto (abertura) e Observação.")
        self.html = tk.BooleanVar()
        ttk.Checkbutton(self.extra, text="Salvar as páginas de processo baixadas (diagnóstico)",
                        variable=self.html).pack(side="left")

    def tarefa(self):
        from .. import verificar_planilha
        caminho, aba = self._validar()
        pasta_html = None
        if self.html.get():
            pasta_html = os.path.join(config.pasta_saida(), f"html 1Doc {datetime.now():%Y-%m-%d_%H%M}")
        return ("Verificar planilha",
                lambda cred: verificar_planilha.executar(caminho, aba, cred, pasta_html=pasta_html), True)


ABAS = (AbaProcessosDoDia, AbaBusca, AbaFaltantes, AbaVerificar)
