from __future__ import annotations

import queue
import threading
import tkinter as tk
from tkinter import messagebox, ttk
from typing import List

from .engine import Options, backup_base, remove_program
from .model import HIGH, Leftover, Program
from .registry import RegistryBackend, list_programs
from .scanner import Environment
from .system import System


def _size(kb: int) -> str:
    if not kb:
        return ""
    mb = kb / 1024
    return f"{mb / 1024:.1f} GB" if mb >= 1024 else f"{mb:.0f} MB"


class App(tk.Tk):
    def __init__(self, reg: RegistryBackend) -> None:
        super().__init__()
        self.reg = reg
        self.title("App Remover")
        self.geometry("900x600")
        self.minsize(640, 400)
        self.programs: List[Program] = []
        self.shown: List[Program] = []
        self.q: "queue.Queue" = queue.Queue()
        self.busy = False

        top = ttk.Frame(self, padding=8)
        top.pack(fill="x")
        ttk.Label(top, text="Search:").pack(side="left")
        self.search = tk.StringVar()
        self.search.trace_add("write", lambda *_: self.apply_filter())
        entry = ttk.Entry(top, textvariable=self.search)
        entry.pack(side="left", fill="x", expand=True, padx=6)
        entry.focus_set()
        self.btn_remove = ttk.Button(top, text="Remove", command=self.on_remove)
        self.btn_remove.pack(side="left", padx=2)
        ttk.Button(top, text="Refresh", command=self.refresh).pack(side="left", padx=2)

        opts = ttk.Frame(self, padding=(8, 0))
        opts.pack(fill="x")
        self.silent = tk.BooleanVar(value=True)
        self.review = tk.BooleanVar(value=False)
        self.force = tk.BooleanVar(value=False)
        ttk.Checkbutton(opts, text="Silent uninstall", variable=self.silent).pack(side="left")
        ttk.Checkbutton(opts, text="Review leftovers before deleting", variable=self.review).pack(side="left", padx=10)
        ttk.Checkbutton(opts, text="Force remove (skip its uninstaller)", variable=self.force).pack(side="left")

        cols = ("name", "publisher", "version", "size")
        self.tree = ttk.Treeview(self, columns=cols, show="headings", selectmode="browse")
        for c, w in zip(cols, (360, 220, 100, 80)):
            self.tree.heading(c, text=c.title())
            self.tree.column(c, width=w, anchor="w")
        self.tree.pack(fill="both", expand=True, padx=8, pady=6)
        self.tree.bind("<Double-1>", lambda _e: self.on_remove())
        self.tree.bind("<Return>", lambda _e: self.on_remove())

        self.status = tk.Text(self, height=9, state="disabled", wrap="word")
        self.status.pack(fill="x", padx=8, pady=(0, 8))
        self.after(100, self.pump)
        self.refresh()

    # -- list --------------------------------------------------------------
    def refresh(self) -> None:
        self.programs = list_programs(self.reg)
        self.apply_filter()

    def apply_filter(self) -> None:
        q = self.search.get().lower().strip()
        self.shown = [p for p in self.programs if q in p.name.lower() or q in p.publisher.lower()]
        self.tree.delete(*self.tree.get_children())
        for i, p in enumerate(self.shown):
            self.tree.insert("", "end", iid=str(i), values=(p.name, p.publisher, p.version, _size(p.size_kb)))

    # -- log / thread plumbing ---------------------------------------------
    def log(self, msg: str) -> None:
        self.q.put(("log", msg))

    def pump(self) -> None:
        try:
            while True:
                kind, payload = self.q.get_nowait()
                if kind == "log":
                    self.status.configure(state="normal")
                    self.status.insert("end", payload + "\n")
                    self.status.see("end")
                    self.status.configure(state="disabled")
                elif kind == "review":
                    items, box, done = payload
                    box.extend(self.review_dialog(items))
                    done.set()
                elif kind == "done":
                    self.busy = False
                    self.btn_remove.configure(state="normal")
                    self.refresh()
        except queue.Empty:
            pass
        self.after(100, self.pump)

    # -- actions -------------------------------------------------------------
    def on_remove(self) -> None:
        sel = self.tree.selection()
        if self.busy or not sel:
            return
        prog = self.shown[int(sel[0])]
        if not messagebox.askyesno("Remove", f"Remove {prog.name} and its leftovers?"):
            return
        self.busy = True
        self.btn_remove.configure(state="disabled")
        opts = Options(silent=self.silent.get(), force=self.force.get())
        review = self.review.get()
        self.log(f"=== {prog.name} ===")
        threading.Thread(target=self.worker, args=(prog, opts, review), daemon=True).start()

    def worker(self, prog: Program, opts: Options, review: bool) -> None:
        def choose(items: List[Leftover]) -> List[Leftover]:
            if not review:
                return [i for i in items if i.confidence == HIGH]
            box: List[Leftover] = []
            done = threading.Event()
            self.q.put(("review", (items, box, done)))
            done.wait()
            return box

        try:
            remove_program(prog, Environment.from_os(self.reg), System(), opts, self.log, choose, backup_base())
        except Exception as e:  # keep the UI alive whatever happens
            self.log(f"ERROR: {e}")
        finally:
            self.q.put(("done", None))

    def review_dialog(self, items: List[Leftover]) -> List[Leftover]:
        win = tk.Toplevel(self)
        win.title("Leftovers")
        win.geometry("760x420")
        win.transient(self)
        win.grab_set()
        tree = ttk.Treeview(win, columns=("what", "why"), show="tree headings", selectmode="none")
        tree.heading("#0", text="Item")
        tree.heading("what", text="Type")
        tree.heading("why", text="Why")
        tree.column("#0", width=430)
        tree.column("what", width=70)
        tree.column("why", width=200)
        checked = {}
        for n, it in enumerate(items):
            checked[str(n)] = it.confidence == HIGH
            tree.insert("", "end", iid=str(n), text=("[x] " if checked[str(n)] else "[ ] ") + it.label(),
                        values=(it.kind, it.reason + ("" if it.confidence == HIGH else " (less certain)")))

        def toggle(event):
            iid = tree.identify_row(event.y)
            if iid:
                checked[iid] = not checked[iid]
                tree.item(iid, text=("[x] " if checked[iid] else "[ ] ") + items[int(iid)].label())

        tree.bind("<Button-1>", toggle)
        tree.pack(fill="both", expand=True, padx=8, pady=8)
        result: List[Leftover] = []

        def ok():
            result.extend(it for n, it in enumerate(items) if checked[str(n)])
            win.destroy()

        bar = ttk.Frame(win, padding=8)
        bar.pack(fill="x")
        ttk.Button(bar, text="Delete checked", command=ok).pack(side="right")
        ttk.Button(bar, text="Skip", command=win.destroy).pack(side="right", padx=6)
        win.bind("<Return>", lambda _e: ok())
        self.wait_window(win)
        return result


def run_gui(reg: RegistryBackend) -> None:
    App(reg).mainloop()
