"""
TesorerIA — backend (FastAPI). Corre igual en local (uvicorn) y en Modal.

Rieles de pago sobre Stellar:
  1. Soroban TreasuryGuard  -> proveedores nacionales en MXNe, con políticas on-chain.
  2. Path Payment (DEX)      -> proveedores internacionales: se paga MXNe y llega USDC exacto.
  3. Pago en lote (batch)    -> nómina: N pagos en 1 transacción atómica.

Además: SEP-1 (stellar.toml), SEP-2 (federation / alias legibles), web call de Retell.
"""
from __future__ import annotations

import json
import os
import pathlib
import re
import secrets as pysecrets
import threading
import time
import unicodedata
import uuid
from datetime import datetime, timezone

import requests
from fastapi import FastAPI, Request
from starlette.concurrency import run_in_threadpool
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse, JSONResponse, PlainTextResponse
from stellar_sdk import Asset, Keypair, Network, Server, TransactionBuilder, scval
from stellar_sdk.contract import ContractClient

HERE = pathlib.Path(__file__).parent
WEB = pathlib.Path(os.environ.get("WEB_DIR", HERE / "web"))
PUB = json.loads(pathlib.Path(os.environ.get("PUBLIC_STATE", HERE / "state" / "public_state.json")).read_text("utf-8"))

PASS = Network.TESTNET_NETWORK_PASSPHRASE
HORIZON = Server(PUB["horizon"])
RPC = PUB["soroban_rpc"]
GUARD_ID = PUB["contracts"]["treasury_guard"]
MXNe = Asset("MXNe", PUB["assets"]["MXNe"]["issuer"])
USDC = Asset("USDC", PUB["assets"]["USDC"]["issuer"])
UNIT = 10_000_000
EXPLORER = "https://stellar.expert/explorer/testnet"
POLICY = PUB["policy"]
USD_PER_XLM = 0.27  # referencia para mostrar costo de red en USD


def _secret(name: str) -> str:
    v = os.environ.get(f"{name.upper()}_SECRET")
    if v:
        return v
    f = HERE / "state" / "wallets.secret.json"
    return json.loads(f.read_text())[name]


KP = {n: Keypair.from_secret(_secret(n)) for n in ("owner", "agent", "treasury")}
LOCKS = {n: threading.Lock() for n in KP}
SUPPLIERS = {s["key"]: s for s in PUB["suppliers"]}
EMPLOYEES = PUB["employees"]


# ---------------------------------------------------------------- store
class Store:
    """modal.Dict en la nube, archivo JSON en local."""

    def __init__(self):
        self.d = None
        if os.environ.get("MODAL_ENVIRONMENT") or os.environ.get("USE_MODAL_DICT"):
            import modal

            self.d = modal.Dict.from_name("tesoreria-store", create_if_missing=True)
        self.path = HERE / "state" / "store.local.json"
        self.lock = threading.Lock()

    def get(self, k, default=None):
        if self.d is not None:
            return self.d.get(k, default)
        data = json.loads(self.path.read_text("utf-8")) if self.path.exists() else {}
        return data.get(k, default)

    def put(self, k, v):
        if self.d is not None:
            self.d[k] = v
            return
        with self.lock:
            data = json.loads(self.path.read_text("utf-8")) if self.path.exists() else {}
            data[k] = v
            self.path.write_text(json.dumps(data, ensure_ascii=False, indent=1), "utf-8")


store = Store()
_mutex = threading.Lock()


def now_iso():
    return datetime.now(timezone.utc).isoformat()


def default_invoices():
    return {
        # folio fiscal nuevo en cada reinicio: el contrato recuerda para siempre los folios ya pagados
        i["id"]: {**i, "uuid": str(uuid.uuid4()).upper(), "status": "pendiente", "hash": None, "paid_at": None, "rail": None,
                  "detail": None, "created": ""}
        for i in PUB["invoices"]
    }


def invoices() -> dict:
    inv = store.get("invoices")
    if not inv:
        inv = default_invoices()
        store.put("invoices", inv)
    return inv


def save_invoice(inv_id, **changes):
    with _mutex:
        inv = invoices()
        inv[inv_id].update(changes)
        store.put("invoices", inv)
        return inv[inv_id]


def log(kind: str, title: str, **extra):
    with _mutex:
        act = store.get("activity", []) or []
        act.insert(0, {"ts": now_iso(), "kind": kind, "title": title, **extra})
        store.put("activity", act[:60])
    _cache["t"] = 0  # que el siguiente refresco lea saldos nuevos
    if extra.get("hash") and not extra.get("fee"):
        threading.Thread(target=_backfill_fee, args=(extra["hash"],), daemon=True).start()


# ---------------------------------------------------------------- helpers
def norm(s: str) -> str:
    s = unicodedata.normalize("NFD", (s or "").lower())
    return "".join(c for c in s if unicodedata.category(c) != "Mn").strip()


def find_supplier(q: str):
    q = norm(q)
    if not q:
        return None
    for s in SUPPLIERS.values():
        if q == s["key"] or q in norm(s["name"]) or q == norm(s["alias"]):
            return s
    words = [w for w in re.split(r"\W+", q) if len(w) > 3]
    for s in SUPPLIERS.values():
        if any(w in norm(s["name"]) for w in words):
            return s
    return None


def find_invoice(factura: str | None, proveedor: str | None):
    inv = invoices()
    if factura:
        f = factura.upper().replace(" ", "")
        if norm(factura) in ("ultima", "la ultima", "nueva", "la nueva", "la que subi", "cfdi"):
            subidas = sorted((v for v in inv.values() if v.get("cfdi")), key=lambda v: v.get("created", ""), reverse=True)
            if subidas:
                return subidas[0]
        digits = re.sub(r"\D", "", f)
        for k, v in inv.items():
            # al menos 3 dígitos para no confundir "F-1" con otra factura
            if k == f or (len(digits) >= 3 and k.endswith(digits)) or (len(f) >= 6 and v.get("uuid", "").startswith(f)):
                return v
    if proveedor:
        s = find_supplier(proveedor)
        if s:
            pend = [v for v in inv.values() if v["supplier"] == s["key"] and v["status"] != "pagada"]
            if pend:
                return pend[0]
            any_ = [v for v in inv.values() if v["supplier"] == s["key"]]
            if any_:
                return any_[0]
    return None


def invoice_to_pay(factura, proveedor):
    """Para mover dinero no se adivina (find_invoice es solo para consultar): una factura sin ambigüedad o ninguna."""
    todas = list(invoices().values())
    s = find_supplier(str(proveedor or ""))
    suyas = [v for v in todas if v["supplier"] == s["key"]] if s else todas
    dicho = str(factura or "").strip()
    if not dicho:  # solo dijo el proveedor: vale si tiene una sola factura
        hits = suyas if s else []
    elif norm(dicho) in ("ultima", "la ultima", "nueva", "la nueva", "la que subi", "cfdi"):
        hits = sorted((v for v in suyas if v.get("cfdi")), key=lambda v: v.get("created", ""), reverse=True)[:1]
    else:
        clave = lambda t: re.sub(r"[\W_]", "", t.upper())
        serie = lambda t: "".join(re.findall(r"[A-Z]+", t.upper()))
        numero = lambda t: re.sub(r"\D", "", t).lstrip("0")
        f, n = clave(dicho), numero(dicho)
        hits = [v for v in suyas if clave(v["id"]) == f]
        if not hits:
            # número completo (nunca una terminación) y, si dijo una serie que existe, de esa serie
            series = set(re.findall(r"[A-Z]+", dicho.upper())) & {serie(v["id"]) for v in todas}
            hits = [v for v in suyas if (n and numero(v["id"]) == n and (not series or serie(v["id"]) in series))
                    or (len(f) >= 8 and clave(v.get("uuid", "")).startswith(f))]
    return (hits[0] if len(hits) == 1 else None), s


def invoice_list(s: dict) -> str:
    return ", ".join(v["id"] + (" (ya pagada)" if v["status"] == "pagada" else "")
                     for v in invoices().values() if v["supplier"] == s["key"])


def money(n, cur="MXNe"):
    return f"{n:,.2f} {cur}"


def guard(fn: str, params=None, signer="agent", read=False):
    kp = KP[signer]
    c = ContractClient(GUARD_ID, RPC, PASS)
    if read:
        at = c.invoke(fn, params or [], source=kp.public_key, signer=kp, parse_result_xdr_fn=scval.to_native, base_fee=10_000)
        return at.result(), None
    # El número de secuencia se toma al construir la tx: construir y enviar deben ir dentro del mismo candado
    with LOCKS[signer]:
        at = c.invoke(fn, params or [], source=kp.public_key, signer=kp, parse_result_xdr_fn=scval.to_native, base_fee=10_000)
        result = at.sign_and_submit()
    tx_hash = None
    try:
        tx_hash = at.built_transaction.hash_hex()
    except Exception:
        pass
    return result, tx_hash


CONTRACT_ERRORS = {
    1: ("NotWhitelisted", "el proveedor no está en la lista blanca autorizada por el director"),
    2: ("ExceedsTxLimit", "el monto rebasa el tope por transacción de ese proveedor"),
    3: ("ExceedsDailyLimit", "se rebasaría el límite de gasto diario de la empresa"),
    4: ("InvalidAmount", "el monto no es válido"),
    5: ("NotFound", "el pago no existe"),
    6: ("NotPending", "el pago ya no está pendiente de aprobación"),
    7: ("InsufficientFunds", "la bóveda no tiene fondos suficientes"),
    8: ("DuplicateInvoice", "esa factura ya se pagó o ya está en proceso; el contrato no permite pagarla dos veces"),
    9: ("Expired", "la solicitud de aprobación caducó"),
    10: ("VaultPaused", "la bóveda se encuentra congelada por protocolo de emergencia antifraude"),
    11: ("EmptyBatch", "el lote de dispersión no contiene partidas válidas"),
    12: ("ReservedFundsLocked", "esos fondos están comprometidos para cubrir pagos pendientes de firma"),
    13: ("TimelockActive", "el plazo programado o ventana de espera aún sigue activo en el ledger"),
    14: ("InvalidBasisPoints", "el porcentaje en puntos base está fuera del rango permitido por la política fiscal"),
    15: ("InstallmentCompleted", "el plan de parcialidades ya fue liquidado en su totalidad o se encuentra inactivo"),
    16: ("CreditExceedsInvoice", "la nota de crédito de egreso no puede exceder ni igualar el total retenido"),
}


def contract_error(e: Exception):
    m = re.search(r"Error\(Contract, #(\d+)\)", str(e))
    if m:
        code = int(m.group(1))
        return code, *CONTRACT_ERRORS.get(code, ("Unknown", "error del contrato"))
    return None, "Error", str(e)[:200]


def tx_fee(tx_hash: str | None, tries: int = 1):
    """Comisión real cobrada por la red. Por defecto un solo intento para no frenar la respuesta de voz;
    si Horizon aún no la indexa, log() la completa en segundo plano."""
    if not tx_hash:
        return None
    for i in range(tries):
        try:
            r = requests.get(f"{PUB['horizon']}/transactions/{tx_hash}", timeout=4)
            if r.ok:
                stroops = int(r.json()["fee_charged"])
                return {"xlm": stroops / UNIT, "usd": stroops / UNIT * USD_PER_XLM, "ledger": r.json()["ledger"]}
        except Exception:
            pass
        if i < tries - 1:
            time.sleep(1.5)
    return None


def _backfill_fee(tx_hash: str):
    fee = tx_fee(tx_hash, tries=6)
    if not fee:
        return
    with _mutex:
        act = store.get("activity", []) or []
        for a in act:
            if a.get("hash") == tx_hash and not a.get("fee"):
                a["fee"] = fee
        store.put("activity", act)


# ---------------------------------------------------------------- lecturas
_cache = {"t": 0, "v": None}


_refreshing = threading.Event()


def balances():
    """Devuelve el último saldo al instante y lo refresca en segundo plano (el dashboard consulta cada 2 s)."""
    if _cache["v"] is not None:
        if time.time() - _cache["t"] >= 3 and not _refreshing.is_set():
            _refreshing.set()

            def bg():
                try:
                    _read_balances()
                finally:
                    _refreshing.clear()

            threading.Thread(target=bg, daemon=True).start()
        return _cache["v"]
    return _read_balances()


def _read_balances():
    out = {"vault_mxnb": 0, "treasury_mxnb": 0, "treasury_usdc": 0, "spent_today": 0}
    try:
        acct = requests.get(f"{PUB['horizon']}/accounts/{KP['treasury'].public_key}", timeout=6).json()
        for b in acct.get("balances", []):
            if b.get("asset_code") == "MXNe":
                out["treasury_mxnb"] = float(b["balance"])
            if b.get("asset_code") == "USDC":
                out["treasury_usdc"] = float(b["balance"])
    except Exception:
        pass
    try:
        out["vault_mxnb"] = guard("balance", read=True)[0] / UNIT
        spent = guard("spent_today", read=True)[0] / UNIT
        out["spent_today"] = spent
    except Exception as e:
        out["error"] = str(e)[:120]
    _cache.update(t=time.time(), v=out)
    return out




def quote_usdc(amount_usdc: float):
    paths = HORIZON.strict_receive_paths(source=[MXNe], destination_asset=USDC, destination_amount=f"{amount_usdc:.7f}").call()
    recs = paths["_embedded"]["records"]
    if not recs:
        return None
    best = min(recs, key=lambda r: float(r["source_amount"]))
    src = float(best["source_amount"])
    return {"mxnb": src, "rate": src / amount_usdc, "path": [p.get("asset_code", "XLM") for p in best["path"]]}


# ---------------------------------------------------------------- seguridad
TOOLS_KEY = os.environ.get("TOOLS_KEY", "")
DASH_KEY = os.environ.get("DASH_KEY", "")
OWNER_PHONE = os.environ.get("OWNER_PHONE", "")


def _key_of(request: Request, name: str) -> str:
    return request.headers.get(f"x-{name}-key") or request.query_params.get("k") or ""


def is_dash(request: Request) -> bool:
    return bool(DASH_KEY) and pysecrets.compare_digest(_key_of(request, "dash"), DASH_KEY)


def tool_allowed(request: Request) -> bool:
    k = _key_of(request, "tesa")
    return is_dash(request) or (bool(TOOLS_KEY) and pysecrets.compare_digest(k, TOOLS_KEY))


def caller_role(body: dict, request: Request) -> str:
    """Quién da la orden. El rol lo decide el servidor, nunca el modelo de lenguaje."""
    if "call" not in body:  # botón del dashboard (ya autenticado con la llave del tablero)
        return "director" if is_dash(request) else "invitado"
    call = body.get("call") or {}
    dyn = call.get("retell_llm_dynamic_variables") or {}
    if call.get("call_type") == "web_call":
        return "director" if dyn.get("rol") == "director" else "invitado"
    frm = re.sub(r"\D", "", call.get("from_number") or "")
    to = re.sub(r"\D", "", call.get("to_number") or "")
    owner = re.sub(r"\D", "", OWNER_PHONE)
    # entrante desde el celular del director, o saliente que Tesa le hizo al director
    if owner and (frm.endswith(owner[-10:]) or (call.get("direction") == "outbound" and to.endswith(owner[-10:]))):
        return "director"
    return "invitado"


def safe_tool(fn):
    """Ninguna herramienta de voz debe tronar: si algo falla, Tesa dice que no se movió dinero."""
    async def wrapper(request: Request):
        if not tool_allowed(request):
            return JSONResponse({"ok": False, "mensaje": "No autorizado."}, 401)
        try:
            body = await request.json()
        except Exception:
            body = {}
        if not isinstance(body, dict):
            body = {}
        args = body.get("args", body) or {}
        role = caller_role(body, request)
        try:
            return await run_in_threadpool(fn, args, role)
        except Exception as e:  # noqa: BLE001
            log("error", "Falla técnica en un pago", reason=str(e)[:160])
            return {"ok": False, "mensaje": "No pude completar la operación por una falla técnica. No se movió dinero. Puedes intentarlo de nuevo."}

    wrapper.__name__ = fn.__name__  # sin functools.wraps: FastAPI debe ver la firma (request), no (a, role)
    return wrapper


def guest_request(supplier: dict | None, amount, currency: str, detail: str):
    """Un invitado (no es el director) pidió mover dinero a un destino autorizado: se registra, no se ejecuta."""
    log("guest", f"Invitado pidió {detail}", amount=amount, currency=currency, supplier=supplier["key"] if supplier else None,
        reason="solo el director puede ordenar pagos; quedó como solicitud")
    return {"ok": False, "solicitud_registrada": True,
            "mensaje": ("Como no eres el director, no puedo ejecutar pagos. Registré tu solicitud y el director la verá "
                        "en su tablero. Y aunque lo intentes, el contrato solo paga a proveedores autorizados.")}


# ---------------------------------------------------------------- rieles de pago
def _invoice_key(inv: dict | None) -> str:
    if inv and inv.get("uuid"):
        return inv["uuid"]
    return f"SIN-FACTURA-{int(time.time() * 1000)}"


def pay_domestic(supplier: dict, amount: float, concept: str, inv: dict | None, address: str | None = None, label: str | None = None):
    t0 = time.time()
    dest = address or supplier["address"]
    params = [scval.to_address(dest), scval.to_int128(int(round(amount * UNIT))), scval.to_string(_invoice_key(inv))]
    try:
        res, h = guard("request_payment", params)
    except Exception as e:
        code, name, why = contract_error(e)
        if code is None:
            raise
        who = label or supplier["name"]
        log("blocked", f"Soroban bloqueó un pago a {who}", amount=amount, currency="MXNe", reason=why, code=name,
            supplier=supplier["key"], invoice=inv["id"] if inv else None)
        if inv and code != 8:
            save_invoice(inv["id"], status="bloqueada", detail=why)
        return {"ok": False, "bloqueado": True, "motivo": why, "codigo": name,
                "mensaje": f"El contrato inteligente rechazó el pago: {why}. No se movió ningún peso."}

    secs = round(time.time() - t0, 1)
    status = int(res["status"]) if isinstance(res, dict) else 1
    pid = int(res["id"]) if isinstance(res, dict) else None
    base = {"rail": "soroban", "hash": h, "contract_payment_id": pid, "seconds": secs}

    if status == 0:  # Pending: espera la firma del director
        token = pysecrets.token_urlsafe(8)
        approvals = store.get("approvals", {}) or {}
        approvals[str(pid)] = {"id": str(pid), "type": "payment", "token": token, "invoice": inv["id"] if inv else None,
                               "supplier": supplier["key"], "amount": amount, "concept": concept, "created": now_iso(),
                               "status": "pendiente"}
        store.put("approvals", approvals)
        if inv:
            save_invoice(inv["id"], status="en_aprobacion", rail="soroban", hash=h, contract_payment_id=pid)
        log("pending", f"Pago a {supplier['name']} retenido para aprobación", amount=amount, currency="MXNe", hash=h,
            payment_id=pid, supplier=supplier["key"])
        notify_owner(str(pid), token, f"autoriza el pago de ${amount:,.0f} MXNe a {supplier['name']}")
        return {"ok": True, "requiere_aprobacion": True, **base,
                "mensaje": (f"El pago de {money(amount)} a {supplier['name']} supera el umbral de "
                            f"{money(POLICY['approval_threshold'])}, así que el contrato lo dejó retenido. "
                            "Ya le envié la solicitud de autorización al director a su celular.")}

    if inv:
        save_invoice(inv["id"], status="pagada", paid_at=now_iso(), **base)
    log("paid", f"Pagado a {supplier['name']}", amount=amount, currency="MXNe", hash=h, rail="soroban", seconds=secs,
        supplier=supplier["key"], invoice=inv["id"] if inv else None)
    return {"ok": True, **base, "explorer": f"{EXPLORER}/tx/{h}",
            "mensaje": f"Listo. Pagué {money(amount)} a {supplier['name']} a través del contrato inteligente. Se liquidó en {secs} segundos."}


def pay_international(supplier: dict, amount_usdc: float, concept: str, inv: dict | None, memo: str | None = None):
    t0 = time.time()
    q = quote_usdc(amount_usdc)
    if not q:
        return {"ok": False, "mensaje": "No encontré liquidez en el DEX para esa conversión. No se movió dinero."}
    send_max = f"{q['mxnb'] * 1.005:.7f}"  # 0.5% de tolerancia
    tx_memo = (str(memo or "").strip() or (inv["id"] if inv else "TesorerIA"))[:28]
    with LOCKS["treasury"]:
        acct = HORIZON.load_account(KP["treasury"].public_key)
        tx = (TransactionBuilder(acct, PASS, base_fee=1000)
              .append_path_payment_strict_receive_op(supplier["address"], MXNe, send_max, USDC, f"{amount_usdc:.7f}", [])
              .add_text_memo(tx_memo).set_timeout(60).build())
        tx.sign(KP["treasury"])
        r = HORIZON.submit_transaction(tx)
    h = r["hash"]
    secs = round(time.time() - t0, 1)
    base = {"rail": "path_payment", "hash": h, "seconds": secs, "fx": q, "memo": tx_memo}
    if inv:
        save_invoice(inv["id"], status="pagada", paid_at=now_iso(), **base)
    log("paid", f"Pagado a {supplier['name']} ({supplier['country']})", amount=amount_usdc, currency="USDC", hash=h,
        rail="path_payment", seconds=secs, fx=q, memo=tx_memo, supplier=supplier["key"], invoice=inv["id"] if inv else None)
    return {"ok": True, **base, "explorer": f"{EXPLORER}/tx/{h}",
            "mensaje": (f"Listo. {supplier['name']} recibió exactamente {money(amount_usdc, 'USDC')}. De nuestra tesorería salieron "
                        f"{money(q['mxnb'])} a un tipo de cambio de {q['rate']:.2f} pesos por dólar, convertidos en el DEX de Stellar "
                        f"en una sola operación atómica. Tardó {secs} segundos.")}


def notify_owner(pid, token, text):
    base = os.environ.get("PUBLIC_BASE_URL", "").rstrip("/")
    link = f"{base}/aprobar?id={pid}&t={token}"
    sid, tok, frm = (os.environ.get(k) for k in ("TWILIO_ACCOUNT_SID", "TWILIO_AUTH_TOKEN", "TWILIO_PHONE_NUMBER"))
    if not (sid and tok and frm and OWNER_PHONE and base):
        return
    try:
        r = requests.post(f"https://api.twilio.com/2010-04-01/Accounts/{sid}/Messages.json", auth=(sid, tok),
                          data={"From": frm, "To": OWNER_PHONE, "Body": f"TesorerIA: {text}. {link}"}, timeout=8)
        if not r.ok:
            log("notice", "No se pudo enviar el SMS al director; usa el QR del tablero", reason=r.text[:120])
    except Exception:
        log("notice", "No se pudo enviar el SMS al director; usa el QR del tablero")


# ---------------------------------------------------------------- CFDI (factura electrónica del SAT)
NS = {"cfdi": "http://www.sat.gob.mx/cfd/4", "tfd": "http://www.sat.gob.mx/TimbreFiscalDigital"}
LISTA_69B = set(PUB.get("lista_69b", []))


def parse_cfdi(xml_text: str) -> dict:
    import xml.etree.ElementTree as ET

    root = ET.fromstring(xml_text.encode("utf-8"))
    if not root.tag.endswith("Comprobante"):
        raise ValueError("el archivo no es un CFDI (no tiene cfdi:Comprobante)")
    em = root.find("cfdi:Emisor", NS)
    rc = root.find("cfdi:Receptor", NS)
    tfd = root.find(".//tfd:TimbreFiscalDigital", NS)
    con = root.find(".//cfdi:Concepto", NS)
    if em is None or rc is None or tfd is None:
        raise ValueError("al CFDI le falta emisor, receptor o timbre fiscal")
    return {
        "uuid": tfd.get("UUID", "").upper(), "fecha": root.get("Fecha"), "serie": root.get("Serie", ""),
        "folio": root.get("Folio", ""), "total": float(root.get("Total", "0")), "moneda": root.get("Moneda", "MXN"),
        "emisor_rfc": em.get("Rfc", "").upper(), "emisor_nombre": em.get("Nombre", ""),
        "receptor_rfc": rc.get("Rfc", "").upper(), "concepto": con.get("Descripcion", "") if con is not None else "",
    }


def validate_cfdi(c: dict):
    """Semáforo antes de pagar. La consulta de estatus al SAT y la lista 69-B son simuladas en el demo."""
    checks = []
    generic = c["emisor_rfc"] == "XEXX010101000"
    sup = find_supplier(c["emisor_nombre"]) if generic else next((s for s in SUPPLIERS.values() if s.get("rfc") == c["emisor_rfc"]), None)
    checks.append({"ok": c["receptor_rfc"] == PUB["company"]["rfc"], "label": "A nombre de Grupo Industrial Tlahuica",
                   "fail": f"la factura está a nombre de otro RFC ({c['receptor_rfc']})"})
    checks.append({"ok": c["emisor_rfc"] not in LISTA_69B, "label": "Emisor fuera de la lista 69-B del SAT",
                   "fail": "el RFC emisor aparece en la lista 69-B del SAT (empresa que factura operaciones simuladas)"})
    checks.append({"ok": sup is not None and sup.get("whitelisted", False), "label": f"Proveedor autorizado: {sup['name'] if sup else '—'}",
                   "fail": f"{c['emisor_nombre'] or c['emisor_rfc']} no es un proveedor autorizado"})
    dup = next((v for v in invoices().values() if v.get("uuid") == c["uuid"]), None)
    checks.append({"ok": dup is None, "label": "Folio fiscal (UUID) nuevo",
                   "fail": f"este folio fiscal ya está registrado como {dup['id']}" if dup else ""})
    checks.append({"ok": True, "label": "Estatus SAT: vigente (simulado)", "fail": ""})
    return checks, sup


# ---------------------------------------------------------------- eventos de la cadena (Soroban RPC)
_chain = {"t": 0, "v": []}


def chain_events():
    if time.time() - _chain["t"] < 4:
        return _chain["v"]
    try:
        from stellar_sdk import xdr as sxdr

        latest = requests.post(RPC, json={"jsonrpc": "2.0", "id": 1, "method": "getLatestLedger"}, timeout=6).json()["result"]["sequence"]
        body = {"jsonrpc": "2.0", "id": 2, "method": "getEvents",
                "params": {"startLedger": max(1, latest - 8_000), "filters": [{"type": "contract", "contractIds": [GUARD_ID]}],
                           "pagination": {"limit": 200}}}
        res = requests.post(RPC, json=body, timeout=8).json().get("result", {})
        out = []
        for ev in res.get("events", []):
            try:
                topics = [scval.to_native(sxdr.SCVal.from_xdr(t)) for t in ev.get("topic", [])]
                kind = topics[0] if topics else "?"
                out.append({"type": kind if isinstance(kind, str) else str(kind), "ledger": ev.get("ledger"),
                            "tx": ev.get("txHash"), "at": ev.get("ledgerClosedAt")})
            except Exception:
                continue
        _chain.update(t=time.time(), v=out[-100:])
    except Exception:
        _chain["t"] = time.time()
    return _chain["v"]


# ---------------------------------------------------------------- app
api = FastAPI(title="TesorerIA")
api.add_middleware(CORSMiddleware, allow_origins=["*"], allow_methods=["*"], allow_headers=["*"])


async def args_of(request: Request) -> dict:
    try:
        body = await request.json()
    except Exception:
        body = {}
    return body.get("args", body) if isinstance(body, dict) else {}


# las páginas siempre frescas: así nadie ve una versión vieja guardada en el navegador
NO_CACHE = {"Cache-Control": "no-cache, no-store, must-revalidate"}


@api.get("/")
def home():
    return FileResponse(WEB / "index.html", headers=NO_CACHE)


@api.get("/aprobar")
def approve_page():
    return FileResponse(WEB / "aprobar.html", headers=NO_CACHE)


@api.get("/reto")
def reto_page():
    return FileResponse(WEB / "reto.html", headers=NO_CACHE)


@api.get("/static/{name}")
def static(name: str):
    p = (WEB / name).resolve()
    if WEB.resolve() not in p.parents or not p.exists():
        return JSONResponse({"error": "not found"}, 404)
    return FileResponse(p)


@api.get("/health")
def health():
    return {"ok": True, "contract": GUARD_ID}


# ---------- SEP-1 / SEP-2
@api.get("/.well-known/stellar.toml")
def stellar_toml(request: Request):
    base = str(request.base_url).rstrip("/").replace("http://", "https://")
    accts = ", ".join(f'"{a}"' for a in (KP["treasury"].public_key, KP["agent"].public_key))
    toml = f'''VERSION = "2.0.0"
NETWORK_PASSPHRASE = "{PASS}"
FEDERATION_SERVER = "{base}/federation"
ACCOUNTS = [{accts}]

[DOCUMENTATION]
ORG_NAME = "TesorerIA (demo: {PUB['company']['name']})"
ORG_DESCRIPTION = "Tesorería por voz para PyMEs sobre Stellar + Soroban"

[[CURRENCIES]]
code = "MXNe"
issuer = "{MXNe.issuer}"
display_decimals = 2
name = "Peso mexicano digital (activo de prueba en testnet)"
is_asset_anchored = true
anchor_asset_type = "fiat"
anchor_asset = "MXN"

[[CURRENCIES]]
code = "USDC"
issuer = "{USDC.issuer}"
display_decimals = 2
name = "USDC de prueba (emitido por TesorerIA en testnet; no es el USDC de Circle)"

[[CURRENCIES]]
contract = "{PUB['contracts']['mxnb_sac']}"
code = "MXNe"
name = "MXNe Stellar Asset Contract"
'''
    return PlainTextResponse(toml, headers={"Access-Control-Allow-Origin": "*"})


@api.get("/federation")
def federation(q: str = "", type: str = "name"):
    if type == "name":
        s = next((s for s in SUPPLIERS.values() if s["alias"] == q.lower() and s["whitelisted"]), None)
        if s:
            return {"stellar_address": s["alias"], "account_id": s["address"], "memo_type": "text", "memo": "TesorerIA"}
    elif type == "id":
        s = next((s for s in SUPPLIERS.values() if s["address"] == q), None)
        if s:
            return {"stellar_address": s["alias"], "account_id": s["address"]}
    return JSONResponse({"detail": "not found"}, 404)


# ---------- Estado para el dashboard (sin llave: solo lectura y sin ligas de aprobación)
@api.get("/api/state")
def state(request: Request):
    inv = invoices()
    approvals = [a for a in (store.get("approvals", {}) or {}).values() if a["status"] == "pendiente"]
    dash = is_dash(request)
    if not dash:
        approvals = [{**a, "token": None} for a in approvals]
    act = store.get("activity", []) or []
    return {
        "company": PUB["company"],
        "network": {"name": "Stellar Testnet", "contract": GUARD_ID, "sac": PUB["contracts"]["mxnb_sac"], "explorer": EXPLORER},
        "accounts": {k: v.public_key for k, v in KP.items()},
        "policy": POLICY,
        "balances": balances(),
        "invoices": [{**v, "supplier_info": SUPPLIERS[v["supplier"]]} for v in inv.values() if v["supplier"] in SUPPLIERS],
        "suppliers": list(SUPPLIERS.values()),
        "employees": EMPLOYEES,
        "payroll": store.get("payroll", {"status": "pendiente"}),
        "approvals": approvals,
        "activity": act,
        "challenge": {"blocked": sum(1 for a in act if a.get("kind") in ("blocked", "guest", "fraud"))},
        "call": store.get("call", {}),
        "dash": dash,
    }


@api.get("/api/conciliacion.csv")
def conciliacion(request: Request):
    """Reporte para el contador: cada factura y la nómina con su folio fiscal y su comprobante en Stellar."""
    if not is_dash(request):
        return JSONResponse({"detail": "solo el tablero del director"}, 403)
    import csv, io
    buf = io.StringIO()
    w = csv.writer(buf)
    w.writerow(["Fecha de pago (UTC)", "Factura", "Folio fiscal (UUID)", "Proveedor", "RFC", "Concepto", "Monto", "Moneda",
                "Riel en Stellar", "Estado", "Firma del director", "Segundos", "Hash de la transacción", "Comprobante"])
    for v in invoices().values():
        s = SUPPLIERS.get(v["supplier"], {})
        h = v.get("hash") or ""
        w.writerow([v.get("paid_at") or "", v["id"], v.get("uuid") or "", s.get("name", v["supplier"]), s.get("rfc", ""),
                    v.get("concept", ""), v["amount"], v["currency"], v.get("rail") or "", v.get("status", ""),
                    "Sí" if v.get("approved") else "", v.get("seconds") or "", h, f"{EXPLORER}/tx/{h}" if h else ""])
    pr = store.get("payroll", {}) or {}
    h = pr.get("hash") or ""
    w.writerow([pr.get("paid_at") or "", "NÓMINA", "", f"{len(EMPLOYEES)} empleados", "", "Nómina quincenal",
                sum(e["salary"] for e in EMPLOYEES), "MXNe", "batch" if h else "", pr.get("status", "pendiente"), "",
                pr.get("seconds") or "", h, f"{EXPLORER}/tx/{h}" if h else ""])
    name = f"conciliacion_{PUB['company'].get('rfc', 'empresa')}_{datetime.now(timezone.utc):%Y%m%d}.csv"
    return PlainTextResponse("﻿" + buf.getvalue(), media_type="text/csv; charset=utf-8",
                             headers={"Content-Disposition": f'attachment; filename="{name}"'})


@api.get("/api/chain")
def chain():
    ev = chain_events()
    return {"contract": GUARD_ID, "events": ev, "tx": sorted({e["tx"] for e in ev if e.get("tx")})}


# ---------- Herramientas del agente de voz (Retell custom functions)
def _nomina_pendiente():
    pr = store.get("payroll", {}) or {}
    return 0 if pr.get("status") == "pagada" else sum(e["salary"] for e in EMPLOYEES)


@api.post("/tools/resumen_tesoreria")
@safe_tool
def t_resumen(a, role):
    b = balances()
    pend = [v for v in invoices().values() if v["status"] in ("pendiente", "bloqueada")]
    mx = sum(v["amount"] for v in pend if v["currency"] == "MXNe")
    us = sum(v["amount"] for v in pend if v["currency"] == "USDC")
    nomina = _nomina_pendiente()
    return {
        "rol": role, "boveda_mxne": round(b["vault_mxnb"], 2), "tesoreria_usdc": round(b["treasury_usdc"], 2),
        "gastado_hoy": round(b["spent_today"], 2), "disponible_hoy": round(POLICY["daily_limit"] - b["spent_today"], 2),
        "facturas_pendientes": len(pend),
        "mensaje": (f"La bóveda tiene {money(b['vault_mxnb'])} y la tesorería {money(b['treasury_usdc'], 'USDC')}. "
                    f"Hoy se han pagado {money(b['spent_today'])} de un límite diario de {money(POLICY['daily_limit'])}. "
                    f"Hay {len(pend)} facturas por pagar: {money(mx)} nacionales y {money(us, 'USDC')} internacionales"
                    + (f", más la nómina de {money(nomina)}." if nomina else ".")),
    }


@api.post("/tools/me_alcanza")
@safe_tool
def t_alcanza(a, role):
    """Pronóstico simple: ¿alcanza lo que hay para todo lo que vence?"""
    b = balances()
    inv = [v for v in invoices().values() if v["status"] in ("pendiente", "en_aprobacion")]
    nomina = _nomina_pendiente()
    mx = sum(v["amount"] for v in inv if v["currency"] == "MXNe")
    usd = sum(v["amount"] for v in inv if v["currency"] == "USDC")
    q = quote_usdc(usd) if usd else None
    usd_mx = q["mxnb"] if q else 0
    disponible = b["vault_mxnb"] + b["treasury_mxnb"]
    necesita = mx + nomina + usd_mx
    sobra = disponible - necesita
    hoy = POLICY["daily_limit"] - b["spent_today"]
    txt = (f"Sí alcanza: necesitas {money(necesita)} para facturas, nómina y dólares, y tienes {money(disponible)}; te sobran {money(sobra)}."
           if sobra >= 0 else f"No alcanza: te faltan {money(-sobra)}.")
    if mx > hoy:
        txt += f" Ojo: hoy el contrato solo permite {money(hoy)} más en pagos nacionales; el resto sale mañana o con tu firma."
    return {"ok": True, "necesita": round(necesita, 2), "disponible": round(disponible, 2), "sobra": round(sobra, 2), "mensaje": txt}


@api.post("/tools/listar_facturas")
@safe_tool
def t_listar(a, role):
    out = []
    for v in invoices().values():
        s = SUPPLIERS[v["supplier"]]
        out.append({"factura": v["id"], "proveedor": s["name"], "pais": s["country"], "monto": v["amount"], "moneda": v["currency"],
                    "concepto": v["concept"], "estado": v["status"], "cfdi_validado": bool(v.get("cfdi"))})
    return {"facturas": out}


def _pay_invoice(inv: dict, role: str, memo: str | None = None):
    s = SUPPLIERS[inv["supplier"]]
    if role != "director":
        return guest_request(s, inv["amount"], inv["currency"], f"pagar la factura {inv['id']} de {s['name']}")
    if inv["status"] == "en_aprobacion":
        return {"ok": False, "mensaje": f"La factura {inv['id']} ya está esperando la autorización del director."}
    if inv["status"] == "procesando":
        return {"ok": False, "mensaje": f"La factura {inv['id']} se está pagando en este momento; ya aparece en pantalla."}
    if inv.get("cfdi_blocked"):
        return {"ok": False, "bloqueado": True, "mensaje": f"No la pago: {inv.get('detail')}. No se movió dinero."}
    if inv["status"] == "pagada":
        if inv["currency"] == "MXNe":
            # se lo pedimos al contrato para que él mismo lo rechace en la cadena
            r = pay_domestic(s, inv["amount"], inv["concept"], inv)
            if r.get("codigo") == "DuplicateInvoice":
                r["mensaje"] = f"La factura {inv['id']} ya estaba pagada y el contrato rechazó pagarla otra vez. No se movió dinero."
            return r
        return {"ok": False, "mensaje": f"La factura {inv['id']} ya estaba pagada.", "hash": inv.get("hash")}
    with _mutex:
        cur = invoices()
        if cur[inv["id"]]["status"] not in ("pendiente", "bloqueada"):
            return {"ok": False, "mensaje": f"La factura {inv['id']} ya está en proceso."}
        cur[inv["id"]]["status"] = "procesando"
        store.put("invoices", cur)
    try:
        if inv["currency"] == "USDC":
            return pay_international(s, inv["amount"], inv["concept"], inv, memo=memo)
        return pay_domestic(s, inv["amount"], inv["concept"], inv)
    finally:
        if invoices()[inv["id"]]["status"] == "procesando":
            save_invoice(inv["id"], status="pendiente")


@api.post("/tools/pagar_factura")
@safe_tool
def t_pagar_factura(a, role):
    inv, s = invoice_to_pay(a.get("factura"), a.get("proveedor"))
    if not inv:
        suyas = [v for v in invoices().values() if s and v["supplier"] == s["key"]]
        if len(suyas) == 1:  # con una sola no se le pide el número: se le propone esa
            v = suyas[0]
            pregunta = (f"La única factura de {s['name']} es la {v['id']} por {money(v['amount'], v['currency'])}"
                        f"{', ya pagada' if v['status'] == 'pagada' else ''}. ¿Te refieres a esa?")
        elif suyas:
            pregunta = f"De {s['name']} tengo: {invoice_list(s)}. ¿Cuál pago?"
        else:
            pregunta = "¿De qué proveedor es, o qué número de factura pago?"
        return {"ok": False, "requiere_factura": True, "mensaje": f"No encontré esa factura, así que no pagué nada. {pregunta}"}
    memo = str(a.get("memo") or "").strip() or None
    return _pay_invoice(inv, role, memo=memo)


def _cuenta_nueva(a, role):
    """Alguien pide pagar a una cuenta que no está autorizada (cambio de cuenta, o 'págame a mí')."""
    prov = find_supplier(a.get("proveedor", "") or "")
    cr = next((c for c in PUB.get("change_requests", []) if prov and c["supplier"] == prov["key"]), None)
    try:
        monto = float(str(a.get("monto") or 0).replace(",", "")) or None
    except ValueError:
        monto = None
    if cr:
        inv = next((v for v in invoices().values() if v["supplier"] == prov["key"] and v["status"] != "pagada"), None)
        amount = monto or (inv["amount"] if inv else 10_000)
        # el intento de abajo va sin folio y solo sirve para que el contrato lo rechace (#1);
        # si el director ya autorizó esa cuenta, el intento sí pagaría (aun pedido por un invitado), así que no se hace
        nueva = guard("get_supplier", [scval.to_address(cr["new_address"])], read=True)[0]
        if nueva and nueva.get("active"):
            if role != "director":
                return guest_request(prov, amount, "MXNe", f"pagar a la cuenta nueva de {prov['name']}")
            return {"ok": False, "requiere_factura": True,
                    "mensaje": (f"La cuenta nueva de {prov['name']} ya la autorizaste, pero no hago pagos sin factura. "
                                "No se movió dinero.")}
        pay_domestic(prov, amount, "Pago a cuenta nueva", None, address=cr["new_address"], label=f"{prov['name']} (cuenta nueva)")
        if role != "director":
            # un invitado (reto del jurado) ve el bloqueo del contrato, pero no crea alertas ni toca el QR del director
            return {"ok": False, "bloqueado": True,
                    "mensaje": ("No lo hice. El contrato bloqueó el pago porque esa cuenta no está autorizada, y además no eres el director. "
                                "No se movió ni un peso.")}
        with _mutex:
            approvals = store.get("approvals", {}) or {}
            prev = approvals.get(cr["id"])
            fresh = not (prev and prev.get("status") == "pendiente")
            if fresh:  # si ya hay una alerta abierta, se conserva su QR
                approvals[cr["id"]] = {"id": cr["id"], "type": "supplier_change", "token": pysecrets.token_urlsafe(8), "supplier": prov["key"],
                                       "amount": amount, "concept": f"Cambio de cuenta solicitado desde {cr['from_email']}",
                                       "new_address": cr["new_address"], "from_email": cr["from_email"], "official_email": cr["official_email"],
                                       "account_age_days": cr["account_age_days"], "created": now_iso(), "status": "pendiente"}
                store.put("approvals", approvals)
            token = approvals[cr["id"]]["token"]
        if fresh:
            log("fraud", f"Posible fraude: cambio de cuenta de {prov['name']}", amount=amount, currency="MXNe", supplier=prov["key"],
                reason=f"cuenta nueva no autorizada; el correo vino de {cr['from_email']}, no de {cr['official_email']}")
            notify_owner(cr["id"], token, f"alerta de posible fraude: {prov['name']} 'cambió' su cuenta")
        return {"ok": False, "bloqueado": True, "posible_fraude": True,
                "mensaje": ("No lo hice. El contrato bloqueó el pago porque esa cuenta nueva no está autorizada. Además, la solicitud llegó "
                            f"de un Gmail y no del correo oficial de {prov['name']}, y la cuenta se creó hace {cr['account_age_days']} días. "
                            "Así empieza el fraude más común contra empresas. "
                            + ("Ya te dejé la alerta en pantalla para que la revises." if fresh else "La alerta sigue abierta en tu pantalla."))}
    # destino desconocido: le pedimos al contrato y él lo rechaza en la cadena
    who = a.get("beneficiario") or a.get("proveedor") or "una cuenta desconocida"
    r = pay_domestic(SUPPLIERS["logistica"], monto or 50_000, "Intento a cuenta desconocida", None,
                     address=Keypair.random().public_key, label=str(who))
    if r.get("bloqueado"):
        r["mensaje"] = (f"No puedo. Le pedí al contrato pagarle a {who} y lo rechazó: esa cuenta no está en la lista blanca del director. "
                        "No se movió ni un peso. Ni yo misma puedo saltarme esa regla.")
    return r


@api.post("/tools/pagar_a_cuenta_nueva")
@safe_tool
def t_cuenta_nueva(a, role):
    return _cuenta_nueva(a, role)


_recent_direct: dict = {}


@api.post("/tools/pagar_proveedor")
@safe_tool
def t_pagar_proveedor(a, role):
    """Pago sin factura, solo para proveedores sin facturas registradas; si no está en el directorio, el contrato lo rechaza."""
    s = find_supplier(a.get("proveedor", ""))
    try:
        monto = float(str(a.get("monto", "0")).replace(",", ""))
    except ValueError:
        monto = 0
    if not s or not s["whitelisted"]:
        return _cuenta_nueva({"beneficiario": a.get("proveedor"), "monto": monto}, role)
    if role != "director":
        return guest_request(s, monto, s["currency"], f"pagarle {money(monto, s['currency'])} a {s['name']}")
    # un pago directo no lleva folio fiscal y el #8 no lo frenaría: si el proveedor tiene facturas, se paga por número
    lista = invoice_list(s)
    if lista:
        return {"ok": False, "requiere_factura": True,
                "mensaje": (f"A {s['name']} le pago por número de factura, para que el contrato revise que no se pague dos veces. "
                            f"Tiene registrada: {lista}. ¿Qué número de factura pago?")}
    if monto <= 0:
        return {"ok": False, "mensaje": f"¿Cuánto le pago a {s['name']}? Dime el monto."}
    concepto = a.get("concepto") or "Pago directo"
    k = (s["key"], round(monto, 2))
    with _mutex:  # si Tesa reintenta el mismo pago (por ejemplo tras un timeout), no se paga dos veces
        if time.time() - _recent_direct.get(k, 0) < 180:
            return {"ok": False, "duplicado": True,
                    "mensaje": f"Ese mismo pago a {s['name']} ya lo hice hace un momento, así que no lo repito. El comprobante está en pantalla."}
        _recent_direct[k] = time.time()
    try:
        r = pay_international(s, monto, concepto, None) if s["currency"] == "USDC" else pay_domestic(s, monto, concepto, None)
    except Exception:
        _recent_direct.pop(k, None)
        raise
    if not r.get("ok"):
        _recent_direct.pop(k, None)
    return r


@api.post("/tools/cotizar_internacional")
@safe_tool
def t_cotizar(a, role):
    inv = find_invoice(a.get("factura"), a.get("proveedor"))
    amount = float(a.get("monto_usd") or (inv["amount"] if inv and inv["currency"] == "USDC" else 0) or 0)
    if amount <= 0:
        return {"ok": False, "mensaje": "¿De cuántos dólares sería el pago?"}
    q = quote_usdc(amount)
    if not q:
        return {"ok": False, "mensaje": "No hay liquidez disponible ahorita."}
    bank_extra = amount * 0.03 * q["rate"] + 50 * q["rate"]  # referencia típica: ~50 USD de comisión + 2–5 % de spread
    return {"ok": True, **q, "referencia_banco_mxn": round(bank_extra),
            "mensaje": (f"Para que lleguen {money(amount, 'USDC')} necesitamos {money(q['mxnb'])}, a {q['rate']:.2f} pesos por dólar, "
                        f"y llega en segundos. Como referencia, un banco típico cobra de 40 a 60 dólares por la transferencia más un spread "
                        f"de 2 a 5 por ciento, unos {bank_extra:,.0f} pesos extra, y tarda de 2 a 5 días.")}


@api.post("/tools/pagar_nomina")
@safe_tool
def t_nomina(a, role):
    total = sum(e["salary"] for e in EMPLOYEES)
    if role != "director":
        return guest_request(None, total, "MXNe", "dispersar la nómina")
    with _mutex:
        pr = store.get("payroll", {}) or {}
        if pr.get("status") in ("pagada", "procesando"):
            return {"ok": False, "mensaje": "La nómina de esta quincena ya fue dispersada.", "hash": pr.get("hash")}
        store.put("payroll", {"status": "procesando"})
    t0 = time.time()
    tx_memo = (str(a.get("memo") or "").strip() or "Nomina quincena 22")[:28]
    try:
        with LOCKS["treasury"]:
            acct = HORIZON.load_account(KP["treasury"].public_key)
            tb = TransactionBuilder(acct, PASS, base_fee=1000).add_text_memo(tx_memo).set_timeout(60)
            for e in EMPLOYEES:
                tb.append_payment_op(e["address"], MXNe, f"{e['salary']:.2f}")
            tx = tb.build()
            tx.sign(KP["treasury"])
            r = HORIZON.submit_transaction(tx)
    except Exception:
        store.put("payroll", {"status": "pendiente"})
        raise
    secs = round(time.time() - t0, 1)
    store.put("payroll", {"status": "pagada", "hash": r["hash"], "total": total, "seconds": secs, "memo": tx_memo, "paid_at": now_iso()})
    log("payroll", f"Nómina dispersada a {len(EMPLOYEES)} empleados", amount=total, currency="MXNe", hash=r["hash"], rail="batch", seconds=secs, memo=tx_memo)
    return {"ok": True, "hash": r["hash"], "seconds": secs, "memo": tx_memo, "explorer": f"{EXPLORER}/tx/{r['hash']}",
            "mensaje": f"Nómina dispersada: {len(EMPLOYEES)} empleados, {money(total)} en una sola transacción atómica. Tardó {secs} segundos."}


@api.post("/tools/estado_pago")
@safe_tool
def t_estado(a, role):
    inv = find_invoice(a.get("factura"), a.get("proveedor"))
    if not inv:
        return {"ok": False, "mensaje": "No encontré ese pago."}
    s = SUPPLIERS[inv["supplier"]]
    txt = {"pagada": "ya está pagada y confirmada en el ledger de Stellar", "pendiente": "sigue pendiente de pago",
           "en_aprobacion": "está esperando la autorización del director", "bloqueada": "fue bloqueada",
           "rechazada": "fue rechazada por el director", "procesando": "se está pagando en este momento"}.get(inv["status"], inv["status"])
    return {"ok": True, "estado": inv["status"], "hash": inv.get("hash"),
            "mensaje": f"La factura {inv['id']} de {s['name']} por {money(inv['amount'], inv['currency'])} {txt}."}


# ---------- Subir CFDI desde el tablero
@api.post("/api/cfdi")
async def upload_cfdi(request: Request):
    if not is_dash(request):
        return JSONResponse({"ok": False, "error": "Solo el tablero del director puede subir facturas"}, 401)
    raw = (await request.body()).decode("utf-8", errors="replace")
    try:
        c = parse_cfdi(raw)
    except Exception as e:
        return JSONResponse({"ok": False, "error": f"No pude leer el CFDI: {str(e)[:120]}"}, 400)
    checks, sup = validate_cfdi(c)
    failed = [ch for ch in checks if not ch["ok"]]
    cur = "USDC" if c["moneda"] == "USD" else "MXNe"
    if any("UUID" in ch["label"] for ch in failed):
        log("blocked", "Factura duplicada rechazada al subirla", reason=next(ch["fail"] for ch in failed if "UUID" in ch["label"]),
            amount=c["total"], currency=cur)
        return {"ok": False, "checks": checks, "mensaje": failed[0]["fail"]}
    inv_id = f"{c['serie'] or 'CFDI'}-{c['folio'] or c['uuid'][:6]}"
    key = sup["key"] if sup else "fantasma"
    with _mutex:
        inv = invoices()
        inv[inv_id] = {"id": inv_id, "supplier": key, "amount": c["total"], "currency": cur,
                       "concept": c["concepto"] or "Factura CFDI", "uuid": c["uuid"], "status": "bloqueada" if failed else "pendiente",
                       "hash": None, "paid_at": None, "rail": None, "detail": "; ".join(ch["fail"] for ch in failed) or None,
                       "cfdi": {"checks": checks, **c}, "cfdi_blocked": bool(failed), "created": now_iso()}
        store.put("invoices", inv)
    if failed:
        log("blocked", f"CFDI de {c['emisor_nombre']} bloqueado antes de pagar", amount=c["total"], currency=cur,
            reason=failed[0]["fail"], supplier=key)
    else:
        log("cfdi", f"CFDI de {sup['name']} validado y listo para pagar", amount=c["total"], currency=cur, supplier=key)
    return {"ok": not failed, "id": inv_id, "checks": checks,
            "mensaje": f"Factura {inv_id} {'bloqueada: ' + failed[0]['fail'] if failed else 'validada y lista para pagar'}"}


# ---------- Aprobación del dueño (segundo factor)
def _approval(pid: str, token: str):
    ap = (store.get("approvals", {}) or {}).get(str(pid))
    if not ap or not ap.get("token") or not pysecrets.compare_digest(ap["token"], token or ""):
        return None
    return ap


@api.get("/api/approval/{pid}")
def approval_info(pid: str, t: str = ""):
    ap = _approval(pid, t)
    if not ap:
        return JSONResponse({"error": "Solicitud inválida o expirada"}, 404)
    return {**ap, "token": None, "supplier_info": SUPPLIERS[ap["supplier"]], "policy": POLICY}


@api.post("/api/approve")
async def approve(request: Request):
    a = await args_of(request)
    pid, t, decision = str(a.get("id")), a.get("t", ""), a.get("decision", "approve")
    ap = _approval(pid, t)
    if not ap or ap["status"] != "pendiente":
        return JSONResponse({"ok": False, "error": "Solicitud inválida o ya resuelta"}, 400)
    return await run_in_threadpool(_decide, ap, pid, decision)


def _decide(ap, pid, decision):
    s = SUPPLIERS[ap["supplier"]]
    approvals = store.get("approvals", {}) or {}
    t0 = time.time()
    if ap.get("type") == "supplier_change":
        if decision != "approve":
            approvals[pid]["status"] = "rechazada"
            store.put("approvals", approvals)
            log("fraud_stopped", f"Fraude evitado: la cuenta nueva de {s['name']} queda bloqueada", supplier=s["key"])
            return {"ok": True, "hash": None, "seconds": 0}
        try:  # el director confirmó por teléfono que el cambio es real: la cuenta nueva entra a la lista blanca
            _, h = guard("set_supplier", [scval.to_address(ap["new_address"]), scval.to_string(s["name"] + " (nueva)"),
                                          scval.to_int128(int(s["max_per_tx"] * UNIT))], signer="owner")
        except Exception as e:
            return JSONResponse({"ok": False, "error": contract_error(e)[2]}, 400)
        approvals[pid]["status"] = "aprobada"
        store.put("approvals", approvals)
        log("approved", f"Director autorizó la cuenta nueva de {s['name']}", hash=h, supplier=s["key"])
        return {"ok": True, "hash": h, "seconds": round(time.time() - t0, 1), "explorer": f"{EXPLORER}/tx/{h}"}

    fn = "approve" if decision == "approve" else "reject"
    try:
        _, h = guard(fn, [scval.to_uint64(int(pid))], signer="owner")
    except Exception as e:
        return JSONResponse({"ok": False, "error": contract_error(e)[2]}, 400)
    secs = round(time.time() - t0, 1)
    approvals[pid]["status"] = "aprobada" if fn == "approve" else "rechazada"
    store.put("approvals", approvals)
    if fn == "approve":
        if ap.get("invoice"):
            save_invoice(ap["invoice"], status="pagada", paid_at=now_iso(), hash=h, rail="soroban", seconds=secs, approved=True)
        log("approved", f"Director aprobó y se pagó a {s['name']}", amount=ap["amount"], currency="MXNe", hash=h, rail="soroban",
            seconds=secs, supplier=s["key"])
    else:
        if ap.get("invoice"):
            save_invoice(ap["invoice"], status="rechazada", hash=h)
        log("rejected", f"Director rechazó el pago a {s['name']}", amount=ap["amount"], currency="MXNe", hash=h, supplier=s["key"])
    _cache["t"] = 0
    return {"ok": True, "hash": h, "seconds": secs, "explorer": f"{EXPLORER}/tx/{h}"}


# ---------- Llamadas (web y teléfono)
def _retell(path: str, body: dict):
    key = os.environ.get("RETELL_API_KEY")
    return requests.post(f"https://api.retellai.com{path}", headers={"Authorization": f"Bearer {key}"}, json=body, timeout=15)


_guest_calls: list = []


@api.post("/api/web-call")
def web_call(request: Request):
    """Con llave del tablero = el director. Sin llave = invitado (reto para los jueces), con tope de llamadas."""
    rol = "director" if is_dash(request) else "invitado"
    if rol == "invitado":
        now = time.time()
        _guest_calls[:] = [t for t in _guest_calls if now - t < 300]
        if len(_guest_calls) >= 8:
            return JSONResponse({"error": "Hay demasiadas llamadas del reto al mismo tiempo; intenta en un minuto."}, 429)
        _guest_calls.append(now)
    r = _retell("/v3/create-web-call", {"agent_id": os.environ.get("RETELL_AGENT_ID"), "retell_llm_dynamic_variables": {"rol": rol}})
    if not r.ok:
        return JSONResponse({"error": r.text[:200]}, 502)
    j = r.json()
    return {"access_token": j["access_token"], "call_id": j.get("call_id"), "transport": j.get("transport"),
            "ice_servers": j.get("ice_servers") or [], "rol": rol}


@api.post("/api/call-director")
def call_director(request: Request):
    """Tesa le marca al director para avisarle algo urgente."""
    if not is_dash(request):
        return JSONResponse({"ok": False}, 401)
    aps = [a for a in (store.get("approvals", {}) or {}).values() if a["status"] == "pendiente"]
    pend = [v for v in invoices().values() if v["status"] == "pendiente"]
    if aps:
        motivo = f"tienes {aps[0]['concept'].lower()} de {SUPPLIERS[aps[0]['supplier']]['name']} esperando tu revisión"
    elif pend:
        s = SUPPLIERS[pend[0]["supplier"]]
        motivo = f"vence hoy la factura {pend[0]['id']} de {s['name']} por {pend[0]['amount']:,.0f} {pend[0]['currency']}"
    else:
        motivo = "todo está al corriente y quería darte el resumen del día"
    saludo = f"Hola director, habla Tesa. Te llamo porque {motivo}. ¿Quieres que lo atienda?"
    body = {"from_number": os.environ.get("TWILIO_PHONE_NUMBER"), "to_number": OWNER_PHONE,
            "override_agent_id": os.environ.get("RETELL_AGENT_ID"),
            "retell_llm_dynamic_variables": {"rol": "director", "saludo": saludo}}
    r = _retell("/v2/create-phone-call", body)
    if not r.ok and "eprecat" in r.text:
        r = _retell("/v3/create-phone-call", body)
    if not r.ok:
        return JSONResponse({"ok": False, "error": r.text[:240]}, 502)
    log("notice", "Tesa está llamando al director", reason=motivo)
    return {"ok": True, "call_id": r.json().get("call_id")}


@api.post("/api/retell-webhook")
async def retell_webhook(request: Request):
    """Estado de llamadas telefónicas para mostrar 'En llamada' en el dashboard."""
    body = await request.json()
    ev, call = body.get("event"), body.get("call", {}) or {}
    if ev in ("call_started", "call_ended", "call_analyzed"):
        store.put("call", {"status": "live" if ev == "call_started" else "ended", "call_id": call.get("call_id"),
                           "type": call.get("call_type"), "ts": now_iso()})
    return {"ok": True}


# ---------- Ensayo: dejar el demo como nuevo
@api.post("/api/demo/reset")
def reset(request: Request):
    if not is_dash(request):
        return JSONResponse({"ok": False}, 403)
    guard("reset_day", [], signer="owner")  # el gasto del día vuelve a 0; el límite de 150k NO cambia
    for k, v in (("invoices", default_invoices()), ("activity", []), ("approvals", {}), ("payroll", {"status": "pendiente"}), ("call", {})):
        store.put(k, v)
    _cache["t"] = 0
    return {"ok": True}


@api.get("/api/contract-capabilities")
def contract_capabilities():
    """Resumen auditable de las capacidades activas del contrato Soroban TreasuryGuard."""
    return {
        "ok": True,
        "network": PUB.get("network", "testnet"),
        "contract_id": GUARD_ID,
        "domain": "https://tesoreria.lat",
        "capabilities": [
            "request_payment", "approve", "reject", "batch_pay",
            "pay_with_withholding", "open_installment_plan", "pay_installment",
            "settle_with_credit_note", "pay_early_discounted", "split_cost_center_pay",
            "pause_vault", "unpause_vault", "withdraw_reserve",
            "schedule_payment", "execute_scheduled", "cancel_scheduled",
            "grant_daily_boost", "propose_admin", "accept_admin",
            "expire_payment", "rotate_operator", "set_supplier", "remove_supplier"
        ],
        "errors": {str(k): {"code": v[0], "message": v[1]} for k, v in CONTRACT_ERRORS.items()},
    }

# ---------- Producción en Stellar Public Mainnet (TreasuryMainnet + TreasuryMainnetOps)
MAINNET_CONTRACT_ID = "CDPDGQLXFSKWFAPKE5OL6DUWRYLEZ3SWL5ENMXEYCZ47H7Z6IHBSHRS7"
MAINNET_OPS_CONTRACT_ID = "CDBUJWHQ34RJS2WLK3RUOHIPQHHT5BCMIE3JIQZPSSDBXMFJHABQDLWC"
MAINNET_RPC_URL = "https://mainnet.sorobanrpc.com"
MAINNET_XLM_SAC = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"
MAINNET_SUPPLIER = "GACBKZS4DGVUF4RKN6ZQNP7UZTNVAXV4BXDZ72IOYR47TFC4GQUXJSYT"
MAINNET_WORKER_B = "GBVZW3KCFVO53MBHEOFYJ5AJ6JHWCJFALTAO63NAJJY7AF5WLNX7X5RB"
MAINNET_DIRECTOR = "GBVZW3KCFVO53MBHEOFYJ5AJ6JHWCJFALTAO63NAJJY7AF5WLNX7X5RB"
_mainnet_lock = threading.Lock()


def _mainnet_secret() -> str:
    sec = os.environ.get("MAINNET_OWNER_SECRET")
    if not sec:
        f_sec = HERE / "state" / "mainnet_wallets.secret.json"
        if f_sec.exists():
            sec = json.loads(f_sec.read_text("utf-8"))["mainnet_owner_sec"]
    if not sec:
        raise RuntimeError("Llave de Mainnet no configurada")
    return sec


def _mainnet_invoke(contract_id: str, fn: str, params: list, detail: str):
    t0 = time.time()
    kp = Keypair.from_secret(_mainnet_secret())
    txs = store.get("mainnet_txs", []) or []
    if len(txs) >= 25:
        raise RuntimeError("Límite de operaciones públicas de demostración alcanzado")
    with _mainnet_lock:
        c = ContractClient(contract_id, MAINNET_RPC_URL, Network.PUBLIC_NETWORK_PASSPHRASE)
        at = c.invoke(fn, params, source=kp.public_key, signer=kp, parse_result_xdr_fn=scval.to_native, base_fee=10_000)
        at.sign_and_submit()
        txh = at.built_transaction.hash_hex()
    secs = round(time.time() - t0, 1)
    entry = {"fn": fn, "contract": contract_id, "detail": detail, "hash": txh, "seconds": secs, "ts": now_iso()}
    cur = store.get("mainnet_txs", []) or []
    cur.insert(0, entry)
    store.put("mainnet_txs", cur[:30])
    return {"ok": True, "fn": fn, "hash": txh, "seconds": secs, "explorer": f"https://stellar.expert/explorer/public/tx/{txh}"}


@api.get("/mainnet")
def mainnet_page():
    return FileResponse(WEB / "mainnet.html", headers=NO_CACHE)


@api.get("/api/mainnet/state")
def mainnet_state():
    v1, v2, sp = 0.0, 0.0, 0.0
    try:
        c_sac = ContractClient(MAINNET_XLM_SAC, MAINNET_RPC_URL, Network.PUBLIC_NETWORK_PASSPHRASE)
        v1 = (c_sac.invoke("balance", [scval.to_address(MAINNET_CONTRACT_ID)], source=MAINNET_DIRECTOR, parse_result_xdr_fn=scval.to_native).result() or 0) / 1e7
        v2 = (c_sac.invoke("balance", [scval.to_address(MAINNET_OPS_CONTRACT_ID)], source=MAINNET_DIRECTOR, parse_result_xdr_fn=scval.to_native).result() or 0) / 1e7
        c_m = ContractClient(MAINNET_CONTRACT_ID, MAINNET_RPC_URL, Network.PUBLIC_NETWORK_PASSPHRASE)
        sp = (c_m.invoke("spent_today", [], source=MAINNET_DIRECTOR, parse_result_xdr_fn=scval.to_native).result() or 0) / 1e7
    except Exception:
        pass
    txs = store.get("mainnet_txs", []) or []
    return {
        "ok": True,
        "network": "Stellar Public Mainnet",
        "contract": MAINNET_CONTRACT_ID,
        "ops_contract": MAINNET_OPS_CONTRACT_ID,
        "supplier": MAINNET_SUPPLIER,
        "vault_xlm": round(v1, 5),
        "ops_vault_xlm": round(v2, 5),
        "spent_xlm": round(sp, 5),
        "next_suffix": f"27{len(txs) + 1}",
        "txs": txs,
    }


@api.post("/api/mainnet/set-supplier")
async def mainnet_set_supplier(request: Request):
    b = await args_of(request)
    sup = str(b.get("supplier") or MAINNET_SUPPLIER).strip()
    max_stroops = max(50_000, min(5_000_000, int(float(b.get("max_xlm") or 0.25) * 1e7)))
    try:
        return await run_in_threadpool(
            _mainnet_invoke, MAINNET_CONTRACT_ID, "set_supplier",
            [scval.to_address(sup), scval.to_int128(max_stroops)], f"Tope {max_stroops/1e7:.2f} XLM"
        )
    except Exception as e:
        return JSONResponse({"ok": False, "error": str(e)[:220]}, 400)


@api.post("/api/mainnet/pay")
async def mainnet_pay(request: Request):
    b = await args_of(request)
    folio = re.sub(r"[^A-Za-z0-9_]", "", str(b.get("folio") or "CFDI27A").upper())[:9] or "CFDI27A"
    amt = max(50_000, min(400_000, int(float(b.get("amount_xlm") or 0.02) * 1e7)))
    try:
        return await run_in_threadpool(
            _mainnet_invoke, MAINNET_CONTRACT_ID, "pay_invoice",
            [scval.to_address(MAINNET_SUPPLIER), scval.to_int128(amt), scval.to_symbol(folio)], f"{folio} · {amt/1e7:.2f} XLM"
        )
    except Exception as e:
        return JSONResponse({"ok": False, "error": str(e)[:220]}, 400)


@api.post("/api/mainnet/pay-tax")
async def mainnet_pay_tax(request: Request):
    b = await args_of(request)
    folio = re.sub(r"[^A-Za-z0-9_]", "", str(b.get("folio") or "RET27A").upper())[:9] or "RET27A"
    bps = max(100, min(3500, int(b.get("tax_bps") or 1000)))
    gross = 200_000  # 0.02 XLM
    try:
        return await run_in_threadpool(
            _mainnet_invoke, MAINNET_OPS_CONTRACT_ID, "pay_with_tax",
            [scval.to_address(MAINNET_SUPPLIER), scval.to_address(MAINNET_DIRECTOR), scval.to_int128(gross), scval.to_uint32(bps), scval.to_symbol(folio)],
            f"{folio} · Retención {bps/100:.1f}%"
        )
    except Exception as e:
        return JSONResponse({"ok": False, "error": str(e)[:220]}, 400)


@api.post("/api/mainnet/payroll")
async def mainnet_payroll(request: Request):
    b = await args_of(request)
    period = re.sub(r"[^A-Za-z0-9_]", "", str(b.get("period") or "QUIN22A").upper())[:9] or "QUIN22A"
    each = max(50_000, min(200_000, int(float(b.get("amount_xlm") or 0.01) * 1e7)))
    try:
        return await run_in_threadpool(
            _mainnet_invoke, MAINNET_OPS_CONTRACT_ID, "batch_payroll",
            [scval.to_address(MAINNET_SUPPLIER), scval.to_address(MAINNET_WORKER_B), scval.to_int128(each), scval.to_symbol(period)],
            f"{period} · 2 pagos"
        )
    except Exception as e:
        return JSONResponse({"ok": False, "error": str(e)[:220]}, 400)


@api.post("/api/mainnet/sweep")
async def mainnet_sweep(request: Request):
    b = await args_of(request)
    dest = str(b.get("dest") or MAINNET_DIRECTOR).strip()
    amt = max(50_000, min(300_000, int(float(b.get("amount_xlm") or 0.02) * 1e7)))
    try:
        return await run_in_threadpool(
            _mainnet_invoke, MAINNET_OPS_CONTRACT_ID, "sweep_reserve",
            [scval.to_address(dest), scval.to_int128(amt)], f"Barrido {amt/1e7:.2f} XLM"
        )
    except Exception as e:
        return JSONResponse({"ok": False, "error": str(e)[:220]}, 400)
