#!/usr/bin/env python3
"""
Telemetry Config Editor (Tkinter)

- Finds telemetry_config.json path by scanning src/config.rs for:
    define_telemetry_schema!(path = "telemetry_config.json")
- Loads JSON if present; otherwise starts with a blank config.
- Lets you edit:
    * endpoints[]: { rust, doc, broadcast_mode }   (schema name auto-generated)
    * types[]: { rust, doc, class, element{kind,data_type}, endpoints[] } (schema name auto-generated)
- For DataTypes, endpoints are managed with a dual-list selector:
    Available <-> Selected

Output JSON schema (matches your proc-macro expectations):
{
  "endpoints": [
    { "rust": "Radio", "name": "RADIO", "doc": "...", "broadcast_mode": "Default" }
  ],
  "types": [
    {
      "rust": "GpsData",
      "name": "GPS_DATA",
      "doc": "...",
      "class": "Data",
      "element": { "kind": "Static", "data_type": "Float32", "count": 3 },
      "endpoints": ["Radio","SdCard"]
    }
  ]
}
"""

from __future__ import annotations

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
    """
    Search config.rs for:
        define_telemetry_schema!(path = "telemetry_config.json");
    Returns crate_root / <path> if found.
    """
    try:
        text = config_rs.read_text(encoding="utf-8")
    except Exception:
        return None

    # allow whitespace/newlines inside invocation
    rx = re.compile(
        r'(?s)define_telemetry_schema!\s*\(\s*[^)]*?\bpath\s*=\s*"([^"]+)"'
    )
    caps = list(rx.finditer(text))
    if not caps:
        return None
    if len(caps) > 1:
        raise RuntimeError(
            f"Multiple define_telemetry_schema!(path=...) found in {config_rs}"
        )
    rel = caps[0].group(1)
    return (crate_root / rel).resolve()


def default_blank_config() -> Dict[str, Any]:
    return {"endpoints": [], "types": []}


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


def ensure_caps_underscore(s: str) -> bool:
    return bool(re.fullmatch(r"[A-Z0-9_]+", s or ""))


def ensure_rust_ident(s: str) -> bool:
    return bool(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", s or ""))


def rust_ident_to_schema_name(rust: str) -> str:
    """
    Convert Rust ident/PascalCase/camelCase/snake_case into SCREAMING_SNAKE_CASE.
    Examples:
      Radio -> RADIO
      SdCard -> SD_CARD
      gpsData -> GPS_DATA
      GPSData -> GPS_DATA
      gps_data -> GPS_DATA
      FooBARBaz -> FOO_BAR_BAZ
    """
    s = (rust or "").strip()
    if not s:
        return ""

    # Replace separators with underscores first
    s = re.sub(r"[\s\-]+", "_", s)

    # If already snake-ish, just scream it
    if "_" in s:
        return re.sub(r"_+", "_", s).strip("_").upper()

    # Insert underscores on transitions:
    # - lower/digit -> upper  (aB, 1A)
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    # - acronym boundary: ABCd -> AB_Cd  (so "GPSData" => GPS_Data before final upper)
    s = re.sub(r"([A-Z])([A-Z][a-z])", r"\1_\2", s)

    s = re.sub(r"_+", "_", s).strip("_")
    return s.upper()


class TelemetryConfigEditor(tk.Tk):
    def __init__(self, crate_root: Path, config_rs: Path, json_path: Optional[Path]):
        super().__init__()
        self.title("Telemetry Config Editor (sedsprintf_rs)")
        self.geometry("1200x760")

        self.crate_root = crate_root
        self.config_rs_path = config_rs
        self.json_path = json_path  # may be None

        self.config_obj: Dict[str, Any] = default_blank_config()
        self.dirty = False

        # UI vars
        self.status_var = tk.StringVar(value="")
        self.json_path_var = tk.StringVar(value=self._json_path_display())

        # Build UI
        self._build_menu()
        self._build_layout()

        # Load if possible
        if self.json_path and self.json_path.exists():
            self.load_from_path(self.json_path)
        else:
            self._set_status(
                "No JSON loaded. Use File → Save As… to create one, or choose an existing file.",
                warn=True,
            )

        self.protocol("WM_DELETE_WINDOW", self.on_close)

        # Start focused/on-top (best-effort)
        self.after(0, self._bring_to_front)  # type: ignore

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
        if self.json_path is None:
            return "(not found)"
        return str(self.json_path)

    # ---------------- UI scaffolding ----------------

    def _build_menu(self):
        menubar = tk.Menu(self)

        filem = tk.Menu(menubar, tearoff=0)
        filem.add_command(label="Open JSON…", command=self.menu_open_json)
        filem.add_command(label="Save", command=self.menu_save)
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

        # Header/status
        hdr = ttk.Frame(top)
        hdr.pack(fill="x")

        ttk.Label(hdr, text="crate root:").pack(side="left")
        ttk.Label(hdr, text=str(self.crate_root), foreground="gray").pack(
            side="left", padx=(6, 14)
        )

        ttk.Label(hdr, text="config.rs:").pack(side="left")
        ttk.Label(hdr, text=str(self.config_rs_path), foreground="gray").pack(
            side="left", padx=(6, 14)
        )

        ttk.Label(hdr, text="json:").pack(side="left")
        ttk.Label(hdr, textvariable=self.json_path_var, foreground="gray").pack(
            side="left", padx=(6, 14)
        )

        # Notebook
        nb = ttk.Notebook(top)
        nb.pack(fill="both", expand=True, pady=(10, 0))
        self.nb = nb

        self.endpoints_tab = ttk.Frame(nb, padding=10)
        self.types_tab = ttk.Frame(nb, padding=10)
        nb.add(self.endpoints_tab, text="Endpoints")
        nb.add(self.types_tab, text="Data Types")

        self._build_endpoints_tab(self.endpoints_tab)
        self._build_types_tab(self.types_tab)

        # Status bar
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
        ttk.Button(btns, text="Delete", command=self.delete_endpoint).pack(
            side="left", padx=6
        )

        self.endpoint_list = tk.Listbox(left, exportselection=False)
        self.endpoint_list.grid(row=1, column=0, sticky="nsew", pady=(8, 0))
        self.endpoint_list.bind("<<ListboxSelect>>", lambda _e: self.on_select_endpoint())

        right = ttk.LabelFrame(parent, text="Edit Endpoint", padding=10)
        right.grid(row=0, column=1, sticky="nsew")
        right.columnconfigure(1, weight=1)

        ttk.Label(right, text="Name (PascalCase):").grid(
            row=0, column=0, sticky="w"
        )
        self.ep_rust_var = tk.StringVar()
        ttk.Entry(right, textvariable=self.ep_rust_var).grid(
            row=0, column=1, sticky="ew", padx=(10, 0)
        )

        # Schema name is auto-generated (no UI field)

        ttk.Label(right, text="Doc (optional):").grid(
            row=1, column=0, sticky="w", pady=(10, 0)
        )
        self.ep_doc_text = tk.Text(right, height=5)
        self.ep_doc_text.grid(row=1, column=1, sticky="ew", padx=(10, 0), pady=(10, 0))

        ttk.Label(right, text="Broadcast mode:").grid(
            row=2, column=0, sticky="w", pady=(10, 0)
        )
        self.ep_bm_var = tk.StringVar(value="Default")
        ttk.Combobox(
            right,
            textvariable=self.ep_bm_var,
            values=BROADCAST_MODE_OPTIONS,
            state="readonly",
        ).grid(row=2, column=1, sticky="w", padx=(10, 0), pady=(10, 0))

        ttk.Button(right, text="Apply Changes", command=self.apply_endpoint_changes).grid(
            row=3, column=0, columnspan=2, sticky="ew", pady=(16, 0)
        )

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
        ttk.Button(btns, text="Delete", command=self.delete_type).pack(
            side="left", padx=6
        )

        self.type_list = tk.Listbox(left, exportselection=False)
        self.type_list.grid(row=1, column=0, sticky="nsew", pady=(8, 0))
        self.type_list.bind("<<ListboxSelect>>", lambda _e: self.on_select_type())

        right = ttk.LabelFrame(parent, text="Edit Data Type", padding=10)
        right.grid(row=0, column=1, sticky="nsew")
        right.columnconfigure(1, weight=1)
        right.rowconfigure(4, weight=1)

        ttk.Label(right, text="Name (PascalCase):").grid(row=0, column=0, sticky="w")
        self.ty_rust_var = tk.StringVar()
        ttk.Entry(right, textvariable=self.ty_rust_var).grid(
            row=0, column=1, sticky="ew", padx=(10, 0)
        )

        # Schema name is auto-generated (no UI field)

        ttk.Label(right, text="Class:").grid(row=1, column=0, sticky="w", pady=(10, 0))
        self.ty_class_var = tk.StringVar(value="Data")
        ttk.Combobox(
            right, textvariable=self.ty_class_var, values=MESSAGE_CLASS_OPTIONS, state="readonly"
        ).grid(row=1, column=1, sticky="w", padx=(10, 0), pady=(10, 0))

        ttk.Label(right, text="Element kind:").grid(row=2, column=0, sticky="w", pady=(10, 0))
        self.ty_kind_var = tk.StringVar(value="Static")
        ttk.Combobox(
            right, textvariable=self.ty_kind_var, values=ELEMENT_KIND_OPTIONS, state="readonly"
        ).grid(row=2, column=1, sticky="w", padx=(10, 0), pady=(10, 0))

        ttk.Label(right, text="Element data type:").grid(
            row=3, column=0, sticky="w", pady=(10, 0)
        )
        self.ty_dtype_var = tk.StringVar(value=DATA_TYPE_OPTIONS[0])
        ttk.Combobox(
            right, textvariable=self.ty_dtype_var, values=DATA_TYPE_OPTIONS, state="readonly"
        ).grid(row=3, column=1, sticky="w", padx=(10, 0), pady=(10, 0))

        # Static count entry (only meaningful for Static)
        ttk.Label(right, text="Static count (for Static only):").grid(
            row=4, column=0, sticky="nw", pady=(10, 0)
        )
        self.ty_count_var = tk.StringVar(value="1")
        ttk.Entry(right, textvariable=self.ty_count_var, width=10).grid(
            row=4, column=1, sticky="nw", padx=(10, 0), pady=(10, 0)
        )

        ttk.Label(right, text="Doc (optional):").grid(
            row=5, column=0, sticky="w", pady=(10, 0)
        )
        self.ty_doc_text = tk.Text(right, height=5)
        self.ty_doc_text.grid(row=5, column=1, sticky="ew", padx=(10, 0), pady=(10, 0))

        # Endpoints dual list selector
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
        ttk.Button(mid_btns, text="← Remove All", command=self.type_ep_remove_all).pack(fill="x", pady=(6, 0))

        self.ty_selected_endpoints = tk.Listbox(epbox, exportselection=False)
        self.ty_selected_endpoints.grid(row=1, column=2, sticky="nsew")

        ttk.Button(right, text="Apply Changes", command=self.apply_type_changes).grid(
            row=7, column=0, columnspan=2, sticky="ew", pady=(16, 0)
        )

    # ---------------- Menu actions ----------------

    def menu_open_json(self):
        p = filedialog.askopenfilename(
            title="Open telemetry_config.json",
            initialdir=str(self.crate_root),
            filetypes=[("JSON files", "*.json"), ("All files", "*.*")],
        )
        if not p:
            return
        path = Path(p).resolve()
        self.load_from_path(path)

    def menu_save(self):
        if self.json_path is None:
            self.menu_save_as()
            return
        self.save_to_path(self.json_path)

    def menu_save_as(self):
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
        found = find_schema_json_from_config_rs(self.config_rs_path, self.crate_root)
        if found is None:
            self._set_status(
                f"Could not find define_telemetry_schema!(path=...) in {self.config_rs_path}",
                warn=True,
            )
            return
        self.json_path = found
        self.json_path_var.set(str(found))
        if found.exists():
            self.load_from_path(found)
        else:
            self._set_status(f"Discovered JSON path: {found} (does not exist yet)", warn=True)

    # ---------------- Load / save ----------------

    def _normalize_schema_names(self) -> bool:
        """
        Ensure every endpoint/type has .name that matches the auto-generated value from .rust.
        Returns True if any changes were made.
        """
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
        obj = safe_read_json(path)
        if obj is None:
            self.config_obj = default_blank_config()
            self._set_status(f"File does not exist: {path}. Starting with blank config.", warn=True)
            self.json_path = path
            self.json_path_var.set(str(path))
            self.dirty = False
            self.refresh_lists()
            return

        self.config_obj = obj
        self.json_path = path
        self.json_path_var.set(str(path))

        # Auto-normalize schema names now that they’re generated.
        normalized = self._normalize_schema_names()
        self.dirty = bool(normalized)

        if normalized:
            self._set_status(f"Loaded {path} (normalized schema names from rust; please Save)", warn=True)
        else:
            self._set_status(f"Loaded {path}")

        self.refresh_lists()

    def save_to_path(self, path: Path):
        # Always normalize before save, so file stays consistent
        self._normalize_schema_names()

        try:
            self.validate_current_config()
        except Exception as e:
            messagebox.showerror("Validation error", str(e))
            return

        safe_write_json(path, self.config_obj)
        self.dirty = False
        self._set_status(f"Saved {path}")

    # ---------------- Helpers ----------------

    def _set_status(self, msg: str, warn: bool = False):
        self.status_var.set(msg)

    def set_dirty(self):
        self.dirty = True
        if self.json_path:
            self._set_status(f"Modified (not saved) — {self.json_path}", warn=True)
        else:
            self._set_status("Modified (not saved) — no JSON path yet", warn=True)

    def on_close(self):
        if self.dirty:
            if not messagebox.askyesno(
                    "Unsaved changes", "You have unsaved changes. Quit anyway?"
            ):
                return
        self.destroy()

    def validate_current_config(self):
        obj = self.config_obj
        if "endpoints" not in obj or "types" not in obj:
            raise RuntimeError("JSON must contain top-level keys: endpoints, types")

        # endpoints
        for i, ep in enumerate(obj["endpoints"]):
            rust = str(ep.get("rust", "")).strip()
            name = str(ep.get("name", "")).strip()

            if not ensure_rust_ident(rust):
                raise RuntimeError(f"endpoints[{i}].rust must be Rust ident/PascalCase, got {rust!r}")

            expected = rust_ident_to_schema_name(rust)
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

        # types
        for i, ty in enumerate(obj["types"]):
            rust = str(ty.get("rust", "")).strip()
            name = str(ty.get("name", "")).strip()

            if not ensure_rust_ident(rust):
                raise RuntimeError(f"types[{i}].rust must be Rust ident/PascalCase, got {rust!r}")

            expected = rust_ident_to_schema_name(rust)
            if name != expected:
                raise RuntimeError(
                    f"types[{i}].name must match generated name from rust ({expected!r}), got {name!r}"
                )

            cls = ty.get("class", "")
            if cls not in MESSAGE_CLASS_OPTIONS:
                raise RuntimeError(f"types[{i}].class must be one of {MESSAGE_CLASS_OPTIONS}, got {cls!r}")

            el = ty.get("element", {})
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
                try:
                    cnti = int(cnt)
                except Exception:
                    raise RuntimeError(f"types[{i}].element.count must be int, got {cnt!r}")
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

    def refresh_lists(self):
        # Endpoints list
        self.endpoint_list.delete(0, tk.END)
        for ep in self.config_obj.get("endpoints", []):
            self.endpoint_list.insert(tk.END, f"{ep.get('rust', '')}  [{ep.get('name', '')}]")

        # Types list
        self.type_list.delete(0, tk.END)
        for ty in self.config_obj.get("types", []):
            self.type_list.insert(tk.END, f"{ty.get('rust', '')}  [{ty.get('name', '')}]")

        # Rebuild "Available endpoints" list (selected list is per-type)
        self.ty_available_endpoints.delete(0, tk.END)
        for ep in self.config_obj.get("endpoints", []):
            r = ep.get("rust", "")
            if r:
                self.ty_available_endpoints.insert(tk.END, r)

    def _selected_index(self, lb: tk.Listbox) -> Optional[int]:
        sel = lb.curselection()
        if not sel:
            return None
        return int(sel[0])

    # ---------------- Endpoints actions ----------------

    def add_endpoint(self):
        rust = "NewEndpoint"
        self.config_obj.setdefault("endpoints", []).append(
            {
                "rust": rust,
                "name": rust_ident_to_schema_name(rust),
                "doc": "",
                "broadcast_mode": "Default",
            }
        )
        self.set_dirty()
        self.refresh_lists()
        self.endpoint_list.selection_clear(0, tk.END)
        self.endpoint_list.selection_set(tk.END)
        self.on_select_endpoint()

    def delete_endpoint(self):
        idx = self._selected_index(self.endpoint_list)
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

        self.set_dirty()
        self.refresh_lists()
        self._refresh_type_editor_if_selected()

    def on_select_endpoint(self):
        idx = self._selected_index(self.endpoint_list)
        if idx is None:
            return
        ep = self.config_obj["endpoints"][idx]
        self.ep_rust_var.set(ep.get("rust", ""))
        self.ep_doc_text.delete("1.0", tk.END)
        self.ep_doc_text.insert("1.0", ep.get("doc", "") or "")
        self.ep_bm_var.set(ep.get("broadcast_mode", "Default") or "Default")

    def apply_endpoint_changes(self):
        idx = self._selected_index(self.endpoint_list)
        if idx is None:
            return
        ep = self.config_obj["endpoints"][idx]

        new_rust = self.ep_rust_var.get().strip()
        new_doc = self.ep_doc_text.get("1.0", tk.END).strip()
        new_bm = self.ep_bm_var.get().strip() or "Default"

        if not ensure_rust_ident(new_rust):
            messagebox.showerror(
                "Invalid rust name",
                "Endpoint rust name must be a valid Rust identifier (PascalCase recommended).",
            )
            return
        if new_bm not in BROADCAST_MODE_OPTIONS:
            messagebox.showerror("Invalid broadcast mode", f"Must be one of {BROADCAST_MODE_OPTIONS}")
            return

        old_rust = ep.get("rust", "")
        ep["rust"] = new_rust
        ep["name"] = rust_ident_to_schema_name(new_rust)
        ep["doc"] = new_doc
        ep["broadcast_mode"] = new_bm

        # If rust name changed, update any types referencing it
        if old_rust != new_rust:
            for ty in self.config_obj.get("types", []):
                eps = ty.get("endpoints", []) or []
                ty["endpoints"] = [new_rust if e == old_rust else e for e in eps]

        self.set_dirty()
        self.refresh_lists()
        self._refresh_type_editor_if_selected()

    # ---------------- Types actions ----------------

    def add_type(self):
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
        self.set_dirty()
        self.refresh_lists()
        self.type_list.selection_clear(0, tk.END)
        self.type_list.selection_set(tk.END)
        self.on_select_type()

    def delete_type(self):
        idx = self._selected_index(self.type_list)
        if idx is None:
            return
        ty = self.config_obj["types"][idx]
        if not messagebox.askyesno("Delete type", f"Delete type {ty.get('rust')}?"):
            return
        del self.config_obj["types"][idx]
        self.set_dirty()
        self.refresh_lists()

    def on_select_type(self):
        idx = self._selected_index(self.type_list)
        if idx is None:
            return
        ty = self.config_obj["types"][idx]

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

        self.ty_doc_text.delete("1.0", tk.END)
        self.ty_doc_text.insert("1.0", ty.get("doc", "") or "")

        selected = ty.get("endpoints", []) or []
        selected_set = set(selected)

        # rebuild available + selected
        all_eps = [
            ep.get("rust", "")
            for ep in self.config_obj.get("endpoints", [])
            if ep.get("rust", "")
        ]
        available = [e for e in all_eps if e not in selected_set]

        self.ty_available_endpoints.delete(0, tk.END)
        for e in available:
            self.ty_available_endpoints.insert(tk.END, e)

        self.ty_selected_endpoints.delete(0, tk.END)
        for e in selected:
            self.ty_selected_endpoints.insert(tk.END, e)

    def apply_type_changes(self):
        idx = self._selected_index(self.type_list)
        if idx is None:
            return
        ty = self.config_obj["types"][idx]

        new_rust = self.ty_rust_var.get().strip()
        new_class = self.ty_class_var.get().strip() or "Data"
        new_kind = self.ty_kind_var.get().strip() or "Static"
        new_dtype = self.ty_dtype_var.get().strip() or DATA_TYPE_OPTIONS[0]
        new_doc = self.ty_doc_text.get("1.0", tk.END).strip()

        if not ensure_rust_ident(new_rust):
            messagebox.showerror(
                "Invalid rust name",
                "Type rust name must be a valid Rust identifier (PascalCase recommended).",
            )
            return
        if new_class not in MESSAGE_CLASS_OPTIONS:
            messagebox.showerror("Invalid class", f"Must be one of {MESSAGE_CLASS_OPTIONS}")
            return
        if new_kind not in ELEMENT_KIND_OPTIONS:
            messagebox.showerror("Invalid kind", f"Must be one of {ELEMENT_KIND_OPTIONS}")
            return
        if new_dtype not in DATA_TYPE_OPTIONS:
            messagebox.showerror("Invalid element data type", "Pick a value from the dropdown.")
            return

        element: Dict[str, Any] = {"kind": new_kind, "data_type": new_dtype}
        if new_kind == "Static":
            cnt_s = self.ty_count_var.get().strip()
            if cnt_s == "":
                messagebox.showerror("Missing count", "Static types require a count.")
                return
            try:
                cnt = int(cnt_s)
            except Exception:
                messagebox.showerror("Invalid count", "Count must be an integer.")
                return
            if cnt < 0:
                messagebox.showerror("Invalid count", "Count must be >= 0.")
                return
            element["count"] = cnt

        sel = self._lb_all(self.ty_selected_endpoints)
        if not sel:
            if not messagebox.askyesno("No endpoints", "This type has no endpoints selected. Save anyway?"):
                return

        ty["rust"] = new_rust
        ty["name"] = rust_ident_to_schema_name(new_rust)
        ty["doc"] = new_doc
        ty["class"] = new_class
        ty["element"] = element
        ty["endpoints"] = sel

        self.set_dirty()
        self.refresh_lists()
        self.type_list.selection_clear(0, tk.END)
        self.type_list.selection_set(idx)
        self.on_select_type()

    def _refresh_type_editor_if_selected(self):
        if self._selected_index(self.type_list) is not None:
            self.on_select_type()

    # ---------------- Dual listbox helpers ----------------

    def _lb_all(self, lb: tk.Listbox) -> List[str]:
        return [lb.get(i) for i in range(lb.size())]

    def _lb_selected(self, lb: tk.Listbox) -> List[str]:
        return [lb.get(i) for i in lb.curselection()]

    def _lb_remove_items(self, lb: tk.Listbox, items: List[str]) -> None:
        if not items:
            return
        s = set(items)
        values = [v for v in self._lb_all(lb) if v not in s]
        lb.delete(0, tk.END)
        for v in values:
            lb.insert(tk.END, v)

    def _lb_add_unique(self, lb: tk.Listbox, items: List[str]) -> None:
        if not items:
            return
        existing = set(self._lb_all(lb))
        for v in items:
            if v not in existing:
                lb.insert(tk.END, v)
                existing.add(v)

    def type_ep_add(self):
        items = self._lb_selected(self.ty_available_endpoints)
        self._lb_remove_items(self.ty_available_endpoints, items)
        self._lb_add_unique(self.ty_selected_endpoints, items)

    def type_ep_remove(self):
        items = self._lb_selected(self.ty_selected_endpoints)
        self._lb_remove_items(self.ty_selected_endpoints, items)
        self._lb_add_unique(self.ty_available_endpoints, items)

    def type_ep_add_all(self):
        items = self._lb_all(self.ty_available_endpoints)
        self.ty_available_endpoints.delete(0, tk.END)
        self._lb_add_unique(self.ty_selected_endpoints, items)

    def type_ep_remove_all(self):
        items = self._lb_all(self.ty_selected_endpoints)
        self.ty_selected_endpoints.delete(0, tk.END)
        self._lb_add_unique(self.ty_available_endpoints, items)


def main():
    start = Path.cwd()
    crate_root = find_project_root(start)

    config_rs = crate_root / "src" / "config.rs"
    if not config_rs.exists():
        if len(sys.argv) >= 2:
            config_rs = Path(sys.argv[1]).resolve()
            crate_root = find_project_root(config_rs)
        else:
            pass

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
