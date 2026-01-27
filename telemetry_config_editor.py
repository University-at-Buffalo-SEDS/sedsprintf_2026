#!/usr/bin/env python3
"""
Telemetry Config Editor (Tkinter)

Key behavior:
- Ctrl+S / Cmd+S saves.
- "Live editing" is in-memory only (no JSON write until Save).
- Debounced apply never writes into the newly-selected item (active edit idx tracked).
- Switching tabs flushes only the tab you're leaving.
- Selection no longer jumps to the top (no listbox rebuild during edits).
- Unsaved-changes prompt is HASH-BASED:
    * We only prompt if current config hash != last-saved/loaded hash.
    * Switching tabs won't mark dirty unless it truly changes config.

TelemetryError rules:
- Always exists, cannot be removed.
- Only editable field is its endpoints list.
"""

from __future__ import annotations

import hashlib
import json
import re
import sys
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, messagebox, ttk
from typing import Any, Dict, List, Optional

DATA_TYPE_OPTIONS = [
    "Float64",
    "Float32",
    "UInt8",
    "UInt16",
    "UInt32",
    "UInt64",
    "UInt128",
    "Int8",
    "Int16",
    "Int32",
    "Int64",
    "Int128",
    "Bool",
    "String",
    "Binary",
    "NoData",
]

MESSAGE_CLASS_OPTIONS = ["Data", "Error", "Warning"]
BROADCAST_MODE_OPTIONS = ["Always", "Never", "Default"]
ELEMENT_KIND_OPTIONS = ["Static", "Dynamic"]

TELEMETRY_ERROR_RUST = "TelemetryError"
TELEMETRY_ERROR_DOC = (
    "Encoded telemetry error text (string payload) (CRITICAL FOR SYSTEM FUNCTIONALITY, DO NOT REMOVE)"
)
TELEMETRY_ERROR_TEMPLATE: Dict[str, Any] = {
    "rust": TELEMETRY_ERROR_RUST,
    "name": "TELEMETRY_ERROR",
    "doc": TELEMETRY_ERROR_DOC,
    "class": "Error",
    "element": {"kind": "Dynamic", "data_type": "String"},
    "endpoints": [],
}


def find_project_root(start: Path) -> Path:
    cur = start.resolve()
    if cur.is_file():
        cur = cur.parent
    while True:
        if (cur / "Cargo.toml").exists():
            return cur
        if cur.parent == cur:
            return start.resolve().parent if start.is_file() else start.resolve()
        cur = cur.parent


def find_schema_json_from_config_rs(config_rs: Path, crate_root: Path) -> Optional[Path]:
    try:
        text = config_rs.read_text(encoding="utf-8")
    except Exception:
        return None

    rx = re.compile(r'(?s)define_telemetry_schema!\s*\(\s*[^)]*?\bpath\s*=\s*"([^"]+)"')
    caps = list(rx.finditer(text))
    if not caps:
        return None
    if len(caps) > 1:
        raise RuntimeError(f"Multiple define_telemetry_schema!(path=...) found in {config_rs}")
    rel = caps[0].group(1)
    return (crate_root / rel).resolve()


def default_blank_config() -> Dict[str, Any]:
    return {"endpoints": [], "types": [json.loads(json.dumps(TELEMETRY_ERROR_TEMPLATE))]}


def safe_read_json(path: Path) -> Optional[Dict[str, Any]]:
    try:
        with path.open("r", encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        return None
    except json.JSONDecodeError as e:
        raise RuntimeError(f"JSON parse error in {path}: {e}") from e


def safe_write_json(path: Path, data: Dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    with tmp.open("w", encoding="utf-8") as f:
        json.dump(data, f, indent=2, sort_keys=False)
        f.write("\n")
    tmp.replace(path)


def ensure_rust_ident(s: str) -> bool:
    return bool(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", s or ""))


def rust_ident_to_schema_name(rust: str) -> str:
    s = (rust or "").strip()
    if not s:
        return ""
    s = re.sub(r"[\s\-]+", "_", s)
    if "_" in s:
        return re.sub(r"_+", "_", s).strip("_").upper()
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    s = re.sub(r"([A-Z])([A-Z][a-z])", r"\1_\2", s)
    s = re.sub(r"_+", "_", s).strip("_")
    return s.upper()


def _endpoint_row_text(ep: Dict[str, Any]) -> str:
    return f"{ep.get('rust', '')}  [{ep.get('name', '')}]"


def _type_row_text(ty: Dict[str, Any]) -> str:
    tag = " (locked)" if str(ty.get("rust", "")).strip() == TELEMETRY_ERROR_RUST else ""
    return f"{ty.get('rust', '')}  [{ty.get('name', '')}]{tag}"


def _update_listbox_row(lb: tk.Listbox, idx: int, text: str) -> None:
    if idx < 0 or idx >= lb.size():
        return
    try:
        y0, _y1 = lb.yview()
    except Exception:
        y0 = None

    sel = lb.curselection()
    sel_idx = int(sel[0]) if sel else None

    lb.delete(idx)
    lb.insert(idx, text)

    if sel_idx is not None:
        try:
            lb.selection_clear(0, tk.END)
            lb.selection_set(sel_idx)
        except Exception:
            pass

    if y0 is not None:
        try:
            lb.yview_moveto(y0)
        except Exception:
            pass


def _selected_index(lb: tk.Listbox) -> Optional[int]:
    sel = lb.curselection()
    if not sel:
        return None
    return int(sel[0])


def _lb_all(lb: tk.Listbox) -> List[str]:
    return [lb.get(i) for i in range(lb.size())]


def _lb_selected(lb: tk.Listbox) -> List[str]:
    return [lb.get(i) for i in lb.curselection()]


def _lb_remove_items(lb: tk.Listbox, items: List[str]) -> None:
    if not items:
        return
    s = set(items)
    values = [v for v in _lb_all(lb) if v not in s]
    lb.delete(0, tk.END)
    for v in values:
        lb.insert(tk.END, v)


def _lb_add_unique(lb: tk.Listbox, items: List[str]) -> None:
    if not items:
        return
    existing = set(_lb_all(lb))
    for v in items:
        if v not in existing:
            lb.insert(tk.END, v)
            existing.add(v)


class TelemetryConfigEditor(tk.Tk):
    def __init__(self, crate_root: Path, config_rs: Path, json_path: Optional[Path]):
        super().__init__()
        self.title("Telemetry Config Editor (sedsprintf_rs)")
        self.geometry("1200x760")

        self.crate_root = crate_root
        self.config_rs_path = config_rs
        self.json_path = json_path

        self.config_obj: Dict[str, Any] = default_blank_config()
        self.status_var = tk.StringVar(value="")
        self.json_path_var = tk.StringVar(value=self._json_path_display())

        # HASH-BASED dirty state
        self._saved_hash: str = ""  # baseline (loaded/saved)
        self._cur_hash: str = ""  # current
        self.dirty = False

        # Live-edit bookkeeping
        self._suspend_live = False
        self._ep_live_job: Optional[str] = None
        self._ty_live_job: Optional[str] = None

        # Active edit indices
        self._ep_edit_idx: Optional[int] = None
        self._ty_edit_idx: Optional[int] = None

        self._build_menu()
        self._build_layout()
        self._bind_shortcuts()
        self._setup_live_edit_traces()

        # Track current tab so we can flush only the tab you're leaving
        self._tab_idx_endpoints = self.nb.index(self.endpoints_tab)
        self._tab_idx_types = self.nb.index(self.types_tab)
        self._active_tab_idx = self.nb.index("current")
        self.nb.bind("<<NotebookTabChanged>>", self._on_tab_changed)

        # Initial load
        if self.json_path and self.json_path.exists():
            self.load_from_path(self.json_path)
        else:
            self._ensure_telemetry_error_invariants(mark_hash=False)
            self.refresh_lists()
            # baseline = current (no "unsaved changes" just for being open)
            self._reset_saved_hash_to_current()
            self._set_status(
                "No JSON loaded. Use File → Save As… to create one, or choose an existing file.",
            )

        self.protocol("WM_DELETE_WINDOW", self.on_close)
        self.after(0, self._bring_to_front)  # type: ignore

    # ---------------- Hash + dirty management ----------------

    def _canonical_bytes(self) -> bytes:
        """
        Stable serialization for hashing.
        - sort_keys=True so dict key ordering doesn't matter.
        - separators minimize whitespace differences.
        - keep list order (endpoints/types order is meaningful).
        """
        blob = json.dumps(self.config_obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
        return blob.encode("utf-8")

    def _compute_hash(self) -> str:
        return hashlib.sha256(self._canonical_bytes()).hexdigest()

    def _update_hash_state(self, *, set_status: bool = False):
        self._cur_hash = self._compute_hash()
        self.dirty = (self._cur_hash != self._saved_hash)
        if set_status:
            if self.dirty:
                if self.json_path:
                    self._set_status(f"Modified (not saved) — {self.json_path}")
                else:
                    self._set_status("Modified (not saved) — no JSON path yet")
            else:
                # don't spam status; only set if we have something meaningful
                if self.json_path:
                    self._set_status(f"Up to date — {self.json_path}")
                else:
                    self._set_status("Up to date")

    def _mark_changed(self):
        # Call after any mutation that might change the config
        self._update_hash_state(set_status=True)

    def _reset_saved_hash_to_current(self):
        self._cur_hash = self._compute_hash()
        self._saved_hash = self._cur_hash
        self.dirty = False

    # ---------------- Shortcuts ----------------

    def _bind_shortcuts(self):
        self.bind_all("<Control-s>", lambda _e: self.menu_save())
        self.bind_all("<Control-S>", lambda _e: self.menu_save())
        self.bind_all("<Command-s>", lambda _e: self.menu_save())
        self.bind_all("<Command-S>", lambda _e: self.menu_save())

    # ---------------- Tab switching flush (only tab you leave) ----------------

    def _on_tab_changed(self, _e=None):
        new_idx = self.nb.index("current")
        prev_idx = self._active_tab_idx

        if prev_idx == self._tab_idx_endpoints:
            self._flush_endpoint_only()
        elif prev_idx == self._tab_idx_types:
            self._flush_type_only()

        self._active_tab_idx = new_idx

    # ---------------- Live editing wiring ----------------

    def _setup_live_edit_traces(self):
        self.ep_rust_var.trace_add("write", lambda *_: self._schedule_live_endpoint_apply())  # type: ignore
        self.ep_bm_var.trace_add("write", lambda *_: self._schedule_live_endpoint_apply())  # type: ignore
        self.ep_doc_text.bind("<<Modified>>", self._on_ep_doc_modified)

        self.ty_rust_var.trace_add("write", lambda *_: self._schedule_live_type_apply())  # type: ignore
        self.ty_class_var.trace_add("write", lambda *_: self._schedule_live_type_apply())  # type: ignore
        self.ty_kind_var.trace_add("write", lambda *_: self._schedule_live_type_apply())  # type: ignore
        self.ty_dtype_var.trace_add("write", lambda *_: self._schedule_live_type_apply())  # type: ignore
        self.ty_count_var.trace_add("write", lambda *_: self._schedule_live_type_apply())  # type: ignore
        self.ty_doc_text.bind("<<Modified>>", self._on_ty_doc_modified)

    def _on_ep_doc_modified(self, _e=None):
        try:
            if self.ep_doc_text.edit_modified():
                self.ep_doc_text.edit_modified(False)
                self._schedule_live_endpoint_apply()
        except Exception:
            pass

    def _on_ty_doc_modified(self, _e=None):
        try:
            if self.ty_doc_text.edit_modified():
                self.ty_doc_text.edit_modified(False)
                self._schedule_live_type_apply()
        except Exception:
            pass

    def _flush_endpoint_only(self):
        try:
            if self._ep_live_job:
                self.after_cancel(self._ep_live_job)
                self._ep_live_job = None
        except Exception:
            pass
        self._live_apply_endpoint()

    def _flush_type_only(self):
        try:
            if self._ty_live_job:
                self.after_cancel(self._ty_live_job)
                self._ty_live_job = None
        except Exception:
            pass
        self._live_apply_type()

    def _flush_all_pending(self):
        self._flush_endpoint_only()
        self._flush_type_only()

    def _schedule_live_endpoint_apply(self):
        if self._suspend_live:
            return
        if self._ep_live_job:
            try:
                self.after_cancel(self._ep_live_job)
            except Exception:
                pass
        self._ep_live_job = self.after(150, self._live_apply_endpoint)  # type: ignore

    def _schedule_live_type_apply(self):
        if self._suspend_live:
            return
        if self._ty_live_job:
            try:
                self.after_cancel(self._ty_live_job)
            except Exception:
                pass
        self._ty_live_job = self.after(150, self._live_apply_type)  # type: ignore

    # ---------------- List row updates (no rebuild = no jump) ----------------

    # ---------------- Debounced applies (apply to active edit idx) ----------------

    def _live_apply_endpoint(self):
        self._ep_live_job = None
        if self._suspend_live:
            return
        idx = self._ep_edit_idx
        if idx is None:
            return
        eps = self.config_obj.get("endpoints", [])
        if idx < 0 or idx >= len(eps):
            return

        ep = eps[idx]
        before = self._compute_hash()

        old_rust = str(ep.get("rust", ""))
        new_rust = self.ep_rust_var.get().strip()
        new_bm = (self.ep_bm_var.get() or "Default").strip()
        new_doc = self.ep_doc_text.get("1.0", tk.END).strip()

        if new_rust and not ensure_rust_ident(new_rust):
            return
        if new_bm not in BROADCAST_MODE_OPTIONS:
            return

        if new_rust:
            ep["rust"] = new_rust
            ep["name"] = rust_ident_to_schema_name(new_rust)
        ep["doc"] = new_doc
        ep["broadcast_mode"] = new_bm

        if old_rust and new_rust and old_rust != new_rust:
            for ty in self.config_obj.get("types", []):
                ty_eps = ty.get("endpoints", []) or []
                ty["endpoints"] = [new_rust if e == old_rust else e for e in ty_eps]

        self._ensure_telemetry_error_invariants(mark_hash=False)

        after = self._compute_hash()
        if after != before:
            self._mark_changed()

        _update_listbox_row(self.endpoint_list, idx, _endpoint_row_text(ep))
        self._refresh_type_editor_if_selected()

    def _live_apply_type(self):
        self._ty_live_job = None
        if self._suspend_live:
            return
        idx = self._ty_edit_idx
        if idx is None:
            return
        tys = self.config_obj.get("types", [])
        if idx < 0 or idx >= len(tys):
            return

        ty = tys[idx]
        before = self._compute_hash()

        is_te = str(ty.get("rust", "")).strip() == TELEMETRY_ERROR_RUST
        sel_eps = _lb_all(self.ty_selected_endpoints)

        if is_te:
            known = {ep.get("rust", "") for ep in self.config_obj.get("endpoints", [])}
            ty["endpoints"] = [e for e in sel_eps if e in known]
            self._ensure_telemetry_error_invariants(mark_hash=False)

            after = self._compute_hash()
            if after != before:
                self._mark_changed()

            _update_listbox_row(self.type_list, idx, _type_row_text(ty))
            return

        new_rust = self.ty_rust_var.get().strip()
        new_class = (self.ty_class_var.get() or "Data").strip()
        new_kind = (self.ty_kind_var.get() or "Static").strip()
        new_dtype = (self.ty_dtype_var.get() or DATA_TYPE_OPTIONS[0]).strip()
        new_doc = self.ty_doc_text.get("1.0", tk.END).strip()

        if new_rust and not ensure_rust_ident(new_rust):
            return
        if new_class not in MESSAGE_CLASS_OPTIONS:
            return
        if new_kind not in ELEMENT_KIND_OPTIONS:
            return
        if new_dtype not in DATA_TYPE_OPTIONS:
            return

        element: Dict[str, Any] = {"kind": new_kind, "data_type": new_dtype}
        if new_kind == "Static":
            cnt_s = self.ty_count_var.get().strip()
            if cnt_s == "":
                return
            try:
                cnt = int(cnt_s)
            except Exception:
                return
            if cnt < 0:
                return
            element["count"] = cnt

        if new_rust:
            ty["rust"] = new_rust
            ty["name"] = rust_ident_to_schema_name(new_rust)
        ty["doc"] = new_doc
        ty["class"] = new_class
        ty["element"] = element
        ty["endpoints"] = sel_eps

        self._ensure_telemetry_error_invariants(mark_hash=False)

        after = self._compute_hash()
        if after != before:
            self._mark_changed()

        _update_listbox_row(self.type_list, idx, _type_row_text(ty))

    # ---------------- Window bring-to-front ----------------

    def _bring_to_front(self):
        try:
            self.lift()
            self.attributes("-topmost", True)
            self.focus_force()
            self.after(100, lambda: self.attributes("-topmost", False))  # type: ignore
        except Exception:
            try:
                self.lift()
                self.focus_force()
            except Exception:
                pass

    def _json_path_display(self) -> str:
        return "(not found)" if self.json_path is None else str(self.json_path)

    # ---------------- UI scaffolding ----------------

    def _build_menu(self):
        menubar = tk.Menu(self)
        filem = tk.Menu(menubar, tearoff=0)
        filem.add_command(label="Open JSON…", command=self.menu_open_json)
        filem.add_command(label="Save", command=self.menu_save, accelerator="Ctrl/Cmd+S")
        filem.add_command(label="Save As…", command=self.menu_save_as)
        filem.add_separator()
        filem.add_command(label="Re-scan config.rs for JSON path", command=self.menu_rescan)
        filem.add_separator()
        filem.add_command(label="Quit", command=self.on_close)
        menubar.add_cascade(label="File", menu=filem)
        self.config(menu=menubar)

    def _build_layout(self):
        top = ttk.Frame(self, padding=10)
        top.pack(fill="both", expand=True)

        hdr = ttk.Frame(top)
        hdr.pack(fill="x")

        ttk.Label(hdr, text="crate root:").pack(side="left")
        ttk.Label(hdr, text=str(self.crate_root), foreground="gray").pack(side="left", padx=(6, 14))

        ttk.Label(hdr, text="config.rs:").pack(side="left")
        ttk.Label(hdr, text=str(self.config_rs_path), foreground="gray").pack(side="left", padx=(6, 14))

        ttk.Label(hdr, text="json:").pack(side="left")
        ttk.Label(hdr, textvariable=self.json_path_var, foreground="gray").pack(side="left", padx=(6, 14))

        nb = ttk.Notebook(top)
        nb.pack(fill="both", expand=True, pady=(10, 0))
        self.nb = nb

        self.endpoints_tab = ttk.Frame(nb, padding=10)
        self.types_tab = ttk.Frame(nb, padding=10)
        nb.add(self.endpoints_tab, text="Endpoints")
        nb.add(self.types_tab, text="Data Types")

        self._build_endpoints_tab(self.endpoints_tab)
        self._build_types_tab(self.types_tab)

        status = ttk.Frame(self, padding=(10, 6))
        status.pack(fill="x", side="bottom")
        ttk.Label(status, textvariable=self.status_var).pack(side="left")

    # ---------------- Endpoints tab ----------------

    def _build_endpoints_tab(self, parent: ttk.Frame):
        parent.columnconfigure(0, weight=1)
        parent.columnconfigure(1, weight=2)
        parent.rowconfigure(0, weight=1)

        left = ttk.LabelFrame(parent, text="Endpoints", padding=8)
        left.grid(row=0, column=0, sticky="nsew", padx=(0, 8))
        left.columnconfigure(0, weight=1)
        left.rowconfigure(1, weight=1)

        btns = ttk.Frame(left)
        btns.grid(row=0, column=0, sticky="ew")
        ttk.Button(btns, text="Add", command=self.add_endpoint).pack(side="left")
        ttk.Button(btns, text="Delete", command=self.delete_endpoint).pack(side="left", padx=6)

        self.endpoint_list = tk.Listbox(left, exportselection=False)
        self.endpoint_list.grid(row=1, column=0, sticky="nsew", pady=(8, 0))
        self.endpoint_list.bind("<<ListboxSelect>>", lambda _e: self.on_select_endpoint())

        right = ttk.LabelFrame(parent, text="Edit Endpoint (live)", padding=10)
        right.grid(row=0, column=1, sticky="nsew")
        right.columnconfigure(1, weight=1)

        ttk.Label(right, text="Name (PascalCase):").grid(row=0, column=0, sticky="w")
        self.ep_rust_var = tk.StringVar()
        ttk.Entry(right, textvariable=self.ep_rust_var).grid(row=0, column=1, sticky="ew", padx=(10, 0))

        ttk.Label(right, text="Doc (optional):").grid(row=1, column=0, sticky="w", pady=(10, 0))
        self.ep_doc_text = tk.Text(right, height=5)
        self.ep_doc_text.grid(row=1, column=1, sticky="ew", padx=(10, 0), pady=(10, 0))

        ttk.Label(right, text="Broadcast mode:").grid(row=2, column=0, sticky="w", pady=(10, 0))
        self.ep_bm_var = tk.StringVar(value="Default")
        ttk.Combobox(
            right, textvariable=self.ep_bm_var, values=BROADCAST_MODE_OPTIONS, state="readonly"
        ).grid(row=2, column=1, sticky="w", padx=(10, 0), pady=(10, 0))

        ttk.Label(
            right,
            text="Edits apply in-memory automatically. Use Ctrl/Cmd+S to write JSON.",
            foreground="gray",
        ).grid(row=3, column=0, columnspan=2, sticky="w", pady=(14, 0))

    # ---------------- Types tab ----------------

    def _build_types_tab(self, parent: ttk.Frame):
        parent.columnconfigure(0, weight=1)
        parent.columnconfigure(1, weight=2)
        parent.rowconfigure(0, weight=1)

        left = ttk.LabelFrame(parent, text="Data Types", padding=8)
        left.grid(row=0, column=0, sticky="nsew", padx=(0, 8))
        left.columnconfigure(0, weight=1)
        left.rowconfigure(1, weight=1)

        btns = ttk.Frame(left)
        btns.grid(row=0, column=0, sticky="ew")
        ttk.Button(btns, text="Add", command=self.add_type).pack(side="left")
        self.type_delete_btn = ttk.Button(btns, text="Delete", command=self.delete_type)
        self.type_delete_btn.pack(side="left", padx=6)

        self.type_list = tk.Listbox(left, exportselection=False)
        self.type_list.grid(row=1, column=0, sticky="nsew", pady=(8, 0))
        self.type_list.bind("<<ListboxSelect>>", lambda _e: self.on_select_type())

        right = ttk.LabelFrame(parent, text="Edit Data Type (live)", padding=10)
        right.grid(row=0, column=1, sticky="nsew")
        right.columnconfigure(1, weight=1)

        ttk.Label(right, text="Name (PascalCase):").grid(row=0, column=0, sticky="w")
        self.ty_rust_var = tk.StringVar()
        self.ty_rust_entry = ttk.Entry(right, textvariable=self.ty_rust_var)
        self.ty_rust_entry.grid(row=0, column=1, sticky="ew", padx=(10, 0))

        ttk.Label(right, text="Class:").grid(row=1, column=0, sticky="w", pady=(10, 0))
        self.ty_class_var = tk.StringVar(value="Data")
        self.ty_class_combo = ttk.Combobox(
            right, textvariable=self.ty_class_var, values=MESSAGE_CLASS_OPTIONS, state="readonly"
        )
        self.ty_class_combo.grid(row=1, column=1, sticky="w", padx=(10, 0), pady=(10, 0))

        ttk.Label(right, text="Element kind:").grid(row=2, column=0, sticky="w", pady=(10, 0))
        self.ty_kind_var = tk.StringVar(value="Static")
        self.ty_kind_combo = ttk.Combobox(
            right, textvariable=self.ty_kind_var, values=ELEMENT_KIND_OPTIONS, state="readonly"
        )
        self.ty_kind_combo.grid(row=2, column=1, sticky="w", padx=(10, 0), pady=(10, 0))
        self.ty_kind_combo.bind("<<ComboboxSelected>>", lambda _e: self._update_count_visibility())

        ttk.Label(right, text="Element data type:").grid(row=3, column=0, sticky="w", pady=(10, 0))
        self.ty_dtype_var = tk.StringVar(value=DATA_TYPE_OPTIONS[0])
        self.ty_dtype_combo = ttk.Combobox(
            right, textvariable=self.ty_dtype_var, values=DATA_TYPE_OPTIONS, state="readonly"
        )
        self.ty_dtype_combo.grid(row=3, column=1, sticky="w", padx=(10, 0), pady=(10, 0))

        self.ty_count_label = ttk.Label(right, text="Static count (for Static only):")
        self.ty_count_label.grid(row=4, column=0, sticky="nw", pady=(10, 0))
        self.ty_count_var = tk.StringVar(value="1")
        self.ty_count_entry = ttk.Entry(right, textvariable=self.ty_count_var, width=10)
        self.ty_count_entry.grid(row=4, column=1, sticky="nw", padx=(10, 0), pady=(10, 0))

        ttk.Label(right, text="Doc (optional):").grid(row=5, column=0, sticky="w", pady=(10, 0))
        self.ty_doc_text = tk.Text(right, height=5)
        self.ty_doc_text.grid(row=5, column=1, sticky="ew", padx=(10, 0), pady=(10, 0))

        epbox = ttk.LabelFrame(right, text="Endpoints for this DataType", padding=8)
        epbox.grid(row=6, column=0, columnspan=2, sticky="nsew", pady=(12, 0))
        epbox.columnconfigure(0, weight=1)
        epbox.columnconfigure(1, weight=0)
        epbox.columnconfigure(2, weight=1)
        epbox.rowconfigure(1, weight=1)

        ttk.Label(epbox, text="Available").grid(row=0, column=0, sticky="w")
        ttk.Label(epbox, text="Selected").grid(row=0, column=2, sticky="w")

        self.ty_available_endpoints = tk.Listbox(epbox, exportselection=False)
        self.ty_available_endpoints.grid(row=1, column=0, sticky="nsew")

        mid_btns = ttk.Frame(epbox)
        mid_btns.grid(row=1, column=1, sticky="ns", padx=8)

        ttk.Button(mid_btns, text="Add →", command=self.type_ep_add).pack(fill="x")
        ttk.Button(mid_btns, text="← Remove", command=self.type_ep_remove).pack(fill="x", pady=(6, 0))
        ttk.Separator(mid_btns, orient="horizontal").pack(fill="x", pady=10)
        ttk.Button(mid_btns, text="Add All →", command=self.type_ep_add_all).pack(fill="x")
        ttk.Button(mid_btns, text="← Remove All", command=self.type_ep_remove_all).pack(
            fill="x", pady=(6, 0)
        )

        self.ty_selected_endpoints = tk.Listbox(epbox, exportselection=False)
        self.ty_selected_endpoints.grid(row=1, column=2, sticky="nsew")

        ttk.Label(
            right,
            text="Edits apply in-memory automatically. Use Ctrl/Cmd+S to write JSON.",
            foreground="gray",
        ).grid(row=7, column=0, columnspan=2, sticky="w", pady=(14, 0))

        self._type_lock_widgets = [
            self.ty_rust_entry,
            self.ty_class_combo,
            self.ty_kind_combo,
            self.ty_dtype_combo,
            self.ty_count_entry,
        ]

        self._update_count_visibility()

    def _update_count_visibility(self):
        kind = (self.ty_kind_var.get() or "Static").strip()
        if kind == "Dynamic":
            try:
                self.ty_count_label.grid_remove()
                self.ty_count_entry.grid_remove()
            except Exception:
                pass
            if not self._suspend_live:
                self.ty_count_var.set("")
        else:
            try:
                self.ty_count_label.grid()
                self.ty_count_entry.grid()
            except Exception:
                pass
            if (self.ty_count_var.get() or "").strip() == "":
                self.ty_count_var.set("1")

    def _set_type_editor_locked_for_telemetry_error(self, locked: bool):
        try:
            self.type_delete_btn.configure(state="disabled" if locked else "normal")
        except Exception:
            pass

        for w in self._type_lock_widgets:
            try:
                w.configure(state="disabled" if locked else "normal")
            except Exception:
                pass

        if not locked:
            try:
                self.ty_class_combo.configure(state="readonly")
                self.ty_kind_combo.configure(state="readonly")
                self.ty_dtype_combo.configure(state="readonly")
            except Exception:
                pass

        try:
            self.ty_doc_text.configure(state="disabled" if locked else "normal")
        except Exception:
            pass

    # ---------------- Menu actions ----------------

    def menu_open_json(self):
        self._flush_all_pending()
        p = filedialog.askopenfilename(
            title="Open telemetry_config.json",
            initialdir=str(self.crate_root),
            filetypes=[("JSON files", "*.json"), ("All files", "*.*")],
        )
        if not p:
            return
        self.load_from_path(Path(p).resolve())

    def menu_save(self):
        if self.json_path is None:
            self.menu_save_as()
            return
        self.save_to_path(self.json_path)

    def menu_save_as(self):
        self._flush_all_pending()
        p = filedialog.asksaveasfilename(
            title="Save telemetry_config.json as…",
            initialdir=str(self.crate_root),
            defaultextension=".json",
            filetypes=[("JSON files", "*.json"), ("All files", "*.*")],
        )
        if not p:
            return
        path = Path(p).resolve()
        self.save_to_path(path)
        self.json_path = path
        self.json_path_var.set(str(self.json_path))

    def menu_rescan(self):
        self._flush_all_pending()
        found = find_schema_json_from_config_rs(self.config_rs_path, self.crate_root)
        if found is None:
            self._set_status(
                f"Could not find define_telemetry_schema!(path=...) in {self.config_rs_path}",
            )
            return
        self.json_path = found
        self.json_path_var.set(str(found))
        if found.exists():
            self.load_from_path(found)
        else:
            self._set_status(f"Discovered JSON path: {found} (does not exist yet)")

    # ---------------- Load / save ----------------

    def _ensure_telemetry_error_invariants(self, mark_hash: bool) -> bool:
        changed = False
        types = self.config_obj.setdefault("types", [])

        idxs = [i for i, t in enumerate(types) if str(t.get("rust", "")).strip() == TELEMETRY_ERROR_RUST]
        if not idxs:
            types.insert(0, json.loads(json.dumps(TELEMETRY_ERROR_TEMPLATE)))
            changed = True
        elif len(idxs) > 1:
            first = idxs[0]
            all_eps: List[str] = []
            for i in idxs:
                eps = types[i].get("endpoints", []) or []
                if isinstance(eps, list):
                    for e in eps:
                        if isinstance(e, str) and e not in all_eps:
                            all_eps.append(e)
            keep = types[first]
            keep["endpoints"] = all_eps
            for i in reversed(idxs[1:]):
                del types[i]
            changed = True

        te = next(t for t in self.config_obj["types"] if str(t.get("rust", "")).strip() == TELEMETRY_ERROR_RUST)

        eps = te.get("endpoints", []) or []
        if not isinstance(eps, list):
            eps = []
            changed = True

        fixed = {
            "rust": TELEMETRY_ERROR_RUST,
            "name": rust_ident_to_schema_name(TELEMETRY_ERROR_RUST),
            "doc": TELEMETRY_ERROR_DOC,
            "class": "Error",
            "element": {"kind": "Dynamic", "data_type": "String"},
        }
        for k, v in fixed.items():
            if te.get(k) != v:
                te[k] = json.loads(json.dumps(v))
                changed = True

        dedup_eps: List[str] = []
        for e in eps:
            if isinstance(e, str) and e not in dedup_eps:
                dedup_eps.append(e)
        if te.get("endpoints") != dedup_eps:
            te["endpoints"] = dedup_eps
            changed = True

        if changed and mark_hash:
            self._mark_changed()
        return changed

    def _normalize_schema_names(self) -> bool:
        changed = False
        for ep in self.config_obj.get("endpoints", []) or []:
            rust = str(ep.get("rust", "")).strip()
            expected = rust_ident_to_schema_name(rust)
            if ep.get("name") != expected:
                ep["name"] = expected
                changed = True

        for ty in self.config_obj.get("types", []) or []:
            rust = str(ty.get("rust", "")).strip()
            expected = rust_ident_to_schema_name(rust)
            if ty.get("name") != expected:
                ty["name"] = expected
                changed = True
        return changed

    def load_from_path(self, path: Path):
        self._flush_all_pending()
        obj = safe_read_json(path)
        if obj is None:
            self.config_obj = default_blank_config()
            self.json_path = path
            self.json_path_var.set(str(path))
            self._ensure_telemetry_error_invariants(mark_hash=False)
            self.refresh_lists()
            self._reset_saved_hash_to_current()
            self._set_status(f"File does not exist: {path}. Starting with blank config.")
            return

        self.config_obj = obj
        self.json_path = path
        self.json_path_var.set(str(path))

        # normalize/enforce (may change config)
        self._normalize_schema_names()
        self._ensure_telemetry_error_invariants(mark_hash=False)

        self.refresh_lists()

        # baseline is the *loaded file state* (but we normalized/enforced already),
        # so if normalization changed anything, it should be considered unsaved.
        # Achieve that by:
        #  - saved_hash = hash of the ORIGINAL file content
        #  - current_hash = hash after normalize/enforce
        saved_hash_original = hashlib.sha256(
            json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
        ).hexdigest()
        self._saved_hash = saved_hash_original
        self._update_hash_state(set_status=False)

        if self.dirty:
            self._set_status(f"Loaded {path} (normalized/enforced invariants; please Save)")
        else:
            self._set_status(f"Loaded {path}")

    def save_to_path(self, path: Path):
        self._flush_all_pending()
        self._ensure_telemetry_error_invariants(mark_hash=False)
        self._normalize_schema_names()

        try:
            self.validate_current_config()
        except Exception as e:
            messagebox.showerror("Validation error", str(e))
            return

        safe_write_json(path, self.config_obj)
        self.json_path = path
        self.json_path_var.set(str(path))

        # after actual save, baseline becomes current
        self._reset_saved_hash_to_current()
        self._set_status(f"Saved {path}")

    # ---------------- Helpers ----------------

    def _set_status(self, msg: str):
        self.status_var.set(msg)

    def on_close(self):
        self._flush_all_pending()
        self._update_hash_state(set_status=False)
        if self.dirty:
            if not messagebox.askyesno("Unsaved changes", "You have unsaved changes. Quit anyway?"):
                return
        self.destroy()

    def validate_current_config(self):
        self._ensure_telemetry_error_invariants(mark_hash=False)

        obj = self.config_obj
        if "endpoints" not in obj or "types" not in obj:
            raise RuntimeError("JSON must contain top-level keys: endpoints, types")

        for i, ep in enumerate(obj["endpoints"]):
            rust = str(ep.get("rust", "")).strip()
            if not ensure_rust_ident(rust):
                raise RuntimeError(f"endpoints[{i}].rust must be Rust ident/PascalCase, got {rust!r}")

            expected = rust_ident_to_schema_name(rust)
            name = str(ep.get("name", "")).strip()
            if name != expected:
                raise RuntimeError(
                    f"endpoints[{i}].name must match generated name from rust ({expected!r}), got {name!r}"
                )

            bm = ep.get("broadcast_mode", "Default")
            if bm not in BROADCAST_MODE_OPTIONS:
                raise RuntimeError(
                    f"endpoints[{i}].broadcast_mode must be one of {BROADCAST_MODE_OPTIONS}, got {bm!r}"
                )

        endpoint_rust_set = {ep.get("rust", "") for ep in obj["endpoints"]}

        for i, ty in enumerate(obj["types"]):
            rust = str(ty.get("rust", "")).strip()
            if not ensure_rust_ident(rust):
                raise RuntimeError(f"types[{i}].rust must be Rust ident/PascalCase, got {rust!r}")

            expected = rust_ident_to_schema_name(rust)
            name = str(ty.get("name", "")).strip()
            if name != expected:
                raise RuntimeError(
                    f"types[{i}].name must match generated name from rust ({expected!r}), got {name!r}"
                )

            cls = ty.get("class", "")
            if cls not in MESSAGE_CLASS_OPTIONS:
                raise RuntimeError(f"types[{i}].class must be one of {MESSAGE_CLASS_OPTIONS}, got {cls!r}")

            el = ty.get("element", {}) or {}
            kind = el.get("kind", "")
            if kind not in ELEMENT_KIND_OPTIONS:
                raise RuntimeError(f"types[{i}].element.kind must be one of {ELEMENT_KIND_OPTIONS}, got {kind!r}")

            dt = el.get("data_type", "")
            if dt not in DATA_TYPE_OPTIONS:
                raise RuntimeError(f"types[{i}].element.data_type must be one of DATA_TYPE_OPTIONS, got {dt!r}")

            if kind == "Static":
                cnt = el.get("count", None)
                if cnt is None:
                    raise RuntimeError(f"types[{i}].element.count is required for Static")
                cnti = int(cnt)
                if cnti < 0:
                    raise RuntimeError(f"types[{i}].element.count must be >= 0, got {cnti}")

            eps = ty.get("endpoints", [])
            if not isinstance(eps, list):
                raise RuntimeError(f"types[{i}].endpoints must be list of endpoint rust names")
            for epr in eps:
                if epr not in endpoint_rust_set:
                    raise RuntimeError(
                        f"types[{i}] ({rust}) references unknown endpoint {epr!r}. "
                        f"Known endpoints: {sorted(endpoint_rust_set)}"
                    )

        te_count = sum(1 for t in obj["types"] if str(t.get("rust", "")).strip() == TELEMETRY_ERROR_RUST)
        if te_count != 1:
            raise RuntimeError("Invariant failed: TelemetryError must exist exactly once")

    def refresh_lists(self):
        self.endpoint_list.delete(0, tk.END)
        for ep in self.config_obj.get("endpoints", []):
            self.endpoint_list.insert(tk.END, _endpoint_row_text(ep))

        self.type_list.delete(0, tk.END)
        for ty in self.config_obj.get("types", []):
            self.type_list.insert(tk.END, _type_row_text(ty))

        self.ty_available_endpoints.delete(0, tk.END)
        for ep in self.config_obj.get("endpoints", []):
            r = ep.get("rust", "")
            if r:
                self.ty_available_endpoints.insert(tk.END, r)

    # ---------------- Endpoints actions ----------------

    def add_endpoint(self):
        self._flush_endpoint_only()
        rust = "NewEndpoint"
        self.config_obj.setdefault("endpoints", []).append(
            {"rust": rust, "name": rust_ident_to_schema_name(rust), "doc": "", "broadcast_mode": "Default"}
        )
        self.refresh_lists()
        self.endpoint_list.selection_clear(0, tk.END)
        self.endpoint_list.selection_set(tk.END)
        self.on_select_endpoint()
        self._mark_changed()
        self._refresh_type_editor_if_selected()

    def delete_endpoint(self):
        self._flush_endpoint_only()
        idx = _selected_index(self.endpoint_list)
        if idx is None:
            return
        ep = self.config_obj["endpoints"][idx]
        if not messagebox.askyesno("Delete endpoint", f"Delete endpoint {ep.get('rust')}?"):
            return

        removed_rust = ep.get("rust", "")
        del self.config_obj["endpoints"][idx]

        for ty in self.config_obj.get("types", []):
            eps = ty.get("endpoints", []) or []
            ty["endpoints"] = [e for e in eps if e != removed_rust]

        self._ensure_telemetry_error_invariants(mark_hash=False)
        self.refresh_lists()
        self._mark_changed()
        self._refresh_type_editor_if_selected()

    def on_select_endpoint(self):
        self._flush_endpoint_only()

        idx = _selected_index(self.endpoint_list)
        if idx is None:
            self._ep_edit_idx = None
            return

        self._ep_edit_idx = idx
        ep = self.config_obj["endpoints"][idx]

        self._suspend_live = True
        try:
            self.ep_rust_var.set(ep.get("rust", ""))
            self.ep_doc_text.delete("1.0", tk.END)
            self.ep_doc_text.insert("1.0", ep.get("doc", "") or "")
            self.ep_doc_text.edit_modified(False)
            self.ep_bm_var.set(ep.get("broadcast_mode", "Default") or "Default")
        finally:
            self._suspend_live = False

    # ---------------- Types actions ----------------

    def add_type(self):
        self._flush_type_only()
        rust = "NewType"
        self.config_obj.setdefault("types", []).append(
            {
                "rust": rust,
                "name": rust_ident_to_schema_name(rust),
                "doc": "",
                "class": "Data",
                "element": {"kind": "Static", "data_type": "Float32", "count": 1},
                "endpoints": [],
            }
        )
        self.refresh_lists()
        self.type_list.selection_clear(0, tk.END)
        self.type_list.selection_set(tk.END)
        self.on_select_type()
        self._mark_changed()

    def delete_type(self):
        self._flush_type_only()
        idx = _selected_index(self.type_list)
        if idx is None:
            return
        ty = self.config_obj["types"][idx]
        if str(ty.get("rust", "")).strip() == TELEMETRY_ERROR_RUST:
            messagebox.showinfo("Protected type", "TelemetryError is required and cannot be removed.")
            return
        if not messagebox.askyesno("Delete type", f"Delete type {ty.get('rust')}?"):
            return
        del self.config_obj["types"][idx]
        self._ensure_telemetry_error_invariants(mark_hash=False)
        self.refresh_lists()
        self._mark_changed()

    def on_select_type(self):
        self._flush_type_only()

        idx = _selected_index(self.type_list)
        if idx is None:
            self._ty_edit_idx = None
            return

        self._ty_edit_idx = idx
        ty = self.config_obj["types"][idx]
        is_te = str(ty.get("rust", "")).strip() == TELEMETRY_ERROR_RUST

        self._suspend_live = True
        try:
            self.ty_rust_var.set(ty.get("rust", ""))
            self.ty_class_var.set(ty.get("class", "Data") or "Data")

            el = ty.get("element", {}) or {}
            kind = el.get("kind", "Static") or "Static"
            dt = el.get("data_type", DATA_TYPE_OPTIONS[0]) or DATA_TYPE_OPTIONS[0]
            self.ty_kind_var.set(kind if kind in ELEMENT_KIND_OPTIONS else "Static")
            self.ty_dtype_var.set(dt if dt in DATA_TYPE_OPTIONS else DATA_TYPE_OPTIONS[0])

            if kind == "Static":
                self.ty_count_var.set(str(el.get("count", 1)))
            else:
                self.ty_count_var.set("")

            self.ty_doc_text.configure(state="normal")
            self.ty_doc_text.delete("1.0", tk.END)
            self.ty_doc_text.insert("1.0", ty.get("doc", "") or "")
            self.ty_doc_text.edit_modified(False)

            self._update_count_visibility()

            selected = ty.get("endpoints", []) or []
            selected_set = set(selected)
            all_eps = [ep.get("rust", "") for ep in self.config_obj.get("endpoints", []) if ep.get("rust", "")]
            available = [e for e in all_eps if e not in selected_set]

            self.ty_available_endpoints.delete(0, tk.END)
            for e in available:
                self.ty_available_endpoints.insert(tk.END, e)

            self.ty_selected_endpoints.delete(0, tk.END)
            for e in selected:
                self.ty_selected_endpoints.insert(tk.END, e)

            self._set_type_editor_locked_for_telemetry_error(is_te)
        finally:
            self._suspend_live = False

    def _refresh_type_editor_if_selected(self):
        if self._ty_edit_idx is None:
            return
        if self._ty_edit_idx < 0 or self._ty_edit_idx >= len(self.config_obj.get("types", [])):
            return
        idx = self._ty_edit_idx
        try:
            self.type_list.selection_clear(0, tk.END)
            self.type_list.selection_set(idx)
        except Exception:
            pass
        self.on_select_type()

    # ---------------- Dual listbox helpers ----------------

    def _type_endpoints_changed(self):
        if self._suspend_live:
            return
        self._schedule_live_type_apply()

    def type_ep_add(self):
        items = _lb_selected(self.ty_available_endpoints)
        _lb_remove_items(self.ty_available_endpoints, items)
        _lb_add_unique(self.ty_selected_endpoints, items)
        self._type_endpoints_changed()

    def type_ep_remove(self):
        items = _lb_selected(self.ty_selected_endpoints)
        _lb_remove_items(self.ty_selected_endpoints, items)
        _lb_add_unique(self.ty_available_endpoints, items)
        self._type_endpoints_changed()

    def type_ep_add_all(self):
        items = _lb_all(self.ty_available_endpoints)
        self.ty_available_endpoints.delete(0, tk.END)
        _lb_add_unique(self.ty_selected_endpoints, items)
        self._type_endpoints_changed()

    def type_ep_remove_all(self):
        items = _lb_all(self.ty_selected_endpoints)
        self.ty_selected_endpoints.delete(0, tk.END)
        _lb_add_unique(self.ty_available_endpoints, items)
        self._type_endpoints_changed()


def main():
    start = Path.cwd()
    crate_root = find_project_root(start)

    config_rs = crate_root / "src" / "config.rs"
    if not config_rs.exists():
        if len(sys.argv) >= 2:
            config_rs = Path(sys.argv[1]).resolve()
            crate_root = find_project_root(config_rs)

    json_path = None
    if config_rs.exists():
        try:
            json_path = find_schema_json_from_config_rs(config_rs, crate_root)
        except Exception as e:
            print(f"warning: {e}", file=sys.stderr)
            json_path = None

    app = TelemetryConfigEditor(crate_root=crate_root, config_rs=config_rs, json_path=json_path)
    app.mainloop()


if __name__ == "__main__":
    main()
